use super::App;
use crate::cli::EvidenceCoverageScope;
use anyhow::{Result, anyhow};
use serde_json::Value;

impl App {
    pub(crate) fn http_evidence_route(
        &self,
        method: &str,
        path: &str,
        query: &str,
        body_json: &Value,
    ) -> Result<(&'static str, String)> {
        match (method, path) {
            ("GET", "/v1/evidence/policy") => {
                http_evidence_json("evidence.policy", self.evidence_policy_value())
            }
            ("GET", "/v1/evidence/obligations") => http_evidence_json(
                "evidence.obligation.list",
                self.evidence_obligations_value(
                    query_param(query, "plan").as_deref(),
                    query_param(query, "item").as_deref(),
                    query_param(query, "criterion").as_deref(),
                ),
            ),
            ("POST", "/v1/evidence/obligations") => http_evidence_json(
                "evidence.obligation.add",
                self.evidence_obligation_add_value(body_json.clone()),
            ),
            ("POST", "/v1/evidence/migrate") => http_evidence_json(
                "evidence.migrate",
                super::evidence::evidence_migration_request(body_json)
                    .and_then(|(input, apply)| self.evidence_migration_value(input, apply)),
            ),
            ("GET", "/v1/evidence/classifications") => http_evidence_json(
                "evidence.classifications",
                Ok(super::evidence::evidence_classifications_value()),
            ),
            ("GET", p) if p.starts_with("/v1/evidence/obligations/") => {
                let id = p.trim_start_matches("/v1/evidence/obligations/");
                http_evidence_json(
                    "evidence.obligation.show",
                    self.evidence_obligation_value(id),
                )
            }
            ("GET", "/v1/evidence/capabilities") => http_evidence_json(
                "evidence.capability.list",
                self.evidence_capabilities_value(),
            ),
            ("GET", p) if p.starts_with("/v1/evidence/capabilities/") => {
                let id = p.trim_start_matches("/v1/evidence/capabilities/");
                http_evidence_json(
                    "evidence.capability.show",
                    self.evidence_capability_value(id),
                )
            }
            ("POST", "/v1/evidence/run") => {
                http_evidence_json("evidence.run", self.evidence_run_value(body_json.clone()))
            }
            ("POST", "/v1/evidence/import") => {
                let artifact_root = body_json
                    .get("artifact_root")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("missing artifact_root"))?;
                let input = body_json
                    .get("input")
                    .cloned()
                    .unwrap_or_else(|| body_json.clone());
                http_evidence_json(
                    "evidence.import",
                    self.evidence_import_value(input, std::path::Path::new(artifact_root)),
                )
            }
            ("POST", "/v1/evidence/host-capture/import") => http_evidence_json(
                "evidence.host_capture.import",
                self.evidence_host_capture_import_value(
                    body_json
                        .get("input")
                        .cloned()
                        .unwrap_or_else(|| body_json.clone()),
                ),
            ),
            ("POST", "/v1/evidence/host-capture/run") => http_evidence_json(
                "evidence.host_capture.run",
                self.evidence_host_capture_run_value(
                    body_json
                        .get("input")
                        .cloned()
                        .unwrap_or_else(|| body_json.clone()),
                ),
            ),
            ("GET", "/v1/evidence/attempts") => http_evidence_json(
                "evidence.attempts",
                self.evidence_attempts_value(
                    query_param(query, "id").as_deref(),
                    query_param(query, "obligation").as_deref(),
                ),
            ),
            ("GET", p) if p.starts_with("/v1/evidence/attempts/") => {
                let id = p.trim_start_matches("/v1/evidence/attempts/");
                http_evidence_json(
                    "evidence.attempts",
                    self.evidence_attempts_value(Some(id), None),
                )
            }
            ("GET", "/v1/evidence/receipts") => http_evidence_json(
                "evidence.receipts",
                self.evidence_receipts_value(
                    query_param(query, "id").as_deref(),
                    query_param(query, "obligation").as_deref(),
                ),
            ),
            ("GET", p) if p.starts_with("/v1/evidence/receipts/") => {
                let id = p.trim_start_matches("/v1/evidence/receipts/");
                http_evidence_json(
                    "evidence.receipts",
                    self.evidence_receipts_value(Some(id), None),
                )
            }
            ("POST", "/v1/evidence/coverage") => http_evidence_json(
                "evidence.coverage",
                http_evidence_scope(body_json).and_then(|scope| {
                    body_json
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("missing id"))
                        .and_then(|id| self.evidence_coverage_value(scope, id))
                }),
            ),
            ("POST", "/v1/evidence/explain") => http_evidence_json(
                "evidence.explain",
                http_evidence_scope(body_json).and_then(|scope| {
                    body_json
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("missing id"))
                        .and_then(|id| self.evidence_explain_value(scope, id))
                }),
            ),
            ("POST", "/v1/evidence/readiness") => http_evidence_json(
                "evidence.readiness",
                http_evidence_scope(body_json).and_then(|scope| {
                    body_json
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("missing id"))
                        .and_then(|id| self.evidence_readiness_value(scope, id))
                }),
            ),
            ("POST", "/v1/evidence/recover-settlement") => http_evidence_json(
                "evidence.recover_settlement",
                self.recover_verification_settlement_value(
                    body_json
                        .get("input")
                        .cloned()
                        .unwrap_or_else(|| body_json.clone()),
                ),
            ),
            _ => Err(anyhow!("route not found: {method} {path}")),
        }
    }
}

fn http_evidence_json(command: &str, result: Result<Value>) -> Result<(&'static str, String)> {
    let (status, envelope) = match result {
        Ok(object) => {
            let envelope = super::evidence::evidence_success_envelope(command, object);
            let status = match super::evidence::evidence_envelope_exit_code(&envelope) {
                super::evidence::EVIDENCE_OK => "200 OK",
                super::evidence::EVIDENCE_UNSATISFIED => "422 Unprocessable Entity",
                super::evidence::EVIDENCE_BLOCKED => "503 Service Unavailable",
                _ => "500 Internal Server Error",
            };
            (status, envelope)
        }
        Err(error) => {
            let envelope = super::evidence::evidence_error_envelope(command, &error);
            let code = envelope["error"]["code"]
                .as_str()
                .unwrap_or("internal_error");
            let status = match code {
                "not_found" => "404 Not Found",
                "conflict" => "409 Conflict",
                "internal_error" => "500 Internal Server Error",
                _ => "400 Bad Request",
            };
            (status, envelope)
        }
    };
    Ok((status, serde_json::to_string(&envelope)?))
}

fn http_evidence_scope(value: &Value) -> Result<EvidenceCoverageScope> {
    match value
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

fn query_param(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| crate::util::url_decode(value))
}
