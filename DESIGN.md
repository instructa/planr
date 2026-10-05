---
name: Taskboard
description: A one-viewport status board for a planning repository, driven by its git history.
colors:
  bg: "#f4f5f7"
  panel: "#ffffff"
  sunken: "#eceef1"
  hover: "#f1f3f6"
  line: "#e2e5e9"
  line-2: "#cdd2d8"
  fg: "#0e1116"
  fg-2: "#3f4753"
  fg-3: "#535b67"
  fg-4: "#646c78"
  sel: "#0e1116"
  sel-fg: "#ffffff"
  sel-soft: "#e6e9ee"
  focus: "#0e1116"
  mark-ink: "#ffffff"
  s-done: "#1f9d55"
  s-review: "#3366e0"
  s-wip: "#8152e6"
  s-open: "#7f8691"
  s-blocked: "#e0464c"
  s-decision: "#b57300"
  s-dropped: "#7f8691"
  bg-dark: "#0a0c0f"
  panel-dark: "#111419"
  sunken-dark: "#0d1014"
  hover-dark: "#171b21"
  line-dark: "#1f242b"
  line-2-dark: "#2c333c"
  fg-dark: "#e8ebef"
  fg-2-dark: "#b0b8c3"
  fg-3-dark: "#8d95a1"
  fg-4-dark: "#7a838f"
  sel-dark: "#e8ebef"
  sel-fg-dark: "#0a0c0f"
  sel-soft-dark: "#1c2129"
  focus-dark: "#e8ebef"
  mark-ink-dark: "#0a0c0f"
  s-done-dark: "#3cc06e"
  s-review-dark: "#6e9bff"
  s-wip-dark: "#a684ff"
  s-open-dark: "#68717c"
  s-blocked-dark: "#ff6268"
  s-decision-dark: "#f2b23e"
  s-dropped-dark: "#6a727d"
typography:
  project:
    fontFamily: "Mona Sans, ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif"
    fontSize: "14px"
    fontWeight: 600
    letterSpacing: "-0.015em"
  title:
    fontFamily: "Mona Sans, ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif"
    fontSize: "14.5px"
    fontWeight: 600
    lineHeight: 1.35
    letterSpacing: "-0.015em"
  title-sm:
    fontFamily: "Mona Sans, ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif"
    fontSize: "13.5px"
    fontWeight: 600
    lineHeight: 1.35
    letterSpacing: "-0.012em"
  body:
    fontFamily: "Mona Sans, ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.45
    letterSpacing: "-0.005em"
  body-sm:
    fontFamily: "Mona Sans, ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.45
  numeral:
    fontFamily: "Monaspace Neon, ui-monospace, SF Mono, Menlo, monospace"
    fontSize: "19px"
    fontWeight: 500
    lineHeight: 1
    letterSpacing: "-0.03em"
  data:
    fontFamily: "Monaspace Neon, ui-monospace, SF Mono, Menlo, monospace"
    fontSize: "11.5px"
    fontWeight: 400
    lineHeight: 1.45
  meta:
    fontFamily: "Monaspace Neon, ui-monospace, SF Mono, Menlo, monospace"
    fontSize: "11px"
    fontWeight: 400
  label:
    fontFamily: "Monaspace Neon, ui-monospace, SF Mono, Menlo, monospace"
    fontSize: "10.5px"
    fontWeight: 600
    lineHeight: 1
    letterSpacing: "0.06em"
  label-sm:
    fontFamily: "Monaspace Neon, ui-monospace, SF Mono, Menlo, monospace"
    fontSize: "10px"
    fontWeight: 600
    letterSpacing: "0.06em"
  micro:
    fontFamily: "Monaspace Neon, ui-monospace, SF Mono, Menlo, monospace"
    fontSize: "10.5px"
    fontWeight: 500
    lineHeight: 1
  tag:
    fontFamily: "Monaspace Neon, ui-monospace, SF Mono, Menlo, monospace"
    fontSize: "9.5px"
    fontWeight: 500
    lineHeight: 1
    letterSpacing: "0.05em"
rounded:
  panel: "8px"
  control: "7px"
  chip: "6px"
  small: "4px"
  tag: "3px"
  cell: "1.5px"
spacing:
  hair: "2px"
  xs: "4px"
  sm: "6px"
  md: "8px"
  gutter: "10px"
  inset: "12px"
  edge: "14px"
