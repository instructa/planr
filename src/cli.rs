use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "planr",
    version,
    about = "Local-first planning and execution coordination for coding agents"
)]
pub(crate) struct Cli {
    #[arg(long, global = true, help = "Path to Planr SQLite database")]
    pub(crate) db: Option<PathBuf>,
    #[arg(long, global = true, help = "Emit JSON output")]
    pub(crate) json: bool,
    #[arg(long, global = true, help = "Disable color in human output")]
    pub(crate) no_color: bool,
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    Project(ProjectArgs),
    Plan(PlanArgs),
    Map(MapArgs),
    Item(ItemArgs),
    Link(LinkArgs),
    Pick(PickArgs),
    Approval(ApprovalArgs),
    Log(LogArgs),
    Close(CloseArgs),
    Review(ReviewArgs),
    Context(ContextArgs),
    Note(NoteArgs),
    Search(SearchArgs),
    Doctor(DoctorArgs),
    Install(InstallArgs),
    Prompt(PromptArgs),
    Mcp,
    Serve(ServeArgs),
    Trace(TraceArgs),
    Scrub(ScrubArgs),
    Artifact(ArtifactArgs),
    Event(EventArgs),
    Debug(DebugArgs),
    Recover(RecoverArgs),
    Export(ExportArgs),
    Import(ImportArgs),
}

#[derive(Args, Debug)]
pub(crate) struct ProjectArgs {
    #[command(subcommand)]
    pub(crate) command: ProjectCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ProjectCommand {
    Init(ProjectInitArgs),
    Show(JsonOnlyArgs),
    List(JsonOnlyArgs),
    Delete(ProjectDeleteArgs),
}

#[derive(Args, Debug)]
pub(crate) struct ProjectInitArgs {
    #[arg(long, value_enum)]
    pub(crate) client: Option<ClientArg>,
    #[arg(long)]
    pub(crate) force: bool,
    #[arg(default_value = "Planr Project")]
    pub(crate) name: String,
}

#[derive(Args, Debug)]
pub(crate) struct ProjectDeleteArgs {
    pub(crate) target: String,
    #[arg(long)]
    pub(crate) with_files: bool,
    #[arg(long)]
    pub(crate) confirm: bool,
}

#[derive(Args, Debug)]
pub(crate) struct JsonOnlyArgs {
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args, Debug)]
pub(crate) struct PlanArgs {
    #[command(subcommand)]
    pub(crate) command: PlanCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PlanCommand {
    New(PlanNewArgs),
    Refine(PlanRefineArgs),
    Split(PlanSplitArgs),
    Check(IdArg),
    Show(IdArg),
    List(PlanListArgs),
    Archive(IdArg),
}

#[derive(Args, Debug)]
pub(crate) struct PlanNewArgs {
    pub(crate) title: String,
    #[arg(long)]
    pub(crate) platform: Option<String>,
    #[arg(long)]
    pub(crate) ai: bool,
    #[arg(long)]
    pub(crate) backend: bool,
}

#[derive(Args, Debug)]
pub(crate) struct PlanRefineArgs {
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) note: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct PlanSplitArgs {
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) slice: String,
}

#[derive(Args, Debug)]
pub(crate) struct PlanListArgs {
    #[arg(long, value_enum)]
    pub(crate) stage: Option<PlanStageArg>,
}

