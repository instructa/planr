use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

mod common;
mod eval;
mod evidence;
mod policy;
mod prime;
mod project_plan;
pub(crate) use common::*;
pub(crate) use eval::*;
pub(crate) use evidence::*;
pub(crate) use policy::PolicyCommand;
pub(crate) use prime::*;
pub(crate) use project_plan::*;

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
    /// Inspect the agent profile registry (.planr/agents.toml) that
    /// drives advisory model routing in pick packets.
    Agents(AgentsArgs),
    Policy {
        #[command(subcommand)]
        command: PolicyCommand,
    },
    Project(ProjectArgs),
    Plan(PlanArgs),
    Map(MapArgs),
    Item(ItemArgs),
    Link(LinkArgs),
    Run(RunArgs),
    Pick(PickArgs),
    Approval(ApprovalArgs),
    Log(LogArgs),
    Close(CloseArgs),
    /// Log evidence and finish the step in one command: completion log,
    /// then canonical settlement, optionally opening a structured ReviewGate.
    Done(DoneArgs),
    Review(ReviewArgs),
    /// Evaluate stored run evidence: suite check, run, show, compare, gate,
    /// invalidate, and rescore through the shared eval services.
    Eval(EvalArgs),
    /// Manage trusted Evidence policy, obligations, readiness, migrations,
    /// capabilities, runs, receipts, coverage, and explanations.
    Evidence(EvidenceArgs),
    Context(ContextArgs),
    Note(NoteArgs),
    Search(SearchArgs),
    Doctor(DoctorArgs),
    /// Compact state block for host hooks (session start / post-compaction).
    Prime(PrimeArgs),
    /// Decide whether a host Stop hook should continue an active Planr goal.
    Stop(StopArgs),
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
pub(crate) struct MapArgs {
    #[command(subcommand)]
    pub(crate) command: Option<MapCommand>,
}

#[derive(Args, Debug)]
pub(crate) struct MapShowArgs {
    #[arg(long)]
    pub(crate) json: bool,
    /// Only show items and links belonging to this plan (plan id)
    #[arg(long)]
    pub(crate) plan: Option<String>,
    /// Human output layout. `diagram` is for human supervision only; coding agents should use `tree` or `--json`.
    #[arg(long, value_enum, default_value_t = MapViewArg::Tree)]
    pub(crate) view: MapViewArg,
    /// Show the complete multi-line node details (diagram view only).
    #[arg(long)]
    pub(crate) full: bool,
}

#[derive(Args, Debug)]
pub(crate) struct MapWatchArgs {
    /// Only watch items and links belonging to this plan (plan id)
    #[arg(long)]
    pub(crate) plan: Option<String>,
    /// Human-only observation layout. Coding agents should use `map show --json` instead.
    #[arg(long, value_enum, default_value_t = MapViewArg::Diagram)]
    pub(crate) view: MapViewArg,
    /// Show the complete multi-line node details (diagram view only).
    #[arg(long)]
    pub(crate) full: bool,
    /// Poll interval in milliseconds.
    #[arg(
        long,
        default_value_t = 1000,
        value_parser = clap::value_parser!(u64).range(100..)
    )]
    pub(crate) interval_ms: u64,
    /// Append changed frames instead of clearing an interactive terminal.
    #[arg(long)]
    pub(crate) no_clear: bool,
    /// Exit after rendering a fully settled scoped map.
    #[arg(long)]
    pub(crate) until_settled: bool,
    /// Bound polling for deterministic smoke tests.
    #[arg(
        long,
        hide = true,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub(crate) iterations: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum MapViewArg {
    #[default]
    Tree,
    Diagram,
}

