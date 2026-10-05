// planr.so task-graph scene, around the closing band: the sample's tasks as board rows floating in
// depth beside the band's text, joined by their `depends` edges. Every few seconds the next recorded change plays: a status mark
// turns, ink runs down the edge to the task it unlocks and that row takes the board's related wash.
// The changes come from the real engine at build time (src/scene.mjs); this file only draws them.
//
// Raw WebGL2, no dependencies. Color only on status marks, as on the board. The canvas never takes
// focus or pointer events. It stays off without WebGL2, on narrow screens and while hidden or
// off-screen, starts only when the band comes near, and draws one still frame under reduced motion.

const data = JSON.parse(document.getElementById('graph-data')?.textContent ?? 'null');
const band = document.querySelector('.closing');
const STATUS = ['open', 'in-progress', 'review', 'done', 'blocked', 'decision'];
const ROW = { w: 236, h: 32 };           // a board task row, in CSS px at depth 1
const ATLAS = { w: 512, h: 64 };         // one row of text per atlas cell, at 2x
const STEP = 2600;                        // ms between changes
const HOLD = 3200;                        // ms the last state stays before the scene resets

const CARD_VS = `#version 300 es
layout(location=0) in vec4 aRect;   // center x, y, width, height (px)
layout(location=1) in vec4 aLook;   // blur 0..1, alpha, atlas row, wash
layout(location=2) in vec4 aMark;   // status from, status to, mix, unused
uniform vec2 uRes;
out vec2 vLocal; out vec2 vUV; out vec4 vLook; out vec4 vMark; out vec2 vPos;
void main() {
  vec2 c = vec2(gl_VertexID & 1, gl_VertexID >> 1);
  vec2 pad = vec2(8.0);
  vec2 px = aRect.xy + (c - 0.5) * (aRect.zw + pad * 2.0);
  vLocal = (c * (aRect.zw + pad * 2.0) - pad) / aRect.zw * vec2(236.0, 32.0);
  vUV = vec2(vLocal.x / 256.0, (aLook.z + vLocal.y / 32.0));
  vLook = aLook; vMark = aMark; vPos = px;
  gl_Position = vec4(px / uRes * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
}`;

const CARD_FS = `#version 300 es
precision highp float;
in vec2 vLocal; in vec2 vUV; in vec4 vLook; in vec4 vMark; in vec2 vPos;
uniform sampler2D uAtlas; uniform float uRows;
uniform vec3 uPanel, uLine, uId, uTitle, uSoft, uInk; uniform vec3 uS[6]; uniform vec4 uClear;
out vec4 o;
float box(vec2 p, vec2 b, float r) { vec2 q = abs(p) - b + r; return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r; }
float seg(vec2 p, vec2 a, vec2 b) { vec2 pa = p - a, ba = b - a; return length(pa - ba * clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0)); }
// One status mark on the board's 12x12 grid, centered: x = shape, y = ink cut-out (signed distances).
vec2 mark(int s, vec2 q) {
  float ring = abs(length(q) - 4.6) - 0.75, disc = length(q) - 5.4;
  if (s == 0) return vec2(ring, 9.0);
  if (s == 1) return vec2(min(ring, max(length(q) - 3.4, -q.x)), 9.0);
  if (s == 2) return vec2(min(ring, length(q) - 2.0), 9.0);
  if (s == 3) return vec2(disc, min(seg(q, vec2(-2.4, 0.1), vec2(-0.8, 1.7)), seg(q, vec2(-0.8, 1.7), vec2(2.4, -1.6))) - 0.75);
  if (s == 4) return vec2(disc, box(q, vec2(2.7, 0.75), 0.75));
  vec2 r = vec2(q.x + q.y, q.y - q.x) * 0.70710678;
  return vec2(box(r, vec2(3.9), 1.6), min(box(q - vec2(0.0, -1.05), vec2(0.7, 1.65), 0.7), length(q - vec2(0.0, 2.2)) - 0.8));
}
vec4 markColor(int s, vec2 q, float aa) {
  vec2 d = mark(s, q);
  float shape = 1.0 - smoothstep(-aa, aa, d.x);
  float cut = 1.0 - smoothstep(-aa, aa, d.y);
  return vec4(mix(uS[s], uInk, cut * step(2.5, float(s))), shape);
}
void main() {
  float blur = vLook.x, aa = fwidth(vLocal.x) * 0.9 + blur * 2.6;
  float d = box(vLocal - vec2(118.0, 16.0), vec2(118.0, 16.0), 6.0);
  float fill = 1.0 - smoothstep(-aa, aa, d);
  float edge = 1.0 - smoothstep(0.0, aa + 0.6, abs(d + 0.5));
  vec3 col = mix(uPanel, uSoft, vLook.w);
  col = mix(col, uLine, edge * 0.9);
  vec2 q = vLocal - vec2(16.0, 16.0);
  vec4 m0 = markColor(int(vMark.x), q, aa), m1 = markColor(int(vMark.y), q, aa);
  float ma = mix(m0.a, m1.a, vMark.z);
  vec3 mc = mix(m0.rgb * m0.a, m1.rgb * m1.a, vMark.z) / max(ma, 0.001);
  col = mix(col, mc, ma);
  vec2 t = texture(uAtlas, vec2(vUV.x, vUV.y / uRows), blur * 2.6).rg;
  col = mix(col, uId, t.r * (1.0 - blur * 0.5));
  col = mix(col, uTitle, t.g * (1.0 - blur * 0.5));
  // Keep the headline clear: rows fade out inside the ellipse around it.
  float e = length((vPos - uClear.xy) / uClear.zw);
  float a = max(fill, edge) * vLook.y * smoothstep(0.92, 1.08, e);
  o = vec4(col * a, a);
}`;

