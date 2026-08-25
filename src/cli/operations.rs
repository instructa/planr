use super::IdArg;
use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub(crate) struct TraceArgs {
    #[command(subcommand)]
    pub(crate) command: TraceCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum TraceCommand {
    Item(IdArg),
}

#[derive(Args, Debug)]
pub(crate) struct ScrubArgs {
    #[arg(long)]
    pub(crate) preview: bool,
    #[arg(long)]
    pub(crate) confirm: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ArtifactArgs {
    #[command(subcommand)]
    pub(crate) command: ArtifactCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ArtifactCommand {
    Add(ArtifactAddArgs),
    Show(IdArg),
    List(ArtifactListArgs),
}

#[derive(Args, Debug)]
pub(crate) struct ArtifactAddArgs {
    /// Artifact name; alternatively pass --name anywhere in the command.
    #[arg(value_name = "NAME")]
    pub(crate) name: Option<String>,
    #[arg(long = "name", value_name = "NAME", conflicts_with = "name")]
    pub(crate) name_flag: Option<String>,
    #[arg(long)]
    pub(crate) item: Option<String>,
    #[arg(long)]
    pub(crate) kind: Option<String>,
    #[arg(long)]
    pub(crate) path: Option<PathBuf>,
    #[arg(long)]
    pub(crate) content: Option<String>,
    #[arg(long)]
    pub(crate) mime: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ArtifactListArgs {
    #[arg(long)]
    pub(crate) item: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct EventArgs {
    #[command(subcommand)]
    pub(crate) command: EventCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum EventCommand {
    List(EventListArgs),
}

#[derive(Args, Debug)]
pub(crate) struct EventListArgs {
    #[arg(long)]
    pub(crate) item: Option<String>,
    #[arg(long, default_value_t = 50)]
    pub(crate) limit: usize,
}

#[derive(Args, Debug)]
pub(crate) struct DebugArgs {
    #[command(subcommand)]
    pub(crate) command: DebugCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum DebugCommand {
    Bundle(DebugBundleArgs),
}

#[derive(Args, Debug)]
pub(crate) struct DebugBundleArgs {
    #[arg(long)]
    pub(crate) item: Option<String>,
    #[arg(long)]
    pub(crate) preview: bool,
}

#[derive(Args, Debug)]
pub(crate) struct RecoverArgs {
    #[command(subcommand)]
    pub(crate) command: RecoverCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum RecoverCommand {
    Sweep(RecoverSweepArgs),
}

#[derive(Args, Debug)]
pub(crate) struct RecoverSweepArgs {
    #[arg(long, default_value_t = 900)]
    pub(crate) older_than_seconds: i64,
    #[arg(long)]
    pub(crate) apply: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ExportArgs {
    #[arg(long)]
    pub(crate) include_plans: bool,
    #[arg(long)]
    pub(crate) include_logs: bool,
    #[arg(long)]
    pub(crate) template_name: Option<String>,
    #[arg(long)]
    pub(crate) tag: Vec<String>,
    #[arg(long)]
    pub(crate) out: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct ImportArgs {
    pub(crate) file: PathBuf,
    #[arg(long)]
    pub(crate) preview: bool,
    #[arg(long)]
    pub(crate) confirm: bool,
}

#[derive(ValueEnum, Clone, Debug)]
pub(crate) enum ClientArg {
    Codex,
    Claude,
    Cursor,
    Grok,
    Pi,
    All,
}

#[derive(ValueEnum, Clone, Debug)]
pub(crate) enum PlanStageArg {
    Product,
    Build,
    Review,
}

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReviewVerdict {
    Complete,
    NotComplete,
    Unclear,
}
