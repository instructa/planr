use super::{IdArg, JsonOnlyArgs};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub(crate) enum PolicyCommand {
    /// Show the parsed repository usage policy.
    Show(JsonOnlyArgs),
    /// Validate the repository usage policy.
    Check,
    /// Evaluate an execution request against the repository usage policy.
    Admit(IdArg),
}
