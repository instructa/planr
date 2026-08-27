# Browser Harness adapter v2

Status: frozen for `planr-browser-harness-adapter` v2.

This reference defines Planr's provider-neutral process adapter for the
external `browser-harness` tool. The adapter implements
`EVIDENCE_ADAPTER_PROTOCOL_V1.md`. Planr Core owns execution and trust; this
adapter owns only the closed browser scenario and its structured observations.

## Capability

The adapter supports `com.planr.web.dom_state` observations with payload schema
`schema://com.planr.web.dom_state.v2`.

Every requirement uses a CSS selector in `subject`. Its `expected` object may
select only these mechanically observed fields:

- exact normalized `text` and rendered `visible` state;
- `contains_text` and `excludes_text` lists;
- a named local-storage document's presence, transaction count, and equality
  to an earlier scenario snapshot;
- selected option text keyed by visible select label;
- unnamed-control and unlabelled-field counts;
- horizontal page overflow;
- browser runtime/console error and external-request counts.

Exactly one requirement in a batch carries a closed `state_transitions`
declaration with `scenario_id` and `steps`. Every sibling carries only the
matching `scenario_ref`; declaration position has no semantic meaning. This
keeps one sealed scenario in the work packet without making the first
observation an authority or duplicating a large action list. All requirements
use the same optional execution method. The scenario contains 1 to 256 steps
from this exact action set:

- `fill` or `select` one control by CSS `subject` or exact visible `label`;
- `click` one control by accessibility role and name (or the closed
  button-or-link role `control`), optionally within a container containing
  exact scope text;
- `reset` one named application storage key and navigate the bound target;
- `snapshot_storage` under one bounded scenario-local ID;
- `reload` the current document;
- `viewport` with a bounded width and height;
- `checkpoint` with one or more sealed requirement IDs and an optional prior
  storage snapshot comparison.

There must be exactly one declaration and every reference must resolve to it.
Checkpoint requirement IDs are unique across the scenario and their union must
equal the exact sealed batch requirement set. A checkpoint can reference only
a storage snapshot created earlier in the same scenario. Arbitrary JavaScript,
unknown actions, unsealed requirements, mixed scenarios, mixed execution
methods, and foreign targets are rejected before Browser Harness starts.

## Execution

An availability probe runs `browser-harness --version`. A real Evidence run
starts Browser Harness once and sends one generated Python program over stdin.
The capability manifest can allow the Browser Harness connection variables,
including `BU_NAME`, `BU_CDP_URL`, and `BU_CDP_WS`. Planr binds the captured
connection configuration to the capability instance and forwards it through
the process environment only when the host independently names each variable in
`PLANR_ADAPTER_ENV_ALLOWLIST`. If no connection variable is present, Browser
Harness uses its configured default connection. Planr does not infer or start a
browser product. A host may additionally classify local, non-secret connection
configuration in `PLANR_ADAPTER_PUBLIC_ENV_ALLOWLIST`. Remote endpoints or
credentials remain non-public unless the host explicitly decides otherwise.

The program:

1. Opens one blank tab and enables Page, Runtime, Network, and Log observation.
2. Navigates to the exact Planr-bound HTTP or HTTPS target.
3. Executes the one sealed scenario in that tab.
4. At each checkpoint, stabilizes and evaluates all named requirements
   together for at most 750 ms.
5. Applies one 15-second total scenario deadline; it never applies a separate
   timeout per requirement.
6. Returns one structured result containing exactly one actual for every
   sealed requirement.
7. Closes the tab on every exit path.

The generated program inserts scenario and requirement data only as JSON. A
requirement cannot add executable Python or JavaScript.

## Result and trust

Each actual conforms to
`docs/contracts/schemas/com.planr.web.dom_state.v2.schema.json`. Planr Core
validates the schema and evaluates the expected predicate independently for
each requirement. One failed checkpoint observation therefore produces a
trusted non-passing receipt without discarding passing siblings.

If every requirement selects the `browser-harness` agent skill, the adapter
also returns the matching supervised invocation record. This record becomes
trusted only after Planr validates the observed process and every sealed
binding. It never assigns provenance or coverage itself.

## Capture and resource policy

The successful path disables Browser Harness recordings and does not capture
screenshots or traces. Structured DOM, storage, selection, accessibility,
layout, runtime, and request observations are the retained proof for this
non-visual capability. The adapter uses one tab and one Browser Harness process
per batch.

The adapter does not advertise a visual observation type. Screenshots cannot
upgrade this capability into visual Evidence.

## Version transition

V2 is a hard cut. The active adapter accepts only
`schema://com.planr.web.dom_state.v2` and the exact checkpoint scenario. It has
no runtime fallback for v1 shared actions followed by independent five-second
polls. Historical v1 receipts remain immutable records; they do not create a
second active adapter path.

## Canonical live fixture

`tests/fixtures/evidence/browser-harness/v2/form.html` and
`form-request.json` exercise the provider-free live boundary: one existing HTTP
target, one Browser Harness session, bounded label-driven actions, one explicit
checkpoint, one structured DOM postcondition, and no model provider.
