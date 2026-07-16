use super::{IdArg, JsonOnlyArgs};
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub(crate) enum PolicyCommand {
    /// Validate .planr/policy.toml; a missing file preserves legacy behavior.
    Check,
    /// Show the parsed policy or its explicit missing/degraded state.
    Show(JsonOnlyArgs),
    Admit(IdArg),
}

#[derive(Args, Debug)]
pub(crate) struct PresetArgs {
    #[command(subcommand)]
    pub(crate) command: PresetCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PresetCommand {
    /// List embedded policies, bindings, and declared safe packs.
    List(JsonOnlyArgs),
    /// Preview or explicitly apply a policy and host-binding composition.
    Apply(PresetApplyArgs),
    /// Simulate offline or explicitly execute the versioned preset evaluation suite.
    Evaluate(PresetEvaluateArgs),
    /// Verify, preview-import, and inspect declarative registry packs.
    Registry(PresetRegistryArgs),
    /// Sign one collector-produced telemetry payload (collector implementation utility).
    #[command(hide = true)]
    TelemetrySign(PresetTelemetrySignArgs),
}

#[derive(Args, Debug)]
pub(crate) struct PresetRegistryArgs {
    #[command(subcommand)]
    pub(crate) command: PresetRegistryCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PresetRegistryCommand {
    /// Verify checksums, compatibility, lifecycle, and pinned maintainer trust.
    Verify(PresetRegistryVerifyArgs),
    /// Preview or confirm an immutable, content-minimized offline-cache import.
    Import(PresetRegistryImportArgs),
    /// List imported packs and their current/stale offline freshness.
    List(PresetRegistryListArgs),
}

#[derive(Args, Debug)]
pub(crate) struct PresetRegistryVerifyArgs {
    /// Registry manifest TOML file.
    pub(crate) manifest: PathBuf,
    /// Entry id inside the manifest.
    #[arg(long)]
    pub(crate) entry: String,
    /// Directory containing the manifest-declared artifacts.
    #[arg(long)]
    pub(crate) content_root: PathBuf,
    /// Separately provisioned maintainer trust store (defaults to repository-local trust).
    #[arg(long)]
    pub(crate) trust_store: Option<PathBuf>,
    /// Evaluate freshness at this Unix timestamp.
    #[arg(long)]
    pub(crate) at_unix: Option<u64>,
    /// Require compatibility with this host.
    #[arg(long)]
    pub(crate) host: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct PresetRegistryImportArgs {
    /// Registry manifest TOML file.
    pub(crate) manifest: PathBuf,
    /// Entry id inside the manifest.
    #[arg(long)]
    pub(crate) entry: String,
    /// Directory containing the manifest-declared artifacts.
    #[arg(long)]
    pub(crate) content_root: PathBuf,
    /// Separately provisioned maintainer trust store (defaults to repository-local trust).
    #[arg(long)]
    pub(crate) trust_store: Option<PathBuf>,
    /// Evaluate freshness at this Unix timestamp.
    #[arg(long)]
    pub(crate) at_unix: Option<u64>,
    /// Require compatibility with this host.
    #[arg(long)]
    pub(crate) host: Option<String>,
    /// Explicitly spell the default non-mutating mode.
    #[arg(long, conflicts_with = "confirm")]
    pub(crate) preview: bool,
    /// Write only verified, manifest-declared files to the immutable local cache.
    #[arg(long)]
    pub(crate) confirm: bool,
}

#[derive(Args, Debug)]
pub(crate) struct PresetRegistryListArgs {
    /// Evaluate cached freshness at this Unix timestamp.
    #[arg(long)]
    pub(crate) at_unix: Option<u64>,
}

#[derive(Args, Debug)]
pub(crate) struct PresetApplyArgs {
    /// Provider-neutral Usage Policy v1 TOML path or built-in id.
    pub(crate) policy: PathBuf,
    /// Versioned host-binding TOML path or built-in id.
    #[arg(long)]
    pub(crate) binding: PathBuf,
    /// Absolute live-host adapter used for challenge-bound Codex capability probes.
    #[arg(long)]
    pub(crate) live_host_command: Option<PathBuf>,
    /// Argument passed to the live-host adapter; repeat as needed.
    #[arg(long, requires = "live_host_command")]
    pub(crate) live_host_arg: Vec<String>,
    /// Signer id pinned in .planr/trusted-telemetry.toml.
    #[arg(long, requires_all = ["live_host_command", "trusted_telemetry_collector"])]
    pub(crate) trusted_telemetry_signer: Option<String>,
    /// Absolute hash-pinned telemetry collector executable.
    #[arg(long, requires_all = ["live_host_command", "trusted_telemetry_signer"])]
    pub(crate) trusted_telemetry_collector: Option<PathBuf>,
    /// Explicitly spell the default non-mutating mode.
    #[arg(long, conflicts_with = "confirm")]
    pub(crate) preview: bool,
    /// Write only the conflict-free artifacts listed by the preview.
    #[arg(long)]
    pub(crate) confirm: bool,
}

#[derive(Args, Debug)]
pub(crate) struct PresetEvaluateArgs {
    /// Also write immutable verification.json and report.md under this repository-relative directory.
    #[arg(long)]
    pub(crate) report_dir: Option<PathBuf>,
    /// Evaluate lifecycle status at this Unix timestamp (defaults to current wall-clock time).
    #[arg(long)]
    pub(crate) at_unix: Option<u64>,
    /// Require compatibility with this host (for example codex, claude-code, or cursor).
    #[arg(long)]
    pub(crate) host: Option<String>,
    /// Explicitly execute every fixture through this absolute-path host adapter.
    #[arg(long)]
    pub(crate) live_host_command: Option<PathBuf>,
    /// Argument passed to the live host adapter; repeat as needed.
    #[arg(long, requires = "live_host_command")]
    pub(crate) live_host_arg: Vec<String>,
    /// Signer id pinned in .planr/trusted-telemetry.toml.
    #[arg(
        long,
        requires_all = ["live_host_command", "trusted_telemetry_collector"]
    )]
    pub(crate) trusted_telemetry_signer: Option<String>,
    /// Absolute path to the collector executable hash-pinned for the signer.
    #[arg(
        long,
        requires_all = ["live_host_command", "trusted_telemetry_signer"]
    )]
    pub(crate) trusted_telemetry_collector: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub(crate) struct PresetTelemetrySignArgs {
    /// File containing a 32-byte Ed25519 seed encoded as 64 hexadecimal characters.
    #[arg(long)]
    pub(crate) private_key_file: PathBuf,
}
