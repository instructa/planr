//! `planr agents init`: writes the starter agent registry. Owns the
//! static cost-tiering scaffold today and, per the agent-pool plan, the
//! flag-driven spec builder and interactive wizard next — kept out of
//! `src/app/agents.rs` so the routing surface module stays within its
//! ownership budget.

use super::App;
use crate::agents::{REGISTRY_RELATIVE_PATH, registry_path};
use crate::cli::AgentsInitArgs;
use crate::util::write_if_missing;
use anyhow::{Result, bail};
use serde_json::json;

/// The `agents init` starter registry: the cost-tiering defaults from
/// docs/GOALS.md as a working file — premium driver that keeps the
/// verdicts, standard steerable implementer, budget helper for
/// token-hungry side work. Must always parse with zero `agents check`
/// warnings (e2e-asserted) so adoption never starts from a broken file.
const REGISTRY_SCAFFOLD: &str = r#"# Planr agent profile registry: advisory model routing for pick packets.
# Planr never dispatches models; hosts stay the authority.
#
# How to tier (docs/GOALS.md "Cost Tiering"):
#   - Tier by effective cost to you (subscription vs API), not sticker price.
#   - Workers can run cheaper: the pick packet bounds their scope.
#   - Verdicts stay premium: never route review work to a budget tier.
#   - Aliases track model generations; pin full ids only when you need
#     determinism.
#
# After editing: `planr agents check`, then `planr install <client> --force`
# to re-render the host role files with the new pins.

# The premium planner/architect/judge.
# client is the host that dispatches it: codex | claude-code | cursor | generic-mcp.
[profiles.driver]
client = "cursor"
model = "fable-5"
effort = "high"
cost_tier = "premium"
capabilities = ["orchestration", "review", "planning"]
notes = "Planner and judge. Verdicts stay on this tier."

# The steerable everyday implementer: strong, fast, cheap on subscription.
[profiles.implementer]
client = "codex"
model = "gpt-5.5"
effort = "xhigh"
cost_tier = "standard"
capabilities = ["code", "steerable"]
notes = "Primary implementer for scoped map items."

# Token-hungry side work (browser verification, codebase analysis) goes to
# the budget tier; results are reported back to the driver.
[profiles.helper]
client = "generic-mcp"
model = "composer-2.5"
effort = "medium"
cost_tier = "budget"
capabilities = ["browser", "analysis"]
notes = "Cheap capacity for verification and analysis side work."

# First matching route wins; per-item pins (`planr item route <id> --set`)
# beat every route below.
[[routes]]
match = { work_type = "code" }
profile = "implementer"
fallbacks = ["driver"]

[[routes]]
match = { work_type = "fix" }
profile = "implementer"
fallbacks = ["driver"]

# Review stays on the strongest tier; routing it to a budget profile
# draws an `agents check` warning.
[[routes]]
match = { work_type = "review" }
profile = "driver"

[route_default]
profile = "implementer"
fallbacks = ["driver"]
"#;

impl App {
    pub(crate) fn agents_init(&self, args: AgentsInitArgs) -> Result<()> {
        let path = registry_path(&self.root);
        if path.exists() && !args.force {
            bail!(
                "{REGISTRY_RELATIVE_PATH} already exists and is never overwritten; edit it directly or re-run with --force to replace it with the scaffold"
            );
        }
        write_if_missing(&path, REGISTRY_SCAFFOLD, args.force)?;
        self.record_event(
            "agents_registry_initialized",
            None,
            json!({"path": REGISTRY_RELATIVE_PATH, "forced": args.force}),
        )?;
        self.emit(
            json!({
                "path": REGISTRY_RELATIVE_PATH,
                "created": true,
                "next": [
                    "planr agents check",
                    "planr install <client> --force",
                ],
            }),
            format!(
                "wrote {REGISTRY_RELATIVE_PATH} (cost-tiering starter: premium driver, standard implementer, budget helper)\nnext: `planr agents check` to validate, then `planr install <client> --force` to render the host role files with the pins"
            ),
        )
    }
}
