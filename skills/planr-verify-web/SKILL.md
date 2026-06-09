---
name: planr-verify-web
description: Live verification for web features. Use when a web map item needs proof it actually runs in a browser before review, including planr-loop step 4. Discovers the host's existing browser capability, runs the changed flow against the dev server, and logs replayable evidence. Ships no browser tooling of its own.
---

# Planr Verify Web

Prove the feature runs. Planr owns the evidence contract; the host owns the browser tooling. Never install or configure browser infrastructure on behalf of this skill.

## Capability Discovery

Check project memory first:

```bash
planr context list
```

If a `capability`-tagged entry records a web verification tool, use it. Otherwise discover what the host has, strongest first:

1. A browser skill the host already provides (for example browser-harness): drive the real flow, screenshot the result.
2. A browser MCP already configured (Playwright, chrome-devtools, native browser tools): same.
3. A scriptable fallback when Node is available: a one-off headless check via `npx playwright` whose exit code is the signal.
4. HTTP-level checks (`curl` against rendered routes or endpoints): weakest tier, only for SSR/API-shaped changes, and the log must say "HTTP-level only, not browser-verified".

Record the decision once so later iterations and other agents reuse it instead of re-discovering:

```bash
planr context add "web verification: <tool>, invoke via <how>" --tag capability
```

A human may pin the capability upfront with the same command; a pinned context always wins over discovery.

## Dev Server

Detect a running dev server before anything else and use it. Never start a second instance. Only start one (in the background, and stop it afterwards) when none is running and the loop is unattended.

## Run The Verification

Exercise the flow the item changed — not the homepage. Interact, assert on rendered output, capture a screenshot when the tier supports it.

Then log evidence on the item:

```bash
planr log add --item <item-id> \
  --summary "live verification (<tier>): <what was exercised and observed>" \
  --cmd "<exact replayable command>"
```

Attach screenshots or traces as artifacts on the item:

```bash
planr artifact add "verify-web screenshot" --item <item-id> --path <screenshot-path> --kind screenshot
```

The replay command is mandatory. The reviewer reruns it instead of trusting this run; a verification that cannot be replayed is not evidence.

## When Verification Is Impossible

No capability, no Node, no reachable server: do not fake it and do not downgrade silently.

```bash
planr context add "live verification blocked: <missing capability>" --item <item-id> --tag blocker
planr approval request <item-id> --reason "manual live verification required"
```

Then pause (or let `planr-loop` pause) until a human resolves it.

## Outcome

- Pass: evidence logged, proceed to `planr review request <item-id>`.
- Fail: the feature does not work — log the failing command and observed behavior, then fix under the same item before requesting review. A failed live run is a finding against the implementation, never a reason to weaken the check.
