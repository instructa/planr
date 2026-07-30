use clap::Args;

#[derive(Args, Debug)]
pub(crate) struct PrimeArgs {
    /// Emit the Claude Code SessionStart hook envelope
    /// (hookSpecificOutput.additionalContext).
    #[arg(long, conflicts_with = "cursor_json")]
    pub(crate) hook_json: bool,
    /// Emit the Cursor command-hook envelope (additional_context).
    #[arg(long)]
    pub(crate) cursor_json: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PrimeEnvelope {
    Plain,
    HookJson,
    CursorJson,
}

impl PrimeArgs {
    pub(crate) fn envelope(&self) -> PrimeEnvelope {
        if self.hook_json {
            PrimeEnvelope::HookJson
        } else if self.cursor_json {
            PrimeEnvelope::CursorJson
        } else {
            PrimeEnvelope::Plain
        }
    }
}
