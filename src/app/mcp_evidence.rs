use super::App;
use crate::cli::EvidenceCoverageScope;
use crate::integrations::mcp_json;
use crate::util::required_arg;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};

impl App {
    pub(crate) fn mcp_evidence_tool_call(&self, name: &str, args: Value) -> Result<Value> {
        match name {
            "planr_evidence_policy" => Ok(mcp_evidence_json(
                "evidence.policy",
                self.evidence_policy_value(),
            )),
            "planr_evidence_obligation_add" => Ok(mcp_evidence_json(
                "evidence.obligation.add",
                self.evidence_obligation_add_value(
                    args.get("input").cloned().unwrap_or_else(|| args.clone()),
                ),
            )),
            "planr_evidence_obligation_list" => Ok(mcp_evidence_json(
                "evidence.obligation.list",
                self.evidence_obligations_value(
                    args.get("plan").and_then(Value::as_str),
                    args.get("item").and_then(Value::as_str),
                    args.get("criterion").and_then(Value::as_str),
                ),
            )),
            "planr_evidence_obligation_show" => Ok(mcp_evidence_json(
                "evidence.obligation.show",
                required_arg(&args, "id").and_then(|id| self.evidence_obligation_value(id)),
            )),
            "planr_evidence_migrate" => Ok(mcp_evidence_json(
                "evidence.migrate",
                super::evidence::evidence_migration_request(&args)
                    .and_then(|(input, apply)| self.evidence_migration_value(input, apply)),
            )),
            "planr_evidence_classifications" => Ok(mcp_evidence_json(
                "evidence.classifications",
                Ok(super::evidence::evidence_classifications_value()),
            )),
            "planr_evidence_capability_list" => Ok(mcp_evidence_json(
                "evidence.capability.list",
                self.evidence_capabilities_value(),
            )),
            "planr_evidence_capability_show" => Ok(mcp_evidence_json(
                "evidence.capability.show",
                required_arg(&args, "id").and_then(|id| self.evidence_capability_value(id)),
            )),
            "planr_evidence_run" => Ok(mcp_evidence_json(
                "evidence.run",
                self.evidence_run_value(args.get("input").cloned().unwrap_or_else(|| args.clone())),
            )),
            "planr_evidence_import" => Ok(mcp_evidence_json(
                "evidence.import",
                args.get("artifact_root")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("missing artifact_root"))
                    .and_then(|artifact_root| {
                        self.evidence_import_value(
                            args.get("input").cloned().unwrap_or_else(|| args.clone()),
                            std::path::Path::new(artifact_root),
                        )
                    }),
            )),
            "planr_evidence_host_capture_import" => Ok(mcp_evidence_json(
                "evidence.host_capture.import",
                self.evidence_host_capture_import_value(
                    args.get("input").cloned().unwrap_or_else(|| args.clone()),
                ),
            )),
            "planr_evidence_host_capture_run" => Ok(mcp_evidence_json(
                "evidence.host_capture.run",
                self.evidence_host_capture_run_value(
                    args.get("input").cloned().unwrap_or_else(|| args.clone()),
                ),
            )),
            "planr_evidence_attempts" => Ok(mcp_evidence_json(
                "evidence.attempts",
                self.evidence_attempts_value(
                    args.get("id").and_then(Value::as_str),
                    args.get("obligation").and_then(Value::as_str),
                ),
            )),
            "planr_evidence_receipts" => Ok(mcp_evidence_json(
                "evidence.receipts",
                self.evidence_receipts_value(
                    args.get("id").and_then(Value::as_str),
                    args.get("obligation").and_then(Value::as_str),
                ),
            )),
            "planr_evidence_coverage" => Ok(mcp_evidence_json(
                "evidence.coverage",
                evidence_scope_arg(&args).and_then(|scope| {
                    required_arg(&args, "id").and_then(|id| self.evidence_coverage_value(scope, id))
                }),
            )),
            "planr_evidence_explain" => Ok(mcp_evidence_json(
                "evidence.explain",
                evidence_scope_arg(&args).and_then(|scope| {
                    required_arg(&args, "id").and_then(|id| self.evidence_explain_value(scope, id))
                }),
            )),
            "planr_evidence_readiness" => Ok(mcp_evidence_json(
                "evidence.readiness",
                evidence_scope_arg(&args).and_then(|scope| {
                    required_arg(&args, "id")
                        .and_then(|id| self.evidence_readiness_value(scope, id))
                }),
            )),
            _ => Err(anyhow!("unknown Planr MCP tool: {name}")),
        }
    }
}

fn mcp_evidence_json(command: &str, result: Result<Value>) -> Value {
    match result {
        Ok(object) => mcp_json(super::evidence::evidence_success_envelope(command, object)),
        Err(error) => {
            let envelope = super::evidence::evidence_error_envelope(command, &error);
            json!({
                "content": [{"type": "text", "text": envelope.to_string()}],
                "isError": true
            })
        }
    }
}

fn evidence_scope_arg(args: &Value) -> Result<EvidenceCoverageScope> {
    match args
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("obligation")
    {
        "obligation" => Ok(EvidenceCoverageScope::Obligation),
        "criterion" => Ok(EvidenceCoverageScope::Criterion),
        "item" => Ok(EvidenceCoverageScope::Item),
        "plan" => Ok(EvidenceCoverageScope::Plan),
        scope => Err(anyhow!("unsupported evidence scope: {scope}")),
    }
}