components:
  panel:
    backgroundColor: "{colors.panel}"
    rounded: "{rounded.panel}"
    padding: "10px 12px 8px"
  task-row:
    textColor: "{colors.fg-2}"
    typography: "{typography.data}"
    padding: "5px 12px"
  task-row-hover:
    backgroundColor: "{colors.hover}"
  task-row-selected:
    backgroundColor: "{colors.sel}"
    textColor: "{colors.sel-fg}"
  task-row-related:
    backgroundColor: "{colors.sel-soft}"
  table-row:
    textColor: "{colors.fg-2}"
    height: "31px"
    padding: "0 12px"
  chip:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.fg-2}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.chip}"
    height: "26px"
    padding: "0 9px"
  chip-pressed:
    backgroundColor: "{colors.sel}"
    textColor: "{colors.sel-fg}"
  tab:
    textColor: "{colors.fg-3}"
    padding: "6px 8px 9px"
  tab-selected:
    textColor: "{colors.fg}"
  tab-count:
    backgroundColor: "{colors.sunken}"
    textColor: "{colors.fg-3}"
    typography: "{typography.micro}"
    rounded: "{rounded.small}"
    padding: "3px 5px"
  search:
    backgroundColor: "{colors.panel}"
    rounded: "{rounded.control}"
    height: "28px"
    padding: "0 6px 0 8px"
  kbd:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.fg-3}"
    typography: "{typography.micro}"
    rounded: "{rounded.small}"
    padding: "3px 5px 2px"
  tag:
    backgroundColor: "{colors.sunken}"
    textColor: "{colors.fg-2}"
    typography: "{typography.tag}"
    rounded: "{rounded.tag}"
    padding: "3px 4px 2px"
  tooltip:
    backgroundColor: "{colors.fg}"
    textColor: "{colors.bg}"
    rounded: "{rounded.control}"
    padding: "7px 9px"
  goal-tile:
    textColor: "{colors.fg-2}"
    rounded: "{rounded.chip}"
    padding: "5px 6px 6px"
  goal-tile-hover:
    backgroundColor: "{colors.hover}"
  popover:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.fg}"
    rounded: "{rounded.panel}"
  link-hover:
    textColor: "{colors.fg}"
---

# Design System: Taskboard

## Overview

**Creative North Star: "The Hairline Console"**

A modern app shell with a light terminal accent: it borrows the terminal's density, monospace data and key-driven movement, and refuses both a terminal costume (box-drawing, ANSI palettes, one face everywhere) and a scrolling report of cards. The whole page is one fixed viewport. A cool neutral ground holds white panels (ink panels in dark) separated by single hairlines; every list scrolls inside its own panel, never the page.

Density is high and ordered. Prose (task titles, run subjects, headings) is set in Mona Sans; everything a machine produced (IDs, counts, hashes, dates, deltas, panel headings, key hints) is set in Monaspace Neon. Both faces come from GitHub's type family, so the two voices read as one. Color carries status and nothing else: it lives only on small drawn SVG marks and on bar or strip segments. Interaction state is achromatic: hover is a faint tint, selection is full ink inversion, related items get a soft ink wash.

Light and dark are the same system mapped onto two token sets; nothing is designed for one theme only. The system is meant to be lifted into other planning-repo boards and a bb plugin surface, so it depends on nothing a plugin host cannot provide: inlined fonts, inline SVG, no network.

**Key Characteristics:**
- One viewport, panel-internal scrolling, no page scroll above the narrowest breakpoint.
- Sans for human text, mono for machine values and panel headings.
- Status color only on drawn marks and bar segments, always paired with a label.
- Selection is ink inversion; there is no brand accent hue.
- Hairline borders and tonal steps for depth; a single shadow, on the tooltip. The warnings popover floats on a flat ground ring instead.
- Container-query layout keyed to the board's own width, not the window.
- Every view is reachable and drivable from the keyboard.

## Colors

A cool, near-achromatic ink-and-paper palette with one small, saturated status set reserved for marks.

