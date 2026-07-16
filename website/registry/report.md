# Preset Evaluation Verification

Fixture: `planr-preset-suite` v1.8.0 (`733c94e049d784eec2fa222f60c230e3f185ccefc84c23c313b1d7e85b60ab22`)  
Verified: 1783987200 / expires: 1815523200 / evaluated: 1783987200  
Runner: planr-instrumented-live-host-runner schema 1 / Planr 1.3.0 / macos aarch64

This report joined Planr-read challenge-bound task artifacts with independently signed Ed25519 telemetry receipts. Verified receipts provide trusted effective-route and usage measurements; missing or invalid receipts remain recommendation-ineligible.

| Candidate | Status | Source | Quality | Reliability | Avg cost | p95 latency | Hashes | Label |
|---|---|---|---:|---:|---:|---:|---:|---|
| `balanced-codex-openai` | `Recommended` | `TrustedTelemetry` | 10000 bps | 10000 bps | 100 μcredits | 228 ms | 7/7 | recommended |
| `low-usage-codex-openai` | `Unverified` | `TrustedTelemetry` | 9663 bps | 4285 bps | 100 μcredits | 197 ms | 7/7 | — |
| `max-quality-codex-openai` | `Recommended` | `TrustedTelemetry` | 10000 bps | 10000 bps | 100 μcredits | 171 ms | 7/7 | recommended |
| `read-only-audit-codex-openai` | `Unverified` | `TrustedTelemetry` | 9831 bps | 7142 bps | 100 μcredits | 169 ms | 7/7 | — |

## Versioned task inputs

- `Exploration` `explore-routing-boundaries` v1.0.0: `277f2f9155f85365324c6ea6cd717baa47bc9c482446217b30d9f94c6d6684df`
- `Implementation` `implement-bounded-policy-change` v1.0.0: `f4704037e6a63746e100d31aa98843adeca458fdb336a3317565b62693d676d2`
- `Mechanical` `mechanical-schema-rewrite` v1.0.0: `96882c36d71af8c0637a17cfb47db66dbef0fbd170c963ba500c8d9f23d48a7a`
- `Browser` `browser-report-smoke` v1.0.0: `fc453ccf8422fa0093edb60b0162e83982e005b07ee569de5dff222b13319827`
- `Visual` `visual-report-regression` v1.0.0: `d71938957b10cbcf8dd67fde7e43853ba66fc23ab3b436fa3287caa6a2b5a1f3`
- `Security` `security-safety-stop` v1.0.0: `dd439a0094897e7ebed01c301fed542e910e8e2b7d6b2a5942c1c44b47082e7f`
- `Subagent` `subagent-sol-luna-dispatch` v1.0.0: `38372ba41431ccf4f2e9b56ea5f93db55e52156e3fa1c2956bf3dca15d63654f`

## Codex Sol/Luna contract

- [x] `fork_turns = all` rejected
- [x] `fork_turns = none` parameters verified
- [x] missing effective model/effort cannot verify
- [x] process-exit effective route evidence verifies

Reproducible evidence: **fail**.
