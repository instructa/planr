# Switchloom Compatibility

This report records Planr-side compatibility evidence for the exact public `switchloom@0.3.2` package. It is intentionally narrow: Planr verifies the package identity, repository-local declaration boundary, and its own consumption/proof semantics. It does not turn Switchloom internals, generated semantic role names, routes, doctor output, or runtime telemetry into a future contract.

## Exact v0.3.2 Behavior Verified

- Public npm metadata for `switchloom@0.3.2` resolved from the npm registry:
  - `dist.integrity`: `sha512-g96AZIFKXpG1toAO+Gri1sjD8q0SxFxtRSLAcRcSVRGDCZ/dUtERepsOB+cSHHH7hUsT0jSKZYPfNqiLvfKk9Q==`
  - `dist.shasum`: `d7d72c74ac3ecd5a3e355edd8e297284cba04403`
  - `dist.tarball`: `https://registry.npmjs.org/switchloom/-/switchloom-0.3.2.tgz`
- `npm pack switchloom@0.3.2 --pack-destination /private/tmp --json` returned a 9-entry package with the expected native binaries, wrapper, provenance file, README, license, and package manifest.
- The packed tarball at `/private/tmp/switchloom-0.3.2.tgz` had SHA256 `0c04e94fc4372845edf395b3ea51139b8a4b46f34404940e06b3f4ec3ce22d20`.
- The reviewed oracle contract expects the v0.3.2 Planr integration bundle `balanced-codex-openai@1.0.0+2.0.0`, bundle SHA256 `bf48f502080ff444ccb67bc4eeacc9391e77dbf5f0f8f277814e9abc2443e6c8`, 10 managed artifacts, 7 Codex-native roles, and 5 Planr work-type routes.
- Repository docs now document the safe external operator flow with explicit `switchloom@0.3.2` commands, no normal-guidance `apply --yes`, and a caveat that the v0.3.2 tagged README preserves stale `0.3.1` examples.
- The immutable live oracle used isolated source clone `/private/tmp/planr-switchloom-source.6gbzx2/source` and completed with `SWITCHLOOM_SOURCE_ROOT=/private/tmp/planr-switchloom-source.6gbzx2/source SWITCHLOOM_TARBALL=/private/tmp/switchloom-0.3.2.tgz PLANR_ORACLE_TEMP_PARENT=/private/tmp npm run verify:switchloom-cross-product`, exit 0. The live root is `/private/tmp/planr-switchloom-cross-product-Lbluca`, with oracle receipt `/private/tmp/planr-switchloom-cross-product-Lbluca/oracle-receipt.json`.
- The retained replay of the same root and isolated source clone exited 0 and wrote `/private/tmp/planr-switchloom-cross-product-Lbluca/replay-receipt.json`. Replay first asserts the current source fingerprint matches the retained live receipt, then separately verifies the source remains unchanged during replay.
- The live and replay receipts bind the same Switchloom source fingerprint before and after each mode: HEAD `8ff05776085d3175211e080143c513c5173abaa4`, clean status SHA256 `e3b0c442...`, inventory SHA256 `790de1be...`, file-hash SHA256 `97a26b12...`, and 303 paths. The maintainer's original sibling Switchloom worktree was not modified for this proof.
- The successful live oracle proves exact package tarball SHA256 `0c04e94fc4372845edf395b3ea51139b8a4b46f34404940e06b3f4ec3ce22d20`, bundle SHA256 `bf48f502080ff444ccb67bc4eeacc9391e77dbf5f0f8f277814e9abc2443e6c8`, 7 profiles, 5 routes, 10 managed artifacts, Planr declaration consumption, Planr audit, uninstall and unrouted Planr behavior, no-auth fail-closed behavior, requested-only rejection, global sentinel preservation, and routing ownership.
- The successful live oracle proves separate native maker and reviewer execution: maker `model_routing_terra_high` on `gpt-5.6-terra` with high effort, reviewer `model_routing_sol_high` on `gpt-5.6-sol` with high effort. Hidden spawn messages are intentionally opaque; the proof is exact successful parent `agent_type`, `task_name`, and `fork_turns` metadata correlated with matching direct child role rollouts, repo-local skill reads, and item-specific Planr evidence. It does not claim plaintext hidden-message recovery.
- One malformed parent `spawn_agent` attempt in the retained live root was retained as a diagnostic and not counted as a successful spawn. The oracle still requires exactly two successful routed spawns and still rejects default, wrong-role, missing-role, or extra successful child execution.
- A separate fresh root, `/private/tmp/planr-switchloom-cross-product-gHtZQI`, was correctly rejected: Codex claimed `spawn_agent` was unavailable and directly handled the work, producing 0 successful routed spawns. That run is negative evidence for host stochastic failure detection, not a passing compatibility run.

## Stable Planr Boundary

- Planr Core is provider-neutral. It reads `.planr/agents.toml`, `.planr/policy.toml`, and route-audit evidence, but does not install, invoke, compile, apply, or uninstall Switchloom output.
- `.planr/agents.toml` profiles, routes, model names, efforts, role names, and fallback chains are opaque data to Planr. Planr may place requested routing data in pick packets, but requested-only values are not effective execution proof.
- `--profile` records the profile a worker reports for a run. Genuine runtime proof belongs in route-audit evidence that separates requested, host-resolved, and effective dimensions.
- Missing effective host evidence remains explicitly unavailable. Planr must not infer it from generated declarations, policy files, or worker claims.
- External lifecycle remains external: Switchloom owns compile, apply, generated host roles, host reload requirements, and uninstall. Planr validates its own consumption with `planr agents check` and records evidence through logs/reviews.
- Planr works without routing declarations. After an external uninstall, unrouted Planr fallback behavior remains the stable Core contract.

## Not Guaranteed For Future Switchloom Work

Future Switchloom work, including thread `019f8a71-5b6c-7c41-9850-7050516fcee4`, may change semantic role names, route names, doctor output, generated artifacts, runtime telemetry, and host-specific evidence shape. Planr must not contract those as stable.

The stable compatibility promise is limited to Planr's boundary: consume provider-neutral declarations when present, keep requested values separate from effective evidence, reject requested-only metadata as proof, avoid owning external lifecycle, and continue operating when routing files are absent.

Any future mismatch in package identity, generated artifacts, Planr declaration parsing, route-audit semantics, host evidence, source-worktree mutation, or security posture should become a new finding or fix item rather than an optimistic compatibility claim.

## Verification Notes

- Rust formatting, clippy, full serial cargo tests, e2e, eval-contract, and routing-ownership tests passed when run outside the sandbox where HTTP/process tests are permitted.
- Docs reference generation, typecheck, reference verification, maintenance verification, and Node 22 production build passed. Node 26 production docs builds hung in the optimized Next build phase and were terminated; the Node 22 build is the relevant repository runtime.
- npm package dry-run passed using a temp npm cache because the user npm cache contains root-owned files.
- Privacy checks passed. The docs app Next.js runtime was upgraded from `16.2.10` to `16.2.11` in `apps/docs/package.json` and `pnpm-lock.yaml`; the lockfile change is limited to Next core/env/SWC and mechanically recalculated peer snapshots. `npm run security:check` passed with BetterLeaks reporting no leaks and Trivy reporting 0 vulnerabilities for both `Cargo.lock` and `pnpm-lock.yaml`.
- The immutable source clone proof supersedes earlier retained-replay-only evidence. Earlier concurrent external mutation and missing-receipt attempts were treated as real failures and are not counted as passing evidence.
