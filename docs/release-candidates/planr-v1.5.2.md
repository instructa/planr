# Planr v1.5.2 Release Candidate Evidence

Candidate produced on 2026-07-20 in an isolated local workspace:

- Workspace: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2`
- Source clone: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/release-candidate-src`
- Local bare remote: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/local-origin.git`
- Config-proof rerun source clone: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/release-candidate-src`
- Config-proof rerun local bare remote: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/local-origin.git`
- Config-proof hash snapshots: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/evidence`
- Public side effects: none. The candidate clone's `origin` points to the local bare remote, and the public `origin` check from this repository returned no `v1.5.2` tag.

## Identity

- Upstream release-scope base commit: `f7763c6197ddf8cdd93227b9aa320f2b168a0523`
- Clean pre-release source commit: `9390af541ca51755e585bf3c1a652717997daeca`
- Clean pre-release source tree: `2569374e6321a88b812cc706257bbda086101437`
- Release commit: `678038db4536723867fc96b0cd0af703fda43ef1`
- Release tree: `80324b8afef14d7d7bd97950eb8352cd9c667e6a`
- Annotated tag object: `12c78b7a870552361b11c5de768a6c5da7c8f115`
- Tag target: `678038db4536723867fc96b0cd0af703fda43ef1`
- Config-proof rerun release commit: `0d6bab060793fd52a5e144b9862c1fbc77d10ec3`
- Config-proof rerun release tree: `80324b8afef14d7d7bd97950eb8352cd9c667e6a`
- Config-proof rerun annotated tag object: `78c6c99afbca24803d58abb7d9a507cafde40741`
- Config-proof rerun tag target: `0d6bab060793fd52a5e144b9862c1fbc77d10ec3`

## Version Checks

Both runtime entry points reported `planr 1.5.2`:

- `target/release/planr --version`
- `node npm/bin/planr.js --version`

All release manifests carried `1.5.2`:

- `Cargo.toml`
- `package.json`
- `plugins/planr/.codex-plugin/plugin.json`
- `plugins/planr/.claude-plugin/plugin.json`
- `.cursor-plugin/plugin.json`

## Artifact Digests

- Native binary `target/release/planr`: `e91917be04e84f97c5d515e673f829c33b21f5128dae87ce836e3721ece50473`
- npm tarball `planr-1.5.2.tgz`: `33df443e6ec20d6d44fcf05469efe56eb75b192c826467c33d47d44672297a34`
- Native archive `dist/planr-darwin-arm64.tar.gz`: `6fdaf803073d99bdb02522100e50094fb277409d52fc3a0c33e9d8bd6707d8ac`
- Archive checksum file `dist/SHA256SUMS`: `6347103b757e5045aef9a83a311ae2bcc43de5b6e9ada8a9efa2d52b84e3359b`
- Payload checksum file `dist/planr-1.5.2/SHA256SUMS`: `d8ac952c247706bb328e1539583b87e55f9e2b211b89c134aab08357d0cc665c`
- Config-proof rerun native binary `target/release/planr`: `00c1b3b46f66563dfe4c909e2ecb75dc448e72bfe75b6cb7d56d86c6539b5d6c`
- Config-proof rerun npm tarball `planr-1.5.2.tgz`: `33df443e6ec20d6d44fcf05469efe56eb75b192c826467c33d47d44672297a34`
- Config-proof rerun native archive `dist/planr-darwin-arm64.tar.gz`: `dbba6abece2cc37d1b7e038094b8b7483ce79e5c770f8ed64048c84446d17681`
- Config-proof rerun archive checksum file `dist/SHA256SUMS`: `1d77872f58c8ef8877cbed11b16ab0ea30524a6b2a9565f5190dfd059d69416c`
- Config-proof rerun payload checksum file `dist/planr-1.5.2/SHA256SUMS`: `ce2adbe238c9fe27838c11f3a89195d9eb8b262411c86d3a8b02304ee14eaec6`

`dist/SHA256SUMS` records:

```text
6fdaf803073d99bdb02522100e50094fb277409d52fc3a0c33e9d8bd6707d8ac  planr-darwin-arm64.tar.gz
```

`dist/planr-1.5.2/SHA256SUMS` records:

