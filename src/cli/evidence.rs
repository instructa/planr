use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub(crate) struct EvidenceArgs {
    #[command(subcommand)]
    pub(crate) command: EvidenceCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum EvidenceCommand {
    /// Parse and report the repository Evidence policy.
    Policy(EvidencePolicyArgs),
    /// Add, list, or show proof obligations.
    Obligation(EvidenceObligationArgs),
    /// List or show registered verification capabilities.
    Capability(EvidenceCapabilityArgs),
    /// Run a registered process capability against a stored obligation.
    Run(EvidenceRunArgs),
    /// Validate and persist an artifact import without creating trusted receipts.
    Import(EvidenceImportArgs),
    /// Validate and persist a fresh external Codex host capture as trusted Evidence.
    HostCapture(EvidenceHostCaptureArgs),
    /// List or show process attempts.
    Attempts(EvidenceRecordArgs),
    /// List or show trusted receipts.
    Receipts(EvidenceRecordArgs),
    /// Evaluate and persist coverage for an Evidence scope.
    Coverage(EvidenceCoverageArgs),
    /// Explain coverage, candidates, and durable records for an Evidence scope.
    Explain(EvidenceCoverageArgs),
    /// Preview or apply explicit pre-Evidence plan migration into binding Evidence obligations.
    Migrate(EvidenceMigrationArgs),
    /// Show canonical Evidence gap classifications and legacy/operator aliases.
    Classifications,
}

#[derive(Args, Debug)]
pub(crate) struct EvidencePolicyArgs {
    #[arg(long)]
    pub(crate) check: bool,
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceObligationArgs {
    #[command(subcommand)]
    pub(crate) command: EvidenceObligationCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum EvidenceObligationCommand {
    Add(EvidenceInputArgs),
    List(EvidenceScopeFilterArgs),
    Show(EvidenceIdArgs),
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceCapabilityArgs {
    #[command(subcommand)]
    pub(crate) command: EvidenceCapabilityCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum EvidenceCapabilityCommand {
    List,
    Show(EvidenceIdArgs),
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceRunArgs {
    /// JSON file containing obligation_id, capability_instance_id, target, environment, execution_contract, and optional env.
    #[arg(long)]
    pub(crate) input: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceImportArgs {
    /// JSON file containing a planr.evidence.import.v1 payload.
    #[arg(long)]
    pub(crate) input: PathBuf,
    /// Artifact root used to resolve import artifact_refs[].uri or ids.
    #[arg(long, default_value = ".")]
    pub(crate) artifact_root: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceHostCaptureArgs {
    #[command(subcommand)]
    pub(crate) command: EvidenceHostCaptureCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum EvidenceHostCaptureCommand {
    Import(EvidenceHostCaptureImportArgs),
    Run(EvidenceHostCaptureRunArgs),
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceHostCaptureImportArgs {
    /// JSON file containing a planr.evidence.host_capture.import.v1 payload.
    #[arg(long)]
    pub(crate) input: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceHostCaptureRunArgs {
    /// JSON file containing a planr.evidence.host_capture.run.v1 payload.
    #[arg(long)]
    pub(crate) input: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceRecordArgs {
    #[arg(long)]
    pub(crate) obligation: Option<String>,
    #[arg(long)]
    pub(crate) id: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceCoverageArgs {
    #[arg(long, value_enum)]
    pub(crate) scope: EvidenceCoverageScope,
    #[arg(long)]
    pub(crate) id: String,
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceMigrationArgs {
    /// JSON file containing a planr.evidence.migration.v1 payload.
    #[arg(long)]
    pub(crate) input: PathBuf,
    /// Apply the migration. Omit for dry-run preview.
    #[arg(long)]
    pub(crate) apply: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum EvidenceCoverageScope {
    Obligation,
    Criterion,
    Item,
    Plan,
}

impl EvidenceCoverageScope {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Obligation => "obligation",
            Self::Criterion => "criterion",
            Self::Item => "item",
            Self::Plan => "plan",
        }
    }
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceInputArgs {
    #[arg(long)]
    pub(crate) input: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceScopeFilterArgs {
    #[arg(long)]
    pub(crate) item: Option<String>,
    #[arg(long)]
    pub(crate) plan: Option<String>,
    #[arg(long)]
    pub(crate) criterion: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct EvidenceIdArgs {
    pub(crate) id: String,
}
