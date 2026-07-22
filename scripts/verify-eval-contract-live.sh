#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${PLANR_BIN:-$ROOT/target/debug/planr}"
NODE_BIN="${NODE_BIN:-$(command -v node)}"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/planr-eval-live.XXXXXX")"
FRESH="$TMP/fresh-repo"
IMPORT_REPO="$TMP/import-repo"
DB="$FRESH/.planr/planr.sqlite"
PACKAGE="$TMP/eval-live-package.json"
COMPARISONS="$TMP/comparisons.jsonl"
CONFIG_BEFORE="$TMP/config-before.txt"
CONFIG_AFTER="$TMP/config-after.txt"

hash_file() {
  if [ -f "$1" ]; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    printf 'absent'
  fi
}

metadata_file() {
  if [ -e "$1" ]; then
    stat -f 'exists type=%HT mode=%Lp size=%z mtime=%m' "$1"
  else
    printf 'absent'
  fi
}

fingerprint_path() {
  local scope="$1" path="$2" mode="${3:-hash}"
  if [ "$mode" = "metadata" ]; then
    printf '%s %s %s\n' "$scope" "$path" "$(metadata_file "$path")"
  else
    printf '%s %s sha256=%s\n' "$scope" "$path" "$(hash_file "$path")"
  fi
}

fingerprint_dir_metadata() {
  local scope="$1" path="$2" depth="${3:-6}"
  if [ -d "$path" ]; then
    local tree_hash
    tree_hash="$(
      {
        find "$path" -maxdepth "$depth" \
          \( -name node_modules -o -name target -o -name .git -o -name Cache -o -name caches -o -name cache \) -prune \
          -o -print0 2>/dev/null | LC_ALL=C sort -z | while IFS= read -r -d '' entry; do
          local rel meta content_hash size
          rel="${entry#"$path"/}"
          [ "$rel" = "$entry" ] && rel="."
          meta="$(metadata_file "$entry")"
          if [ -f "$entry" ]; then
            size="$(stat -f '%z' "$entry" 2>/dev/null || printf '0')"
            if [ "$size" -le 262144 ]; then
              content_hash="$(hash_file "$entry")"
              printf '%s\t%s\tsha256=%s\n' "$rel" "$meta" "$content_hash"
            else
              printf '%s\t%s\tcontent-sha256=skipped-large-file\n' "$rel" "$meta"
            fi
          else
            printf '%s\t%s\n' "$rel" "$meta"
          fi
        done
      } | shasum -a 256 | awk '{print $1}'
    )"
    printf '%s %s recursive-metadata-content-sha256=%s\n' "$scope" "$path" "$tree_hash"
  else
    printf '%s %s absent\n' "$scope" "$path"
  fi
}

fingerprint_command() {
  local scope="$1"
  shift
  local output
  if output="$("$@" 2>/dev/null)"; then
    printf '%s command=%q output-sha256=%s\n' "$scope" "$*" "$(printf '%s' "$output" | shasum -a 256 | awk '{print $1}')"
  else
    printf '%s command=%q unavailable\n' "$scope" "$*"
  fi
}

fingerprint_scoped_config() {
  fingerprint_path planr "$ROOT/.planr/agents.toml"
  fingerprint_path planr "$ROOT/.planr/policy.toml"
  fingerprint_path codex "$HOME/.codex/config.toml"
  fingerprint_path codex "$HOME/.codex/auth.json" metadata
  fingerprint_dir_metadata codex "$HOME/.codex/plugins" 4
  fingerprint_path claude "$HOME/.claude/settings.json"
  fingerprint_path claude "$HOME/.claude.json"
  fingerprint_path claude "$HOME/.claude" metadata
  fingerprint_dir_metadata claude "$HOME/.claude/agents" 5
  fingerprint_dir_metadata claude "$HOME/.claude/commands" 5
  fingerprint_dir_metadata claude "$HOME/.claude/plugins" 5
  fingerprint_path cursor "$HOME/.cursor/mcp.json"
  fingerprint_path cursor "$HOME/.cursor/settings.json"
  fingerprint_path cursor "$HOME/.cursor" metadata
  fingerprint_dir_metadata cursor "$HOME/.cursor/plans" 5
  fingerprint_dir_metadata cursor "$HOME/.cursor/plugins" 4
  fingerprint_path shell "$HOME/.zshrc"
  fingerprint_path shell "$HOME/.zprofile"
  fingerprint_path shell "$HOME/.bashrc"
  fingerprint_path shell "$HOME/.bash_profile"
  fingerprint_path shell "$HOME/.profile"
  fingerprint_path shell "$HOME/.config/fish/config.fish"
  fingerprint_path git "$HOME/.gitconfig"
  fingerprint_path git "$ROOT/.git/config"
  fingerprint_command git git config --global --list --show-origin
  fingerprint_path credentials "$HOME/.ssh/config" metadata
  fingerprint_dir_metadata credentials "$HOME/.ssh" 3
  fingerprint_path credentials "$HOME/.netrc" metadata
  fingerprint_path credentials "$HOME/.git-credentials" metadata
  fingerprint_path credentials "$HOME/.config/gh/hosts.yml" metadata
  fingerprint_command keychain security default-keychain
  fingerprint_command keychain security list-keychains
  printf 'xdg env XDG_CONFIG_HOME=%s XDG_DATA_HOME=%s XDG_STATE_HOME=%s XDG_CACHE_HOME=%s\n' \
    "${XDG_CONFIG_HOME:-}" "${XDG_DATA_HOME:-}" "${XDG_STATE_HOME:-}" "${XDG_CACHE_HOME:-}"
  local xdg_config="${XDG_CONFIG_HOME:-$HOME/.config}"
  local xdg_data="${XDG_DATA_HOME:-$HOME/.local/share}"
  local xdg_state="${XDG_STATE_HOME:-$HOME/.local/state}"
  fingerprint_path xdg "$xdg_config" metadata
  fingerprint_dir_metadata xdg "$xdg_config/fish" 5
  fingerprint_dir_metadata xdg "$xdg_config/gh" 5
  fingerprint_dir_metadata xdg "$xdg_config/codex" 5
  fingerprint_dir_metadata xdg "$xdg_config/claude" 5
  fingerprint_path xdg "$xdg_config/cursor" metadata
  fingerprint_path xdg "$xdg_data" metadata
  fingerprint_dir_metadata xdg "$xdg_data/planr" 5
  fingerprint_dir_metadata xdg "$xdg_data/codex" 5
  fingerprint_path xdg "$xdg_state" metadata
  fingerprint_dir_metadata xdg "$xdg_state/planr" 5
  fingerprint_dir_metadata xdg "$xdg_state/codex" 5
}

