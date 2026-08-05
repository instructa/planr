use super::{IdArg, JsonOnlyArgs};
use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub(crate) enum PolicyCommand {
    /// Show the parsed repository usage policy.
    Show(JsonOnlyArgs),
    /// Validate the repository usage policy.
    Check,
    /// Preview or atomically apply a supported pre-release policy upgrade.
    Upgrade(PolicyUpgradeArgs),
    /// Evaluate an execution request against the repository usage policy.
    Admit(IdArg),
}

#[derive(Args, Debug)]
pub(crate) struct PolicyUpgradeArgs {
    /// Atomically replace the legacy policy with the exact previewed canonical policy.
    #[arg(long)]
    pub(crate) apply: bool,
}