#[derive(Args, Debug)]
pub(crate) struct MapArgs {
    #[command(subcommand)]
    pub(crate) command: Option<MapCommand>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum MapCommand {
    Show(JsonOnlyArgs),
    Build(MapBuildArgs),
    Lane(MapLaneArgs),
    Pressure,
    Status,
    Preview(MapPreviewArgs),
    Unlocks(MapUnlocksArgs),
    Lookahead(MapLookaheadArgs),
    Export(MapExportArgs),
    Import(MapImportArgs),
}

#[derive(Args, Debug)]
pub(crate) struct MapBuildArgs {
    #[arg(long)]
    pub(crate) from: String,
}

#[derive(Args, Debug)]
pub(crate) struct MapLaneArgs {
    #[arg(long)]
    pub(crate) critical: bool,
}

#[derive(Args, Debug)]
pub(crate) struct MapPreviewArgs {
    #[arg(long)]
    pub(crate) close: String,
}

#[derive(Args, Debug)]
pub(crate) struct MapUnlocksArgs {
    pub(crate) item_id: String,
}

#[derive(Args, Debug)]
pub(crate) struct MapLookaheadArgs {
    #[arg(long)]
    pub(crate) from: Option<String>,
    #[arg(long, default_value_t = 10)]
    pub(crate) limit: usize,
}

#[derive(Args, Debug)]
pub(crate) struct MapExportArgs {
    #[arg(long, default_value = "json")]
    pub(crate) format: String,
}

#[derive(Args, Debug)]
pub(crate) struct MapImportArgs {
    pub(crate) file: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct ItemArgs {
    #[command(subcommand)]
    pub(crate) command: ItemCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ItemCommand {
    Create(ItemCreateArgs),
    Show(IdArg),
    Update(ItemUpdateArgs),
    Breakdown(ItemBreakdownArgs),
    Insert(ItemInsertArgs),
    Amend(ItemAmendArgs),
    Replan(ItemReplanArgs),
    Cancel(ItemCancelArgs),
}

#[derive(Args, Debug)]
pub(crate) struct ItemCreateArgs {
    pub(crate) title: String,
    #[arg(long)]
    pub(crate) description: String,
    #[arg(long)]
    pub(crate) after: Option<String>,
    #[arg(long)]
    pub(crate) timeout_seconds: Option<i64>,
    #[arg(long)]
    pub(crate) max_retries: Option<i64>,
    #[arg(long, default_value = "exponential")]
    pub(crate) retry_backoff: String,
    #[arg(long)]
    pub(crate) retry_delay_ms: Option<i64>,
    #[arg(long)]
    pub(crate) pre: Option<String>,
    #[arg(long)]
    pub(crate) post: Option<String>,
    #[arg(long, hide = true)]
    pub(crate) work_type: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ItemUpdateArgs {
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) title: Option<String>,
    #[arg(long)]
    pub(crate) description: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ItemBreakdownArgs {
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) into: String,
}

#[derive(Args, Debug)]
pub(crate) struct ItemInsertArgs {
    pub(crate) title: String,
    #[arg(long)]
    pub(crate) description: String,
    #[arg(long)]
    pub(crate) after: String,
    #[arg(long)]
    pub(crate) before: Option<String>,
    #[arg(long)]
    pub(crate) preview: bool,
    #[arg(long)]
    pub(crate) confirm: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ItemAmendArgs {
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) note: String,
    #[arg(long, default_value = "amendment")]
    pub(crate) tag: String,
}

#[derive(Args, Debug)]
pub(crate) struct ItemReplanArgs {
    pub(crate) parent_id: String,
    #[arg(long)]
    pub(crate) into: String,
    #[arg(long)]
    pub(crate) preview: bool,
    #[arg(long)]
    pub(crate) confirm: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ItemCancelArgs {
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) preview: bool,
    #[arg(long)]
    pub(crate) confirm: bool,
}

#[derive(Args, Debug)]
pub(crate) struct LinkArgs {
    #[command(subcommand)]
    pub(crate) command: LinkCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum LinkCommand {
    Add(LinkAddArgs),
    Remove(LinkRemoveArgs),
}

#[derive(Args, Debug)]
pub(crate) struct LinkAddArgs {
    pub(crate) from_item: String,
    pub(crate) to_item: String,
    #[arg(long, default_value = "blocks")]
    pub(crate) r#type: String,
}

#[derive(Args, Debug)]
pub(crate) struct LinkRemoveArgs {
    pub(crate) from_item: String,
    pub(crate) to_item: String,
    #[arg(long)]
    pub(crate) r#type: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct PickArgs {
    #[command(subcommand)]
    pub(crate) command: Option<PickCommand>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PickCommand {
    Release(PickReleaseArgs),
    Heartbeat(PickHeartbeatArgs),
    Progress(PickProgressArgs),
    Pause(PickPauseArgs),
    Resume(PickResumeArgs),
    Stale(PickStaleArgs),
}

#[derive(Args, Debug)]
pub(crate) struct PickReleaseArgs {
    pub(crate) item_id: String,
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Args, Debug)]
pub(crate) struct PickHeartbeatArgs {
    pub(crate) item_id: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct PickProgressArgs {
    pub(crate) item_id: String,
    #[arg(long)]
    pub(crate) percent: i64,
    #[arg(long)]
    pub(crate) note: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct PickPauseArgs {
    pub(crate) item_id: String,
    #[arg(long)]
    pub(crate) note: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct PickResumeArgs {
    pub(crate) item_id: String,
}

#[derive(Args, Debug)]
pub(crate) struct PickStaleArgs {
    #[arg(long, default_value_t = 900)]
    pub(crate) older_than_seconds: i64,
    #[arg(long)]
    pub(crate) release: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ApprovalArgs {
    #[command(subcommand)]
    pub(crate) command: ApprovalCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ApprovalCommand {
    Request(ApprovalRequestArgs),
    Approve(ApprovalDecisionArgs),
    Deny(ApprovalDecisionArgs),
    List(ApprovalListArgs),
}

#[derive(Args, Debug)]
pub(crate) struct ApprovalRequestArgs {
    pub(crate) item_id: String,
    #[arg(long)]
    pub(crate) reason: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ApprovalDecisionArgs {
    pub(crate) item_id: String,
    #[arg(long)]
    pub(crate) by: String,
    #[arg(long)]
    pub(crate) comment: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ApprovalListArgs {
    #[arg(long)]
    pub(crate) open: bool,
}

#[derive(Args, Debug)]
pub(crate) struct LogArgs {
    #[command(subcommand)]
    pub(crate) command: LogCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum LogCommand {
    Add(LogAddArgs),
    Show(IdArg),
    List(LogListArgs),
}

#[derive(Args, Debug)]
pub(crate) struct LogAddArgs {
    #[arg(long)]
    pub(crate) item: String,
    #[arg(long)]
    pub(crate) summary: String,
    /// Changed file; repeat the flag or pass a comma-separated list.
    #[arg(long, value_delimiter = ',')]
    pub(crate) files: Vec<String>,
    #[arg(long)]
    pub(crate) cmd: Vec<String>,
    #[arg(long)]
    pub(crate) tests: Vec<String>,
    #[arg(long, default_value = "completion")]
    pub(crate) kind: String,
}

#[derive(Args, Debug)]
pub(crate) struct LogListArgs {
    #[arg(long)]
    pub(crate) item: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct CloseArgs {
    pub(crate) item_id: Option<String>,
    #[arg(long)]
    pub(crate) summary: String,
    #[arg(long)]
    pub(crate) next: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ReviewArgs {
    #[command(subcommand)]
    pub(crate) command: ReviewCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ReviewCommand {
    Request(ReviewRequestArgs),
    Annotate(ReviewAnnotateArgs),
    Ingest(ReviewIngestArgs),
    Artifact(ReviewArtifactArgs),
    Evidence(ReviewEvidenceArgs),
    Close(ReviewCloseArgs),
    List(ReviewListArgs),
    Show(IdArg),
}

#[derive(Args, Debug)]
pub(crate) struct ReviewRequestArgs {
    pub(crate) item_id: String,
}

#[derive(Args, Debug)]
pub(crate) struct ReviewAnnotateArgs {
    pub(crate) item_id: String,
    #[arg(long)]
    pub(crate) message: String,
    #[arg(long, default_value = "info")]
    pub(crate) severity: String,
    #[arg(long)]
    pub(crate) file: Option<String>,
    #[arg(long)]
    pub(crate) line: Option<u64>,
    #[arg(long)]
    pub(crate) author: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ReviewIngestArgs {
    pub(crate) item_id: String,
    #[arg(long, value_name = "PATH")]
    pub(crate) from: Option<PathBuf>,
    #[arg(long)]
    pub(crate) stdin: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ReviewArtifactArgs {
    pub(crate) review_item_id: String,
    #[arg(long)]
    pub(crate) out: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub(crate) struct ReviewEvidenceArgs {
    pub(crate) item_id: String,
    #[arg(long)]
    pub(crate) pr_url: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ReviewCloseArgs {
    pub(crate) review_item_id: String,
    #[arg(long, value_enum)]
    pub(crate) verdict: ReviewVerdict,
    #[arg(long)]
    pub(crate) findings: Vec<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ReviewListArgs {
    #[arg(long)]
    pub(crate) open: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ContextArgs {
    #[command(subcommand)]
    pub(crate) command: ContextCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ContextCommand {
    Add(ContextAddArgs),
    List(ContextListArgs),
}

#[derive(Args, Debug)]
pub(crate) struct NoteArgs {
    #[command(subcommand)]
    pub(crate) command: ContextCommand,
}

#[derive(Args, Debug)]
pub(crate) struct ContextAddArgs {
    pub(crate) text: String,
    #[arg(long)]
    pub(crate) item: Option<String>,
    #[arg(long, default_value = "discovery")]
    pub(crate) tag: String,
}

#[derive(Args, Debug)]
pub(crate) struct ContextListArgs {
    #[arg(long)]
    pub(crate) item: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct SearchArgs {
    pub(crate) query: String,
}

#[derive(Args, Debug)]
pub(crate) struct DoctorArgs {
    #[arg(long, value_enum)]
    pub(crate) client: Option<ClientArg>,
}

#[derive(Args, Debug)]
pub(crate) struct InstallArgs {
    #[command(subcommand)]
    pub(crate) command: InstallCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum InstallCommand {
    Codex(InstallClientArgs),
    Claude(InstallClientArgs),
    Cursor(InstallClientArgs),
}

#[derive(Args, Debug)]
pub(crate) struct InstallClientArgs {
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args, Debug)]
pub(crate) struct PromptArgs {
    #[command(subcommand)]
    pub(crate) command: PromptCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PromptCommand {
    Cli(PromptPrintArgs),
    Mcp(PromptPrintArgs),
    Http(PromptPrintArgs),
}

#[derive(Args, Debug)]
pub(crate) struct PromptPrintArgs {
    #[arg(long, value_enum)]
    pub(crate) client: Option<ClientArg>,
}

#[derive(Args, Debug)]
pub(crate) struct ServeArgs {
    #[arg(long, short, default_value_t = 7526)]
    pub(crate) port: u16,
}

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

#[derive(Args, Debug)]
pub(crate) struct IdArg {
    pub(crate) id: String,
}

#[derive(ValueEnum, Clone, Debug)]
pub(crate) enum ClientArg {
    Codex,
    Claude,
    Cursor,
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

impl ReviewVerdict {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            ReviewVerdict::Complete => "complete",
            ReviewVerdict::NotComplete => "not-complete",
            ReviewVerdict::Unclear => "unclear",
        }
    }
}