compact_json() {
  node -e 'let d="";process.stdin.on("data",c=>d+=c);process.stdin.on("end",()=>process.stdout.write(JSON.stringify(JSON.parse(d))))'
}

json_field() {
  local expr="$1"
  node -e "let raw='';process.stdin.on('data',c=>raw+=c);process.stdin.on('end',()=>{const d=JSON.parse(raw);process.stdout.write(String(($expr)(d)))})"
}

compare() {
  local label="$1" base="$2" cand="$3" expected_code="$4"
  shift 4
  set +e
  local out
  out=$("$BIN" --db "$DB" --json eval compare "$base" "$cand" "$@")
  local code=$?
  set -e
  if [ "$code" -ne "$expected_code" ]; then
    printf 'compare %s expected exit %s got %s\n%s\n' "$label" "$expected_code" "$code" "$out" >&2
    exit 1
  fi
  printf '%s\t%s\n' "$label" "$(printf '%s' "$out" | compact_json)" >> "$COMPARISONS"
}

expect_failure() {
  local label="$1"
  shift
  set +e
  local output
  output="$("$@" 2>&1)"
  local code=$?
  set -e
  if [ "$code" -eq 0 ]; then
    printf 'expected failure for %s but command passed\n%s\n' "$label" "$output" >&2
    exit 1
  fi
}

cargo build --quiet --manifest-path "$ROOT/Cargo.toml"
fingerprint_scoped_config > "$CONFIG_BEFORE"

mkdir -p "$FRESH"
cp -R "$ROOT/examples" "$FRESH/examples"
cd "$FRESH"
git init -q
"$BIN" --db "$DB" project init "Eval Contract Live Oracle" >/dev/null
EVIDENCE_ITEM=$("$BIN" --db "$DB" --json item create "Eval evidence owner" --description "live CLI evidence owner" | json_field 'd => d.item.id')
LOG_ID=$("$BIN" --db "$DB" --json log add --item "$EVIDENCE_ITEM" --kind verification --summary "live eval oracle seed log" | json_field 'd => d.log.id')

node - "$FRESH/examples/eval/planr-lifecycle-smoke.suite.json" "$NODE_BIN" <<'NODE'
const fs = require('fs');
const crypto = require('crypto');
const [suitePath, nodeBin] = process.argv.slice(2);
const suite = JSON.parse(fs.readFileSync(suitePath, 'utf8'));
for (const testCase of suite.cases) {
  testCase.subject.argv[0] = nodeBin;
}
function normalize(value) {
  if (Array.isArray(value)) return value.map(normalize);
  if (value && typeof value === 'object') {
    const out = {};
    for (const key of Object.keys(value).sort()) out[key] = normalize(value[key]);
    return out;
  }
  return value;
}
const digestSource = JSON.parse(JSON.stringify(suite));
delete digestSource.digest;
suite.digest = `sha256:${crypto.createHash('sha256').update(JSON.stringify(normalize(digestSource))).digest('hex')}`;
fs.writeFileSync(suitePath, JSON.stringify(suite, null, 2));
NODE
"$BIN" --db "$DB" --json eval suite-check --input "$FRESH/examples/eval/planr-lifecycle-smoke.suite.json" >/dev/null