### Primary
- **Ink** (light #0e1116 / dark #e8ebef, `fg`, `sel`, `focus`): primary text, the selected-tab underline, the focus ring, and the fill of every selected or pressed element. Ink is the system's only "accent"; it marks where you are, never what state a task is in.
- **Inverted Ink** (light #ffffff / dark #0a0c0f, `sel-fg`, `mark-ink`): text on selected rows and pressed chips, and the cut-out glyph inside filled status marks.

### Secondary
Status signals, one hue per task status, drawn only as marks and bar segments.
- **Done Green** (light #1f9d55 / dark #3cc06e, `s-done`)
- **Review Blue** (light #3366e0 / dark #6e9bff, `s-review`)
- **In-progress Violet** (light #8152e6 / dark #a684ff, `s-wip`)
- **Open Slate** (light #7f8691 / dark #68717c, `s-open`); `s-dropped` shares this neutral and is told apart by its mark shape.
- **Blocked Red** (light #e0464c / dark #ff6268, `s-blocked`)
- **Decision Amber** (light #b57300 / dark #f2b23e, `s-decision`): the owner is the blocker.

Light Open Slate was darkened from #878e98 to #7f8691 so the ring holds 3:1 on every light surface a mark sits on; the old value fell to 2.97:1 on the hover tint. It stays 1.45:1 apart from `fg-4`, so marks still read lighter than the quietest text. Do not lighten it again.

| Surface (light) | #7f8691 | #878e98 (before) |
|---|---|---|
| `panel` #ffffff | 3.67:1 | 3.31:1 |
| `bg` #f4f5f7 | 3.36:1 | 3.03:1 |
| `hover` #f1f3f6 | 3.30:1 | 2.97:1 |
| `sunken` #eceef1 | 3.16:1 | 2.84:1 |
| `sel-soft` #e6e9ee | 3.02:1 | 2.72:1 |
| `sel` #0e1116 (selected row) | 5.15:1 | 5.72:1 |

### Neutral
- **Cool Ground** (light #f4f5f7 / dark #0a0c0f, `bg`): the page behind the panels, the top bar, tab strip and footer.
- **Panel White** (light #ffffff / dark #111419, `panel`): every panel, the search field, chips, key caps.
- **Sunken** (light #eceef1 / dark #0d1014, `sunken`): tab counts and tags; the one step below panel.
- **Hover Tint** (light #f1f3f6 / dark #171b21, `hover`): row and control hover.
- **Soft Ink** (light #e6e9ee / dark #1c2129, `sel-soft`): rows related to the selection (dependencies and dependents) and the search focus halo.
- **Hairline** (light #e2e5e9 / dark #1f242b, `line`): panel borders, row dividers, tab-strip and footer rules.
- **Strong Hairline** (light #cdd2d8 / dark #2c333c, `line-2`): hover borders, the dashed track divider, toolbar separators, the history spine, scrollbar thumbs.
- **Text steps** (`fg-2` #3f4753 / #b0b8c3 for titles and secondary text; `fg-3` #535b67 / #8d95a1 for meta and inactive tabs; `fg-4` #646c78 / #7a838f for the quietest text: hashes, counts in panel headers, placeholders, footer hints). Light `fg-4` was raised to hold at least 4.5:1 on both panel and ground; do not lighten it again.

### Named Rules
**The Status Is a Mark Rule.** Status hues appear only as drawn SVG marks, strip segments, bar segments and change cells. Text, backgrounds, borders and selection never take a status hue.

**The Ink Selection Rule.** Selection and pressed state are a full inversion to ink (`sel` / `sel-fg`); related items use `sel-soft`. Never introduce a colored accent for "active".

**The Twin Token Rule.** Every color is defined for light and dark under the same name; components reference the role token only, and the theme switch (`data-theme` or `prefers-color-scheme`) swaps values, never component rules.

## Typography

**Body Font:** Mona Sans, variable 200-900 (with ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif)
**Label/Mono Font:** Monaspace Neon 400/500/600 (with ui-monospace, SF Mono, Menlo, monospace)

**Character:** A humanist-grotesk sans for anything a person wrote, paired with a same-family texture-healing mono for anything a machine counted or hashed. The pairing keeps a terminal's precision without a terminal's monotony. Both faces are inlined as base64 woff2 at build time with `font-display: block`; no font is fetched at view time.

### Hierarchy
- **Project** (600, 14px, -0.015em): the project name in the top bar.
- **Title** (600, 14.5px, 1.35): the task-detail heading. **Title-sm** (600, 13.5px, 1.35) is the run-detail heading. Both use `text-wrap: pretty`.
- **Body** (400, 13px, 1.45, -0.005em): task titles in rows, run subjects, empty-state sentences. Task titles truncate to one line; at container width ≥ 1000px, panels that allow it clamp to two lines.
- **Body-sm** (400, 12px): chip labels and "none" lines. Count labels under the summary numerals use 11.5px sans.
- **Numeral** (mono 500, 19px, 1, -0.03em): the status counts in the summary strip; the only large type on the page.
- **Data** (mono 400, 11.5px, 1.45): IDs (at 500), run times, history entries, track names.
- **Meta** (mono 400, 11px): hashes, row meta ("blocks 3", "open → done"), facts, panel-header counts, footer.
- **Label** (mono 600, 10.5px, 1, 0.06em, uppercase): panel headings ("Needs you", "Recent runs").
- **Label-sm** (mono 600, 10px, 0.06em, uppercase): group headings inside panels (including "Goals" in the
  summary), table column headers, day groups, detail section headings, and the labels of detail notes
  ("Check", "Warning").
- **Micro** (mono 500, 10.5px, 1): key caps and tab counts.
- **Tag** (mono 500, 9.5px, 1, 0.05em, uppercase): inline row tags ("goal", "ready").

### Named Rules
**The Two Voices Rule.** If a person wrote it, it is sans; if the system produced it (ID, count, hash, date, delta, key), it is mono. Never set a task title in mono or an ID in sans.

**The Label Is the Heading Rule.** Uppercase mono labels are the heading of the panel or group they sit in, followed directly by content. They never sit above another heading as a kicker or eyebrow.

**The One Numeral Rule.** Only the summary status counts use the 19px numeral. Everywhere else, numbers stay at data or meta size.

## Layout

The shell is a four-row grid filling 100vh: top bar, tab strip, view, footer. The body never scrolls; each view is absolutely positioned in the main row, and every list scrolls inside its own `.scroll` region. The shell is a size container (`container-type: inline-size`) and every responsive rule is a container query against the board's own width, so the same page works in a 600-900px bb sidebar, an inline preview or a full tab.

- **Overview:** a summary panel (header line, full-width status strip, count row, goal queue, per-track bars) above a two-column grid: Needs you, the approver's whole queue, takes the full left column; Recent runs (1.25fr), Up next and Bottlenecks (1fr each) stack on the right.
- **Runs:** a stacked-bar trend panel (60px) with day ticks, above a split of run list (0.95fr) and run changes (1.05fr).
- **Tasks:** a three-row filter toolbar above a split of task table and detail panel (min 300px, 0.85fr); with nothing selected the table takes the full width.

Spacing rhythm: 10px gutter between and around panels, 12px panel inset, 14px top-bar and footer edge, 6px between chips, 2px between strip segments and change cells. Rows are 5px × 12px (task rows), 7px × 12px (run rows), 31px minimum (table rows) under a 32px table header. Controls are 28px tall, chips 26px.

Container breakpoints:
- **≤ 520px:** HEAD hash hidden; track bars one column; the overview grid becomes four stacked panels, Needs
  you first (min 360px, the others min 220px), and the overview view scrolls (the only page-level scroll); the
  table hides the "goal" and "ready" tags; the footer keeps only the view and filter hints and the warnings
  count.
- **≤ 640px:** the table drops the Track column; summary counts wrap to three columns; each toolbar chip row stops wrapping and scrolls sideways (scrollbar hidden).
- **≤ 760px:** the runs split stacks vertically; the footer drops the "generated from" note.
- **≤ 1080px:** the tasks split stacks table above detail.
- **≥ 1000px:** task titles in wrapping panels clamp to two lines; ID columns size to their content.
- **≥ 1240px:** track bars four across; wider count spacing.
- **≥ 1680px:** the overview grid becomes one row of four panels, Needs you first.

Column widths for IDs are set in `ch` from the longest visible ID (min 6ch, capped at 14ch below 1000px) so IDs align in a column.

### Named Rules
**The One Viewport Rule.** Overview, attention panels and navigation stay above the fold at sidebar width; when content grows, a panel scrolls internally rather than the page.

**The Container Not Window Rule.** Breakpoints are container queries on the shell. Never key layout to `@media` viewport width; the board must behave by the width it is given.

**The Pinned Width Rule.** The shell, every view and the summary use a single `minmax(0, 1fr)` column, so no string can widen the page beyond the board: the document's scroll width always equals its client width. Long single-line values truncate with an ellipsis and keep the full value in a `title` or tooltip; secondary parts wrap instead (summary header line, Goals heading). In the top bar the HEAD line shrinks first, the project name next, and the search field keeps its width; the "uncommitted" marker sits right after the hash, so truncation drops the time first.

## Elevation & Depth

The system is flat. Depth is conveyed by tone and hairlines: cool ground behind, panel one step lighter than the ground in both themes, sunken one step below panel, and a 1px `line` border around every panel. Hover deepens tone; selection inverts it. Two layers float: the tooltip, which alone carries a shadow, and the warnings popover, which is set off by a stronger hairline and a flat ring of ground color instead of a shadow.

### Shadow Vocabulary
- **Tooltip lift** (`box-shadow: 0 8px 24px rgba(0,0,0,.2)`): the hover/focus tooltip, which inverts to ink on ground.
- **Search focus halo** (`box-shadow: 0 0 0 3px var(--sel-soft)`): a flat ring, not a shadow, around the focused search field.
- **Inverted tag ring** (`box-shadow: inset 0 0 0 1px currentColor`): a tag inside a selected row turns into an outline.
- **Ground ring** (`box-shadow: 0 0 0 4px var(--bg)`): a flat 4px ring of page-background color around the warnings popover, with a 1px `line-2` border inside it. It separates the popover from the panels beneath without a shadow.

### Named Rules
**The Hairline Panel Rule.** Panels are separated by a 1px hairline and a tonal step, never by a drop shadow. A shadow means "floating above the page" and belongs to the tooltip alone; the warnings popover floats without one, on a stronger hairline and the ground ring.

## Shapes

Gently rounded, small-radius geometry that tightens as elements shrink: panels and the warnings popover 8px, the search field, theme button and tooltip 7px, chips, hoverable count tiles, goal tiles and close buttons 6px, key caps, tab counts and the focus ring 4px, tags and strip ends 3px, change cells 1.5px. Strips round only their outer ends. Key caps have a 2px bottom border, a physical key cue. The focus ring is a 2px ink outline at 2px offset.

Status marks are drawn on a 12×12 grid as SVG, never glyphs or icon fonts. The vocabulary is one shape language, so status reads without color:
- **Open:** hollow ring (r 4.6, 1.5 stroke).
- **In progress:** ring with the right half filled.
- **Review:** ring with a solid center dot.
- **Done:** filled disc with a check cut out in `mark-ink`.
- **Blocked:** filled disc with a horizontal bar cut out.
- **Decision:** filled rotated square (diamond) with an exclamation cut out; the only non-circular mark, because it is the only status waiting on the owner.
- **Dropped:** ring with a diagonal slash.
- **Removed:** dashed ring (also legacy goals).
- **New:** a plus in `fg-2`, no status hue.

Goal states reuse this vocabulary with the same meaning rather than adding shapes: running takes the in-progress mark, approved the open ring, draft the decision diamond (it waits on the owner), review the review mark, done the done disc, legacy the dashed ring. The state word always sits next to the mark.

### Named Rules
**The Shape Carries Status Rule.** Each status must be identifiable by mark shape alone, and every mark sits next to a text label, count or tooltip. Add new statuses as new 12×12 shapes, never as new colors on an existing shape.

## Components

### Navigation
Quiet, underline-led. The top bar holds the project name, HEAD hash, an "uncommitted" marker in `fg` 600 when the working tree differs, and the time in mono (`fg-3`), the search field pushed right, and a 28px theme toggle (sun/moon drawn SVG). Tabs are 13px/500 sans in `fg-3`; the selected tab turns `fg` with a 2px ink underline sitting on the tab strip's hairline. Each tab carries a mono count in a sunken 4px pill. A thin footer lists key hints as key caps plus a word; on the right sit the warnings count (when the index has warnings) and the source commit. The warnings count is a quiet button: mono count in `fg` 600, the word in `fg-3`, hover takes the panel tint; it opens the warnings popover.

### Chips
- **Style:** 26px, 6px radius, panel background, hairline border, 12px sans label in `fg-2`, mono count in `fg-3`; status chips lead with their mark.
- **State:** hover lifts the border to `line-2` and text to `fg`; pressed (`aria-pressed`) inverts to ink with the count at 70% opacity. Chips toggle: pressing an active chip returns to "All".
- **Goal chips** lead with their goal-state mark and carry a tooltip with the goal's title, stream, budget and progress.
- **Tasks toolbar:** three rows. Row one holds the presets (All, Needs you, To check, Up next, Moved since the
  baseline day, plus a run chip while a run filter is active; Needs you is blocked and decision tasks, To
  check is tasks in review); row two the goal chips, a separator and the track chips; row three the status
  chips. Presets and statuses form one exclusive filter; goal and track are separate filters that combine with
  it and with each other. Pressing an active goal or track chip clears only that filter. At ≤ 640px each row
  stays on one line and scrolls sideways.

### Cards / Containers (Panels)
- **Corner Style:** 8px.
- **Background:** `panel` on `bg`.
- **Shadow Strategy:** none (see Elevation & Depth).
- **Border:** 1px `line`.
- **Internal Padding:** header 10px 12px 8px; header is an uppercase mono label, a quiet mono count, and an optional right-aligned "all →" link in `fg-3`. Body rows run edge to edge with a hairline above each row.

### Inputs / Fields
- **Style:** the search field is 28px, 7px radius, panel background, hairline border, led by a mono `›` prompt in `fg-4` and closed by a `/` key cap. The input itself has no border or outline.
- **Focus:** border to `line-2` plus a 3px `sel-soft` halo. Typing switches to the Tasks view and filters live; Enter opens the first match.

### Task Row
The core list item. A four-column grid: 12px mark, ID column in `ch`, sans title (truncating), right-aligned mono meta. A 5px `fg-2` dot after the ID marks "moved since the last day's baseline". Hover tints to `hover`. In the task table, the selected row inverts to ink (all text and tags to `sel-fg`, tags become outlines) and its dependencies and dependents take `sel-soft`. Every row carries a tooltip with ID, status, track, full title and goal names.

Row meta reads "unlocks N · blocks N": unlocks counts the tasks that become ready when this one is satisfied (review or done), blocks the open tasks that depend on it. Both come from the data; the board never derives them. Up next lists ready tasks of running or approved goals first, with the goal name in the meta. Bottlenecks show the top 12 unfinished tasks by blocks, then unlocks; review and done tasks already satisfy their dependents, so they never appear there.

### Needs You Queue
Needs you is the approver's whole queue: everything that waits on them and nothing else. The panel header counts it ("1 goal · 7 to decide · 25 to check", truncating when narrow). The body holds up to three groups, each under a sticky label-sm group heading with a quiet count and, where a Tasks filter exists, a right-aligned "all →" (the same sticky heading as the Runs day groups):
- **Goals:** draft goals ("approve") and review goals ("check"), as rows with the goal name in the ID column, a "goal" tag and the action as meta. A row opens Tasks filtered to the goal.
- **To decide:** blocked and decision tasks ranked by unlocks, then blocks, then status; "all →" opens the Needs you filter.
- **To check:** tasks in review (built and tested by agents, waiting only for an approver's manual check), most recently moved first, with the run time as meta. The first five rows show, then a quiet "N more in Tasks →" row; "all →" and that row open the To check filter.

Rows stay on one line, even where other panels clamp titles to two. Empty groups are left out; with nothing at all, the panel says so in one sentence.

### Run Row
Mono time, mono hash in `fg-4`, sans subject, and a second line of change cells (one 5×10px cell per changed task, colored by target status; new tasks in `fg-4`, capped at 16 with "+n") followed by a "2 done · 1 new · 3 moved" summary; a task created as done counts as new. The current run inverts to ink. Run lists group by day under sticky label-sm headers.

### Status Strip and Bars
The signature data form. A horizontal strip of status segments (8px tall in the summary, 6px per track) with 2px gaps and rounded outer ends; segment width is proportional to count, minimum 2px. The runs trend is one stacked bar per run (60px max) using the same segment colors; hover or press draws a 1.5px ink outline at 2px offset. Strips carry an `aria-label` listing every status and count, and each segment a tooltip with share in percent.

### Count Tiles
Each summary count is a button: 19px mono numeral, a mono delta since the previous day's last run (`fg-4` when unchanged as "±0", ink at 600 when it moved), and the mark plus status label beneath. Clicking filters the Tasks view.

The summary header line reads "110 tasks · 4 tracks" on the left and "Δ since 02.10. · 20 moved" on the right.
The line wraps when narrow, with the delta part moving to its own line.

### Goal Queue
The goal queue sits inside the summary panel between the count row and the track bars, behind the same dashed `line-2` divider. A label-sm "Goals" heading leads, followed by mono state counts in `fg-4` ("1 running · 1 approved · 1 draft") and quiet "+N legacy" and "+N done" notes. The notes are focusable and list the goal names and files in a tooltip; legacy and done goals get no tile.

Below the heading, one tile per running, approved, draft and review goal, in that order, in an auto-fit grid (tiles at least 196px, 20px column gap). Tiles are borderless buttons like the count tiles: 6px radius, 5px 6px 6px padding, hover tint. Each tile has three lines:
- the goal-state mark, the state word (mono 11px, `fg`, 500), stream and budget (mono, `fg-3`, truncating), and progress right-aligned (`fg-2`): built work, (done + review)/total, because review tasks are finished except for the manual check;
- the goal name (mono 500, 11.5px, `fg`) and its title (sans 12.5px, `fg-2`, truncating);
- a 6px status strip of the goal's scope tasks (an empty sunken track when the scope is empty) and a mono note
  in `fg-3`: "1 to check · 1 ready · 3 waiting", or "·". "To check" comes from the goal's `review` count and
  leads, since it is the approver's part.

The tile's tooltip adds the title, budget, thread and counts (built, done, to check, ready, waiting); a draft tile says it waits for approval, a review tile that it is over budget or waits for sign-off. Clicking a tile opens Tasks filtered to that goal. Without goal files the queue reads "No goal files yet."; with goals but none queued, "No goal is running, approved or waiting for approval."

### Detail Notes
The task detail lists notes under its facts: a label-sm uppercase label in `fg-2` ("Check", "Warning")
followed by a 12px sans sentence in `fg-3`. A task in review gets the check note first: built and tested by
agents, waiting for an approver's manual check. The warnings shown there are the index warnings that start
with the task's ID. The detail facts also name the task's goals as links (they filter Tasks by goal) and its
unlocks count.

### Warnings Popover
The second floating layer, opened from the footer's warnings count. It sits bottom-right above the footer, min(420px, viewport − 20px) wide and at most min(70vh, 520px) tall: panel background, 1px `line-2` border, 8px radius, the ground ring and no shadow. The header is an uppercase "Warnings" label, a quiet count and a 24px close button; the body scrolls. Warnings are grouped by message (sans 12.5px in `fg-2`, count in `fg-4`), each followed by the affected keys in mono: task IDs are links that open the task, other keys (goal files) are plain text. Esc, a click outside or the close button hide it; Esc closes it before anything else.

### Link
Inline text links (`.lnk`) inherit the surrounding text color and add a `line-2` underline at a 3px offset; on
hover the text turns `fg` and the underline takes the text color. Used for goal names in the task detail,
warning IDs and "Clear filters"; history commit buttons share the look.

### Tooltip
An ink panel (`fg` on `bg`, inverted) at 11.5px sans, mono detail at 75% opacity, max 300px, 7px radius, the one shadow. Appears on mouse hover and on keyboard focus (`:focus-visible` only), clamped to the viewport.

### Keyboard Model
- `1` `2` `3` switch Overview, Runs, Tasks.
- `/` focuses the filter; Enter opens the first match; Esc clears the query, then blurs.
- `j` / `k` (or arrow keys) move the selected task in Tasks and the selected run in Runs, scrolling it into view.
- Esc closes the warnings popover first; outside the field it then closes the task detail, then clears filters.
- Keys with Cmd, Ctrl or Alt are left to the host.
- The footer always shows these hints as key caps.

### Motion
Background and text color transitions of 120ms ease-out on rows, chips, tabs, count tiles, goal tiles, the warnings count and the theme button, only under `prefers-reduced-motion: no-preference`. No layout or entrance animation.

### Named Rules
**The Escape Twice Rule.** Every data value is escaped before it enters markup. Tooltip markup is built as
HTML, stored in `data-tip` and parsed again when shown, so each value inside it is escaped first and the whole
string is escaped once more for the attribute. Lookups keyed by data (status labels, marks, goal-state marks)
accept only their own keys; numeric fields are coerced or escaped like text.


## Brand

The brand grows out of the board's one idea that is not neutral: status is a shape. The files live in
`site/assets/brand/`; the favicon is `site/assets/favicon.svg`.

### Mark
The mark is a lowercase p whose bowl is the board's in-progress mark: a ring with its right half filled,
the state planr exists to move work into. Its stem leaves the ring at 9 o'clock and runs down, the lane
to what comes next. It reads as a letter, a status and a direction at once, and it owes nothing to the
common dev-tool shapes (striped circle, triangle, spark, prompt).

- **Geometry** (`planr-mark.svg`, `viewBox="0 0 104 148"`): ring outer radius 52, stroke 17, so the
  inner radius is 35; fill radius 26, which leaves a 9-unit gap inside the ring; stem 17 wide, tangent
  to the ring's left edge, from the bowl's center 96 units down. Unlike a type p, the stem starts at the
  bowl's center, so the ring stays whole above it.
- **Color:** one ink, `currentColor`; `fg` on `bg` in either theme. Never a status hue, a gradient or a
  second color for the fill.
- **Size:** at 24 px tall and up use the master. Below that use the favicon drawing, which is heavier
  (stroke 3 on a 32 grid, ring radius 8, fill gap 1.75) so the gap survives 16 px.

### Logo
The logo is the wordmark with the mark as its first letter (`planr-logo.svg`, `viewBox="0 0 481 194"`).
Every letter is built from the mark's kit: the same 17-unit stroke, the same 52-radius ring for p and a,
arches of center-line radius 37 for n and r. x-height 100, ascender 148, descender 46, round overshoot 2.
Letter gaps are 23 (p–l), 22 (l–a), 27 (a–n) and 27 (n–r); the r's arm ends at 64°.

- The mark never stands next to the wordmark: the logo already starts with it. Use the mark alone where
  the name is already present or space is square (favicon, avatars, the video's end card).
- Clear space around the logo or mark is 34 units (twice the stroke). The logo's minimum height is 18 px.
- Set "planr" in lowercase in text; only the logo draws it with the mark.

### Favicon
`favicon.svg` is the small-size mark in `#f4f5f7` on a `#0e1116` tile (32 grid, 7 radius). The tile
keeps it legible on any tab strip in either theme.

### Motion
The reveal (`planr-mark-animated.svg`, `planr-logo-animated.svg`; timings in `MOTION.md`): the stem
rises and loops clockwise into the ring as one stroke (640 ms), the letters rise in order every 70 ms
(560 ms each), and the fill sweeps from 12 to 6 o'clock (480 ms). It plays once, ends on the static
geometry and shows only the final frame under reduced motion.

### Hero field
The landing hero (`site/assets/header.js`) is a halftone of status marks: a fine grid (18 CSS px, 14 on
phones) where each cell grows an open ring, fills it from the right and closes it into a disc as a slow
noise field passes; the field drifts right. It is ink at 11% (light) or 14% (dark), clears softly around
the headline, lead, install box and links, and frames the hero from its sides. Phones get a fainter
version that stays behind the text at 20% strength and no pointer response.

### Named Rules
**The Status Is the Logo Rule.** The mark and the hero only ever use the board's status shapes and ink.
A new brand visual derives from the same ring, stroke and fill, never from new symbols or hues.

**The One p Rule.** The logo starts with the mark; never set the mark beside the word "planr".

## Do's and Don'ts

### Do:
- **Do** set every machine value (ID, count, hash, date, delta, key) in Monaspace Neon and every human sentence or title in Mona Sans.
- **Do** draw status as a 12×12 SVG mark from the defined vocabulary, next to a label, count or tooltip.
- **Do** show selection and pressed state as ink inversion (`sel` / `sel-fg`) and related items with `sel-soft`.
- **Do** separate panels with a 1px `line` hairline and a tonal step; keep the tooltip as the only shadowed layer.
- **Do** define every new color twice (light and dark) under one role name.
- **Do** write responsive rules as container queries on the shell and let long lists scroll inside their panel.
- **Do** give every new view or list a keyboard path and list its keys in the footer.
- **Do** use uppercase mono labels only as the heading of the panel or group they open.
- **Do** escape every data value before it enters markup, and twice inside tooltip markup.
- **Do** let long strings truncate with a tooltip or wrap their secondary parts; the page width never grows past the board.

### Don't:
- **Don't** put a status hue on text, backgrounds, borders or the selected state.
- **Don't** add a brand accent color; ink is the only emphasis.
- **Don't** use glyph or emoji status icons, icon fonts or remote icon sprites; draw SVG.
- **Don't** add drop shadows to panels, cards or chips.
- **Don't** make the page scroll at sidebar width; scroll inside panels.
- **Don't** dress the board as a terminal: no box-drawing characters, ANSI palettes or single-font-everywhere.
- **Don't** fetch fonts, scripts or images at view time; inline them at build.
- **Don't** place an uppercase mono label above a heading as a kicker or eyebrow.
- **Don't** signal warnings with a hue; use a text tag, a label or a count.