#[derive(Subcommand, Debug)]
pub(crate) enum MapCommand {
    /// Inspect map state. The diagram view is for humans; coding agents should use the default tree or JSON.
    Show(MapShowArgs),
    /// Human-only live observer. Coding agents should use `map show --json` or the event stream.
    Watch(MapWatchArgs),
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
    /// Show or pin the item's advisory model route. Without flags,
    /// prints the resolved route and whether an override or policy won.
    Route(ItemRouteArgs),
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
    /// Work type (free-form): built-in vocabulary like code/fix/review,
    /// or a registry route's use case (frontend, backend, ...) so model
    /// routing binds. Defaults to generic.
    #[arg(long)]
    pub(crate) work_type: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ItemUpdateArgs {
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) title: Option<String>,
    #[arg(long)]
    pub(crate) description: Option<String>,
    /// Retag the item's work type (free-form, e.g. a registry route's
    /// use case like `frontend`); model routing re-resolves on next pick.
    #[arg(long)]
    pub(crate) work_type: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ItemBreakdownArgs {
    pub(crate) id: String,
    /// Child title; repeat the flag per child, or pass one value with
    /// newline- or comma-separated titles
    #[arg(long, required = true)]
    pub(crate) into: Vec<String>,
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
pub(crate) struct ItemRouteArgs {
    pub(crate) id: String,
    /// Pin the item to this profile id from .planr/agents.toml; the pin
    /// beats every policy route until cleared.
    #[arg(long, value_name = "PROFILE", conflicts_with = "clear")]
    pub(crate) set: Option<String>,
    /// Remove the pinned profile so policy routing applies again.
    #[arg(long)]
    pub(crate) clear: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ItemCancelArgs {
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) preview: bool,
    #[arg(long)]
    pub(crate) confirm: bool,
    /// Why the item is cancelled; recorded on the item_cancelled event.
    #[arg(long)]
    pub(crate) reason: Option<String>,
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
    /// Lease canonical `code`, `review`, or `verification` work.
    #[arg(long)]
    pub(crate) work_type: Option<String>,
    /// Only lease items belonging to this plan (plan id), so plan-scoped
    /// goal runs never pick work outside their contract.
    #[arg(long)]
    pub(crate) plan: Option<String>,
    /// Read the next work packet (incl. its routing block) without
    /// leasing it — for drivers that dispatch; the worker takes the lease.
    #[arg(long)]
    pub(crate) peek: bool,
}

#[derive(Args, Debug)]
pub(crate) struct RunArgs {
    #[command(subcommand)]
    pub(crate) command: RunCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum RunCommand {
    Batch(RunBatchArgs),
    Restart(RunRestartArgs),
    ResolveBudgetHold(RunResolveBudgetHoldArgs),
    RepairVerificationAdmission(RunRepairVerificationAdmissionArgs),
    SettleRepair(RunSettleRepairArgs),
}

#[derive(Args, Debug)]
pub(crate) struct RunRepairVerificationAdmissionArgs {
    #[arg(long)]
    pub(crate) plan: String,
    #[arg(long)]
    pub(crate) run: String,
    #[arg(long)]
    pub(crate) freeze: String,
    #[arg(long)]
    pub(crate) revision: u64,
    #[arg(long, value_enum)]
    pub(crate) reason: VerificationAdmissionRepairReasonArg,
    #[arg(long)]
    pub(crate) run_index_digest: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum VerificationAdmissionRepairReasonArg {
    ReadinessBlocked,
    RunIndexSealFailed,
    SealedRunRejected,
    CapabilityAdmissionFailed,
}

#[derive(Args, Debug)]
pub(crate) struct RunSettleRepairArgs {
    #[arg(long)]
    pub(crate) plan: String,
    #[arg(long)]
    pub(crate) invalidation: String,
    #[arg(long)]
    pub(crate) summary: String,
    #[arg(long, value_delimiter = ',')]
    pub(crate) files: Vec<String>,
    #[arg(long)]
    pub(crate) cmd: Vec<String>,
    #[arg(long)]
    pub(crate) tests: Vec<String>,
}

#[derive(Args, Debug)]
pub(crate) struct RunResolveBudgetHoldArgs {
    #[arg(long)]
    pub(crate) plan: String,
}

#[derive(Args, Debug)]
pub(crate) struct RunRestartArgs {
    #[arg(long)]
    pub(crate) plan: String,
    #[arg(long, value_enum)]
    pub(crate) reason: FeatureRunRestartReasonArg,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum FeatureRunRestartReasonArg {
    IncompatibleBudget,
    PrematureSourceFreeze,
    InconsistentVerification,
}

#[derive(Args, Debug)]
pub(crate) struct RunBatchArgs {
    #[command(subcommand)]
    pub(crate) command: RunBatchCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum RunBatchCommand {
    Roll(RunBatchRollArgs),
    Replace(RunBatchReplaceArgs),
}

#[derive(Args, Debug)]
pub(crate) struct RunBatchRollArgs {
    #[arg(long)]
    pub(crate) plan: String,
}

#[derive(Args, Debug)]
pub(crate) struct RunBatchReplaceArgs {
    #[arg(long)]
    pub(crate) plan: String,
    #[arg(long)]
    pub(crate) successor_maker: String,
    #[arg(long, value_enum)]
    pub(crate) reason: MakerReplacementReasonArg,
    #[arg(long)]
    pub(crate) reference: String,
    #[arg(long)]
    pub(crate) explanation: String,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum MakerReplacementReasonArg {
    Unavailable,
    ContextLost,
    OwnershipIncompatible,
    BatchCapReached,
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
    /// Log kind: completion (default), progress, or verification.
    /// Verification logs are narrative claims; only frozen pre-Evidence plans
    /// use them for compatibility. Binding Evidence requires trusted receipts.
    #[arg(long, default_value = "completion")]
    pub(crate) kind: String,
    #[arg(long)]
    pub(crate) profile: Option<String>,
    #[arg(long, value_name = "PATH")]
    pub(crate) route_audit: Option<PathBuf>,
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
pub(crate) struct DoneArgs {
    pub(crate) item_id: Option<String>,
    #[arg(long)]
    pub(crate) summary: String,
    /// Changed file; repeat the flag or pass a comma-separated list.
    #[arg(long, value_delimiter = ',')]
    pub(crate) files: Vec<String>,
    #[arg(long)]
    pub(crate) cmd: Vec<String>,
    #[arg(long)]
    pub(crate) tests: Vec<String>,
    /// Open a checkpoint through one allowed structured escalation reason.
    #[arg(
        long,
        value_enum,
        requires = "escalation_ref",
        requires = "escalation_explanation"
    )]
    pub(crate) escalate: Option<DoneEscalationReason>,
    /// Stable user, policy, finding, or external-side-effect reference.
    #[arg(long)]
    pub(crate) escalation_ref: Option<String>,
    /// Why the referenced condition requires an interrupting checkpoint.
    #[arg(long)]
    pub(crate) escalation_explanation: Option<String>,
    /// Pick the next ready item after finishing this step.
    #[arg(long)]
    pub(crate) next: bool,
    #[arg(long)]
    pub(crate) profile: Option<String>,
    #[arg(long, value_name = "PATH")]
    pub(crate) route_audit: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum DoneEscalationReason {
    UserRequested,
    PolicyRequired,
    ProtectedRiskDiscovered,
    ExternalSideEffect,
    DataIntegrityRisk,
}

#[derive(Args, Debug)]
pub(crate) struct StopArgs {
    #[command(subcommand)]
    pub(crate) command: Option<StopCommand>,
    /// Read the host hook JSON envelope from a file instead of stdin.
    #[arg(long, value_name = "PATH")]
    pub(crate) input: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum StopCommand {
    /// Activate Stop enforcement for a plan in this explicit session/thread.
    Activate(StopActivateArgs),
    /// Cancel/deactivate Stop enforcement for a plan in this explicit session/thread.
    Deactivate(StopDeactivateArgs),
    /// Explicitly resume bounded Stop enforcement after a terminal unchanged gap.
    Resume(StopActivateArgs),
}

#[derive(Args, Debug)]
pub(crate) struct StopActivateArgs {
    #[arg(long)]
    pub(crate) plan: String,
    #[arg(long)]
    pub(crate) session: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct StopDeactivateArgs {
    #[arg(long)]
    pub(crate) plan: Option<String>,
    #[arg(long)]
    pub(crate) session: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ReviewArgs {
    #[command(subcommand)]
    pub(crate) command: ReviewCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ReviewCommand {
    Annotate(ReviewAnnotateArgs),
    Ingest(ReviewIngestArgs),
    Evidence(ReviewEvidenceArgs),
    Close(ReviewCloseArgs),
    Findings(ReviewFindingsArgs),
    List(ReviewListArgs),
    Show(IdArg),
}

#[derive(Args, Debug)]
pub(crate) struct ReviewFindingsArgs {
    pub(crate) review_gate_id: String,
    /// Finding id repaired by the responsible maker; repeat for all open findings.
    #[arg(long = "resolve", required = true)]
    pub(crate) resolve: Vec<String>,
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
pub(crate) struct ReviewEvidenceArgs {
    pub(crate) item_id: String,
    #[arg(long)]
    pub(crate) pr_url: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ReviewCloseArgs {
    pub(crate) review_gate_id: String,
    #[arg(long, value_enum)]
    pub(crate) verdict: ReviewVerdict,
    #[arg(long)]
    pub(crate) findings: Vec<String>,
    /// Independent reviewer identity recorded on the durable attempt.
    /// Defaults to this process's worker id.
    #[arg(long)]
    pub(crate) reviewer: Option<String>,
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
    /// Only list notes stored with this tag (e.g. `goal-contract`)
    #[arg(long)]
    pub(crate) tag: Option<String>,
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
    Grok(InstallClientArgs),
    Pi(InstallClientArgs),
}

#[derive(Args, Debug)]
pub(crate) struct InstallClientArgs {
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Skip project MCP config. Codex installs hooks only; Claude/Cursor install roles and skills; Grok installs roles/skills with no hooks; Pi has no MCP.
    #[arg(long)]
    pub(crate) no_mcp: bool,
    /// Overwrite Planr-owned role/skill files; otherwise preserve hand edits.
    #[arg(long)]
    pub(crate) force: bool,
    /// Skip installing host hooks (`planr prime` injection). Hooks are installed by default where supported; Grok and Pi have no hook contract.
    #[arg(long)]
    pub(crate) no_hooks: bool,
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
    /// Model-prioritization block from the registry plus host dispatch traps.
    Routing(PromptPrintArgs),
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
