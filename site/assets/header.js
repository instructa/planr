// planr.so hero field: a halftone made of the board's status marks. Every cell of a fine grid holds
// one mark, and a slow field decides its state: nothing, then an open ring, then a ring that fills from
// the right (in progress) until it closes into a disc (done). Dense regions read as finished work,
// thin ones as open work, and the field drifts to the right, the way a plan moves forward.
// Ink only, no status color, no text. It clears softly around the headline, lead, install box and
// links; on wide screens a fine pointer moves nearby marks a step forward.
//
// Raw WebGL2, no dependencies, two passes: the field is computed once per cell into a small texture,
// then each pixel only draws its cell's mark. The canvas never takes focus or pointer events. Phones
// get a lighter version: smaller cells, a fainter field that stays readable behind the text, no
// pointer. It pauses off-screen or in a hidden tab, caps the pixel ratio and draws one still frame
// under prefers-reduced-motion.
//
// Below the hero the same marks continue on the landing (margins(), further down): one fixed canvas
// behind the content, quiet & ink-only, redrawn only when the page scrolls or changes size.

const hero = document.querySelector('.hero');

const CELL = 18;            // CSS px per mark on wide screens
const CELL_NARROW = 14;     // CSS px per mark on phones
const NARROW = 640;         // below this width the light version runs
const MAX_DPR = 1.5;
const SCALE = 540;          // CSS px per unit of the noise field (stretched 1.6× along x into streams)
const FEATHER = 150;        // CSS px over which the field returns around the content
const FEATHER_NARROW = 64;
const INTRO = 2600;         // ms for the field to fill in after load
const STILL_AT = 37;        // s: the moment the reduced-motion frame shows

const QUAD_VS = `#version 300 es
void main() {
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

// Pass 1, one fragment per cell: r = progress 0..1, g = strength 0..1 after the clearing and fades.
const FIELD_FS = `#version 300 es
precision highp float;
uniform vec2 uOff;          // grid origin in canvas px
uniform float uCell;        // cell size in canvas px
uniform float uScale;       // canvas px per noise unit
uniform float uTime;        // s
uniform float uIntro;       // 0..1 entrance
uniform vec2 uSize;         // canvas px
uniform vec4 uClear[4];     // content boxes in canvas px: left, top, right, bottom
uniform int uClearCount;
uniform vec3 uShape;        // feather px, strength kept inside the clearing, bottom fade px
uniform vec4 uPointer;      // x, y, radius (canvas px), strength 0..1
out vec4 o;

vec3 permute(vec3 x) { return mod((x * 34.0 + 1.0) * x, 289.0); }

// 2D simplex noise (Ashima Arts, Stefan Gustavson; MIT).
float snoise(vec2 v) {
  const vec4 C = vec4(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
  vec2 i = floor(v + dot(v, C.yy));
  vec2 x0 = v - i + dot(i, C.xx);
  vec2 i1 = x0.x > x0.y ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
  vec4 x12 = x0.xyxy + C.xxzz;
  x12.xy -= i1;
  i = mod(i, 289.0);
  vec3 p = permute(permute(i.y + vec3(0.0, i1.y, 1.0)) + i.x + vec3(0.0, i1.x, 1.0));
  vec3 m = max(0.5 - vec3(dot(x0, x0), dot(x12.xy, x12.xy), dot(x12.zw, x12.zw)), 0.0);
  m = m * m;
  m = m * m;
  vec3 x = 2.0 * fract(p * C.www) - 1.0;
  vec3 h = abs(x) - 0.5;
  vec3 a0 = x - floor(x + 0.5);
  m *= 1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h);
  vec3 g = vec3(a0.x * x0.x + h.x * x0.y, a0.yz * x12.xz + h.yz * x12.yw);
  return 130.0 * dot(m, g);
}