const EDGE_VS = `#version 300 es
layout(location=0) in vec4 aSeg;    // x1, y1, x2, y2 (px)
layout(location=1) in vec4 aInfo;   // blur, alpha, pulse position, pulse strength
uniform vec2 uRes;
out vec2 vP; out vec4 vSeg; out vec4 vInfo;
void main() {
  vec2 c = vec2(gl_VertexID & 1, gl_VertexID >> 1);
  vec2 a = aSeg.xy, b = aSeg.zw, dir = normalize(b - a), n = vec2(-dir.y, dir.x);
  float w = 6.0 + aInfo.x * 4.0;
  vec2 px = mix(a - dir * w, b + dir * w, c.x) + n * (c.y - 0.5) * 2.0 * w;
  vP = px; vSeg = aSeg; vInfo = aInfo;
  gl_Position = vec4(px / uRes * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
}`;

const EDGE_FS = `#version 300 es
precision highp float;
in vec2 vP; in vec4 vSeg; in vec4 vInfo;
uniform vec3 uLine, uFg; uniform vec4 uClear;
out vec4 o;
void main() {
  vec2 pa = vP - vSeg.xy, ba = vSeg.zw - vSeg.xy;
  float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
  float d = length(pa - ba * h);
  float aa = 0.7 + vInfo.x * 2.0;
  float line = 1.0 - smoothstep(0.5 - aa, 0.5 + aa, d);
  float pulse = exp(-pow((h - vInfo.z) * 9.0, 2.0)) * vInfo.w;
  float dot_ = (1.0 - smoothstep(1.2, 1.2 + aa * 1.5, d)) * pulse;
  vec3 col = mix(uLine, uFg, clamp(pulse * 1.4, 0.0, 1.0));
  float e = length((vP - uClear.xy) / uClear.zw);
  float a = max(line * vInfo.y, dot_) * smoothstep(0.92, 1.08, e);
  o = vec4(col * a, a);
}`;

const ease = t => 1 - Math.pow(1 - Math.min(1, Math.max(0, t)), 3);
const hex = v => {
  const m = v.trim().match(/^#([\da-f]{6})$/i);
  return m ? [0, 2, 4].map(i => parseInt(m[1].slice(i, i + 2), 16) / 255) : [0.5, 0.5, 0.5];
};

function compile(gl, vs, fs) {
  const program = gl.createProgram();
  for (const [type, src] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]]) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, src);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      throw Error(gl.getShaderInfoLog(shader));
    }
    gl.attachShader(program, shader);
  }
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw Error(gl.getProgramInfoLog(program));
  }
  return program;
}