```text
e91917be04e84f97c5d515e673f829c33b21f5128dae87ce836e3721ece50473  planr
a1e27e1cfb3d9d540dfe05f5d1212ee98be97b1f48e203d48209e31243383ba3  README.md
61a9b281d04d62cdea4bef57d5e0b702c191b3219456409ac5ed651569228a30  LICENSE.md
```

## Gates Run

- `scripts/release.sh 1.5.2 'standalone Planr core with optional Switchloom handoff'`
  - `cargo test`: 71 unit tests, 67 e2e tests, and 3 routing ownership tests passed.
  - `npm pack --dry-run`: reported `planr@1.5.2`, 65 files, shasum `a3903ce5658be6f6140321232885f86f6515ae70`.
  - `scripts/security-local.sh`: betterleaks found no leaks; Trivy reported 0 vulnerabilities for `Cargo.lock` and `pnpm-lock.yaml`.
  - Pushed `main` and `v1.5.2` only to `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/local-origin.git`.
- `scripts/build-release.sh`: built `dist/planr-darwin-arm64.tar.gz`.
- `npm pack`: created `planr-1.5.2.tgz` locally; no npm publish was run.
- Config-proof rerun:
  - `scripts/release.sh 1.5.2 'standalone Planr core with optional Switchloom handoff'`: 71 unit tests, 67 e2e tests, and 3 routing ownership tests passed; betterleaks found no leaks; Trivy reported 0 vulnerabilities; pushed only to `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/local-origin.git`.
  - `scripts/build-release.sh`: built `dist/planr-darwin-arm64.tar.gz`.
  - `npm pack`: created `planr-1.5.2.tgz` locally; no npm publish was run.
  - `target/release/planr --version` and `node npm/bin/planr.js --version`: both reported `planr 1.5.2`.
- `git tag -l v1.5.2`: returned no local tag in `/Users/kregenrek/projects/planr`.
- `git ls-remote --heads --tags origin 'v1.5.2'`: returned no public remote ref.
- Candidate clone artifact inventory: no Switchloom files, no legacy routing-owner directories, and no routing manifests were present.

## User Config Hash Proof

No release command targeted user/global host config files. The config-proof rerun captured hashes immediately before and after each release-owned run, stored without file contents:

- `scripts/release.sh`
  - Before: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/evidence/config-before-release-sh.txt`
  - After: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/evidence/config-after-release-sh.txt`
  - Equality proof: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/evidence/config-release-sh-equality.txt` records `config hashes unchanged across scripts/release.sh`.
- `scripts/build-release.sh`
  - Before: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/evidence/config-before-build-release.txt`
  - After: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/evidence/config-after-build-release.txt`
  - Equality proof: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/evidence/config-build-release-equality.txt` records `config hashes unchanged across scripts/build-release.sh`.
- `npm pack`
  - Before: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/evidence/config-before-npm-pack.txt`
  - After: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/evidence/config-after-npm-pack.txt`
  - Equality proof: `/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/config-proof/evidence/config-npm-pack-equality.txt` records `config hashes unchanged across npm pack`.

Every before/after snapshot contains the same hashes:

- `/Users/kregenrek/.codex/config.toml`: `82224997e7d4d5147f892a8aa12f7f5f04e77c38267da6ba1274a9267b23f040`
- `/Users/kregenrek/.claude/settings.json`: `99a0eb17993d0dd0c952df03bec38d284fd119a9b0ed928f3201b1792224049f`
- `/Users/kregenrek/.claude.json`: `a5f901ce3b38943ed2591333b4cb6dbab0cab101f8d02ea348fc476021f55afb`
- `/Users/kregenrek/.cursor/mcp.json`: `cf7217adfcb48b532dfdbced62ca68eab4c60996ef38d510a8efb3d286ce5baf`

## Switchloom v0.2.1 Optional Dogfood Evidence

Fresh non-replay dogfood ran on 2026-07-20 with the release-candidate Planr v1.5.2 binary and exact public Switchloom v0.2.1 bytes. The durable local audit receipt is:

- Receipt: `/private/tmp/planr-switchloom-cross-product-QEvWps/oracle-receipt.json`
- Receipt SHA-256: `c01f4b980977a15028f7a82cc6fc237aa50fb4b35310871ba3ad245ec1c699ee`
- Mode/result: `live`, `ok=true`
- Fresh root: `/private/tmp/planr-switchloom-cross-product-QEvWps`