node - "$TMP" "$FRESH/examples/eval/planr-lifecycle-smoke.suite.json" "$NODE_BIN" <<'NODE'
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const [tmp, suitePath, nodeBin] = process.argv.slice(2);
const suite = JSON.parse(fs.readFileSync(suitePath, 'utf8'));
function normalize(value) {
  if (Array.isArray(value)) return value.map(normalize);
  if (value && typeof value === 'object') {
    const out = {};
    for (const key of Object.keys(value).sort()) out[key] = normalize(value[key]);
    return out;
  }
  return value;
}
function recomputeDigest(manifest) {
  const source = JSON.parse(JSON.stringify(manifest));
  delete source.digest;
  manifest.digest = `sha256:${crypto.createHash('sha256').update(JSON.stringify(normalize(source))).digest('hex')}`;
  return manifest;
}
const qualitySuite = recomputeDigest({
  schema_version: suite.schema_version,
  suite_id: 'planr-quality-supplied',
  suite_version: suite.suite_version,
  fixtures: suite.fixtures,
  scorers: suite.scorers,
  cases: Array.from({length: 20}, (_, index) => ({
    case_id: `case-q-${index}`,
    fixture_id: suite.fixtures[0].id,
    fixture_ids: [suite.fixtures[0].id],
    scorer_id: suite.scorers[0].id,
    scorer_ids: [`${suite.scorers[0].id}@${suite.scorers[0].version}`],
    measures: ['duration_ms'],
    sampling: {repetitions: 3, warmups: 0, seed: 21 + index, min_successful_samples: 3}
  })),
  comparison_policy: suite.comparison_policy,
  safety: suite.safety
});
fs.writeFileSync(path.join(tmp, 'quality-suite.json'), JSON.stringify(qualitySuite, null, 2));
const meteringSuite = recomputeDigest({
  schema_version: suite.schema_version,
  suite_id: 'planr-metering-supplied',
  suite_version: suite.suite_version,
  fixtures: suite.fixtures,
  scorers: suite.scorers,
  cases: suite.cases.map(testCase => ({
    ...testCase,
    measures: ['wall_time_ms', 'cost_micros']
  })),
  comparison_policy: suite.comparison_policy,
  safety: suite.safety
});
fs.writeFileSync(path.join(tmp, 'metering-suite.json'), JSON.stringify(meteringSuite, null, 2));
const ESTIMATE_PROVENANCE = {
  pricing_reference_id: 'openai-prices',
  pricing_reference_version: '2026-07-01',
  currency: 'USD',
  effective_at: '2026-07-01T00:00:00Z'
};
const ESTIMATE_PROVENANCE_LABEL = `${ESTIMATE_PROVENANCE.pricing_reference_id}@${ESTIMATE_PROVENANCE.pricing_reference_version}`;
function manifestFor() {
  return JSON.parse(JSON.stringify(suite));
}
function input(id, mode, extra = {}) {
  return {
    id,
    suite_digest: suite.digest,
    subject: {kind: 'planr_lifecycle_subject', revision: id, argv: ['examples/eval/subjects/lifecycle-subject.mjs']},
    runner_version: 'bounded-lifecycle-runner-v1',
    testbed_fingerprint: {os: 'live', arch: process.arch, planr_version: 'target-debug'},
    source_state: {commit: id, testbed_id: 'fresh-repo-live', mode},
    repo_root: '.',
    runner_manifest: manifestFor(),
    ...extra
  };
}
for (const [name, body] of [
  ['baseline-runner.json', input('baseline-run', 'baseline')],
  ['better-runner.json', input('better-run', 'better')],
  ['performance-bad-runner.json', input('performance-bad-run', 'worse')],
  ['interrupted-runner.json', input('interrupted-run', 'resume', {subject: {kind: 'planr_lifecycle_subject', revision: 'resume-suite', argv: ['examples/eval/subjects/lifecycle-subject.mjs']}, interrupt_after_cases: 1})],
  ['resumed-runner.json', input('resumed-run', 'resume', {subject: {kind: 'planr_lifecycle_subject', revision: 'resume-suite', argv: ['examples/eval/subjects/lifecycle-subject.mjs']}, resume_of: 'interrupted-run'})],
  ['rescored-baseline-runner.json', input('rescored-baseline-run', 'baseline', {subject: {kind: 'planr_lifecycle_subject', revision: 'baseline-run', argv: ['examples/eval/subjects/lifecycle-subject.mjs']}, source_state: {commit: 'baseline-run', testbed_id: 'fresh-repo-live', mode: 'baseline'}, rescore_of: 'baseline-run'})],
  ['runtime-fixture-mutated-runner.json', input('runtime-fixture-mutated-run', 'baseline')]
]) {
  fs.writeFileSync(path.join(tmp, name), JSON.stringify(body, null, 2));
}
function mutatedInput(id, mutate) {
  const body = input(id, 'baseline');
  mutate(body.runner_manifest);
  fs.writeFileSync(path.join(tmp, `${id}.json`), JSON.stringify(body, null, 2));
}
mutatedInput('altered-cases-runner', manifest => { manifest.cases[0].case_id = 'planr-lifecycle-mutated'; });
mutatedInput('altered-scorer-runner', manifest => { manifest.cases[0].scorer_ids = ['planr-lifecycle-scorer@9.9.9']; });
mutatedInput('altered-fixture-runner', manifest => { manifest.fixtures[0].digest = 'sha256:0000000000000000000000000000000000000000000000000000000000000000'; });
mutatedInput('altered-safety-runner', manifest => { manifest.safety.allow_shell = true; });
mutatedInput('altered-argv-runner', manifest => { manifest.cases[0].subject.argv = [...manifest.cases[0].subject.argv, '--mutated']; });
function samples(values) {
  return values.map((value, index) => ({
    repetition_index: index,
    seed: 21 + index,
    measure: 'duration_ms',
    value,
    unit: 'ms',
    source: 'process',
    metering_basis: 'actual_trusted',
    basis_source: 'process',
    basis_confidence: 'verified'
  }));
}
function caseEvidence(manifest, caseId, values, status = 'pass', qualityPass = true) {
  return {case: {case_id: caseId, scorer_id: manifest.scorers[0].id, scorer_version: manifest.scorers[0].version, fixture_digest: manifest.fixtures[0].digest, status, repetition_count: values.length, assertions: [{kind: 'safety_pass', status: 'pass'}, {kind: 'quality_pass', status: qualityPass ? 'pass' : 'fail'}], sampling: {min_successful_samples: 3}}, samples: samples(values)};
}
function meteredCaseEvidence(manifest, testCase, basis, costs) {
  return {
    case: {
      case_id: testCase.case_id,
      scorer_id: manifest.scorers[0].id,
      scorer_version: manifest.scorers[0].version,
      fixture_digest: manifest.fixtures[0].digest,
      status: 'pass',
      repetition_count: costs.length * 2,
      assertions: [{kind: 'safety_pass', status: 'pass'}, {kind: 'quality_pass', status: 'pass'}],
      sampling: {min_successful_samples: 3}
    },
    samples: costs.flatMap((cost, index) => {
      const attempt = {
        id: `attempt-${basis}-${testCase.case_id}-${index}`,
        attempt_index: 0,
        terminal_status: 'verified_success',
        countable: true,
        effective_client: 'codex',
        effective_provider: 'openai',
        effective_runtime: 'codex-cli',
        effective_model: 'gpt-5.6-terra',
        effective_effort: 'high',
        effective_profile_id: `live-${basis}`,
        profile_config_digest: 'sha256:3333333333333333333333333333333333333333333333333333333333333333',
        runner_harness_version: 'live-supplied-metering-v1',
        outcome: {status: 'verified_success'}
      };
      const costSample = {
        repetition_index: index,
        seed: 900 + index,
        measure: 'cost_micros',
        value: basis === 'unavailable' ? null : cost,
        unit: 'micros',
        source: 'metering',
        metering_basis: basis,
        basis_source: basis === 'actual_trusted' ? 'provider_invoice' : basis,
        basis_confidence: basis === 'actual_trusted' ? 'verified' : basis,
        attempt
      };
      if (basis === 'estimated') {
        costSample.estimate_provenance = {...ESTIMATE_PROVENANCE};
      }
      return [
        {
          repetition_index: index,
          seed: 900 + index,
          measure: 'wall_time_ms',
          value: 50 + index,
          unit: 'ms',
          source: 'process',
          metering_basis: 'actual_trusted',
          basis_source: 'process',
          basis_confidence: 'verified',
          attempt
        },
        costSample
      ];
    })
  };
}
function suiteCases(values, status = 'pass', qualityPass = true) {
  return suite.cases.map(testCase => caseEvidence(suite, testCase.case_id, values, status, qualityPass));
}
function meteredCases(basis, costs) {
  return meteringSuite.cases.map(testCase => meteredCaseEvidence(meteringSuite, testCase, basis, costs));
}
function supplied(id, cases, extra = {}, manifest = suite) {
  return {id, suite_digest: manifest.digest, subject: {kind: 'planr_lifecycle_subject', revision: id, argv: ['examples/eval/subjects/lifecycle-subject.mjs']}, runner_version: 'supplied-adversarial-v1', testbed_fingerprint: {os: 'live', arch: process.arch, planr_version: 'target-debug'}, source_state: {commit: id, testbed_id: 'fresh-repo-live'}, status: 'success', cases, ...extra};
}
const qualityCases = qualitySuite.cases.map((testCase, index) => caseEvidence(qualitySuite, testCase.case_id, [100, 101, 99], 'pass', index >= 8));
const qualityBaseCases = qualitySuite.cases.map(testCase => caseEvidence(qualitySuite, testCase.case_id, [100, 101, 99], 'pass', true));
const suppliedRuns = [
  supplied('wrong-fast-run', suiteCases([1, 1, 1], 'fail')),
  supplied('no-material-baseline-run', suiteCases([100, 101, 99])),
  supplied('same-run', suiteCases([98, 99, 97])),
  supplied('quality-base-run', qualityBaseCases, {}, qualitySuite),
  supplied('quality-bad-run', qualityCases, {}, qualitySuite),
  supplied('under-covered-run', []),
  supplied('under-sampled-run', suiteCases([1])),
  supplied('no-data-run', suiteCases([])),
  supplied('metering-actual-run', meteredCases('actual_trusted', [100, 110, 120]), {}, meteringSuite),
  supplied('metering-estimated-run', meteredCases('estimated', [200, 220, 240]), {}, meteringSuite),
  supplied('metering-unavailable-run', meteredCases('unavailable', [0, 0, 0]), {}, meteringSuite),
  supplied('noisy-run', suiteCases([8, 800, 8, 800, 8])),
  supplied('mismatch-run', suiteCases([1, 1, 1]), {testbed_fingerprint: {os: 'other', arch: process.arch, planr_version: 'target-debug'}}),
  supplied('stale-run', suiteCases([1, 1, 1]))
];
for (const body of suppliedRuns) fs.writeFileSync(path.join(tmp, `${body.id}.json`), JSON.stringify(body, null, 2));
fs.writeFileSync(path.join(tmp, 'supplied-rogue-case-invalid.json'), JSON.stringify(supplied('supplied-rogue-case-run', [caseEvidence(suite, 'case-rogue', [1, 1, 1])]), null, 2));
NODE