// Task IDs and titles in the site's own faces, packed as masks: red is the ID, green the title.
async function atlas(cards) {
  await Promise.all(['500 23px "Monaspace Neon"', '400 26px "Mona Sans"'].map(f => document.fonts.load(f)));
  const canvas = document.createElement('canvas');
  canvas.width = ATLAS.w;
  canvas.height = ATLAS.h * cards.length;
  const ctx = canvas.getContext('2d');
  ctx.textBaseline = 'middle';
  ctx.globalCompositeOperation = 'lighter';
  cards.forEach((card, i) => {
    const y = i * ATLAS.h + 33;
    ctx.font = '500 23px "Monaspace Neon"';
    ctx.fillStyle = '#f00';
    ctx.fillText(card.id, 64, y);
    const x = 64 + ctx.measureText(card.id).width + 20;
    ctx.font = '400 26px "Mona Sans"';
    ctx.fillStyle = '#0f0';
    const fits = text => x + ctx.measureText(text).width <= (ROW.w - 12) * 2;
    let title = card.title;
    if (!fits(title)) {
      while (title.length > 1 && !fits(`${title}…`)) {
        title = title.slice(0, -1).trimEnd();
      }
      title += '…';
    }
    ctx.fillText(title, x, y);
  });
  return canvas;
}

