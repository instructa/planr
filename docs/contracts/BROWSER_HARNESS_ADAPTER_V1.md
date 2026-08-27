# Browser Harness adapter v1

Status: historical. The current adapter hard-cuts to
`BROWSER_HARNESS_ADAPTER_V2.md`; it does not accept this v1 payload schema or
its shared-action/final-observation execution shape.

This reference defines Planr's first process adapter for the external
`browser-harness` tool. The adapter implements
`EVIDENCE_ADAPTER_PROTOCOL_V1.md`. Planr Core does not depend on this adapter.

## Capability

The adapter supports `com.planr.web.dom_state` observations with the payload
schema `schema://com.planr.web.dom_state.v1`.

Each requirement uses these fields:

- `subject`: A CSS selector for the observed element.
- `expected.text`: The exact trimmed `textContent`, if text is required.
- `expected.visible`: The required rendered visibility, if visibility is
  required.
- `state_transitions`: A shared array of at most 64 closed actions for the
  batch. A `click` action has an accessibility `role` and `name`. A `fill` or
  `select` action has a CSS selector `subject` and a string `value`.

All requirements in one batch must use the same transition array and the same
optional execution method. The adapter rejects arbitrary actions, arbitrary
JavaScript, mixed methods, and unsupported observation types before it starts
the browser tool.

## Execution

An availability probe runs `browser-harness --version`. A real Evidence run
starts `browser-harness` once and sends one generated Python program over
stdin. The program completes these steps in one browser session:

1. Open the bound HTTP or HTTPS target.
2. Apply bounded fill and select actions through native DOM value setters and
   input/change events.
3. Find each click target in the accessibility tree by its role and name, then
   click the center of its CDP box model.
4. Read each DOM postcondition until it passes or the five-second observation
   window ends.
5. Return one structured result for the full requirement batch.
6. Close the tab.

The generated program inserts requirement data only as JSON. A requirement
cannot add executable Python or JavaScript.

## Result

Each `actual` object contains:

```json
{
  "schema_ref": "schema://com.planr.web.dom_state.v1",
  "selector": "#status",
  "text": "B",
  "visible": true
}
```

Planr Core validates this object against the registered JSON Schema. Core then
evaluates the requirement's `expected` predicate.

If every requirement selects the `browser-harness` agent skill, the adapter
also returns the matching supervised invocation record. This record is adapter
output. It becomes trusted only after Planr validates the observed process and
all sealed bindings.

## Capture policy

The v1 success path disables Browser Harness recordings. The generated program
does not call screenshot, recording, or trace helpers. The structured DOM
observation is the retained proof for a non-visual criterion.

The adapter does not advertise a visual observation type. Planr therefore
rejects it during capability matching for a visual requirement. A recording or
screenshot cannot upgrade `com.planr.web.dom_state` into visual Evidence.

## Historical fixture

The executable v1 fixture was removed with the v2 hard cut. The maintained
provider-free fixture lives under `tests/fixtures/evidence/browser-harness/v2/`.