"$BIN" --db "$DB" --json eval suite-check --input "$TMP/quality-suite.json" >/dev/null
"$BIN" --db "$DB" --json eval suite-check --input "$TMP/metering-suite.json" >/dev/null

set_subject_state() {
  local mode="$1" counter_path="$2"
  mkdir -p .planr
  node - "$mode" "$counter_path" <<'NODE'
const fs = require('fs');
const [mode, counterPath] = process.argv.slice(2);
fs.writeFileSync('.planr/eval-subject-state.json', JSON.stringify({mode, counter_path: counterPath}, null, 2));
NODE
}

run_runner_input() {
  local mode="$1" counter_path="$2" file="$3"
  set_subject_state "$mode" "$counter_path"
  "$BIN" --db "$DB" --json eval run --input "$file" >/dev/null
}

run_runner_input baseline .planr/eval-lifecycle-counts-baseline.json "$TMP/baseline-runner.json"
run_runner_input better .planr/eval-lifecycle-counts-better.json "$TMP/better-runner.json"
run_runner_input worse .planr/eval-lifecycle-counts-worse.json "$TMP/performance-bad-runner.json"
run_runner_input resume .planr/eval-lifecycle-counts-resume.json "$TMP/interrupted-runner.json"
run_runner_input resume .planr/eval-lifecycle-counts-resume.json "$TMP/resumed-runner.json"