void main() {
  vec2 c = uOff + (floor(gl_FragCoord.xy) + 0.5) * uCell;
  vec2 q = c / (uScale * vec2(1.6, 1.0));
  float t = uTime;
  vec2 warp = vec2(snoise(q * 0.7 + vec2(0.0, t * 0.02)), snoise(q * 0.7 + vec2(5.2, 1.3) - vec2(t * 0.018, 0.0)));
  float n = snoise(q + warp * 0.85 - vec2(t * 0.03, 0.0));
  float v = smoothstep(0.0, 0.85, n);

  float clear = 1.0;
  for (int i = 0; i < 4; i++) {
    if (i >= uClearCount) {
      break;
    }
    vec4 b = uClear[i];
    vec2 d = abs(c - (b.xy + b.zw) * 0.5) - (b.zw - b.xy) * 0.5;
    float dist = length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
    clear = min(clear, smoothstep(0.0, uShape.x, dist));
  }
  float m = mix(uShape.y, 1.0, clear);
  m *= smoothstep(0.0, uCell * 2.5, c.y) * (1.0 - smoothstep(uSize.y - uShape.z, uSize.y, c.y));
  // Toward the edges of the page the field grows a little denser, so it frames the hero.
  v += 0.12 * smoothstep(0.2, 1.0, abs(c.x / uSize.x - 0.5) * 2.0);
  v += uPointer.w * 0.42 * (1.0 - smoothstep(0.0, uPointer.z, length(c - uPointer.xy)));
  v = clamp(v - (1.0 - uIntro) * 1.15, 0.0, 1.0);
  o = vec4(v, m, 0.0, 1.0);
}`;

// Pass 2, every pixel: the mark of its cell. Progress grows a ring, then fills it from the right
// (half at 0.5, the in-progress mark) until the gap closes into a disc.
const MARK_FS = `#version 300 es
precision highp float;
uniform sampler2D uField;
uniform vec2 uOff;
uniform float uCell;
uniform float uHeight;
uniform vec3 uInk;
uniform float uAlpha;
out vec4 o;
void main() {
  vec2 p = vec2(gl_FragCoord.x, uHeight - gl_FragCoord.y) - uOff;
  ivec2 ij = ivec2(floor(p / uCell));
  ivec2 size = textureSize(uField, 0);
  if (any(lessThan(ij, ivec2(0))) || any(greaterThanEqual(ij, size))) {
    discard;
  }
  vec2 f = texelFetch(uField, ij, 0).rg;
  if (f.r < 0.06 || f.g < 0.004) {
    discard;
  }
  vec2 q = p - (vec2(ij) + 0.5) * uCell;
  float d = length(q);
  float grow = smoothstep(0.06, 0.34, f.r);
  float w = max(uCell * 0.08, 1.0);
  float r = uCell * mix(0.15, 0.33, grow);
  float ring = (1.0 - smoothstep(w * 0.5 - 0.5, w * 0.5 + 0.5, abs(d - r))) * grow * 0.8;
  float k = smoothstep(0.42, 0.95, f.r);
  float gap = w * 0.85 * (1.0 - smoothstep(0.8, 1.0, k));
  float ri = r - w * 0.5 - gap;
  float disc = 1.0 - smoothstep(ri - 0.5, ri + 0.5, d);
  float edge = ri * (1.0 - 2.0 * k);
  float fill = disc * smoothstep(edge - 0.5, edge + 0.5, q.x) * step(0.002, k);
  float a = max(ring, fill) * f.g * uAlpha;
  o = vec4(uInk * a, a);
}`;

// The field below the hero, one fragment per cell of the visible window. The grid is fixed to the
// page, so marks scroll with the content. Marks sit in the margins, very faint behind the content
// column & a little stronger in a halo around each schematic. Further down the page they are denser
// & fuller, and each fills a little more as it passes the reading line in the upper window.
const MARGIN_FS = `#version 300 es
precision highp float;
uniform vec2 uOff;          // grid origin in canvas px; y follows the scroll
uniform float uCell;
uniform float uScale;
uniform float uScroll;      // page offset of the window in canvas px
uniform vec2 uSpan;         // page y where the field starts & ends, canvas px
uniform vec2 uColumn;       // content column left & right, canvas px
uniform vec2 uView;         // window size, canvas px
uniform vec4 uHalo[6];      // schematics in the window: left, top, right, bottom
uniform int uHaloCount;
uniform vec4 uShape;        // margin feather, strength behind content, halo strength, halo reach
uniform float uRead;        // 1 = marks fill as they pass the reading line, 0 = still
out vec4 o;

vec3 permute(vec3 x) { return mod((x * 34.0 + 1.0) * x, 289.0); }

