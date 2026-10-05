---
version: 1
slug: "taskboard-template-html"
primary_target: "taskboard.template.html"
related_targets: []
---

## Scope

Taskboard page (`taskboard.template.html`), Operate mode. Viewed mainly in the bb sidebar / inline preview (600–900 px), also as a full browser tab. English UI.

## Direction contract

THESIS: A modern app shell that answers "what changed, what needs me, what is next" in one viewport; it refuses both the scrolling report-of-cards and a literal TUI costume.

OWN-WORLD: Cool neutral ground, white/ink panels separated by hairlines, Mona Sans for prose, Monaspace Neon for IDs, counts, hashes, dates and section labels (GitHub type family; Geist was dropped as overused); status color only on small drawn SVG marks and bar segments; selection is ink inversion, not an accent hue. Light and dark from the same tokens.

STORY: The owner opens the board between agent runs, sees deltas since yesterday and what is waiting on them, scans the latest runs, then drills into one task's dependencies and history.

FIRST VIEWPORT: Top bar (project, HEAD hash, updated, search with › prompt), tabs Overview / Runs / Tasks with mono counts, then Overview: full-width status strip with counts and deltas plus per-track bars, a 2×2 grid of Needs you, Recent runs, Up next, Bottlenecks, each scrolling inside; key hints in a thin footer.

FORM: user-steered modern product UI with a light terminal flavor (replaces the rolled tig direction after user feedback); seed key 1a008804.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