for file in "$TMP"/*-run.json; do
  "$BIN" --db "$DB" --json eval run --input "$file" >/dev/null
done
sqlite3 "$DB" "UPDATE eval_runs SET completed_at = datetime('now', '-240 hours') WHERE id = 'stale-run';"

expect_failure supplied_rogue_case "$BIN" --db "$DB" --json eval run --input "$TMP/supplied-rogue-case-invalid.json"

expect_failure mutated_cases "$BIN" --db "$DB" --json eval run --input "$TMP/altered-cases-runner.json"
expect_failure mutated_scorer "$BIN" --db "$DB" --json eval run --input "$TMP/altered-scorer-runner.json"
expect_failure mutated_fixture "$BIN" --db "$DB" --json eval run --input "$TMP/altered-fixture-runner.json"
expect_failure mutated_safety "$BIN" --db "$DB" --json eval run --input "$TMP/altered-safety-runner.json"
expect_failure mutated_argv "$BIN" --db "$DB" --json eval run --input "$TMP/altered-argv-runner.json"
node - "$TMP/baseline-runner.json" "$TMP" "$FRESH" <<'NODE'
const fs = require('fs');
const [inputPath, tmp, fresh] = process.argv.slice(2);
const input = JSON.parse(fs.readFileSync(inputPath, 'utf8'));
for (const [name, repoRoot] of [
  ['absolute-repo-root-runner.json', tmp],
  ['traversal-repo-root-runner.json', '..']
]) {
  fs.writeFileSync(`${tmp}/${name}`, JSON.stringify({...input, id: name.replace('.json', ''), repo_root: repoRoot}, null, 2));
}
try { fs.unlinkSync(`${fresh}/escape-root`); } catch {}
fs.symlinkSync(tmp, `${fresh}/escape-root`);
fs.writeFileSync(`${tmp}/symlink-repo-root-runner.json`, JSON.stringify({...input, id: 'symlink-repo-root-runner', repo_root: 'escape-root'}, null, 2));
NODE
expect_failure absolute_repo_root "$BIN" --db "$DB" --json eval run --input "$TMP/absolute-repo-root-runner.json"
expect_failure traversal_repo_root "$BIN" --db "$DB" --json eval run --input "$TMP/traversal-repo-root-runner.json"
expect_failure symlink_repo_root "$BIN" --db "$DB" --json eval run --input "$TMP/symlink-repo-root-runner.json"

compare improved baseline-run better-run 0
IMPROVED_ID="$(awk -F '\t' '$1=="improved"{print $2}' "$COMPARISONS" | json_field 'd => d.object.comparison.id')"
compare wrong_but_faster baseline-run wrong-fast-run 1
compare quality_regressed quality-base-run quality-bad-run 1
compare performance_regressed baseline-run performance-bad-run 1
compare no_material_difference no-material-baseline-run same-run 0
compare stale baseline-run stale-run 2
compare no_data baseline-run no-data-run 2
compare mismatch baseline-run mismatch-run 2
compare under_covered baseline-run under-covered-run 2
compare under_sampled baseline-run under-sampled-run 2
compare noisy baseline-run noisy-run 2
compare interrupted_resumed interrupted-run resumed-run 2
compare recomputed baseline-run better-run 0 --recompute-of "$IMPROVED_ID"
RECOMPUTED_ID="$(awk -F '\t' '$1=="recomputed"{print $2}' "$COMPARISONS" | json_field 'd => d.object.comparison.id')"

expect_failure dangling_recompute "$BIN" --db "$DB" --json eval compare baseline-run better-run --recompute-of evcmp-missing
expect_failure unrelated_recompute "$BIN" --db "$DB" --json eval compare baseline-run better-run --recompute-of "$(awk -F '\t' '$1=="wrong_but_faster"{print $2}' "$COMPARISONS" | json_field 'd => d.object.comparison.id')"

"$BIN" --db "$DB" --json eval rescore baseline-run --id rescored-baseline-run >/dev/null
run_runner_input baseline .planr/eval-lifecycle-counts-rescore.json "$TMP/rescored-baseline-runner.json"
node - "$FRESH/examples/eval/planr-lifecycle-smoke.suite.json" <<'NODE'
const fs = require('fs');
const suitePath = process.argv[2];
const suite = JSON.parse(fs.readFileSync(suitePath, 'utf8'));
fs.writeFileSync(suite.fixtures[0].path, JSON.stringify({mutated_after_suite_check: true}, null, 2));
NODE
expect_failure runtime_fixture_mutation "$BIN" --db "$DB" --json eval run --input "$TMP/runtime-fixture-mutated-runner.json"
compare completed_rescore baseline-run rescored-baseline-run 0 --rescore-of baseline-run
RESCORE_COMPARISON_ID="$(awk -F '\t' '$1=="completed_rescore"{print $2}' "$COMPARISONS" | json_field 'd => d.object.comparison.id')"
expect_failure dangling_rescore "$BIN" --db "$DB" --json eval compare baseline-run rescored-baseline-run --rescore-of missing-run
expect_failure unrelated_rescore "$BIN" --db "$DB" --json eval compare better-run rescored-baseline-run --rescore-of better-run

INVALIDATION_JSON=$("$BIN" --db "$DB" --json eval invalidate run better-run --reason "live invalidation" --reason-code live --replacement-hint "rescore candidate" | compact_json)
compare invalidated_rescored baseline-run better-run 2
INVALIDATED_COMPARISON_ID="$(awk -F '\t' '$1=="invalidated_rescored"{print $2}' "$COMPARISONS" | json_field 'd => d.object.comparison.id')"

"$BIN" --db "$DB" --json eval gate "$IMPROVED_ID" >/dev/null
"$BIN" --db "$DB" --json eval evidence-ref comparison "$IMPROVED_ID" log "$LOG_ID" --item "$EVIDENCE_ITEM" >/dev/null
"$BIN" --db "$DB" export --include-logs --out "$PACKAGE" >/dev/null
mkdir -p "$IMPORT_REPO"
(
  cd "$IMPORT_REPO"
  git init -q
  "$BIN" --db "$IMPORT_REPO/.planr/planr.sqlite" project init "Eval Import Oracle" >/dev/null
  "$BIN" --db "$IMPORT_REPO/.planr/planr.sqlite" --json import "$PACKAGE" --preview >/dev/null
  "$BIN" --db "$IMPORT_REPO/.planr/planr.sqlite" --json import "$PACKAGE" --confirm >/dev/null
  "$BIN" --db "$IMPORT_REPO/.planr/planr.sqlite" --json eval show comparison "$IMPROVED_ID" >/dev/null
)

fingerprint_scoped_config > "$CONFIG_AFTER"
cmp "$CONFIG_BEFORE" "$CONFIG_AFTER"

node - "$COMPARISONS" "$DB" "$PACKAGE" "$FRESH" "$IMPORT_REPO" "$EVIDENCE_ITEM" "$LOG_ID" "$INVALIDATION_JSON" "$RECOMPUTED_ID" "$RESCORE_COMPARISON_ID" "$INVALIDATED_COMPARISON_ID" "$CONFIG_BEFORE" "$BIN" <<'NODE'
const fs = require('fs');
const {execFileSync} = require('child_process');
const [comparisonsPath, db, pkg, fresh, imported, item, log, invalidationRaw, recomputedId, rescoreComparisonId, invalidatedComparisonId, configPath, bin] = process.argv.slice(2);
const ESTIMATE_PROVENANCE = {
  pricing_reference_id: 'openai-prices',
  pricing_reference_version: '2026-07-01',
  currency: 'USD',
  effective_at: '2026-07-01T00:00:00Z'
};
const ESTIMATE_PROVENANCE_LABEL = `${ESTIMATE_PROVENANCE.pricing_reference_id}@${ESTIMATE_PROVENANCE.pricing_reference_version}`;
const rows = fs.readFileSync(comparisonsPath, 'utf8').trim().split('\n').map(line => {
  const tab = line.indexOf('\t');
  const label = line.slice(0, tab);
  const doc = JSON.parse(line.slice(tab + 1));
  return {label, verdict: doc.object.verdict, reasons: doc.object.comparison.reasons, comparison_id: doc.object.comparison.id};
});
const expected = {improved:'improved', wrong_but_faster:'regressed', quality_regressed:'regressed', performance_regressed:'regressed', no_material_difference:'no_material_difference', stale:'insufficient_evidence', no_data:'insufficient_evidence', mismatch:'insufficient_evidence', under_covered:'insufficient_evidence', under_sampled:'insufficient_evidence', noisy:'insufficient_evidence', interrupted_resumed:'insufficient_evidence', recomputed:'improved', completed_rescore:'no_material_difference', invalidated_rescored:'insufficient_evidence'};
for (const row of rows) if (row.verdict !== expected[row.label]) throw new Error(`${row.label} expected ${expected[row.label]} got ${row.verdict}`);
const expectedReasons = {
  improved: ['candidate_improved'],
  wrong_but_faster: ['correctness_regressed'],
  quality_regressed: ['quality_regressed'],
  performance_regressed: ['performance_regressed'],
  no_material_difference: ['no_material_effect'],
  stale: ['evidence_stale'],
  no_data: ['samples_below_minimum'],
  mismatch: ['testbed_incompatible'],
  under_covered: ['coverage_below_minimum'],
  under_sampled: ['samples_below_minimum'],
  noisy: ['variance_too_high'],
  interrupted_resumed: ['coverage_below_minimum'],
  recomputed: ['candidate_improved'],
  completed_rescore: ['no_material_effect'],
  invalidated_rescored: ['run_invalidated']
};
const remediation = {
  correctness_regressed: 'fix failing lifecycle assertions before promotion',
  quality_regressed: 'recapture candidate with non-inferior quality evidence',
  performance_regressed: 'investigate candidate latency regression',
  evidence_stale: 'recapture fresh baseline and candidate evidence',
  samples_below_minimum: 'increase successful paired samples to the declared minimum',
  testbed_incompatible: 'rerun both sides on the same testbed fingerprint',
  coverage_below_minimum: 'capture every required suite case',
  variance_too_high: 'stabilize noisy measurements and rerun',
  run_invalidated: 'rescore or replace the invalidated run',
  candidate_improved: 'allow promotion gate to pass',
  no_material_effect: 'keep baseline unless other evidence justifies promotion'
};
for (const row of rows) {
  for (const reason of expectedReasons[row.label]) {
    if (!row.reasons.includes(reason)) throw new Error(`${row.label} did not assert expected reason ${reason}; got ${JSON.stringify(row.reasons)}`);
    if (!remediation[reason]) throw new Error(`${row.label} reason ${reason} has no remediation mapping`);
  }
}
function sql(query) { return execFileSync('sqlite3', [db, query], {encoding: 'utf8'}).trim(); }
function show(dbPath, kind, id) {
  return JSON.parse(execFileSync(bin, ['--db', dbPath, '--json', 'eval', 'show', kind, id], {encoding: 'utf8'}));
}
function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object') {
    const out = {};
    for (const key of Object.keys(value).sort()) out[key] = canonical(value[key]);
    return out;
  }
  return value;
}
function assertJsonEqual(label, actual, expected) {
  const left = JSON.stringify(canonical(actual));
  const right = JSON.stringify(canonical(expected));
  if (left !== right) throw new Error(`${label} mismatch\nactual=${left}\nexpected=${right}`);
}
function runProjection(dbPath, runId) {
  const run = show(dbPath, 'run', runId).object.run;
  return {
    id: run.id,
    status: run.status,
    attempt_lineage: run.attempt_lineage,
    sample_metering: run.sample_metering,
    efficiency_summary: run.efficiency_summary,
    cases: run.cases.map(c => ({
      case_id: c.case_id,
      attempts: c.attempts,
      samples: c.samples.map(s => ({
        repetition_index: s.repetition_index,
        seed: s.seed,
        measure: s.measure,
        value: s.value,
        unit: s.unit,
        source: s.source,
        attempt_id: s.attempt_id,
        attempt_index: s.attempt_index,
        metering_basis: s.metering_basis,
        basis_source: s.basis_source,
        basis_confidence: s.basis_confidence,
        estimate_provenance: s.estimate_provenance
      }))
    }))
  };
}
function comparisonProjection(dbPath, comparisonId) {
  const comparison = show(dbPath, 'comparison', comparisonId).object;
  return {
    verdict: comparison.verdict,
    reasons: comparison.comparison.reasons,
    baseline_run_id: comparison.comparison.baseline_run_id,
    candidate_run_id: comparison.comparison.candidate_run_id,
    baseline_efficiency_summary: comparison.baseline_efficiency_summary,
    candidate_efficiency_summary: comparison.candidate_efficiency_summary,
    efficiency_summary: comparison.efficiency_summary,
    effort_recommendation: comparison.effort_recommendation
  };
}
function allSamples(run) {
  return run.cases.flatMap(c => c.samples);
}
function assertEstimateProvenanceObject(label, provenance) {
  if (!provenance || typeof provenance !== 'object' || Array.isArray(provenance)) {
    throw new Error(`${label} missing canonical estimate provenance`);
  }
  for (const [key, value] of Object.entries(ESTIMATE_PROVENANCE)) {
    if (provenance[key] !== value) throw new Error(`${label} provenance ${key} ${provenance[key]} != ${value}`);
  }
}
function assertEstimateProvenanceLabels(label, metric) {
  const actual = metric.estimate_provenance || [];
  assertJsonEqual(`${label} estimate provenance labels`, actual, [ESTIMATE_PROVENANCE_LABEL]);
}
function assertSampleBasis(run, basis, confidence, valueIsNull, provenanceRequired) {
  const costSamples = allSamples(run).filter(sample => sample.measure === 'cost_micros');
  if (costSamples.length === 0) throw new Error(`${run.id} has no cost_micros samples`);
  for (const sample of costSamples) {
    if (sample.metering_basis !== basis) throw new Error(`${run.id} expected ${basis} got ${sample.metering_basis}`);
    if (sample.basis_confidence !== confidence) throw new Error(`${run.id} expected confidence ${confidence} got ${sample.basis_confidence}`);
    if ((sample.value === null) !== valueIsNull) throw new Error(`${run.id} unexpected cost value ${sample.value}`);
    if (provenanceRequired) assertEstimateProvenanceObject(`${run.id} raw sample ${sample.attempt_id}`, sample.estimate_provenance);
    if (!provenanceRequired && sample.estimate_provenance !== null) throw new Error(`${run.id} unexpected estimate provenance`);
  }
  const projectedCostSamples = run.sample_metering.filter(sample => sample.measure === 'cost_micros');
  if (projectedCostSamples.length === 0) throw new Error(`${run.id} has no projected cost_micros samples`);
  for (const sample of projectedCostSamples) {
    if (sample.metering_basis !== basis) throw new Error(`${run.id} projected expected ${basis} got ${sample.metering_basis}`);
    if (provenanceRequired) assertEstimateProvenanceObject(`${run.id} projected sample ${sample.attempt_id}`, sample.estimate_provenance);
    if (!provenanceRequired && sample.estimate_provenance !== null) throw new Error(`${run.id} projected unexpected estimate provenance`);
  }
}
function assertEfficiencyArithmetic(run, expectedBasis, expectedState) {
  const summary = run.efficiency_summary;
  const cost = summary.total_cost_micros;
  const perAttempt = summary.cost_per_attempt_micros;
  const perSuccess = summary.cost_per_verified_success_micros;
  if (cost.basis !== expectedBasis || perAttempt.basis !== expectedBasis || perSuccess.basis !== expectedBasis) {
    throw new Error(`${run.id} expected efficiency basis ${expectedBasis}: ${JSON.stringify(summary)}`);
  }
  if (perAttempt.state !== expectedState || perSuccess.state !== expectedState) {
    throw new Error(`${run.id} expected efficiency state ${expectedState}: ${JSON.stringify(summary)}`);
  }
  const rawCosts = allSamples(run).filter(sample => sample.measure === 'cost_micros');
  if (expectedState === 'unavailable') {
    if (cost.value !== null || perAttempt.value !== null || perSuccess.value !== null) {
      throw new Error(`${run.id} unavailable cost must stay null: ${JSON.stringify(summary)}`);
    }
    return;
  }
  const total = rawCosts.reduce((sum, sample) => sum + sample.value, 0);
  const attempts = summary.countable_attempts;
  const successes = summary.verified_successes;
  if (cost.value !== total) throw new Error(`${run.id} total cost ${cost.value} != ${total}`);
  if (perAttempt.value !== total / attempts) throw new Error(`${run.id} per-attempt cost ${perAttempt.value} != ${total / attempts}`);
  if (perSuccess.value !== total / successes) throw new Error(`${run.id} per-success cost ${perSuccess.value} != ${total / successes}`);
  if (expectedBasis === 'estimated') {
    assertEstimateProvenanceLabels(`${run.id} total cost`, cost);
    assertEstimateProvenanceLabels(`${run.id} per-attempt cost`, perAttempt);
    assertEstimateProvenanceLabels(`${run.id} per-success cost`, perSuccess);
  }
}
const suiteDigest = sql("SELECT suite_digest FROM eval_runs WHERE id = 'baseline-run'");
const improvedId = rows.find(row => row.label === 'improved').comparison_id;
const importDb = `${imported}/.planr/planr.sqlite`;
for (const runId of ['baseline-run', 'metering-actual-run', 'metering-estimated-run', 'metering-unavailable-run']) {
  assertJsonEqual(`${runId} source/import projection`, runProjection(importDb, runId), runProjection(db, runId));
}
for (const comparisonId of [improvedId, recomputedId, rescoreComparisonId, invalidatedComparisonId]) {
  assertJsonEqual(`${comparisonId} source/import projection`, comparisonProjection(importDb, comparisonId), comparisonProjection(db, comparisonId));
}
const actualRun = runProjection(db, 'metering-actual-run');
const estimatedRun = runProjection(db, 'metering-estimated-run');
const unavailableRun = runProjection(db, 'metering-unavailable-run');
const importedActualRun = runProjection(importDb, 'metering-actual-run');
const importedEstimatedRun = runProjection(importDb, 'metering-estimated-run');
const importedUnavailableRun = runProjection(importDb, 'metering-unavailable-run');
assertSampleBasis(actualRun, 'actual_trusted', 'verified', false, false);
assertSampleBasis(estimatedRun, 'estimated', 'estimated', false, true);
assertSampleBasis(unavailableRun, 'unavailable', 'unavailable', true, false);
assertSampleBasis(importedActualRun, 'actual_trusted', 'verified', false, false);
assertSampleBasis(importedEstimatedRun, 'estimated', 'estimated', false, true);
assertSampleBasis(importedUnavailableRun, 'unavailable', 'unavailable', true, false);
assertEfficiencyArithmetic(actualRun, 'actual_trusted', 'available');
assertEfficiencyArithmetic(estimatedRun, 'estimated', 'available');
assertEfficiencyArithmetic(unavailableRun, 'unavailable', 'unavailable');
assertEfficiencyArithmetic(importedActualRun, 'actual_trusted', 'available');
assertEfficiencyArithmetic(importedEstimatedRun, 'estimated', 'available');
assertEfficiencyArithmetic(importedUnavailableRun, 'unavailable', 'unavailable');
if (sql("SELECT status FROM eval_runs WHERE id = 'interrupted-run'") !== 'inconclusive') throw new Error('interrupted parent did not persist inconclusive status');
if (sql("SELECT resume_of FROM eval_runs WHERE id = 'resumed-run'") !== 'interrupted-run') throw new Error('resume_of missing');
if (sql("SELECT status FROM eval_runs WHERE id = 'resumed-run'") !== 'success') throw new Error('resumed-run did not complete');
if (sql("SELECT COUNT(*) FROM eval_case_results WHERE run_id = 'interrupted-run' AND case_id = 'planr-lifecycle-baseline' AND status = 'pass'") !== '1') throw new Error('parent completed case missing');
if (sql("SELECT COUNT(*) FROM eval_case_results WHERE run_id = 'resumed-run' AND case_id = 'planr-lifecycle-baseline'") !== '0') throw new Error('resumed run reran reusable case');
const counts = JSON.parse(fs.readFileSync(`${fresh}/.planr/eval-lifecycle-counts-resume.json`, 'utf8'));
if (counts['planr-lifecycle-baseline'] !== 3 || counts['planr-lifecycle-follow-up'] !== 3) throw new Error(`unexpected isolated resume execution counts: ${JSON.stringify(counts)}`);
if (sql(`SELECT recompute_of FROM eval_comparisons WHERE id = '${recomputedId}'`) !== improvedId) throw new Error('comparison recompute provenance missing');
if (sql("SELECT status || ':' || rescore_of FROM eval_runs WHERE id = 'rescored-baseline-run'") !== 'success:baseline-run') throw new Error('completed rescore row missing');
if (sql("SELECT COUNT(*) FROM eval_runs WHERE id IN ('supplied-rogue-case-run', 'runtime-fixture-mutated-run')") !== '0') throw new Error('negative eval inputs created run rows');
if (sql(`SELECT rescore_of FROM eval_comparisons WHERE id = '${rescoreComparisonId}'`) !== 'baseline-run') throw new Error('rescore comparison provenance missing');
if (sql("SELECT COUNT(*) FROM eval_samples WHERE run_id = 'baseline-run'") !== '48') throw new Error('original baseline raw samples changed');
if (sql("SELECT COUNT(*) FROM eval_samples WHERE run_id = 'rescored-baseline-run'") !== '48') throw new Error('completed rescore samples missing');
const invalidation = JSON.parse(invalidationRaw);
console.log(JSON.stringify({verdict:'pass', testbed_id:'fresh-repo-live', fresh_repo:fresh, import_repo:imported, db, package:pkg, evidence_item_id:item, seed_log_id:log, config_fingerprints:fs.readFileSync(configPath,'utf8').trim().split('\n'), suite_digest:suiteDigest, comparisons:rows, asserted_remediations: remediation, recomputed_comparison_id:recomputedId, rescore_comparison_id:rescoreComparisonId, invalidated_comparison_id:invalidatedComparisonId, invalidation_id:invalidation.object.invalidation.id, metering_oracle:{actual_trusted:actualRun.efficiency_summary.cost_per_verified_success_micros, estimated:estimatedRun.efficiency_summary.cost_per_verified_success_micros, unavailable:unavailableRun.efficiency_summary.cost_per_verified_success_micros}, source_import_projection_compared:['baseline-run','metering-actual-run','metering-estimated-run','metering-unavailable-run',improvedId,recomputedId,rescoreComparisonId,invalidatedComparisonId], resume:{parent_run_id:'interrupted-run', resumed_run_id:'resumed-run', reused_parent_case:'planr-lifecycle-baseline', execution_counts:counts}, rescore_run_id:'rescored-baseline-run'}, null, 2));
NODE
