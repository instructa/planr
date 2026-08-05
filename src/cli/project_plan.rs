use super::{ClientArg, IdArg, JsonOnlyArgs, PlanStageArg, PromptPrintArgs};
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub(crate) struct AgentsArgs {
    #[command(subcommand)]
    pub(crate) command: AgentsCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum AgentsCommand {
    /// Show resolved profiles, routes, and validation warnings.
    List(JsonOnlyArgs),
    /// Validate the registry; exits non-zero only on parse failure.
    Check,
    /// Write a commented starter registry with cost-tiering defaults.
    Init(AgentsInitArgs),
    /// The driver dispatch block (alias for `planr prompt routing`).
    Routing(PromptPrintArgs),
}

#[derive(Args, Debug)]
pub(crate) struct AgentsInitArgs {
    /// Overwrite an existing .planr/agents.toml.
    #[arg(long)]
    pub(crate) force: bool,
    /// Declare a profile: <id>=<client>/<model>[@<effort>][#<tier>]. Repeatable.
    #[arg(long = "profile", value_name = "SPEC")]
    pub(crate) profiles: Vec<String>,
    /// Pair a declared profile with a skill: <profile>=<skill>. Repeatable.
    #[arg(long = "skill", value_name = "SPEC")]
    pub(crate) skills: Vec<String>,
    /// Route a work type: <work_type>=<profile>[,<fallback>...]. Repeatable.
    #[arg(long = "route", value_name = "SPEC")]
    pub(crate) routes: Vec<String>,
    /// Default route: <profile>[,<fallback>...].
    #[arg(long = "default-route", value_name = "SPEC")]
    pub(crate) default_route: Option<String>,
    /// Build the registry through guided prompts (requires a terminal).
    #[arg(
        long,
        conflicts_with_all = ["profiles", "skills", "routes", "default_route"]
    )]
    pub(crate) interactive: bool,
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
    /// Preview or atomically apply canonical project path relocation.
    Relocate(ProjectRelocateArgs),
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
pub(crate) struct ProjectRelocateArgs {
    pub(crate) project_id: String,
    #[arg(long)]
    pub(crate) destination: PathBuf,
    /// Apply the validated relocation. Without this flag the command is read-only.
    #[arg(long)]
    pub(crate) apply: bool,
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
    Audit(IdArg),
    /// Create or show the one plan-scoped final product review.
    FinalReview(IdArg),
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
