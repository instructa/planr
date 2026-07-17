# Documentation components

The documentation shell uses Fumadocs primitives first and keeps Planr-specific components small, accessible, and composable.

## `CommandBlock`

Use for a single executable command that readers should copy verbatim.

```mdx
<CommandBlock command="planr project show --json" label="Inspect" />
```

- `command` is the exact clipboard value and rendered code.
- `label` identifies the command's purpose; it defaults to `Terminal`.
- The copy control exposes `Copy command` and changes to `Command copied` after success.
- For multi-command or annotated examples, use fenced code blocks and Fumadocs code presentation instead.

## `PathCard`

Use on curated landing surfaces where a reader chooses an outcome. It requires `href`, `eyebrow`, `title`, `description`, and a decorative `icon`. Do not use it for dense reference indexes; Fumadocs `Cards` and `Card` are the canonical MDX primitives there.

## `PlanrMark`

The compact product mark is used in global navigation. It is decorative by default (`aria-hidden`) and must be paired with visible Planr text when it communicates brand identity.

## Authoring rules

- Keep one `h1` supplied by the page shell and use ordered headings beneath it.
- Every task page ends with verification, failure/recovery, and next steps.
- Never encode meaning with color alone.
- Give interactive controls visible focus, an accessible name, and at least a 44px practical target where possible.
- Respect reduced-motion preferences and test both light and dark themes.
- Navigation ordering lives in explicit `meta.json` files.