async function start() {
  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl2', {
    alpha: true, premultipliedAlpha: true, antialias: false, depth: false, stencil: false,
    powerPreference: 'low-power',
  });
  if (!gl) {
    return;
  }
  const layer = document.createElement('div');
  layer.className = 'scene graph';
  layer.setAttribute('aria-hidden', 'true');
  layer.append(canvas);

  const cards = data.cards.map((c, i) => ({
    ...c, row: i, from: STATUS.indexOf(c.status), to: STATUS.indexOf(c.status), mix: 1, wash: 0,
    phase: i * 1.7, lift: 0, x: 0, y: 0, w: 0, h: 0, rect: null,
  }));
  const byId = new Map(cards.map(c => [c.id, c]));
  const edges = data.edges.map(([a, b]) => ({ a: byId.get(a), b: byId.get(b), pulse: 0, age: 9 }));

  const cardProgram = compile(gl, CARD_VS, CARD_FS);
  const edgeProgram = compile(gl, EDGE_VS, EDGE_FS);
  const texture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, await atlas(cards));
  gl.generateMipmap(gl.TEXTURE_2D);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

  const instanced = (program, sizes) => {
    const vao = gl.createVertexArray();
    const buffer = gl.createBuffer();
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    const stride = sizes.reduce((a, b) => a + b, 0) * 4;
    let offset = 0;
    sizes.forEach((size, loc) => {
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, size, gl.FLOAT, false, stride, offset);
      gl.vertexAttribDivisor(loc, 1);
      offset += size * 4;
    });
    gl.bindVertexArray(null);
    return { program, vao, buffer, floats: stride / 4 };
  };
  const cardDraw = instanced(cardProgram, [4, 4, 4]);
  const edgeDraw = instanced(edgeProgram, [4, 4]);
  const cardData = new Float32Array(cards.length * cardDraw.floats);
  const edgeData = new Float32Array(edges.length * edgeDraw.floats);
  const uniform = (program, name) => gl.getUniformLocation(program, name);

  const reduce = matchMedia('(prefers-reduced-motion: reduce)');
  let width = 0, height = 0, dpr = 1, clear = [0, 0, 1, 1], wings = 0, enabled = false;
  const pointer = { x: 0, y: 0, tx: 0, ty: 0, px: -1, py: -1 };

  function colors() {
    const css = getComputedStyle(document.documentElement);
    const set = (program, pairs) => {
      gl.useProgram(program);
      for (const [u, name] of pairs) {
        gl.uniform3fv(uniform(program, u), hex(css.getPropertyValue(name)));
      }
    };
    set(cardProgram, [['uPanel', '--panel'], ['uLine', '--line-2'], ['uId', '--fg-2'], ['uTitle', '--fg-3'],
      ['uSoft', '--sel-soft'], ['uInk', '--mark-ink']]);
    gl.uniform3fv(uniform(cardProgram, 'uS'), ['--s-open', '--s-wip', '--s-review', '--s-done', '--s-blocked',
      '--s-decision'].flatMap(name => hex(css.getPropertyValue(name))));
    set(edgeProgram, [['uLine', '--line-2'], ['uFg', '--fg']]);
  }

  // The layer covers the band, full width; the clear area is an ellipse around the band's text.
  function measure() {
    const box = band.getBoundingClientRect();
    const top = box.top + scrollY;
    const parts = [...band.querySelectorAll('.center > *')].map(e => e.getBoundingClientRect());
    const l = Math.min(...parts.map(r => r.left)), r = Math.max(...parts.map(r => r.right));
    const t = Math.min(...parts.map(r => r.top)) + scrollY - top, b = Math.max(...parts.map(r => r.bottom)) + scrollY - top;
    width = document.documentElement.clientWidth;
    height = Math.max(200, box.height);
    layer.style.top = `${top}px`;
    layer.style.height = `${height}px`;
    clear = [(l + r) / 2, (t + b) / 2, (r - l) / 2 + 36, (b - t) / 2 + 40];
    wings = clear[0] - clear[2];
    dpr = Math.min(devicePixelRatio || 1, 1.5);
    const w = Math.round(width * dpr), h = Math.round(height * dpr);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    enabled = wings >= 150;
    layer.hidden = !enabled;
  }

  // Story: the recorded changes, one every STEP ms, then a hold and a quiet reset.
  const initial = cards.map(c => c.to);
  let clock = 0, stepAt = 1400, next = 0;
  function play(dt) {
    clock += dt;
    if (clock < stepAt) {
      return;
    }
    if (next < data.steps.length) {
      const step = data.steps[next++];
      const card = byId.get(step.id);
      if (card) {
        card.from = card.to;
        card.to = STATUS.indexOf(step.to);
        card.mix = 0;
      }
      for (const id of step.unlocked) {
        const edge = edges.find(e => e.a.id === step.id && e.b.id === id);
        if (edge) {
          edge.age = 0;
        }
      }
      stepAt = clock + (next < data.steps.length ? STEP : HOLD);
    } else {
      cards.forEach((c, i) => {
        c.from = c.to;
        c.to = initial[i];
        c.mix = 0;
      });
      next = 0;
      stepAt = clock + STEP;
    }
  }

  function layout(t, still) {
    const scale = Math.min(1, Math.max(0.72, wings / 360));
    for (const c of cards) {
      const s = scale / c.z;
      c.w = ROW.w * s;
      c.h = ROW.h * s;
      // u runs from the clear area to the page edge; every row stays whole on the page.
      const sign = c.side === 'l' ? -1 : 1;
      const inner = clear[0] + sign * (clear[2] + c.w * 0.55);
      const outer = c.side === 'l' ? c.w * 0.5 + 16 : width - c.w * 0.5 - 16;
      let x = sign * (outer - inner) > 0 ? inner + (outer - inner) * c.u : inner;
      let y = 18 + c.v * (height - 70);
      if (!still) {
        x += Math.cos(t * 0.00021 + c.phase) * 3 / c.z + pointer.x * 16 / c.z;
        y += Math.sin(t * 0.00027 + c.phase) * 5 / c.z + pointer.y * 10 / c.z;
      }
      c.x = x;
      c.y = y;
      const over = Math.abs(pointer.px - x) < c.w / 2 && Math.abs(pointer.py - y) < c.h / 2;
      c.lift += ((over && !still ? 1 : 0) - c.lift) * 0.12;
    }
  }

  function draw(t, dt, still) {
    layout(t, still);
    const order = [...cards].sort((a, b) => (b.z - b.lift * 0.4) - (a.z - a.lift * 0.4));
    order.forEach((c, i) => {
      if (!still) {
        c.mix = Math.min(1, c.mix + dt / 520);
        c.wash = Math.max(0, c.wash - dt / 2400);
      }
      const near = (2.4 - c.z) / 1.4;
      const blur = Math.max(0, Math.min(1, (c.z - 1.05) / 1.25)) * (1 - c.lift);
      const grow = 1 + c.lift * 0.06;
      cardData.set([c.x, c.y, c.w * grow, c.h * grow, blur, (0.5 + near * 0.5) * (0.78 + c.lift * 0.22),
        c.row, c.wash, c.from, c.to, ease(c.mix), 0], i * cardDraw.floats);
    });
    edges.forEach((e, i) => {
      if (!still) {
        e.age += dt / 1000;
        if (e.age > 0.9 && e.age - dt / 1000 <= 0.9) {
          e.b.wash = 1;
        }
      }
      const z = (e.a.z + e.b.z) / 2;
      const pulse = e.age < 1.2 ? ease(e.age / 0.9) : 0;
      const strength = e.age < 1.2 ? Math.sin(Math.min(1, e.age / 1.2) * Math.PI) : 0;
      edgeData.set([e.a.x, e.a.y + e.a.h / 2, e.b.x, e.b.y - e.b.h / 2,
        Math.max(0, Math.min(1, (z - 1.05) / 1.25)), 0.55 + (2.4 - z) * 0.2, pulse, strength], i * edgeDraw.floats);
    });
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    for (const [d, buffer, count] of [[edgeDraw, edgeData, edges.length], [cardDraw, cardData, cards.length]]) {
      gl.useProgram(d.program);
      gl.uniform2f(uniform(d.program, 'uRes'), width, height);
      gl.uniform4fv(uniform(d.program, 'uClear'), clear);
      if (d === cardDraw) {
        gl.uniform1f(uniform(d.program, 'uRows'), cards.length);
        gl.uniform1i(uniform(d.program, 'uAtlas'), 0);
      }
      gl.bindVertexArray(d.vao);
      gl.bindBuffer(gl.ARRAY_BUFFER, d.buffer);
      gl.bufferData(gl.ARRAY_BUFFER, buffer, gl.DYNAMIC_DRAW);
      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, count);
    }
  }

  // Run only while the band is on screen, the tab is visible and motion is welcome.
  let frame = 0, last = 0, visible = true;
  function tick(now) {
    frame = 0;
    const dt = Math.min(50, last ? now - last : 16);
    last = now;
    pointer.x += (pointer.tx - pointer.x) * 0.05;
    pointer.y += (pointer.ty - pointer.y) * 0.05;
    play(dt);
    draw(now, dt, false);
    run();
  }
  function run() {
    const go = enabled && visible && !document.hidden && !reduce.matches;
    if (go && !frame) {
      frame = requestAnimationFrame(tick);
    } else if (!go && frame) {
      cancelAnimationFrame(frame);
      frame = 0;
      last = 0;
    }
  }
  function still() {
    if (enabled) {
      draw(0, 0, true);
    }
  }
  function refresh() {
    measure();
    if (reduce.matches) {
      still();
    }
    run();
  }

  document.body.prepend(layer);
  measure();
  colors();
  new ResizeObserver(refresh).observe(document.documentElement);
  new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting;
    run();
  }).observe(layer);
  document.addEventListener('visibilitychange', run);
  reduce.addEventListener('change', refresh);
  const recolor = () => {
    colors();
    if (reduce.matches || !frame) {
      still();
    }
  };
  new MutationObserver(recolor).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', recolor);
  addEventListener('pointermove', e => {
    const r = layer.getBoundingClientRect();
    pointer.tx = (e.clientX / innerWidth - 0.5) * 2;
    pointer.ty = (e.clientY / innerHeight - 0.5) * 2;
    pointer.px = e.clientX - r.left;
    pointer.py = e.clientY - r.top;
  }, { passive: true });
  canvas.addEventListener('webglcontextlost', () => {
    enabled = false;
    run();
    layer.remove();
  });
  if (reduce.matches) {
    still();
  }
  run();
  requestAnimationFrame(() => layer.classList.add('on'));
}

// Narrow screens have no room beside the band's text: the scene never starts there, so phones skip
// the WebGL context and the text atlas entirely. It starts once the band is within a screen of view.
const wide = matchMedia('(min-width: 980px)');
if (data && band) {
  let started = false;
  const boot = () => {
    if (started || !wide.matches) {
      return;
    }
    started = true;
    start().catch(() => document.querySelector('.scene.graph')?.remove());
  };
  new IntersectionObserver((entries, observer) => {
    if (entries.some(e => e.isIntersecting)) {
      observer.disconnect();
      boot();
      wide.addEventListener('change', boot);
    }
  }, { rootMargin: '100% 0px' }).observe(band);
}
