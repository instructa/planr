use serde_json::{Value, json};

fn prop(kind: &str, description: &str) -> Value {
    json!({"type": kind, "description": description})
}

fn string_array(description: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": description})
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}

pub fn mcp_tools() -> Vec<Value> {
    vec![
        tool(
            "planr_project_show",
            "Show current Planr project",
            json!({}),
            &[],
        ),
        tool(
            "planr_trace_item",
            "Show an item trace including durable requested, resolved, and effective route observations",
            json!({"item_id": prop("string", "Item id")}),
            &["item_id"],
        ),
        tool(
            "planr_map_show",
            "Show Planr map (optionally scoped to one plan)",
            json!({"plan": prop("string", "Only show items and links of this plan id")}),
            &[],
        ),
        tool("planr_map_status", "Show richer map status", json!({}), &[]),
        tool(
            "planr_map_preview",
            "Preview graph effects before mutation",
            json!({"close": prop("string", "Item id whose close effect should be previewed")}),
            &["close"],
        ),
        tool(
            "planr_map_unlocks",
            "Show what an item would unlock",
            json!({"item_id": prop("string", "Item id")}),
            &["item_id"],
        ),
        tool(
            "planr_map_lookahead",
            "Show near-term ready and blocked work",
            json!({"from": prop("string", "Optional item id to start the lookahead from"), "limit": prop("integer", "Maximum entries to return (default 10)")}),
            &[],
        ),
        tool(
            "planr_plan_create",
            "Create a product plan package",
            json!({"title": prop("string", "Plan title"), "platform": prop("string", "Target platform label"), "ai": prop("boolean", "Include AI feature planning files"), "backend": prop("boolean", "Include backend planning files")}),
            &["title"],
        ),
        tool(
            "planr_plan_refine",
            "Append refinement context to a plan",
            json!({"id": prop("string", "Plan id"), "note": prop("string", "Refinement note to append")}),
            &["id"],
        ),
        tool(
            "planr_plan_split",
            "Create a build plan from a product plan",
            json!({"id": prop("string", "Source product plan id"), "slice": prop("string", "Build slice name")}),
            &["id", "slice"],
        ),
        tool(
            "planr_plan_check",
            "Validate a plan record and path",
            json!({"id": prop("string", "Plan id")}),
            &["id"],
        ),
        tool(
            "planr_plan_audit",
            "Audit a plan's goal contract: clause-by-clause pass/fail with evidence",
            json!({"id": prop("string", "Plan id")}),
            &["id"],
        ),
        tool(
            "planr_plan_final_review",
            "Create or show the one plan-scoped final product review after outcome work and material item reviews settle",
            json!({"id": prop("string", "Plan id")}),
            &["id"],
        ),
        tool(
            "planr_run_restart",
            "Retire one plan's active FeatureRun only for a typed incompatible-budget, premature-source-freeze, or inconsistent-verification lifecycle reason",
            json!({
                "plan": prop("string", "Plan id"),
                "reason": {
                    "type": "string",
                    "enum": ["incompatible-budget", "premature-source-freeze", "inconsistent-verification"],
                    "description": "Typed restart reason"
                }
            }),
            &["plan", "reason"],
        ),
        tool(
            "planr_run_resolve_budget_hold",
            "Resume one compatible budget-held FeatureRun only after its persisted reservation, deadline, phase, lease generation, and owner are revalidated",
            json!({"plan": prop("string", "Plan id")}),
            &["plan"],
        ),
        tool(
            "planr_run_repair_verification_admission",
            "Atomically repair one exact pre-receipt verification admission failure without requiring a verification item",
            json!({
                "plan_id": prop("string", "Exact plan id"),
                "run_id": prop("string", "Exact FeatureRun id"),
                "freeze_id": prop("string", "Exact active source-freeze id"),
                "run_revision": prop("integer", "Exact optimistic FeatureRun revision"),
                "reason": {
                    "type": "string",
                    "enum": ["readiness-blocked", "run-index-seal-failed", "sealed-run-rejected", "capability-admission-failed"],
                    "description": "Closed pre-receipt repair reason"
                },
                "run_index_digest": prop("string", "Required only for post-seal reasons; forbidden for pre-seal reasons")
            }),
            &["plan_id", "run_id", "freeze_id", "run_revision", "reason"],
        ),
        tool(
            "planr_plan_link",
            "Link a plan source to an item",
            json!({"source_id": prop("string", "Plan source id"), "item_id": prop("string", "Item id"), "relationship": prop("string", "Link relationship (default references)"), "section_id": prop("string", "Optional plan section id")}),
            &["source_id", "item_id"],
        ),
        tool(
            "planr_map_build",
            "Create map items from a plan",
            json!({"from": prop("string", "Plan id to build items from")}),
            &["from"],
        ),
        tool(
            "planr_item_create",
            "Create a map item",
            json!({"title": prop("string", "Item title"), "description": prop("string", "Item description"), "work_type": prop("string", "Work type (default generic)"), "after": prop("string", "Existing item id this item depends on"), "timeout_seconds": prop("integer", "Runtime timeout before the pick is stale"), "max_retries": prop("integer", "Maximum automatic retries"), "retry_delay_ms": prop("integer", "Base retry delay in milliseconds"), "retry_backoff": prop("string", "Retry backoff strategy"), "pre": prop("string", "Pre-condition note"), "post": prop("string", "Post-condition note")}),
            &["title", "description"],
        ),
        tool(
            "planr_item_breakdown",
            "Break an item into chained child items (parent parks as a gate)",
            json!({"id": prop("string", "Parent item id"), "into": prop("string", "Child titles separated by newlines or commas")}),
            &["id", "into"],
        ),
        tool(
            "planr_item_insert",
            "Insert an item between linked work",
            json!({"title": prop("string", "New item title"), "description": prop("string", "New item description"), "after": prop("string", "Item id the new item comes after"), "before": prop("string", "Optional item id the new item comes before"), "confirm": prop("boolean", "Apply the insert instead of previewing")}),
            &["title", "description", "after"],
        ),
        tool(
            "planr_item_amend",
            "Add future-work context to an item",
            json!({"id": prop("string", "Item id"), "note": prop("string", "Amendment content"), "tag": prop("string", "Context kind label (default amendment)")}),
            &["id", "note"],
        ),
        tool(
            "planr_item_replan",
            "Preview or replace pending child work",
            json!({"parent_id": prop("string", "Parent item id"), "into": prop("string", "Comma-separated replacement child titles"), "confirm": prop("boolean", "Apply the replan instead of previewing")}),
            &["parent_id", "into"],
        ),
        tool(
            "planr_agents_list",
            "Show the agent profile registry: profiles, routes, and validation warnings",
            json!({}),
            &[],
        ),
        tool(
            "planr_policy_show",
            "Show the parsed provider-neutral Usage Policy v1 or its missing/degraded state",
            json!({}),
            &[],
        ),
        tool(
            "planr_policy_check",
            "Validate .planr/policy.toml; missing preserves advisory routing and malformed policy fails closed",
            json!({}),
            &[],
        ),
        tool(
            "planr_policy_admit",
            "Evaluate a bounded task contract and execution permission request before delegation",
            json!({"request": prop("object", "ExecutionAdmissionRequest object")}),
            &["request"],
        ),
        tool(
            "planr_item_route",
            "Show an item's resolved advisory route and whether an override or policy won",
            json!({"item_id": prop("string", "Item id")}),
            &["item_id"],
        ),
        tool(
            "planr_item_route_set",
            "Pin an item to an agent profile from the registry (beats every policy route)",
            json!({"item_id": prop("string", "Item id"), "profile": prop("string", "Profile id declared in .planr/agents.toml")}),
            &["item_id", "profile"],
        ),
        tool(
            "planr_item_route_clear",
            "Remove an item's pinned profile so policy routing applies again",
            json!({"item_id": prop("string", "Item id")}),
            &["item_id"],
        ),
        tool(
            "planr_pick_item",
            "Atomically pick the next ready item",
            json!({"work_type": prop("string", "Lease code outcomes, ReviewGates (`review`), or verification work"), "plan": prop("string", "Only lease work belonging to this plan id; ReviewGate and verification leases require it"), "peek": prop("boolean", "Read the next work packet (incl. routing) without leasing it; the worker takes the lease")}),
            &[],
        ),
        tool(
            "planr_pick_heartbeat",
            "Record worker heartbeat for picked work",
            json!({"item_id": prop("string", "Item id (defaults to this worker's current pick)")}),
            &[],
        ),
        tool(
            "planr_pick_progress",
            "Record progress for picked work",
            json!({"item_id": prop("string", "Item id"), "percent": prop("integer", "Progress percent 0-100"), "note": prop("string", "Optional progress note")}),
            &["item_id"],
        ),
        tool(
            "planr_pick_pause",
            "Pause picked work without releasing it",
            json!({"item_id": prop("string", "Item id"), "note": prop("string", "Optional pause note")}),
            &["item_id"],
        ),
        tool(
            "planr_pick_resume",
            "Resume picked work",
            json!({"item_id": prop("string", "Item id")}),
            &["item_id"],
        ),
        tool(
            "planr_pick_stale",
            "Inspect stale picked work",
            json!({"older_than_seconds": prop("integer", "Staleness threshold in seconds (default 900)")}),
            &[],
        ),
        tool(
            "planr_recover_sweep",
            "Preview or apply stale, timed-out, and retryable recovery",
            json!({"older_than_seconds": prop("integer", "Staleness threshold in seconds (default 900)"), "apply": prop("boolean", "Apply recovery instead of previewing")}),
            &[],
        ),
        tool(
            "planr_approval_request",
            "Request human approval before close",
            json!({"item_id": prop("string", "Item id"), "reason": prop("string", "Why approval is needed")}),
            &["item_id"],
        ),
        tool(
            "planr_approval_approve",
            "Approve item close gate",
            json!({"item_id": prop("string", "Item id"), "by": prop("string", "Approver identity"), "comment": prop("string", "Optional approval comment")}),
            &["item_id", "by"],
        ),
        tool(
            "planr_approval_deny",
            "Deny item close gate",
            json!({"item_id": prop("string", "Item id"), "by": prop("string", "Denier identity"), "comment": prop("string", "Optional denial comment")}),
            &["item_id", "by"],
        ),
        tool(
            "planr_approval_list",
            "List item approval gates",
            json!({"open": prop("boolean", "Only list open approval requests")}),
            &[],
        ),
        tool(
            "planr_artifact_add",
            "Attach artifact metadata or small content",
            json!({"name": prop("string", "Artifact name"), "item": prop("string", "Optional item id"), "kind": prop("string", "Artifact kind (default evidence)"), "path": prop("string", "Optional file path reference"), "content": prop("string", "Optional inline content"), "mime": prop("string", "MIME type (default text/plain)")}),
            &["name"],
        ),
        tool(
            "planr_artifact_list",
            "List artifacts",
            json!({"item": prop("string", "Optional item id filter")}),
            &[],
        ),
        tool(
            "planr_artifact_show",
            "Show artifact details",
            json!({"id": prop("string", "Artifact id")}),
            &["id"],
        ),
        tool(
            "planr_event_list",
            "List persisted events",
            json!({"item": prop("string", "Optional item id filter"), "limit": prop("integer", "Maximum events (default 50)")}),
            &[],
        ),
        tool(
            "planr_debug_bundle",
            "Preview a privacy-minimized debug bundle",
            json!({"item": prop("string", "Optional item id filter")}),
            &[],
        ),
        tool(
            "planr_evidence_policy",
            "Validate and show the repository Evidence policy",
            json!({}),
            &[],
        ),
        tool(
            "planr_evidence_obligation_list",
            "List proof obligations, optionally filtered by plan, item, or criterion",
            json!({"plan": prop("string", "Optional plan id"), "item": prop("string", "Optional item id"), "criterion": prop("string", "Optional criterion id")}),
            &[],
        ),
        tool(
            "planr_evidence_obligation_show",
            "Show one proof obligation",
            json!({"id": prop("string", "Proof obligation id")}),
            &["id"],
        ),
        tool(
            "planr_evidence_migrate",
            "Preview or apply explicit plan-scoped migration from legacy verification claims to binding Evidence obligations",
            json!({"input": prop("object", "planr.evidence.migration.v1 payload with plan_id and obligations[]"), "apply": prop("boolean", "Apply the migration; omit or false for preview")}),
            &["input"],
        ),
        tool(
            "planr_evidence_classifications",
            "Show canonical Evidence gap reasons and legacy/operator aliases",
            json!({}),
            &[],
        ),
        tool(
            "planr_evidence_capability_list",
            "List verification capability manifests and instances",
            json!({}),
            &[],
        ),
        tool(
            "planr_evidence_capability_show",
            "Show one verification capability instance",
            json!({"id": prop("string", "Capability instance id")}),
            &["id"],
        ),
        tool(
            "planr_evidence_run",
            "Run a configured process capability and record trusted process evidence",
            json!({"input": prop("object", "Untrusted run request with obligation_id, capability_instance_id, execution_contract, target, and environment")}),
            &["input"],
        ),
        tool(
            "planr_evidence_import",
            "Import an untrusted validated artifact proposal without allowing trusted receipt construction",
            json!({"input": prop("object", "Untrusted evidence proposal"), "artifact_root": prop("string", "Directory that contains referenced artifacts")}),
            &["input", "artifact_root"],
        ),
        tool(
            "planr_evidence_host_capture_import",
            "Validate and persist a fresh external Codex host capture as trusted Evidence",
            json!({"input": prop("object", "planr.evidence.host_capture.import.v1 payload with obligation_id and import_root")}),
            &["input"],
        ),
        tool(
            "planr_evidence_host_capture_run",
            "Run a policy-registered host capture helper and persist Planr-observed Evidence",
            json!({"input": prop("object", "planr.evidence.host_capture.run.v1 payload with obligation_id and manifest_id")}),
            &["input"],
        ),
        tool(
            "planr_evidence_attempts",
            "List or show Evidence process attempts",
            json!({"id": prop("string", "Optional attempt id"), "obligation": prop("string", "Optional obligation id filter")}),
            &[],
        ),
        tool(
            "planr_evidence_receipts",
            "List or show trusted Evidence receipts",
            json!({"id": prop("string", "Optional receipt id"), "obligation": prop("string", "Optional obligation id filter")}),
            &[],
        ),
        tool(
            "planr_evidence_coverage",
            "Evaluate Evidence coverage for an obligation, criterion, item, or plan",
            json!({"scope": prop("string", "obligation, criterion, item, or plan"), "id": prop("string", "Scope id")}),
            &["scope", "id"],
        ),
        tool(
            "planr_evidence_explain",
            "Explain Evidence coverage with policy, attempts, receipts, and repository snapshot context",
            json!({"scope": prop("string", "obligation, criterion, item, or plan"), "id": prop("string", "Scope id")}),
            &["scope", "id"],
        ),
        tool(
            "planr_evidence_readiness",
            "Check active Evidence obligations, payload schemas, registered capabilities, and runtime availability before goal work",
            json!({"scope": prop("string", "obligation, criterion, item, or plan"), "id": prop("string", "Scope id")}),
            &["scope", "id"],
        ),
        tool(
            "planr_evidence_recover_settlement",
            "Recover exact verified maker continuation, complete an exact stranded verified continuation, backfill proven risk-review obligation lineage, or reconcile a proven superseded historical invalidation",
            json!({"input": prop("object", "planr.evidence.recover_settlement.v1, planr.evidence.recover_verified_continuation.v1, planr.evidence.backfill_risk_review_obligations.v1, or planr.evidence.reconcile_historical_invalidation.v1 payload")}),
            &["input"],
        ),
        tool(
            "planr_eval_suite_check",
            "Store or verify an immutable eval suite snapshot",
            json!({"input": prop("object", "Suite snapshot object with digest and normalized_manifest"), "source_path": prop("string", "Optional source path for the suite manifest")}),
            &["input"],
        ),
        tool(
            "planr_eval_run",
            "Start an eval run, optionally record case evidence, and optionally finish it",
            json!({"input": prop("object", "Run object with suite_digest, subject, optional cases, and optional status")}),
            &["input"],
        ),
        tool(
            "planr_eval_show",
            "Show a stored eval suite, run, comparison, or invalidation",
            json!({"kind": prop("string", "suite, run, comparison, or invalidation"), "id": prop("string", "Eval record id")}),
            &["kind", "id"],
        ),
        tool(
            "planr_eval_compare",
            "Compare two stored eval runs and persist the comparison",
            json!({"baseline_run_id": prop("string", "Baseline eval run id"), "candidate_run_id": prop("string", "Candidate eval run id"), "policy_digest": prop("string", "Comparison policy digest (default)")}),
            &["baseline_run_id", "candidate_run_id"],
        ),
        tool(
            "planr_eval_gate",
            "Gate on a stored eval comparison verdict",
            json!({"comparison_id": prop("string", "Eval comparison id")}),
            &["comparison_id"],
        ),
        tool(
            "planr_eval_invalidate",
            "Invalidate an eval run or comparison",
            json!({"target_kind": prop("string", "run or comparison"), "target_id": prop("string", "Target id"), "reason": prop("string", "Invalidation reason"), "reason_codes": string_array("Machine-readable reason codes"), "replacement_hint": prop("string", "Optional replacement hint")}),
            &["target_kind", "target_id", "reason"],
        ),
        tool(
            "planr_eval_rescore",
            "Start a rescore eval run from an existing run",
            json!({"run_id": prop("string", "Source eval run id"), "id": prop("string", "Optional id for the rescore run")}),
            &["run_id"],
        ),
        tool(
            "planr_eval_evidence_ref",
            "Attach an eval run or comparison to an existing Planr log or artifact without closing work",
            json!({"target_kind": prop("string", "run or comparison"), "target_id": prop("string", "Eval target id"), "attachment_kind": prop("string", "log or artifact"), "attachment_id": prop("string", "Planr attachment id"), "item_id": prop("string", "Item id that owns the evidence")}),
            &[
                "target_kind",
                "target_id",
                "attachment_kind",
                "attachment_id",
                "item_id",
            ],
        ),
        tool(
            "planr_log_add",
            "Add evidence log to an item",
            json!({"item": prop("string", "Item id"), "summary": prop("string", "What was done"), "kind": prop("string", "Log kind (default completion)"), "files": string_array("Changed file paths"), "commands": string_array("Commands run"), "tests": string_array("Tests run with results"), "profile": prop("string", "Registry profile the run actually executed on (advisory mismatch check)"), "route_observation": prop("object", "Requested/resolved/effective route, transition, policy/binding provenance, and metering")}),
            &["item", "summary"],
        ),
        tool(
            "planr_review_annotate",
            "Attach review annotation feedback",
            json!({"item_id": prop("string", "Item id"), "message": prop("string", "Annotation message"), "severity": prop("string", "Severity (default info)"), "author": prop("string", "Annotation author"), "file": prop("string", "File path the annotation refers to"), "line": prop("integer", "Line number the annotation refers to")}),
            &["item_id", "message"],
        ),
        json!({
            "name": "planr_review_ingest",
            "description": "Ingest hook-compatible review feedback",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "item_id": prop("string", "Item id"),
                    "feedback": {"type": "object", "description": "Review feedback payload"},
                    "payload": {"type": "object", "description": "Alternative feedback payload key"}
                },
                "required": ["item_id"],
                "additionalProperties": true
            }
        }),
        tool(
            "planr_review_evidence",
            "Collect scoped Git and PR evidence for an item",
            json!({"item_id": prop("string", "Item id"), "pr_url": prop("string", "Optional pull request URL to record")}),
            &["item_id"],
        ),
        tool(
            "planr_review_gate_close",
            "Complete a leased durable ReviewGate attempt",
            json!({"review_gate_id": prop("string", "ReviewGate id"), "verdict": prop("string", "Verdict: complete/accepted or changes-requested"), "findings": string_array("Findings discovered during review"), "reviewer": prop("string", "Independent reviewer identity (defaults to the worker id)")}),
            &["review_gate_id"],
        ),
        tool(
            "planr_review_findings_resolve",
            "Resolve durable ReviewGate findings and return the same gate to pending re-review",
            json!({"review_gate_id": prop("string", "ReviewGate id"), "finding_ids": string_array("Finding ids resolved by the responsible maker")}),
            &["review_gate_id", "finding_ids"],
        ),
        tool(
            "planr_close_item",
            "Settle an outcome through the canonical FeatureRun service",
            json!({"item_id": prop("string", "Item id"), "summary": prop("string", "Completion summary"), "files": string_array("Changed files"), "commands": string_array("Commands run"), "tests": string_array("Tests run"), "profile": prop("string", "Executed route profile"), "escalation_reason": prop("string", "Structured escalation reason"), "escalation_reference": prop("string", "Required durable escalation reference"), "escalation_explanation": prop("string", "Required escalation explanation")}),
            &["item_id"],
        ),
        tool(
            "planr_context_create",
            "Add project or item context",
            json!({"content": prop("string", "Context content"), "item": prop("string", "Optional item id"), "kind": prop("string", "Context kind label (default discovery)")}),
            &["content"],
        ),
        tool(
            "planr_search",
            "Search items, plans, logs, and context",
            json!({"query": prop("string", "Search query")}),
            &["query"],
        ),
        tool(
            "planr_log_read",
            "Read one log entry",
            json!({"id": prop("string", "Log id")}),
            &["id"],
        ),
    ]
}