Replayable command:

```bash
PLANR_WORKER_ID=switchloom_optional_dogfood PLANR_BIN=/Users/kregenrek/projects/planr-dogfood/release-v1.5.2/release-candidate-src/target/release/planr SWITCHLOOM_TARBALL=/private/tmp/switchloom-public-0.2.1.MN008L/switchloom-0.2.1.tgz PLANR_ORACLE_TEMP_PARENT=/private/tmp node scripts/verify-switchloom-cross-product.mjs
```

Public Switchloom package identity:

- Package: `switchloom@0.2.1`
- Tarball: `/private/tmp/switchloom-public-0.2.1.MN008L/switchloom-0.2.1.tgz`
- npm SHA-1 shasum: `e813283f54d0d64b5fd4835e17687aaaf3b0a6cb`
- SHA-512 integrity: `sha512-vUKHxYXHt7Sx7MkYQz5MRZ0Ll544iHoadHGCgvJPUYkpUzQWtzjt1o3xhyeQwExCA6tuLQ5vZnLPz+fO5uMiXg==`
- Local SHA-256: `028176063ce20b4981aa4e13199b25169b2f8296f648eeeec9291e6955e7549a`

Host routing evidence from the receipt:

- Codex CLI: `codex-cli 0.144.5`
- Maker role/model/effort: `model_routing_terra_high`, `gpt-5.6-terra`, `high`
- Reviewer role/model/effort: `model_routing_sol_high`, `gpt-5.6-sol`, `high`
- `fork_turns_all_used`: `false`
- Public Codex JSONL: `/private/tmp/planr-switchloom-cross-product-QEvWps/codex-live.jsonl`
- Parent rollout: `/Users/kregenrek/.codex/sessions/2026/07/20/rollout-2026-07-20T20-05-19-019f80b4-2e89-7470-a432-6937ddcbc7ac.jsonl`
- Maker rollout: `/Users/kregenrek/.codex/sessions/2026/07/20/rollout-2026-07-20T20-05-50-019f80b4-a7d3-73d1-bf34-9762ad7d9474.jsonl`
- Reviewer rollout: `/Users/kregenrek/.codex/sessions/2026/07/20/rollout-2026-07-20T20-06-37-019f80b5-5d7f-7441-97f3-c743d97e0a8c.jsonl`

Fresh Planr audit and uninstall evidence:

- Fresh oracle plan: `pln-773f1d88`
- Fresh oracle item: `i-build-first-slice-5c85`
- Audit result: holds with clauses `items_settled`, `reviews_complete`, `approvals_clear`, and `verification_logged`.
- Post-uninstall result: Switchloom uninstall removed only managed files and unrouted Planr still worked; the script reran the Planr audit after uninstall and required `holds=true`.
- Requested-only routing metadata was rejected as effective proof: `/private/tmp/planr-switchloom-cross-product-QEvWps/requested-only-route-audit.json`.

Protected configuration and source inventory proof:

- `/Users/kregenrek/.codex/config.toml`: `82224997e7d4d5147f892a8aa12f7f5f04e77c38267da6ba1274a9267b23f040`
- `/Users/kregenrek/.claude/settings.json`: `99a0eb17993d0dd0c952df03bec38d284fd119a9b0ed928f3201b1792224049f`
- `/Users/kregenrek/.claude.json`: `a5f901ce3b38943ed2591333b4cb6dbab0cab101f8d02ea348fc476021f55afb`
- `/Users/kregenrek/.cursor/mcp.json`: `cf7217adfcb48b532dfdbced62ca68eab4c60996ef38d510a8efb3d286ce5baf`
- Switchloom source root: `/Users/kregenrek/projects/model-routing`
- Switchloom source HEAD: `191f6aac91ff0f73fcae68ab269eb2d89c9e76e9`
- Switchloom source status SHA-256: `2ca939e3e14ba8971210e93152120323641d21b0ac465b02a85335860b01758c`
- Switchloom tracked files SHA-256: `0292bca86e80e9c63d5848984cab229b17df6e1162f1b0d8ec577474a9ac3591`

This section records paths, digests, roles, and verification outcomes only. It does not copy rollout contents, prompts, responses, credentials, or user configuration contents into the repository.
