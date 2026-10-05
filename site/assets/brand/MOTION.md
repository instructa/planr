# planr logo motion

The reveal for the site and the video. Two files play it in the browser:
`planr-mark-animated.svg` (the mark) and `planr-logo-animated.svg` (mark and wordmark). Both end on
exactly the geometry of `planr-mark.svg` and `planr-logo.svg`. Rebuild it from the numbers below;
everything is in SVG user units of the logo (`viewBox="0 0 481 194"`). The mark file uses the same
shapes moved up by 46 (`viewBox="0 0 104 148"`).

The idea: a line rises from below and loops into a ring (a task appears on the plan), the letters
become ready one after another, then the ring fills from 12 to 6 o'clock (in progress).

## Elements

| Element | Shape | Drawn as |
|---|---|---|
| `S` stem & ring | One open path on the center line: from (8.5, 194) straight up to (8.5, 98), then a full circle clockwise around (52, 98), radius 43.5, starting & ending at 9 o'clock. Length 369.32: stem 96, circle 273.32. | Stroke 17 wide, butt caps, no fill. Revealed from its start: visible length 0 → 369.32. |
| `F` progress fill | The right half disc of radius 26 around (52, 98). | A pie from 12 o'clock, growing clockwise to 6 o'clock: sweep 0° → 180°. In SVG a circle of radius 13 with a 26-wide stroke, rotated −90°, dash 40.85. |
| `l` `a` `n` `r` | The filled letter shapes of `planr-logo.svg`. | Opacity 0 → 1 and y offset +14 → 0. |

The stem joins the circle tangentially, so `S` reads as one pen stroke with no corner. `F` never
touches the ring: a 9-unit gap stays between them, as on the board's in-progress mark.

## Timing

`t = 0` is the first frame. Frames are at 60 fps.

| Element | Start | Duration | End | Frames | Easing |
|---|---|---|---|---|---|
| `S` | 0 ms | 640 ms | 640 ms | 0–38 | `cubic-bezier(.65, 0, .35, 1)` |
| `l` | 360 ms | 560 ms | 920 ms | 22–55 | `cubic-bezier(.16, 1, .3, 1)` |
| `a` | 430 ms | 560 ms | 990 ms | 26–59 | same |
| `n` | 500 ms | 560 ms | 1060 ms | 30–64 | same |
| `r` | 570 ms | 560 ms | 1130 ms | 34–68 | same |
| `F` | 560 ms | 480 ms | 1040 ms | 34–62 | `cubic-bezier(.5, 0, .2, 1)` |

The logo reveal takes 1130 ms; the mark alone ends at 1040 ms.
Letters follow each other every 70 ms.

Moments to check a rebuild against: the stroke finishes the stem at 259 ms, passes 12 o'clock at
308 ms and 3 o'clock at 350 ms, just before `l` starts. At 560 ms, when `F` starts, `S` is 98.6%
drawn. Progress for `S` and `F` is the eased value times the full length or sweep. The letters ease
opacity and offset with the same curve.

## Rules

- One play, no loop. In the video, hold the final frame for at least 600 ms before a cut.
- Ink only: `currentColor`, which is `#0e1116` on light and `#e8ebef` on dark (the board's `fg`).
  `F` is the same ink as the stroke, never a status color.
- No overshoot, bounce, scale, blur or glow. The mark never scales in; only `S` draws and `F` sweeps.
- Reduced motion (`prefers-reduced-motion: reduce`) shows the final frame at once, with no fade.
  Both SVGs show the final frame when CSS animation does not run.
- Inline the SVG when it should take the text color; as an `<img>`, `currentColor` is black.

## The hero field (for brand visuals)

The site hero (`site/assets/header.js`) uses the same mark language as a field: each cell's mark grows
as an open ring, fills from the right through the half-filled state and closes into a disc. The field
drifts right at about 26 CSS px per second and fills in over 2.6 s after load (ease-out cubic). A video
background that wants the same texture can reuse those three states and the rightward drift.
