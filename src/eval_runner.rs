#![allow(dead_code)] // Internal eval runner service; CLI/MCP adapters are downstream map items.

use crate::app::EvalReusableCaseEvidence;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub(crate) struct EvalRunnerManifest {
    pub(crate) schema_version: String,
    pub(crate) suite_id: String,
    pub(crate) scorers: Vec<EvalScorerIdentity>,
    pub(crate) fixtures: Vec<EvalFixture>,
    pub(crate) cases: Vec<EvalRunnerCase>,
    pub(crate) safety: EvalRunnerSafety,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalScorerIdentity {
    pub(crate) id: String,
    pub(crate) version: String,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalFixture {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) digest: String,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalRunnerCase {
    pub(crate) case_id: String,
    pub(crate) fixture_ids: Vec<String>,
    pub(crate) scorer_id: String,
    pub(crate) scorer_version: String,
    pub(crate) scorer_control: Option<EvalScorerControl>,
    pub(crate) subject: EvalRunnerSubject,
    pub(crate) attempt_transitions: Vec<EvalAttemptTransition>,
    pub(crate) assertions: Vec<EvalAssertion>,
    pub(crate) measures: Vec<String>,
    pub(crate) sampling: EvalSampling,
    pub(crate) timeout_ms: u64,
    pub(crate) output_limit_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum EvalScorerControlKind {
    Oracle,
    KnownBad,
    NoOp,
}

impl EvalScorerControlKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Oracle => "oracle",
            Self::KnownBad => "known_bad",
            Self::NoOp => "no_op",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvalScorerControlExpectation {
    Pass,
    Fail,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalScorerControl {
    pub(crate) kind: EvalScorerControlKind,
    pub(crate) expect: EvalScorerControlExpectation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvalAttemptTransitionKind {
    Retry,
    Fallback,
    Escalation,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalAttemptTransition {
    pub(crate) attempt_index: usize,
    pub(crate) kind: EvalAttemptTransitionKind,
    pub(crate) parent_attempt_index: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalRunnerSubject {
    pub(crate) kind: String,
    pub(crate) argv: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum EvalAssertion {
    ExitCode(i32),
    StdoutContains(String),
    StdoutJsonField { field: String, equals: Value },
    EventSequence(Vec<String>),
    StdoutNonEmpty,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalSampling {
    pub(crate) repetitions: usize,
    pub(crate) warmups: usize,
    pub(crate) seed: u64,
    pub(crate) max_attempts: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalRunnerSafety {
    pub(crate) allow_shell: bool,
    pub(crate) max_concurrency: usize,
    pub(crate) allow_environment_capture: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalRunOptions {
    pub(crate) concurrency: usize,
    pub(crate) reusable_cases: Vec<EvalReusableCaseEvidence>,
    pub(crate) cancellation: CancellationToken,
}

struct BoundedCommandInput<'a> {
    repo_root: &'a Path,
    argv: &'a [String],
    timeout: Duration,
    output_limit_bytes: usize,
    seed: u64,
    repetition_index: usize,
    warmup: bool,
    cancellation: &'a CancellationToken,
}

impl Default for EvalRunOptions {
    fn default() -> Self {
        Self {
            concurrency: 1,
            reusable_cases: Vec::new(),
            cancellation: CancellationToken::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub(crate) fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EvalRunnerOutput {
    pub(crate) suite_id: String,
    pub(crate) fixture_digests: HashMap<String, String>,
    pub(crate) cases: Vec<EvalCaseEvidence>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalCaseEvidence {
    pub(crate) case_id: String,
    pub(crate) status: String,
    pub(crate) reasons: Vec<String>,
    pub(crate) repetition_outcomes: Vec<EvalRepetitionOutcome>,
    pub(crate) samples: Vec<EvalSampleEvidence>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalRepetitionOutcome {
    pub(crate) repetition_index: usize,
    pub(crate) status: String,
    pub(crate) reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalSampleEvidence {
    pub(crate) repetition_index: usize,
    pub(crate) warmup: bool,
    pub(crate) seed: u64,
    pub(crate) attempt_index: usize,
    pub(crate) retry_of_attempt_index: Option<usize>,
    pub(crate) fallback_of_attempt_index: Option<usize>,
    pub(crate) escalation_of_attempt_index: Option<usize>,
    pub(crate) duration_ms: u128,
    pub(crate) command: EvalCommandEvidence,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalCommandEvidence {
    pub(crate) argv: Vec<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) timed_out: bool,
    pub(crate) interrupted: bool,
    pub(crate) stdout_digest: String,
    pub(crate) stderr_digest: String,
    pub(crate) stdout_excerpt: String,
    pub(crate) stderr_excerpt: String,
}

pub(crate) fn eval_runner_manifest_from_value(value: &Value) -> Result<EvalRunnerManifest> {
    let cases = required_array(value, "cases")?
        .iter()
        .map(eval_runner_case_from_value)
        .collect::<Result<Vec<_>>>()?;
    let safety = value.get("safety").unwrap_or(&Value::Null);
    Ok(EvalRunnerManifest {
        schema_version: string_field(value, "schema_version")
            .unwrap_or_else(|| "eval.suite.v1".to_string()),
        suite_id: required_string(value, "suite_id")?,
        scorers: value
            .get("scorers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(eval_scorer_identity_from_value)
            .collect::<Result<Vec<_>>>()?,
        fixtures: required_array(value, "fixtures")?
            .iter()
            .map(|fixture| {
                Ok(EvalFixture {
                    id: required_string(fixture, "id")?,
                    path: required_string(fixture, "path")?,
                    digest: required_string(fixture, "digest")?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        cases,
        safety: EvalRunnerSafety {
            allow_shell: bool_field(safety, "allow_shell").unwrap_or(false),
            max_concurrency: integer_field(safety, "max_concurrency").unwrap_or(1).max(0) as usize,
            allow_environment_capture: bool_field(safety, "allow_environment_capture")
                .unwrap_or(false),
        },
    })
}

fn eval_runner_case_from_value(value: &Value) -> Result<EvalRunnerCase> {
    let subject = required_value(value, "subject")?;
    let sampling = value.get("sampling").unwrap_or(&Value::Null);
    Ok(EvalRunnerCase {
        case_id: required_string(value, "case_id")?,
        fixture_ids: string_array_field(value, "fixture_ids")
            .or_else(|| string_field(value, "fixture_id").map(|fixture_id| vec![fixture_id]))
            .unwrap_or_default(),
        scorer_id: string_field(value, "scorer_id").unwrap_or_else(|| "default".to_string()),
        scorer_version: string_field(value, "scorer_version").unwrap_or_else(|| {
            value
                .get("scorer_ids")
                .and_then(Value::as_array)
                .and_then(|values| values.first())
                .and_then(Value::as_str)
                .and_then(|identity| identity.split_once('@').map(|(_, version)| version))
                .unwrap_or("v1")
                .to_string()
        }),
        scorer_control: value
            .get("scorer_control")
            .map(eval_scorer_control_from_value)
            .transpose()?,
        subject: EvalRunnerSubject {
            kind: required_string(subject, "kind")?,
            argv: string_array_field(subject, "argv").unwrap_or_default(),
        },
        attempt_transitions: value
            .get("attempt_transitions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(eval_attempt_transition_from_value)
            .collect::<Result<Vec<_>>>()?,
        assertions: value
            .get("assertions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(eval_assertion_from_value)
            .collect::<Result<Vec<_>>>()?,
        measures: string_array_field(value, "measures").unwrap_or_default(),
        sampling: EvalSampling {
            repetitions: integer_field(sampling, "repetitions").unwrap_or(1).max(0) as usize,
            warmups: integer_field(sampling, "warmups").unwrap_or(0).max(0) as usize,
            seed: integer_field(sampling, "seed").unwrap_or(0).max(0) as u64,
            max_attempts: integer_field(sampling, "max_attempts").unwrap_or(1).max(1) as usize,
        },
        timeout_ms: integer_field(value, "timeout_ms").unwrap_or(30_000).max(0) as u64,
        output_limit_bytes: integer_field(value, "output_limit_bytes")
            .unwrap_or(65_536)
            .max(0) as usize,
    })
}

fn eval_scorer_identity_from_value(value: &Value) -> Result<EvalScorerIdentity> {
    Ok(EvalScorerIdentity {
        id: required_string(value, "id")?,
        version: required_string(value, "version")?,
    })
}

fn eval_scorer_control_from_value(value: &Value) -> Result<EvalScorerControl> {
    let kind = match required_string(value, "kind")?.as_str() {
        "oracle" => EvalScorerControlKind::Oracle,
        "known_bad" => EvalScorerControlKind::KnownBad,
        "no_op" | "null" => EvalScorerControlKind::NoOp,
        other => bail!("unsupported eval scorer control kind: {other}"),
    };
    let expect = match required_string(value, "expect")?.as_str() {
        "pass" => EvalScorerControlExpectation::Pass,
        "fail" => EvalScorerControlExpectation::Fail,
        other => bail!("unsupported eval scorer control expectation: {other}"),
    };
    Ok(EvalScorerControl { kind, expect })
}

fn eval_attempt_transition_from_value(value: &Value) -> Result<EvalAttemptTransition> {
    let kind = match required_string(value, "kind")?.as_str() {
        "retry" => EvalAttemptTransitionKind::Retry,
        "fallback" => EvalAttemptTransitionKind::Fallback,
        "escalation" => EvalAttemptTransitionKind::Escalation,
        other => bail!("unsupported eval attempt transition kind: {other}"),
    };
    Ok(EvalAttemptTransition {
        attempt_index: integer_field(value, "attempt_index")
            .context("attempt transition missing attempt_index")?
            .max(0) as usize,
        kind,
        parent_attempt_index: integer_field(value, "parent_attempt_index")
            .context("attempt transition missing parent_attempt_index")?
            .max(0) as usize,
    })
}

fn eval_assertion_from_value(value: &Value) -> Result<EvalAssertion> {
    match required_string(value, "kind")?.as_str() {
        "exit_code" => Ok(EvalAssertion::ExitCode(
            integer_field(value, "expected").unwrap_or(0) as i32,
        )),
        "stdout_contains" => Ok(EvalAssertion::StdoutContains(required_string(
            value, "expected",
        )?)),
        "stdout_json_field" => Ok(EvalAssertion::StdoutJsonField {
            field: required_string(value, "field")?,
            equals: required_value(value, "equals")?.clone(),
        }),
        "event_sequence" => Ok(EvalAssertion::EventSequence(
            string_array_field(value, "expected").unwrap_or_default(),
        )),
        "stdout_non_empty" => Ok(EvalAssertion::StdoutNonEmpty),
        other => bail!("unsupported eval assertion kind: {other}"),
    }
}

pub(crate) fn run_eval_manifest(
    repo_root: &Path,
    manifest: &EvalRunnerManifest,
    options: EvalRunOptions,
) -> Result<EvalRunnerOutput> {
    let repo_root = repo_root
        .canonicalize()
        .context("canonicalizing repository root")?;
    let fixture_digests = validate_manifest(&repo_root, manifest, &options)?;
    let reusable_cases = reusable_case_map(&options.reusable_cases);
    let cases_to_run = manifest
        .cases
        .iter()
        .enumerate()
        .filter(|(_, case)| !can_resume_case(case, &reusable_cases))
        .map(|(index, case)| (index, case.clone()))
        .collect::<Vec<_>>();
    let cases = run_cases_bounded(&repo_root, manifest, &options, cases_to_run)?;
    Ok(EvalRunnerOutput {
        suite_id: manifest.suite_id.clone(),
        fixture_digests,
        cases,
    })
}

pub(crate) fn validate_eval_manifest(
    repo_root: &Path,
    manifest: &EvalRunnerManifest,
    options: &EvalRunOptions,
) -> Result<HashMap<String, String>> {
    let repo_root = repo_root
        .canonicalize()
        .context("canonicalizing repository root")?;
    validate_manifest(&repo_root, manifest, options)
}

fn validate_manifest(
    repo_root: &Path,
    manifest: &EvalRunnerManifest,
    options: &EvalRunOptions,
) -> Result<HashMap<String, String>> {
    if manifest.schema_version != "eval.suite.v1" {
        bail!("unsupported_schema_version");
    }
    if manifest.cases.is_empty() {
        bail!("manifest_invalid: cases must not be empty");
    }
    if options.concurrency == 0 || options.concurrency > manifest.safety.max_concurrency {
        bail!("manifest_invalid: concurrency exceeds declared safety bound");
    }
    if manifest.safety.allow_shell {
        bail!("shell_not_allowed");
    }
    if manifest.safety.allow_environment_capture {
        bail!("manifest_invalid: environment capture is unsafe");
    }
    let mut seen_fixtures = HashSet::new();
    let mut fixture_digests = HashMap::new();
    for fixture in &manifest.fixtures {
        if !seen_fixtures.insert(fixture.id.clone()) {
            bail!("duplicate_identity: fixture {}", fixture.id);
        }
        validate_sha256_digest_format(&fixture.digest)
            .with_context(|| format!("invalid eval fixture {} digest", fixture.id))?;
        let path = resolve_fixture_path(repo_root, &fixture.path)?;
        let bytes = fs::read(&path).with_context(|| format!("reading fixture {}", fixture.id))?;
        let actual_digest = sha256_prefixed(&bytes);
        if actual_digest != fixture.digest {
            bail!(
                "eval fixture {} digest mismatch: expected {}, computed {}",
                fixture.id,
                fixture.digest,
                actual_digest
            );
        }
        fixture_digests.insert(fixture.id.clone(), actual_digest);
    }
    let mut seen_cases = HashSet::new();
    let mut known_scorers = HashSet::new();
    for scorer in &manifest.scorers {
        if !known_scorers.insert(format!("{}@{}", scorer.id, scorer.version)) {
            bail!(
                "duplicate_identity: scorer {}@{}",
                scorer.id,
                scorer.version
            );
        }
    }
    let mut control_kinds_by_scorer = HashMap::<String, HashSet<EvalScorerControlKind>>::new();
    for case in &manifest.cases {
        if !seen_cases.insert(case.case_id.clone()) {
            bail!("duplicate_identity: case {}", case.case_id);
        }
        let scorer_identity = format!("{}@{}", case.scorer_id, case.scorer_version);
        if !known_scorers.is_empty() && !known_scorers.contains(&scorer_identity) {
            bail!("dangling_reference: scorer {scorer_identity}");
        }
        if case.subject.argv.is_empty() {
            bail!("manifest_invalid: subject argv must not be empty");
        }
        reject_shell_subject(&case.subject.argv)?;
        if case.sampling.repetitions == 0 {
            bail!("manifest_invalid: repetitions must be positive");
        }
        let mut seen_transitions = HashSet::new();
        for transition in &case.attempt_transitions {
            if transition.attempt_index == 0
                || transition.attempt_index >= case.sampling.max_attempts.max(1)
                || transition.parent_attempt_index >= transition.attempt_index
            {
                bail!("manifest_invalid: invalid attempt transition");
            }
            if !seen_transitions.insert(transition.attempt_index) {
                bail!(
                    "duplicate_identity: attempt transition {}",
                    transition.attempt_index
                );
            }
        }
        if case.timeout_ms == 0 || case.output_limit_bytes == 0 {
            bail!("manifest_invalid: execution bounds must be positive");
        }
        if let Some(control) = &case.scorer_control {
            match (control.kind, control.expect) {
                (EvalScorerControlKind::Oracle, EvalScorerControlExpectation::Pass)
                | (EvalScorerControlKind::KnownBad, EvalScorerControlExpectation::Fail)
                | (EvalScorerControlKind::NoOp, EvalScorerControlExpectation::Fail) => {}
                (EvalScorerControlKind::Oracle, EvalScorerControlExpectation::Fail) => {
                    bail!("scorer_control_incorrect_polarity: oracle must expect pass");
                }
                (
                    EvalScorerControlKind::KnownBad | EvalScorerControlKind::NoOp,
                    EvalScorerControlExpectation::Pass,
                ) => {
                    bail!("scorer_control_incorrect_polarity: negative control must expect fail");
                }
            }
            if case.sampling.repetitions < 2 {
                bail!("scorer_control_nondeterministic: controls require at least two repetitions");
            }
            if case.sampling.max_attempts != 1 {
                bail!("scorer_control_nondeterministic: controls must not retry or fallback");
            }
            if control.kind == EvalScorerControlKind::NoOp
                && !case
                    .assertions
                    .iter()
                    .any(|assertion| matches!(assertion, EvalAssertion::StdoutNonEmpty))
            {
                bail!(
                    "scorer_control_unexpected_no_op_success: no-op controls must assert output absence"
                );
            }
            let kinds = control_kinds_by_scorer.entry(scorer_identity).or_default();
            if !kinds.insert(control.kind) {
                bail!(
                    "duplicate_identity: scorer control {}",
                    control.kind.as_str()
                );
            }
        }
        for fixture_id in &case.fixture_ids {
            if !fixture_digests.contains_key(fixture_id) {
                bail!("dangling_reference: fixture {fixture_id}");
            }
        }
    }
    for (scorer, kinds) in &control_kinds_by_scorer {
        if !kinds.contains(&EvalScorerControlKind::Oracle) {
            bail!("scorer_control_missing_oracle: {scorer}");
        }
        if !kinds.contains(&EvalScorerControlKind::KnownBad)
            && !kinds.contains(&EvalScorerControlKind::NoOp)
        {
            bail!("scorer_control_missing_negative: {scorer}");
        }
    }
    Ok(fixture_digests)
}

fn validate_sha256_digest_format(digest: &str) -> Result<()> {
    let Some(hex) = digest.strip_prefix("sha256:") else {
        bail!("expected sha256:<64 hex>");
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("expected sha256:<64 hex>");
    }
    Ok(())
}

fn reusable_case_map(reusable: &[EvalReusableCaseEvidence]) -> HashMap<String, usize> {
    reusable
        .iter()
        .map(|case| {
            (
                case.case_id().to_string(),
                case.completed_repetition_count(),
            )
        })
        .collect()
}

fn can_resume_case(case: &EvalRunnerCase, reusable: &HashMap<String, usize>) -> bool {
    reusable
        .get(&case.case_id)
        .is_some_and(|count| *count >= case.sampling.repetitions)
}

fn run_cases_bounded(
    repo_root: &Path,
    manifest: &EvalRunnerManifest,
    options: &EvalRunOptions,
    cases_to_run: Vec<(usize, EvalRunnerCase)>,
) -> Result<Vec<EvalCaseEvidence>> {
    if cases_to_run.is_empty() {
        return Ok(Vec::new());
    }
    let queue = Arc::new(Mutex::new(cases_to_run.into_iter()));
    let results = Arc::new(Mutex::new(Vec::<(usize, EvalCaseEvidence)>::new()));
    let errors = Arc::new(Mutex::new(Vec::<anyhow::Error>::new()));
    let concurrency = options.concurrency;
    thread::scope(|scope| {
        for _ in 0..concurrency {
            let queue = Arc::clone(&queue);
            let results = Arc::clone(&results);
            let errors = Arc::clone(&errors);
            let cancellation = options.cancellation.clone();
            scope.spawn(move || {
                loop {
                    let next = {
                        let mut queue = queue.lock().expect("eval queue mutex poisoned");
                        queue.next()
                    };
                    let Some((index, case)) = next else {
                        break;
                    };
                    match run_case(repo_root, &case, manifest, options) {
                        Ok(evidence) => results
                            .lock()
                            .expect("eval results mutex poisoned")
                            .push((index, evidence)),
                        Err(error) => {
                            errors
                                .lock()
                                .expect("eval errors mutex poisoned")
                                .push(error);
                            cancellation.cancel();
                            break;
                        }
                    }
                }
            });
        }
    });
    let mut errors = errors.lock().expect("eval errors mutex poisoned");
    if let Some(error) = errors.pop() {
        return Err(error);
    }
    let mut results = results.lock().expect("eval results mutex poisoned").clone();
    results.sort_by_key(|(index, _)| *index);
    Ok(results
        .into_iter()
        .map(|(_, evidence)| evidence)
        .collect::<Vec<_>>())
}

fn resolve_fixture_path(repo_root: &Path, path: &str) -> Result<PathBuf> {
    let requested = Path::new(path);
    if requested.is_absolute() {
        bail!("absolute_path");
    }
    if path.starts_with('~') {
        bail!("unsafe_path");
    }
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("unsafe_path");
    }
    let root = repo_root
        .canonicalize()
        .context("canonicalizing repo root")?;
    let resolved = root.join(requested);
    let canonical = resolved
        .canonicalize()
        .with_context(|| format!("canonicalizing fixture path {path}"))?;
    if !canonical.starts_with(&root) {
        bail!("symlink_escape");
    }
    if !canonical.is_file() {
        bail!("unsafe_path: fixture must be a file");
    }
    Ok(canonical)
}

fn reject_shell_subject(argv: &[String]) -> Result<()> {
    let program = Path::new(&argv[0])
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&argv[0]);
    if matches!(
        program,
        "sh" | "bash" | "zsh" | "cmd" | "cmd.exe" | "powershell" | "pwsh"
    ) {
        bail!("shell_not_allowed");
    }
    for arg in argv {
        if arg.contains("$(") || arg.contains('`') || arg.contains("${") {
            bail!("shell_not_allowed");
        }
    }
    Ok(())
}

fn run_case(
    repo_root: &Path,
    case: &EvalRunnerCase,
    manifest: &EvalRunnerManifest,
    options: &EvalRunOptions,
) -> Result<EvalCaseEvidence> {
    let mut samples = Vec::new();
    let mut reasons = Vec::new();
    let mut repetition_outcomes = Vec::new();
    let total = case.sampling.warmups + case.sampling.repetitions;
    for index in 0..total {
        if options.cancellation.is_cancelled() {
            reasons.push("run_interrupted".to_string());
            return Ok(EvalCaseEvidence {
                case_id: case.case_id.clone(),
                status: "inconclusive".to_string(),
                reasons,
                repetition_outcomes,
                samples,
            });
        }
        let warmup = index < case.sampling.warmups;
        let repetition_index = if warmup {
            index
        } else {
            index - case.sampling.warmups
        };
        let seed = case.sampling.seed + repetition_index as u64;
        let max_attempts = if warmup {
            1
        } else {
            case.sampling.max_attempts.max(1)
        };
        let mut prior_attempt_index = None;
        for attempt_index in 0..max_attempts {
            let transition = attempt_transition_for(case, attempt_index, prior_attempt_index);
            let sample = run_sample(EvalSampleRunInput {
                repo_root,
                case,
                repetition_index,
                warmup,
                seed,
                attempt_index,
                transition: if warmup { None } else { transition },
                options,
            })?;
            let sample_reasons = if warmup {
                Vec::new()
            } else {
                assert_sample(case, &sample)
            };
            let should_retry = !warmup
                && attempt_index + 1 < max_attempts
                && sample_reasons
                    .iter()
                    .any(|reason| matches!(reason.as_str(), "process_failed" | "process_timeout"));
            let should_continue_for_transition = !warmup
                && attempt_index + 1 < max_attempts
                && case
                    .attempt_transitions
                    .iter()
                    .any(|transition| transition.attempt_index == attempt_index + 1);
            prior_attempt_index = Some(attempt_index);
            samples.push(sample);
            if should_retry || should_continue_for_transition {
                continue;
            }
            let sample_status = status_for_reasons(&sample_reasons);
            reasons.extend(sample_reasons.clone());
            repetition_outcomes.push(EvalRepetitionOutcome {
                repetition_index,
                status: sample_status.to_string(),
                reasons: sample_reasons,
            });
            break;
        }
    }
    reasons.sort();
    reasons.dedup();
    let status = status_for_reasons(&reasons);
    if manifest.safety.max_concurrency == 0 {
        bail!("manifest_invalid: concurrency bound must be positive");
    }
    Ok(EvalCaseEvidence {
        case_id: case.case_id.clone(),
        status: status.to_string(),
        reasons,
        repetition_outcomes,
        samples,
    })
}

fn status_for_reasons(reasons: &[String]) -> &'static str {
    if reasons.is_empty() {
        "pass"
    } else if reasons.iter().any(|reason| reason == "run_interrupted") {
        "inconclusive"
    } else if reasons
        .iter()
        .any(|reason| reason == "process_timeout" || reason == "process_failed")
    {
        "error"
    } else {
        "fail"
    }
}

fn attempt_transition_for(
    case: &EvalRunnerCase,
    attempt_index: usize,
    retry_of_attempt_index: Option<usize>,
) -> Option<(EvalAttemptTransitionKind, usize)> {
    case.attempt_transitions
        .iter()
        .find(|transition| transition.attempt_index == attempt_index)
        .map(|transition| (transition.kind, transition.parent_attempt_index))
        .or_else(|| retry_of_attempt_index.map(|parent| (EvalAttemptTransitionKind::Retry, parent)))
}

struct EvalSampleRunInput<'a> {
    repo_root: &'a Path,
    case: &'a EvalRunnerCase,
    repetition_index: usize,
    warmup: bool,
    seed: u64,
    attempt_index: usize,
    transition: Option<(EvalAttemptTransitionKind, usize)>,
    options: &'a EvalRunOptions,
}

fn run_sample(input: EvalSampleRunInput<'_>) -> Result<EvalSampleEvidence> {
    let start = Instant::now();
    let command = run_bounded_command(BoundedCommandInput {
        repo_root: input.repo_root,
        argv: &input.case.subject.argv,
        timeout: Duration::from_millis(input.case.timeout_ms),
        output_limit_bytes: input.case.output_limit_bytes,
        seed: input.seed,
        repetition_index: input.repetition_index,
        warmup: input.warmup,
        cancellation: &input.options.cancellation,
    })?;
    Ok(EvalSampleEvidence {
        repetition_index: input.repetition_index,
        warmup: input.warmup,
        seed: input.seed,
        attempt_index: input.attempt_index,
        retry_of_attempt_index: input
            .transition
            .filter(|(kind, _)| *kind == EvalAttemptTransitionKind::Retry)
            .map(|(_, parent)| parent),
        fallback_of_attempt_index: input
            .transition
            .filter(|(kind, _)| *kind == EvalAttemptTransitionKind::Fallback)
            .map(|(_, parent)| parent),
        escalation_of_attempt_index: input
            .transition
            .filter(|(kind, _)| *kind == EvalAttemptTransitionKind::Escalation)
            .map(|(_, parent)| parent),
        duration_ms: start.elapsed().as_millis(),
        command,
    })
}

fn run_bounded_command(input: BoundedCommandInput<'_>) -> Result<EvalCommandEvidence> {
    let mut child = Command::new(&input.argv[0])
        .args(&input.argv[1..])
        .env_clear()
        .env("PLANR_EVAL_SEED", input.seed.to_string())
        .env(
            "PLANR_EVAL_REPETITION_INDEX",
            input.repetition_index.to_string(),
        )
        .env("PLANR_EVAL_WARMUP", if input.warmup { "1" } else { "0" })
        .current_dir(input.repo_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {}", input.argv[0]))?;
    let stdout = child.stdout.take().context("capturing child stdout")?;
    let stderr = child.stderr.take().context("capturing child stderr")?;
    let output_exceeded = Arc::new(AtomicBool::new(false));
    let stdout_handle = drain_limited(
        stdout,
        input.output_limit_bytes,
        Arc::clone(&output_exceeded),
    );
    let stderr_handle = drain_limited(
        stderr,
        input.output_limit_bytes,
        Arc::clone(&output_exceeded),
    );
    let deadline = Instant::now() + input.timeout;
    let mut timed_out = false;
    let mut interrupted = false;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if input.cancellation.is_cancelled() {
            interrupted = true;
            let _ = child.kill();
            break;
        }
        if output_exceeded.load(Ordering::SeqCst) {
            let _ = child.kill();
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let status = child.wait()?;
    let stdout = join_drain(stdout_handle)?;
    let stderr = join_drain(stderr_handle)?;
    if output_exceeded.load(Ordering::SeqCst)
        || stdout.truncated
        || stderr.truncated
        || stdout.bytes.len() > input.output_limit_bytes
        || stderr.bytes.len() > input.output_limit_bytes
    {
        bail!("output_limit_exceeded");
    }
    Ok(EvalCommandEvidence {
        argv: input.argv.to_vec(),
        exit_code: status.code(),
        timed_out,
        interrupted,
        stdout_digest: sha256_prefixed(&stdout.bytes),
        stderr_digest: sha256_prefixed(&stderr.bytes),
        stdout_excerpt: bounded_utf8_excerpt(&stdout.bytes, input.output_limit_bytes),
        stderr_excerpt: bounded_utf8_excerpt(&stderr.bytes, input.output_limit_bytes),
    })
}

#[derive(Debug)]
struct DrainedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

fn drain_limited<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    output_exceeded: Arc<AtomicBool>,
) -> thread::JoinHandle<Result<DrainedOutput>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        let mut truncated = false;
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            if bytes.len() + read > limit {
                let allowed = limit.saturating_sub(bytes.len());
                bytes.extend_from_slice(&buffer[..allowed]);
                truncated = true;
                output_exceeded.store(true, Ordering::SeqCst);
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        Ok(DrainedOutput { bytes, truncated })
    })
}

fn join_drain(handle: thread::JoinHandle<Result<DrainedOutput>>) -> Result<DrainedOutput> {
    handle
        .join()
        .map_err(|_| anyhow::anyhow!("output drain thread panicked"))?
}

fn assert_sample(case: &EvalRunnerCase, sample: &EvalSampleEvidence) -> Vec<String> {
    let mut reasons = Vec::new();
    if sample.command.timed_out {
        reasons.push("process_timeout".to_string());
        return reasons;
    }
    if sample.command.interrupted {
        reasons.push("run_interrupted".to_string());
        return reasons;
    }
    for assertion in &case.assertions {
        match assertion {
            EvalAssertion::ExitCode(expected) => {
                if sample.command.exit_code != Some(*expected) {
                    reasons.push("process_failed".to_string());
                }
            }
            EvalAssertion::StdoutContains(needle) => {
                if !sample.command.stdout_excerpt.contains(needle) {
                    reasons.push("assertion_failed".to_string());
                }
            }
            EvalAssertion::StdoutJsonField { field, equals } => {
                match serde_json::from_str::<Value>(&sample.command.stdout_excerpt) {
                    Ok(value) if value.get(field) == Some(equals) => {}
                    Ok(_) => reasons.push("assertion_failed".to_string()),
                    Err(_) => reasons.push("invalid_output".to_string()),
                }
            }
            EvalAssertion::EventSequence(expected) => {
                let observed = sample
                    .command
                    .stdout_excerpt
                    .lines()
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .filter_map(|value| {
                        value
                            .get("event")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect::<Vec<_>>();
                if &observed != expected {
                    reasons.push("assertion_failed".to_string());
                }
            }
            EvalAssertion::StdoutNonEmpty => {
                if sample.command.stdout_excerpt.trim().is_empty() {
                    reasons.push("missing_output".to_string());
                }
            }
        }
    }
    reasons.sort();
    reasons.dedup();
    reasons
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

fn bounded_utf8_excerpt(bytes: &[u8], limit: usize) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(limit)]).to_string()
}

fn required_value<'a>(value: &'a Value, name: &str) -> Result<&'a Value> {
    value
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("missing required eval runner field: {name}"))
}

fn required_array<'a>(value: &'a Value, name: &str) -> Result<&'a Vec<Value>> {
    required_value(value, name)?
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("eval runner field {name} must be an array"))
}

fn string_field(value: &Value, name: &str) -> Option<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn required_string(value: &Value, name: &str) -> Result<String> {
    string_field(value, name)
        .ok_or_else(|| anyhow::anyhow!("missing required eval runner field: {name}"))
}

fn string_array_field(value: &Value, name: &str) -> Option<Vec<String>> {
    value.get(name).and_then(Value::as_array).map(|values| {
        values
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn integer_field(value: &Value, name: &str) -> Option<i64> {
    value.get(name).and_then(Value::as_i64)
}

fn bool_field(value: &Value, name: &str) -> Option<bool> {
    value.get(name).and_then(Value::as_bool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{os::unix::fs as unix_fs, time::Duration};

    fn fixture_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("fixture.txt"),
            b"fixture:basic-project:v1\n",
        )
        .unwrap();
        dir
    }

    fn base_manifest(argv: Vec<&str>) -> EvalRunnerManifest {
        EvalRunnerManifest {
            schema_version: "eval.suite.v1".to_string(),
            suite_id: "suite".to_string(),
            scorers: Vec::new(),
            fixtures: vec![EvalFixture {
                id: "fixture".to_string(),
                path: "fixture.txt".to_string(),
                digest: "sha256:339966ca2904a4e86569a7f407874b6a421a910edea972a2bee1b949ea3c1112"
                    .to_string(),
            }],
            cases: vec![EvalRunnerCase {
                case_id: "case-a".to_string(),
                fixture_ids: vec!["fixture".to_string()],
                scorer_id: "scorer".to_string(),
                scorer_version: "1.0.0".to_string(),
                scorer_control: None,
                subject: EvalRunnerSubject {
                    kind: "planr_cli".to_string(),
                    argv: argv.into_iter().map(str::to_string).collect(),
                },
                attempt_transitions: Vec::new(),
                assertions: vec![EvalAssertion::ExitCode(0), EvalAssertion::StdoutNonEmpty],
                measures: vec!["duration_ms".to_string()],
                sampling: EvalSampling {
                    repetitions: 2,
                    warmups: 1,
                    seed: 123,
                    max_attempts: 1,
                },
                timeout_ms: 5_000,
                output_limit_bytes: 1_024,
            }],
            safety: EvalRunnerSafety {
                allow_shell: false,
                max_concurrency: 1,
                allow_environment_capture: false,
            },
        }
    }

    #[test]
    fn eval_runner_repetitions_warmups_seeds_and_resume_are_deterministic() {
        let root = fixture_root();
        let manifest = base_manifest(vec!["printf", "ok"]);
        let output = run_eval_manifest(root.path(), &manifest, EvalRunOptions::default()).unwrap();
        assert_eq!(output.fixture_digests.len(), 1);
        let case = &output.cases[0];
        assert_eq!(case.status, "pass");
        assert_eq!(case.samples.len(), 3);
        assert_eq!(
            case.samples
                .iter()
                .map(|sample| (sample.repetition_index, sample.warmup, sample.seed))
                .collect::<Vec<_>>(),
            vec![(0, true, 123), (0, false, 123), (1, false, 124)]
        );

        let rerun = run_eval_manifest(root.path(), &manifest, EvalRunOptions::default()).unwrap();
        assert_eq!(rerun.cases.len(), 1, "caller-provided counts cannot skip");
    }

    #[test]
    fn eval_runner_rejects_unsafe_paths_shell_and_environment_capture_before_execution() {
        let root = fixture_root();
        let mut absolute = base_manifest(vec!["printf", "ok"]);
        absolute.fixtures[0].path = root
            .path()
            .join("fixture.txt")
            .to_string_lossy()
            .to_string();
        assert!(run_eval_manifest(root.path(), &absolute, EvalRunOptions::default()).is_err());

        let mut traversal = base_manifest(vec!["printf", "ok"]);
        traversal.fixtures[0].path = "../fixture.txt".to_string();
        assert!(run_eval_manifest(root.path(), &traversal, EvalRunOptions::default()).is_err());

        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), b"secret").unwrap();
        unix_fs::symlink(
            outside.path().join("secret.txt"),
            root.path().join("escape"),
        )
        .unwrap();
        let mut symlink = base_manifest(vec!["printf", "ok"]);
        symlink.fixtures[0].path = "escape".to_string();
        assert!(run_eval_manifest(root.path(), &symlink, EvalRunOptions::default()).is_err());

        let shell = base_manifest(vec!["sh", "-c", "printf ok"]);
        assert!(run_eval_manifest(root.path(), &shell, EvalRunOptions::default()).is_err());

        let mut env_capture = base_manifest(vec!["printf", "ok"]);
        env_capture.safety.allow_environment_capture = true;
        assert!(run_eval_manifest(root.path(), &env_capture, EvalRunOptions::default()).is_err());
    }

    #[test]
    fn eval_runner_admits_oracle_and_negative_scorer_controls_only_with_correct_polarity() {
        let root = fixture_root();
        let mut manifest = base_manifest(vec!["printf", "ok"]);
        manifest.scorers = vec![EvalScorerIdentity {
            id: "scorer".to_string(),
            version: "1.0.0".to_string(),
        }];
        manifest.cases[0].scorer_control = Some(EvalScorerControl {
            kind: EvalScorerControlKind::Oracle,
            expect: EvalScorerControlExpectation::Pass,
        });
        manifest.cases[0].sampling.warmups = 0;
        manifest.cases[0].sampling.repetitions = 2;
        let mut no_op = manifest.cases[0].clone();
        no_op.case_id = "no-op".to_string();
        no_op.scorer_control = Some(EvalScorerControl {
            kind: EvalScorerControlKind::NoOp,
            expect: EvalScorerControlExpectation::Fail,
        });
        no_op.subject.argv = vec!["true".to_string()];
        no_op.assertions = vec![EvalAssertion::StdoutNonEmpty];
        manifest.cases.push(no_op);
        assert!(validate_eval_manifest(root.path(), &manifest, &EvalRunOptions::default()).is_ok());

        let mut wrong_polarity = manifest.clone();
        wrong_polarity.cases[1].scorer_control = Some(EvalScorerControl {
            kind: EvalScorerControlKind::NoOp,
            expect: EvalScorerControlExpectation::Pass,
        });
        let error =
            validate_eval_manifest(root.path(), &wrong_polarity, &EvalRunOptions::default())
                .unwrap_err()
                .to_string();
        assert!(error.contains("scorer_control_incorrect_polarity"));

        let mut missing_negative = manifest.clone();
        missing_negative.cases.pop();
        let error =
            validate_eval_manifest(root.path(), &missing_negative, &EvalRunOptions::default())
                .unwrap_err()
                .to_string();
        assert!(error.contains("scorer_control_missing_negative"));
    }

    #[test]
    fn eval_runner_classifies_controlled_subject_outcomes() {
        let root = fixture_root();
        let good = run_eval_manifest(
            root.path(),
            &base_manifest(vec!["printf", "{\"ok\":true}"]),
            EvalRunOptions::default(),
        )
        .unwrap();
        assert_eq!(good.cases[0].status, "pass");

        let mut incorrect = base_manifest(vec!["printf", "wrong"]);
        incorrect.cases[0].assertions = vec![EvalAssertion::StdoutContains("expected".to_string())];
        let failed = run_eval_manifest(root.path(), &incorrect, EvalRunOptions::default()).unwrap();
        assert_eq!(failed.cases[0].status, "fail");
        assert_eq!(failed.cases[0].reasons, vec!["assertion_failed"]);

        let mut failed_subject = base_manifest(vec!["false"]);
        failed_subject.cases[0].assertions = vec![EvalAssertion::ExitCode(0)];
        let process_failed =
            run_eval_manifest(root.path(), &failed_subject, EvalRunOptions::default()).unwrap();
        assert_eq!(process_failed.cases[0].status, "error");
        assert_eq!(process_failed.cases[0].reasons, vec!["process_failed"]);

        let missing_output = run_eval_manifest(
            root.path(),
            &base_manifest(vec!["true"]),
            EvalRunOptions::default(),
        )
        .unwrap();
        assert_eq!(missing_output.cases[0].status, "fail");
        assert_eq!(missing_output.cases[0].reasons, vec!["missing_output"]);
    }

    #[test]
    fn eval_runner_preserves_mixed_control_outcomes_by_repetition() {
        let root = fixture_root();
        let mut manifest = base_manifest(vec!["printf", "ok"]);
        manifest.scorers = vec![EvalScorerIdentity {
            id: "scorer".to_string(),
            version: "1.0.0".to_string(),
        }];
        manifest.cases[0].scorer_control = Some(EvalScorerControl {
            kind: EvalScorerControlKind::Oracle,
            expect: EvalScorerControlExpectation::Pass,
        });
        manifest.cases[0].sampling.warmups = 0;
        manifest.cases[0].sampling.repetitions = 2;
        let mut no_op = manifest.cases[0].clone();
        no_op.case_id = "no-op".to_string();
        no_op.scorer_control = Some(EvalScorerControl {
            kind: EvalScorerControlKind::NoOp,
            expect: EvalScorerControlExpectation::Fail,
        });
        no_op.subject.argv = vec![
            "python3".to_string(),
            "-c".to_string(),
            "import os; print('ok' if os.environ['PLANR_EVAL_REPETITION_INDEX'] == '0' else '', end='')"
                .to_string(),
        ];
        no_op.assertions = vec![EvalAssertion::StdoutNonEmpty];
        manifest.cases.push(no_op);

        let output = run_eval_manifest(root.path(), &manifest, EvalRunOptions::default()).unwrap();
        let no_op = output
            .cases
            .iter()
            .find(|case| case.case_id == "no-op")
            .unwrap();
        assert_eq!(no_op.status, "fail");
        assert_eq!(
            no_op
                .repetition_outcomes
                .iter()
                .map(|outcome| (outcome.repetition_index, outcome.status.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, "pass"), (1, "fail")]
        );
    }

    #[test]
    fn eval_runner_retries_failed_subject_with_explicit_attempt_lineage() {
        let root = fixture_root();
        let counter = root.path().join("retry-counter.txt");
        let mut manifest = base_manifest(vec![
            "python3",
            "-c",
            "import pathlib, sys; p=pathlib.Path('retry-counter.txt'); n=int(p.read_text() or '0') if p.exists() else 0; p.write_text(str(n+1)); print('ok'); sys.exit(1 if n == 0 else 0)",
        ]);
        manifest.cases[0].sampling.repetitions = 1;
        manifest.cases[0].sampling.warmups = 0;
        manifest.cases[0].sampling.max_attempts = 2;
        manifest.cases[0].assertions =
            vec![EvalAssertion::ExitCode(0), EvalAssertion::StdoutNonEmpty];

        let output = run_eval_manifest(root.path(), &manifest, EvalRunOptions::default()).unwrap();
        assert_eq!(output.cases[0].status, "pass");
        assert_eq!(output.cases[0].samples.len(), 2);
        assert_eq!(output.cases[0].samples[0].attempt_index, 0);
        assert_eq!(output.cases[0].samples[0].retry_of_attempt_index, None);
        assert_eq!(output.cases[0].samples[0].command.exit_code, Some(1));
        assert_eq!(output.cases[0].samples[1].attempt_index, 1);
        assert_eq!(output.cases[0].samples[1].retry_of_attempt_index, Some(0));
        assert_eq!(output.cases[0].samples[1].command.exit_code, Some(0));
        assert_eq!(fs::read_to_string(counter).unwrap(), "2");
    }

    #[test]
    fn eval_runner_records_explicit_retry_fallback_and_escalation_lineage() {
        let root = fixture_root();
        let manifest_value = json!({
            "schema_version": "eval.suite.v1",
            "suite_id": "suite",
            "fixtures": [{
                "id": "fixture",
                "path": "fixture.txt",
                "digest": "sha256:339966ca2904a4e86569a7f407874b6a421a910edea972a2bee1b949ea3c1112"
            }],
            "cases": [{
                "case_id": "case-a",
                "fixture_ids": ["fixture"],
                "scorer_id": "scorer",
                "scorer_version": "1.0.0",
                "subject": {"kind": "planr_cli", "argv": ["printf", "ok"]},
                "assertions": [{"kind": "exit_code", "expected": 0}],
                "measures": ["duration_ms"],
                "sampling": {"repetitions": 1, "warmups": 0, "seed": 10, "max_attempts": 4},
                "attempt_transitions": [
                    {"attempt_index": 1, "kind": "retry", "parent_attempt_index": 0},
                    {"attempt_index": 2, "kind": "fallback", "parent_attempt_index": 1},
                    {"attempt_index": 3, "kind": "escalation", "parent_attempt_index": 2}
                ],
                "timeout_ms": 1000,
                "output_limit_bytes": 1024
            }],
            "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
        });
        let manifest = eval_runner_manifest_from_value(&manifest_value).unwrap();

        let output = run_eval_manifest(root.path(), &manifest, EvalRunOptions::default()).unwrap();
        let samples = &output.cases[0].samples;

        assert_eq!(samples.len(), 4);
        assert_eq!(samples[1].retry_of_attempt_index, Some(0));
        assert_eq!(samples[1].fallback_of_attempt_index, None);
        assert_eq!(samples[2].fallback_of_attempt_index, Some(1));
        assert_eq!(samples[2].retry_of_attempt_index, None);
        assert_eq!(samples[3].escalation_of_attempt_index, Some(2));
        assert_eq!(samples[3].fallback_of_attempt_index, None);
    }

    #[test]
    fn eval_runner_bounds_timeout_output_concurrency_and_cancellation() {
        let root = fixture_root();
        let mut slow = base_manifest(vec!["sleep", "1"]);
        slow.cases[0].timeout_ms = 20;
        let timed_out = run_eval_manifest(root.path(), &slow, EvalRunOptions::default()).unwrap();
        assert_eq!(timed_out.cases[0].status, "error");
        assert_eq!(timed_out.cases[0].reasons, vec!["process_timeout"]);

        let mut output_limited = base_manifest(vec!["printf", "abcdef"]);
        output_limited.cases[0].output_limit_bytes = 3;
        assert!(
            run_eval_manifest(root.path(), &output_limited, EvalRunOptions::default()).is_err()
        );

        let too_parallel = EvalRunOptions {
            concurrency: 2,
            ..EvalRunOptions::default()
        };
        assert!(
            run_eval_manifest(
                root.path(),
                &base_manifest(vec!["printf", "ok"]),
                too_parallel
            )
            .is_err()
        );

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let interrupted = run_eval_manifest(
            root.path(),
            &base_manifest(vec!["printf", "ok"]),
            EvalRunOptions {
                cancellation,
                ..EvalRunOptions::default()
            },
        )
        .unwrap();
        assert_eq!(interrupted.cases[0].status, "inconclusive");
        assert_eq!(interrupted.cases[0].reasons, vec!["run_interrupted"]);

        thread::sleep(Duration::from_millis(1));
    }

    #[test]
    fn eval_runner_runs_cases_with_bounded_parallelism_and_stable_order() {
        let root = fixture_root();
        fs::write(
            root.path().join("overlap.py"),
            r#"import fcntl, os, pathlib, sys, time

lock = pathlib.Path("overlap.lock").open("a+")
state_path = pathlib.Path("overlap.state")

def read_state():
    if not state_path.exists():
        return [0, 0, 0]
    return [int(value) for value in state_path.read_text().split(",")]

def write_state(state):
    temporary = pathlib.Path(f"overlap-{os.getpid()}.tmp")
    temporary.write_text(",".join(str(value) for value in state))
    temporary.replace(state_path)

fcntl.flock(lock, fcntl.LOCK_EX)
active, max_active, entered = read_state()
active += 1
entered += 1
write_state([active, max(max_active, active), entered])
fcntl.flock(lock, fcntl.LOCK_UN)

deadline = time.monotonic() + 5
while True:
    fcntl.flock(lock, fcntl.LOCK_EX)
    active, max_active, entered = read_state()
    fcntl.flock(lock, fcntl.LOCK_UN)
    if entered >= 2:
        break
    if time.monotonic() >= deadline:
        sys.exit(2)
    time.sleep(0.01)

fcntl.flock(lock, fcntl.LOCK_EX)
active, max_active, entered = read_state()
write_state([active - 1, max_active, entered])
fcntl.flock(lock, fcntl.LOCK_UN)
"#,
        )
        .unwrap();
        let mut manifest = base_manifest(vec!["python3", "overlap.py"]);
        manifest.safety.max_concurrency = 2;
        manifest.cases[0].sampling.repetitions = 1;
        manifest.cases[0].sampling.warmups = 0;
        manifest.cases[0].assertions = vec![EvalAssertion::ExitCode(0)];
        let mut second = manifest.cases[0].clone();
        second.case_id = "case-b".to_string();
        manifest.cases.push(second);

        let output = run_eval_manifest(
            root.path(),
            &manifest,
            EvalRunOptions {
                concurrency: 2,
                ..EvalRunOptions::default()
            },
        )
        .unwrap();
        assert!(output.cases.iter().all(|case| case.status == "pass"));
        assert_eq!(
            fs::read_to_string(root.path().join("overlap.state")).unwrap(),
            "0,2,2",
            "both cases must cross the overlap barrier with at most two active processes"
        );
        assert_eq!(
            output
                .cases
                .iter()
                .map(|case| case.case_id.as_str())
                .collect::<Vec<_>>(),
            vec!["case-a", "case-b"],
            "bounded scheduler must preserve manifest order"
        );
    }

    #[test]
    fn eval_runner_delivers_seed_and_uses_repository_cwd() {
        let root = fixture_root();
        fs::write(root.path().join("cwd-marker.txt"), b"repo").unwrap();
        let mut manifest = base_manifest(vec![
            "python3",
            "-c",
            "import os, pathlib; print(os.environ['PLANR_EVAL_SEED'] + ':' + pathlib.Path('cwd-marker.txt').read_text())",
        ]);
        manifest.cases[0].sampling.repetitions = 1;
        manifest.cases[0].sampling.warmups = 0;
        manifest.cases[0].sampling.seed = 777;
        manifest.cases[0].assertions = vec![EvalAssertion::StdoutContains("777:repo".to_string())];
        let first = run_eval_manifest(root.path(), &manifest, EvalRunOptions::default()).unwrap();
        let second = run_eval_manifest(root.path(), &manifest, EvalRunOptions::default()).unwrap();
        assert_eq!(first.cases[0].status, "pass");
        assert_eq!(
            first.cases[0].samples[0].command.stdout_digest,
            second.cases[0].samples[0].command.stdout_digest
        );
    }

    #[test]
    fn eval_runner_cancels_active_process_before_timeout() {
        let root = fixture_root();
        let cancellation = CancellationToken::new();
        let cancel_from_thread = cancellation.clone();
        let active_marker = root.path().join("active-marker.txt");
        let marker_from_thread = active_marker.clone();
        let handle = thread::spawn(move || {
            let readiness_deadline = Instant::now() + Duration::from_secs(5);
            while !marker_from_thread.exists() && Instant::now() < readiness_deadline {
                thread::sleep(Duration::from_millis(5));
            }
            let observed_active_process = marker_from_thread.exists();
            cancel_from_thread.cancel();
            observed_active_process
        });
        let mut manifest = base_manifest(vec![
            "python3",
            "-c",
            "import os, pathlib, time; os.write(1, b'started'); pathlib.Path('active-marker.txt').write_text('started'); time.sleep(10)",
        ]);
        manifest.cases[0].timeout_ms = 10_000;
        let started = Instant::now();
        let output = run_eval_manifest(
            root.path(),
            &manifest,
            EvalRunOptions {
                cancellation,
                ..EvalRunOptions::default()
            },
        )
        .unwrap();
        assert!(
            handle.join().unwrap(),
            "child process never reached readiness"
        );
        assert!(started.elapsed() < Duration::from_secs(7));
        assert_eq!(output.cases[0].status, "inconclusive");
        assert_eq!(output.cases[0].reasons, vec!["run_interrupted"]);
        assert!(
            output.cases[0].samples[0]
                .command
                .stdout_excerpt
                .contains("started")
        );
    }

    #[test]
    fn eval_runner_drains_and_limits_chatty_stdout_and_stderr() {
        let root = fixture_root();
        let mut stdout_limited = base_manifest(vec![
            "python3",
            "-c",
            "import sys; sys.stdout.write('x' * 200000); sys.stdout.flush()",
        ]);
        stdout_limited.cases[0].output_limit_bytes = 65_536;
        assert!(
            run_eval_manifest(root.path(), &stdout_limited, EvalRunOptions::default()).is_err()
        );

        let mut stderr_limited = base_manifest(vec![
            "python3",
            "-c",
            "import sys; sys.stderr.write('x' * 200000); sys.stderr.flush()",
        ]);
        stderr_limited.cases[0].output_limit_bytes = 65_536;
        assert!(
            run_eval_manifest(root.path(), &stderr_limited, EvalRunOptions::default()).is_err()
        );
    }

    #[test]
    fn eval_runner_scores_json_fields_and_event_trajectory() {
        let root = fixture_root();
        let mut json_case = base_manifest(vec!["printf", "{\"passed\":true}"]);
        json_case.cases[0].assertions = vec![EvalAssertion::StdoutJsonField {
            field: "passed".to_string(),
            equals: json!(true),
        }];
        assert_eq!(
            run_eval_manifest(root.path(), &json_case, EvalRunOptions::default())
                .unwrap()
                .cases[0]
                .status,
            "pass"
        );

        let mut trajectory = base_manifest(vec![
            "printf",
            "{\"event\":\"project.created\"}\n{\"event\":\"map.built\"}\n",
        ]);
        trajectory.cases[0].assertions = vec![EvalAssertion::EventSequence(vec![
            "project.created".to_string(),
            "map.built".to_string(),
        ])];
        assert_eq!(
            run_eval_manifest(root.path(), &trajectory, EvalRunOptions::default())
                .unwrap()
                .cases[0]
                .status,
            "pass"
        );
    }
}
