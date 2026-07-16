# Preset Evaluation Verification

Fixture: `planr-preset-suite` v1.8.0 (`042dbb3ff8569bd56502777d79bd622d00d4f2ae06e4dbb6b3d08bf3af807a21`)
Verified: 1783987200 / expires: 1815523200 / evaluated: 1784160000
Runner: planr-offline-policy-simulator schema 1 / Planr 1.4.0 / macos aarch64

This command is an offline policy simulation: it does not execute task workflows or observe effective host routes, so it cannot produce recommendations.

| Candidate | Status | Source | Quality | Reliability | Avg cost | p95 latency | Hashes | Label |
|---|---|---|---:|---:|---:|---:|---:|---|
| `balanced-codex-openai` | `Verified` | `EstimatedProjection` | 10000 bps | 10000 bps | 1067142 μcredits | 4800 ms | 7/7 | — |
| `low-usage-codex-openai` | `Verified` | `EstimatedProjection` | 8857 bps | 4285 bps | 711428 μcredits | 3200 ms | 7/7 | — |
| `max-quality-codex-openai` | `Verified` | `EstimatedProjection` | 10000 bps | 10000 bps | 1422857 μcredits | 6400 ms | 7/7 | — |
| `read-only-audit-codex-openai` | `Verified` | `EstimatedProjection` | 9428 bps | 7142 bps | 1067142 μcredits | 4800 ms | 7/7 | — |

## Versioned task inputs

- `Exploration` `explore-routing-boundaries` v1.0.0: `277f2f9155f85365324c6ea6cd717baa47bc9c482446217b30d9f94c6d6684df`
- `Implementation` `implement-bounded-policy-change` v1.0.0: `f4704037e6a63746e100d31aa98843adeca458fdb336a3317565b62693d676d2`
- `Mechanical` `mechanical-schema-rewrite` v1.0.0: `96882c36d71af8c0637a17cfb47db66dbef0fbd170c963ba500c8d9f23d48a7a`
- `Browser` `browser-report-smoke` v1.0.0: `fc453ccf8422fa0093edb60b0162e83982e005b07ee569de5dff222b13319827`
- `Visual` `visual-report-regression` v1.0.0: `d71938957b10cbcf8dd67fde7e43853ba66fc23ab3b436fa3287caa6a2b5a1f3`
- `Security` `security-safety-stop` v1.0.0: `dd439a0094897e7ebed01c301fed542e910e8e2b7d6b2a5942c1c44b47082e7f`
- `Subagent` `subagent-sol-terra-luna-dispatch` v1.0.0: `4cdaf5ee82144acbd136a1588f8a0425e811606a011e84972b98a3f7ddcdf703`

## Codex Sol/Terra/Luna contract

- [x] `fork_turns = all` rejected
- [x] `fork_turns = none` parameters verified
- [x] missing effective model/effort cannot verify
- [x] process-exit effective route evidence verifies

Reproducible evidence: **pass**.
