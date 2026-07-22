use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub(crate) struct EvalArgs {
    #[command(subcommand)]
    pub(crate) command: EvalCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum EvalCommand {
    /// Store or verify an immutable suite snapshot from a normalized manifest.
    SuiteCheck(EvalSuiteCheckArgs),
    /// Start a run, optionally record case evidence, and optionally finish it.
    Run(EvalRunArgs),
    /// Show a stored suite, run, comparison, or invalidation.
    Show(EvalShowArgs),
    /// Compare two stored eval runs and persist the comparison.
    Compare(EvalCompareArgs),
    /// Gate on a stored comparison verdict.
    Gate(EvalGateArgs),
    /// Invalidate a run or comparison.
    Invalidate(EvalInvalidateArgs),
    /// Start a rescore run from an existing run.
    Rescore(EvalRescoreArgs),
    /// Attach an eval run/comparison to an existing Planr log, review, or artifact.
    EvidenceRef(EvalEvidenceRefArgs),
}

#[derive(Args, Debug)]
pub(crate) struct EvalSuiteCheckArgs {
    /// JSON file containing suite_id, suite_version, digest, and normalized_manifest.
    #[arg(long)]
    pub(crate) input: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct EvalRunArgs {
    /// JSON file containing suite_digest, subject, testbed_fingerprint, and optional cases/status.
    #[arg(long)]
    pub(crate) input: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct EvalShowArgs {
    #[arg(value_enum)]
    pub(crate) kind: EvalShowKind,
    pub(crate) id: String,
}

#[derive(ValueEnum, Clone, Debug)]
pub(crate) enum EvalShowKind {
    Suite,
    Run,
    Comparison,
    Invalidation,
}

#[derive(Args, Debug)]
pub(crate) struct EvalCompareArgs {
    pub(crate) baseline_run_id: String,
    pub(crate) candidate_run_id: String,
    #[arg(long, default_value = "default")]
    pub(crate) policy_digest: String,
    /// Existing comparison id this comparison recomputes.
    #[arg(long)]
    pub(crate) recompute_of: Option<String>,
    /// Existing run id whose completed rescore this comparison evaluates.
    #[arg(long)]
    pub(crate) rescore_of: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct EvalGateArgs {
    pub(crate) comparison_id: String,
}

#[derive(Args, Debug)]
pub(crate) struct EvalInvalidateArgs {
    #[arg(value_enum)]
    pub(crate) target_kind: EvalInvalidationTargetKind,
    pub(crate) target_id: String,
    #[arg(long)]
    pub(crate) reason: String,
    #[arg(long = "reason-code")]
    pub(crate) reason_codes: Vec<String>,
    #[arg(long)]
    pub(crate) replacement_hint: Option<String>,
}

#[derive(ValueEnum, Clone, Debug)]
pub(crate) enum EvalInvalidationTargetKind {
    Run,
    Comparison,
}

#[derive(Args, Debug)]
pub(crate) struct EvalRescoreArgs {
    pub(crate) run_id: String,
    #[arg(long)]
    pub(crate) id: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct EvalEvidenceRefArgs {
    #[arg(value_enum)]
    pub(crate) target_kind: EvalEvidenceTargetKind,
    pub(crate) target_id: String,
    #[arg(value_enum)]
    pub(crate) attachment_kind: EvalEvidenceAttachmentKind,
    pub(crate) attachment_id: String,
    #[arg(long)]
    pub(crate) item: String,
}

#[derive(ValueEnum, Clone, Debug)]
pub(crate) enum EvalEvidenceTargetKind {
    Run,
    Comparison,
}

#[derive(ValueEnum, Clone, Debug)]
pub(crate) enum EvalEvidenceAttachmentKind {
    Log,
    Review,
    Artifact,
}
