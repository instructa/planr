# Planr verification model

Planr is not a loop engine. Hosts such as Codex, Claude Code, Cursor, MCP clients, or plain CLI scripts can drive the loop. Planr owns durable state, verification points, evidence, review gates, approval state, and recovery hints.

## Evidence classes

Planr now distinguishes these classes:

- Agent-authored evidence: a normal `log add --kind verification` entry. It is useful narrative, but it is not treated as deterministic pass or fail evidence in strict modes.
- Replayable evidence: evidence that contains an exact command or artifact path that another worker can challenge later.
- Planr-executed evidence: evidence created by `planr verify run`. Planr executes the command, stores exit code, stdout and stderr summaries, assertion results, timing, run id, and capability state.
- Browser evidence: a Planr-executed command marked `--kind browser`. Playwright, CDP scripts, browser-harness tools, or host-native browser commands are strong when they include machine-checkable assertions. Screenshot-only checks remain weak.
- Test evidence: Planr-executed unit, integration, e2e, lint, typecheck, or custom commands.
- Git evidence: adaptive audit evidence. Git is optional globally. Audit can inspect it when present and can gate on it only when an opt-in policy asks for that.
- Human evidence: review findings, approvals, and manual artifact references. Human evidence can approve risk, but it does not replace deterministic verification in strict autonomous modes.

Heartbeat is liveness only. It is never verification.

## Commands

Create acceptance-specific verification points:

```sh
planr verify point add <item-id> --text "leaderboard submits the current failed-run score, not bestScore" --kind browser --source acceptance:score
planr verify point list --plan <plan-id>
```

Run deterministic verification:

```sh
planr verify run <item-id> \
  --kind test \
  --cmd "cargo test" \
  --assert-stdout-contains "test result" \
  --timeout-seconds 120
```

Run browser verification through any available project or host tool:

```sh
planr verify run <item-id> \
  --kind browser \
  --cmd "npx playwright test tests/score.spec.ts" \
  --point <point-id>
```

Replay evidence:

```sh
planr verify replay <evidence-id>
```

Audit a plan with stronger autonomous gates:

```sh
planr plan audit <plan-id> --strict --require-verification-points --git-policy auto
planr plan audit <plan-id> --autonomous --git-policy require-clean
```

## Audit modes

Default mode is backwards compatible. It reports Planr-executed evidence when available, but it does not force every small CLI use into a heavy ritual.

Strict mode requires at least one Planr-executed passing verification record. If `--require-verification-points` is set, all required verification points must pass.

Autonomous mode is strict mode with an explicit signal that a host loop is driving work. It is intended for `planr-loop` and similar skills.

Degraded capability mode is represented as evidence status `blocked`. A missing browser tool, missing Git, missing secrets, or unavailable external service is recorded as blocked capability instead of pretending the check passed.

## Git policy

- `off`: do not inspect Git.
- `auto`: inspect Git when `.git` and the `git` executable are available, but never gate on it.
- `require-clean`: fail audit when Git is missing or the working tree is dirty.
- `require-scoped`: same gate as clean in the first implementation. Higher-level loops can evolve this into file-scope ownership without making Git a global requirement.

## Browser verification

Strong browser evidence is a Planr-executed browser command with machine-checkable assertions. Playwright tests, CDP scripts, browser-harness commands, or host-native browser tools count when the command exits deterministically and assertions are recorded.

Weak browser evidence includes screenshots without assertions or prose-only observations. Weak evidence can be useful for review, but it does not satisfy strict autonomous completion by itself.

Blocked browser evidence records that the browser capability was requested but unavailable. The loop should report the blocked capability and ask for manual approval or a fallback path.