// 2D simplex noise (Ashima Arts, Stefan Gustavson; MIT).
float snoise(vec2 v) {
  const vec4 C = vec4(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
  vec2 i = floor(v + dot(v, C.yy));
  vec2 x0 = v - i + dot(i, C.xx);
  vec2 i1 = x0.x > x0.y ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
  vec4 x12 = x0.xyxy + C.xxzz;
  x12.xy -= i1;
  i = mod(i, 289.0);
  vec3 p = permute(permute(i.y + vec3(0.0, i1.y, 1.0)) + i.x + vec3(0.0, i1.x, 1.0));
  vec3 m = max(0.5 - vec3(dot(x0, x0), dot(x12.xy, x12.xy), dot(x12.zw, x12.zw)), 0.0);
  m = m * m;
  m = m * m;
  vec3 x = 2.0 * fract(p * C.www) - 1.0;
  vec3 h = abs(x) - 0.5;
  vec3 a0 = x - floor(x + 0.5);
  m *= 1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h);
  vec3 g = vec3(a0.x * x0.x + h.x * x0.y, a0.yz * x12.xz + h.yz * x12.yw);
  return 130.0 * dot(m, g);
}

void main() {
  vec2 c = uOff + (floor(gl_FragCoord.xy) + 0.5) * uCell;
  float y = c.y + uScroll;
  vec2 q = vec2(c.x, y) / (uScale * vec2(1.6, 1.0));
  vec2 warp = vec2(snoise(q * 0.7), snoise(q * 0.7 + vec2(5.2, 1.3)));
  float n = snoise(q + warp * 0.85);

  float page = clamp((y - uSpan.x) / max(uSpan.y - uSpan.x, 1.0), 0.0, 1.0);
  float read = uRead * (1.0 - smoothstep(0.2, 0.65, c.y / uView.y));
  float v = smoothstep(mix(0.05, -0.5, page), mix(0.9, 0.35, page), n);
  v = min(v * mix(0.55, 1.0, page) + 0.18 * read * step(0.06, v), 1.0);

  float outside = max(uColumn.x - c.x, c.x - uColumn.y);
  float m = mix(uShape.y, 1.0, smoothstep(0.0, uShape.x, outside));
  for (int i = 0; i < 6; i++) {
    if (i >= uHaloCount) {
      break;
    }
    vec4 b = uHalo[i];
    vec2 d = (abs(c - (b.xy + b.zw) * 0.5) - (b.zw - b.xy) * 0.5) * vec2(1.0, 1.8);
    float dist = length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
    if (dist > 0.0) {
      m = max(m, uShape.z * (1.0 - smoothstep(0.0, uShape.w, dist)));
    }
  }
  m *= smoothstep(uSpan.x - uShape.w, uSpan.x + uShape.w * 3.0, y) * (1.0 - smoothstep(uSpan.y - uShape.w * 4.0, uSpan.y, y));
  o = vec4(v, m, 0.0, 1.0);
}`;

const hex = v => {
  const m = v.trim().match(/^#([\da-f]{6})$/i);
  return m ? [0, 2, 4].map(i => parseInt(m[1].slice(i, i + 2), 16) / 255) : [0.5, 0.5, 0.5];
};

function program(gl, vs, fs) {
  const p = gl.createProgram();
  for (const [type, src] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]]) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, src);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      throw Error(gl.getShaderInfoLog(shader));
    }
    gl.attachShader(p, shader);
  }
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
    throw Error(gl.getProgramInfoLog(p));
  }
  const u = {};
  for (let i = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS); i--;) {
    const name = gl.getActiveUniform(p, i).name.replace(/\[0\]$/, '');
    u[name] = gl.getUniformLocation(p, name);
  }
  return { p, u };
}

// A layer for one field: its canvas & context, the field pass (`fs`) & the shared mark pass.
function surface(name, fs) {
  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl2', {
    alpha: true, premultipliedAlpha: true, antialias: false, depth: false, stencil: false,
    powerPreference: 'low-power',
  });
  if (!gl) {
    return null;
  }
  const layer = document.createElement('div');
  layer.className = `scene ${name}`;
  layer.setAttribute('aria-hidden', 'true');
  layer.append(canvas);
  const texture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, texture);
  for (const [k, v] of [[gl.TEXTURE_MIN_FILTER, gl.NEAREST], [gl.TEXTURE_MAG_FILTER, gl.NEAREST],
    [gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE], [gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE]]) {
    gl.texParameteri(gl.TEXTURE_2D, k, v);
  }
  return {
    canvas, gl, layer, texture,
    field: program(gl, QUAD_VS, fs),
    marks: program(gl, QUAD_VS, MARK_FS),
    vao: gl.createVertexArray(),
    fbo: gl.createFramebuffer(),
    // Sizes the field texture to the grid.
    grid(cols, rows) {
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, cols, rows, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbo);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    },
    // Pass 2: every pixel draws its cell's mark from the field texture.
    paint(off, cell, ink, alpha) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
      const u = this.marks.u;
      gl.useProgram(this.marks.p);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.uniform1i(u.uField, 0);
      gl.uniform2f(u.uOff, off[0], off[1]);
      gl.uniform1f(u.uCell, cell);
      gl.uniform1f(u.uHeight, canvas.height);
      gl.uniform3fv(u.uInk, ink);
      gl.uniform1f(u.uAlpha, alpha);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    },
  };
}

// The page's ink & whether it is dark, read from the theme tokens.
function theme() {
  const css = getComputedStyle(document.documentElement);
  const bg = hex(css.getPropertyValue('--bg'));
  return { ink: hex(css.getPropertyValue('--fg')), dark: bg[0] * 0.2126 + bg[1] * 0.7152 + bg[2] * 0.0722 < 0.5 };
}

function start() {
  const s = surface('header', FIELD_FS);
  if (!s) {
    return;
  }
  const { canvas, gl, layer, field, vao, fbo } = s;

  const reduce = matchMedia('(prefers-reduced-motion: reduce)');
  const fine = matchMedia('(hover: hover) and (pointer: fine)');
  let width = 0, height = 0, top = 0, dpr = 1, cell = CELL, cols = 0, rows = 0, off = [0, 0];
  let narrow = false, boxes = new Float32Array(16), count = 0, ink = [0.5, 0.5, 0.5], alpha = 0.15;

  function colors() {
    const t = theme();
    ink = t.ink;
    alpha = (t.dark ? 0.14 : 0.11) * (narrow ? 0.8 : 1);
  }

  function measure() {
    const layerBox = layer.getBoundingClientRect();
    top = layerBox.top + scrollY;
    width = document.documentElement.clientWidth;
    height = Math.max(240, hero.getBoundingClientRect().bottom + scrollY - top + 40);
    layer.style.height = `${height}px`;
    narrow = width < NARROW;
    dpr = Math.min(devicePixelRatio || 1, MAX_DPR);
    const w = Math.round(width * dpr), h = Math.round(height * dpr);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    cell = (narrow ? CELL_NARROW : CELL) * dpr;
    const c = Math.ceil(w / cell) + 1, r = Math.ceil(h / cell) + 1;
    if (c !== cols || r !== rows) {
      cols = c;
      rows = r;
      s.grid(cols, rows);
    }
    // The grid is centred, so the page's middle line runs between two columns.
    off = [(w - cols * cell) / 2, 0];
    const pad = narrow ? 4 : 10;
    const parts = [...hero.querySelectorAll('h1, .lead, .install, .hlinks')].slice(0, 4);
    boxes.fill(0);
    parts.forEach((e, i) => {
      const b = e.getBoundingClientRect();
      boxes.set([b.left - pad - layerBox.left, b.top - pad - layerBox.top, b.right + pad - layerBox.left,
        b.bottom + pad - layerBox.top].map(v => v * dpr), i * 4);
    });
    count = parts.length;
    colors();
  }

  // The pointer eases toward where it is; its pull fades in and out instead of switching.
  const pointer = { x: 0, y: 0, tx: 0, ty: 0, s: 0, ts: 0 };
  function move(e) {
    pointer.tx = e.pageX * dpr;
    pointer.ty = (e.pageY - top) * dpr;
    pointer.ts = e.pageY - top < height ? 1 : 0;
  }
  const leave = () => {
    pointer.ts = 0;
  };

  function draw(time, intro) {
    gl.bindVertexArray(vao);
    gl.disable(gl.BLEND);
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.viewport(0, 0, cols, rows);
    gl.useProgram(field.p);
    gl.uniform2f(field.u.uOff, off[0], off[1]);
    gl.uniform1f(field.u.uCell, cell);
    gl.uniform1f(field.u.uScale, SCALE * dpr);
    gl.uniform1f(field.u.uTime, time);
    gl.uniform1f(field.u.uIntro, intro);
    gl.uniform2f(field.u.uSize, canvas.width, canvas.height);
    gl.uniform4fv(field.u.uClear, boxes);
    gl.uniform1i(field.u.uClearCount, count);
    gl.uniform3f(field.u.uShape, (narrow ? FEATHER_NARROW : FEATHER) * dpr, narrow ? 0.2 : 0.04, 120 * dpr);
    gl.uniform4f(field.u.uPointer, pointer.x, pointer.y, 170 * dpr, pointer.s);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    s.paint(off, cell, ink, alpha);
  }

  // One draw per frame: some embedded browsers only keep frames coming while every frame changes the
  // canvas. The scene runs only while it can be seen; its clock stops while it is paused.
  let frame = 0, last = 0, clock = 0, visible = true, lost = false;
  const intro = () => 1 - Math.pow(1 - Math.min(1, clock / INTRO), 3);
  function tick(now) {
    frame = 0;
    const dt = last ? Math.min(100, now - last) : 16;
    last = now;
    clock += dt;
    const k = Math.min(1, dt / 220);
    pointer.x += (pointer.tx - pointer.x) * k;
    pointer.y += (pointer.ty - pointer.y) * k;
    pointer.s += (pointer.ts * (fine.matches && !narrow ? 1 : 0) - pointer.s) * Math.min(1, dt / 600);
    draw(8 + clock / 1000, intro());
    run();
  }
  function run() {
    const go = visible && !lost && !document.hidden && !reduce.matches;
    if (go && !frame) {
      frame = requestAnimationFrame(tick);
    } else if (!go && frame) {
      cancelAnimationFrame(frame);
      frame = 0;
      last = 0;
    }
  }
  const still = () => draw(reduce.matches ? STILL_AT : 8 + clock / 1000, reduce.matches ? 1 : intro());
  function refresh() {
    measure();
    if (reduce.matches || !frame) {
      still();
    }
    run();
  }

  document.body.prepend(layer);
  measure();
  new ResizeObserver(refresh).observe(document.documentElement);
  new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting;
    run();
  }).observe(layer);
  document.addEventListener('visibilitychange', run);
  reduce.addEventListener('change', refresh);
  addEventListener('pointermove', move, { passive: true });
  document.documentElement.addEventListener('pointerleave', leave);
  const recolor = () => {
    colors();
    if (reduce.matches || !frame) {
      still();
    }
  };
  new MutationObserver(recolor).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', recolor);
  canvas.addEventListener('webglcontextlost', () => {
    lost = true;
    run();
    layer.remove();
  });
  still();
  run();
  requestAnimationFrame(() => layer.classList.add('on'));
}

// The field below the hero: one canvas fixed to the window, behind the content. Nothing moves on its
// own; it draws when the page scrolls, resizes or changes theme, and only while the stretch between
// the hero & the closing band is in view. Under reduced motion marks keep one state while scrolling.
const M_CELL = 18, M_CELL_NARROW = 14;
const M_FEATHER = 64;       // CSS px outside the content column over which the margins fade in
const M_BEHIND = 0.12;      // strength behind the content column: about 1–1.5% ink
const M_HALO = 0.3;         // strength right at a schematic's edge
const M_REACH = 56;         // CSS px the halo reaches out to the sides, less above & below

function margins() {
  const main = document.querySelector('.landing');
  const end = document.querySelector('.closing');
  const s = main && end && surface('margin', MARGIN_FS);
  if (!s) {
    return;
  }
  const { canvas, gl, layer, field, vao, fbo } = s;
  const reduce = matchMedia('(prefers-reduced-motion: reduce)');
  let dpr = 1, cell = M_CELL, cols = 0, rows = 0, narrow = false, ink = [0.5, 0.5, 0.5], alpha = 0.1;
  let span = [0, 0], column = [0, 0], blank = false;
  const halos = new Float32Array(24);

  function colors() {
    const t = theme();
    ink = t.ink;
    alpha = (t.dark ? 0.12 : 0.1) * (narrow ? 0.8 : 1);
  }

  function measure() {
    const w = document.documentElement.clientWidth, h = innerHeight;
    narrow = w < NARROW;
    dpr = Math.min(devicePixelRatio || 1, MAX_DPR);
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    cell = (narrow ? M_CELL_NARROW : M_CELL) * dpr;
    const c = Math.ceil(canvas.width / cell) + 1, r = Math.ceil(canvas.height / cell) + 2;
    if (c !== cols || r !== rows) {
      cols = c;
      rows = r;
      s.grid(cols, rows);
    }
    const box = main.getBoundingClientRect(), css = getComputedStyle(main);
    column = [box.left + parseFloat(css.paddingLeft), box.right - parseFloat(css.paddingRight)].map(v => v * dpr);
    span = [hero.getBoundingClientRect().bottom + scrollY - 40, end.getBoundingClientRect().top + scrollY]
      .map(v => v * dpr);
    colors();
  }

  function draw() {
    const scroll = scrollY * dpr;
    // Nothing of the stretch below the hero is in the window: clear once & rest.
    if (scroll + canvas.height < span[0] - M_REACH * dpr || scroll > span[1]) {
      if (!blank) {
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        blank = true;
      }
      return;
    }
    blank = false;
    // Schematics near the window get a halo; the rest are skipped.
    let count = 0;
    halos.fill(0);
    for (const e of main.querySelectorAll('.visual > :first-child, .vframe')) {
      const b = e.getBoundingClientRect();
      if (count < 6 && b.bottom > -M_REACH && b.top < innerHeight + M_REACH) {
        halos.set([b.left, b.top, b.right, b.bottom].map(v => v * dpr), count * 4);
        count++;
      }
    }
    // The grid is fixed to the page: its rows start at a multiple of the cell above the window.
    const first = Math.floor(scroll / cell) * cell;
    const off = [(canvas.width - cols * cell) / 2, first - scroll];
    gl.bindVertexArray(vao);
    gl.disable(gl.BLEND);
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.viewport(0, 0, cols, rows);
    gl.useProgram(field.p);
    const u = field.u;
    gl.uniform2f(u.uOff, off[0], off[1]);
    gl.uniform1f(u.uCell, cell);
    gl.uniform1f(u.uScale, SCALE * dpr);
    gl.uniform1f(u.uScroll, scroll);
    gl.uniform2f(u.uSpan, span[0], span[1]);
    gl.uniform2f(u.uColumn, column[0], column[1]);
    gl.uniform2f(u.uView, canvas.width, canvas.height);
    gl.uniform4fv(u.uHalo, halos);
    gl.uniform1i(u.uHaloCount, count);
    gl.uniform4f(u.uShape, M_FEATHER * dpr, M_BEHIND, M_HALO, M_REACH * dpr);
    gl.uniform1f(u.uRead, reduce.matches ? 0 : 1);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    s.paint(off, cell, ink, alpha);
  }

  // At most one draw per frame, and only when something changed.
  let frame = 0;
  const request = () => {
    if (!frame && !document.hidden) {
      frame = requestAnimationFrame(() => {
        frame = 0;
        draw();
      });
    }
  };
  const refresh = () => {
    measure();
    blank = false;
    request();
  };

  document.body.prepend(layer);
  measure();
  addEventListener('scroll', request, { passive: true });
  addEventListener('resize', refresh);
  new ResizeObserver(refresh).observe(document.body);
  document.addEventListener('visibilitychange', request);
  reduce.addEventListener('change', request);
  const recolor = () => {
    colors();
    request();
  };
  new MutationObserver(recolor).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', recolor);
  canvas.addEventListener('webglcontextlost', () => layer.remove());
  draw();
  requestAnimationFrame(() => layer.classList.add('on'));
}

// It starts after load, when the browser is idle, on every screen size.
if (hero) {
  const boot = () => {
    try {
      start();
    } catch {
      document.querySelector('.scene.header')?.remove();
    }
    try {
      margins();
    } catch {
      document.querySelector('.scene.margin')?.remove();
    }
  };
  const idle = window.requestIdleCallback ?? (f => setTimeout(f, 200));
  const later = () => idle(boot);
  if (document.readyState === 'complete') {
    later();
  } else {
    addEventListener('load', later, { once: true });
  }
}
