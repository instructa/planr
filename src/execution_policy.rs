//! Provider-neutral execution permissions and pre-dispatch admission.
//!
//! This is deliberately separate from model routing. It decides whether a
//! bounded task may execute with declared filesystem, network, tool, command,
//! environment, hook, secret, and approval permissions; it never selects a
//! model or mutates map state.

use crate::usage_policy::{
    BudgetAmounts, BudgetSnapshot, ExecutionBudget, FeatureRunBudgetContract, FeatureRunBudgetMode,
    PolicyDiagnostic, TaskContract, UsageLimits, deadline_unix_ms, validate_task_contract,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPolicy {
    pub max_read_scope_entries: u32,
    pub max_write_scope_entries: u32,
    pub roles: BTreeMap<String, RolePermissions>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RolePermissions {
    pub filesystem: FilesystemPermissions,
    #[serde(default)]
    pub network_hosts: BTreeSet<String>,
    #[serde(default)]
    pub tools: BTreeSet<String>,
    #[serde(default)]
    pub mcp_servers: BTreeSet<String>,
    #[serde(default)]
    pub commands: BTreeSet<CommandSpec>,
    #[serde(default)]
    pub environment: BTreeSet<String>,
    #[serde(default)]
    pub hooks: BTreeSet<String>,
    #[serde(default)]
    pub secret_references: BTreeSet<String>,
    #[serde(default)]
    pub approvals: BTreeSet<ApprovalKind>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemPermissions {
    #[serde(default)]
    pub read_roots: BTreeSet<String>,
    #[serde(default)]
    pub write_roots: BTreeSet<String>,
    #[serde(default)]
    pub allow_overwrite: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    Write,
    Overwrite,
    Network,
    Tool,
    Command,
    Environment,
    Hook,
    Secret,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestructiveOperation {
    DeletePath,
    ForcePush,
    HardReset,
    PrivilegedCommand,
    GlobalConfigurationWrite,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionRequest {
    #[serde(default)]
    pub network_hosts: BTreeSet<String>,
    #[serde(default)]
    pub tools: BTreeSet<String>,
    #[serde(default)]
    pub mcp_servers: BTreeSet<String>,
    #[serde(default)]
    pub commands: BTreeSet<CommandSpec>,
    #[serde(default)]
    pub environment: BTreeSet<String>,
    #[serde(default)]
    pub hooks: BTreeSet<String>,
    #[serde(default)]
    pub secret_references: BTreeSet<String>,
    #[serde(default)]
    pub overwrite_existing: bool,
    #[serde(default)]
    pub destructive_operations: BTreeSet<DestructiveOperation>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IsolationMode {
    Shared,
    Worktree { id: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveExecution {
    pub item_id: String,
    #[serde(default)]
    pub read_scope: Vec<String>,
    #[serde(default)]
    pub write_scope: Vec<String>,
    pub isolation: IsolationMode,
    pub scope_known: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConcurrencySnapshot {
    pub active_agents: u32,
    pub parallel_readers: u32,
    pub parallel_writers: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAdmissionRequest {
    pub item_id: String,
    pub pick_token: String,
    pub role: String,
    pub contract: TaskContract,
    #[serde(default)]
    pub permissions: PermissionRequest,
    pub isolation: IsolationMode,
    #[serde(default)]
    pub approvals: BTreeSet<ApprovalKind>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionDiff {
    #[serde(default)]
    pub read_scope: BTreeSet<String>,
    #[serde(default)]
    pub write_scope: BTreeSet<String>,
    #[serde(default)]
    pub network_hosts: BTreeSet<String>,
    #[serde(default)]
    pub tools: BTreeSet<String>,
    #[serde(default)]
    pub mcp_servers: BTreeSet<String>,
    #[serde(default)]
    pub commands: BTreeSet<CommandSpec>,
    #[serde(default)]
    pub environment: BTreeSet<String>,
    #[serde(default)]
    pub hooks: BTreeSet<String>,
    #[serde(default)]
    pub secret_references: BTreeSet<String>,
    pub overwrite_existing: bool,
}

impl PermissionDiff {
    pub fn is_empty(&self) -> bool {
        self.read_scope.is_empty()
            && self.write_scope.is_empty()
            && self.network_hosts.is_empty()
            && self.tools.is_empty()
            && self.mcp_servers.is_empty()
            && self.commands.is_empty()
            && self.environment.is_empty()
            && self.hooks.is_empty()
            && self.secret_references.is_empty()
            && !self.overwrite_existing
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExecutionAdmission {
    Allowed {
        role: String,
        permission_diff: PermissionDiff,
        concurrent_with: Vec<String>,
    },
    Rejected {
        reason: AdmissionRejectionReason,
        message: String,
        repair: String,
        safety_stop: bool,
        permission_diff: PermissionDiff,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetTaskHoldReason {
    InvalidBudgetState,
    TaskMaximaRequired,
    UnexpectedTaskMaxima,
    InvalidTaskMaxima,
    BudgetExhausted,
    DownstreamReserveProtected,
    RunDeadlineExceeded,
    TaskDeadlineExceeded,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BudgetTaskAdmission {
    Admitted {
        execution_budget: Option<ExecutionBudget>,
    },
    Held {
        reason: BudgetTaskHoldReason,
        message: String,
    },
}

impl BudgetTaskAdmission {
    pub const fn is_admitted(&self) -> bool {
        matches!(self, Self::Admitted { .. })
    }

    pub const fn hold_reason(&self) -> Option<BudgetTaskHoldReason> {
        match self {
            Self::Admitted { .. } => None,
            Self::Held { reason, .. } => Some(*reason),
        }
    }
}

/// Pure budget admission for one task. The application layer must build `snapshot` from the
/// persisted immutable contract, append-only observations, and active reservations while holding
/// its reservation transaction. This function never loads policy or storage.
pub fn admit_budget_task(
    contract: &FeatureRunBudgetContract,
    snapshot: &BudgetSnapshot,
    declared_maxima: Option<BudgetAmounts>,
    admitted_at_unix_ms: u64,
) -> BudgetTaskAdmission {
    if snapshot.contract_digest != contract.digest
        || snapshot.mode != contract.mode
        || admitted_at_unix_ms < contract.started_at_unix_ms
    {
        return budget_held(
            BudgetTaskHoldReason::InvalidBudgetState,
            "persisted budget snapshot does not match its FeatureRun contract",
        );
    }

    match contract.mode {
        FeatureRunBudgetMode::Unbounded => {
            if contract.limits.is_some()
                || contract.phase_reserves.is_some()
                || snapshot.remaining.is_some()
                || snapshot.available.is_some()
            {
                return budget_held(
                    BudgetTaskHoldReason::InvalidBudgetState,
                    "unbounded FeatureRun contains bounded budget state",
                );
            }
            if declared_maxima.is_some() {
                return budget_held(
                    BudgetTaskHoldReason::UnexpectedTaskMaxima,
                    "unbounded FeatureRun tasks cannot declare numeric budget maxima",
                );
            }
            BudgetTaskAdmission::Admitted {
                execution_budget: None,
            }
        }
        FeatureRunBudgetMode::Bounded => {
            let Some(limits) = contract.limits else {
                return budget_held(
                    BudgetTaskHoldReason::InvalidBudgetState,
                    "bounded FeatureRun is missing its persisted limits",
                );
            };
            let (Some(remaining), Some(available)) = (snapshot.remaining, snapshot.available)
            else {
                return budget_held(
                    BudgetTaskHoldReason::InvalidBudgetState,
                    "bounded FeatureRun snapshot is incomplete",
                );
            };
            let Some(maxima) = declared_maxima else {
                return budget_held(
                    BudgetTaskHoldReason::TaskMaximaRequired,
                    "bounded task admission requires wall, tool-call, and token maxima",
                );
            };
            let execution_budget = match ExecutionBudget::new(admitted_at_unix_ms, maxima) {
                Ok(value) => value,
                Err(error) => {
                    return budget_held(
                        BudgetTaskHoldReason::InvalidTaskMaxima,
                        format!("invalid declared task maxima: {error}"),
                    );
                }
            };
            let run_deadline =
                match deadline_unix_ms(contract.started_at_unix_ms, limits.wall_seconds) {
                    Ok(value) => value,
                    Err(error) => {
                        return budget_held(
                            BudgetTaskHoldReason::InvalidBudgetState,
                            format!("invalid persisted run deadline: {error}"),
                        );
                    }
                };
            if admitted_at_unix_ms >= run_deadline {
                return budget_held(
                    BudgetTaskHoldReason::RunDeadlineExceeded,
                    "the persisted FeatureRun wall deadline has elapsed",
                );
            }
            if execution_budget.deadline_unix_ms > run_deadline {
                return budget_held(
                    BudgetTaskHoldReason::TaskDeadlineExceeded,
                    "declared task wall maximum extends past the FeatureRun deadline",
                );
            }
            let unprotected = remaining.saturating_sub(snapshot.reserved);
            if !amounts_fit(maxima, unprotected) {
                return budget_held(
                    BudgetTaskHoldReason::BudgetExhausted,
                    "declared task maxima exceed remaining unreserved FeatureRun capacity",
                );
            }
            if !amounts_fit(maxima, available) {
                return budget_held(
                    BudgetTaskHoldReason::DownstreamReserveProtected,
                    "declared task maxima would consume protected downstream phase capacity",
                );
            }
            BudgetTaskAdmission::Admitted {
                execution_budget: Some(execution_budget),
            }
        }
    }
}

fn amounts_fit(requested: BudgetAmounts, available: BudgetAmounts) -> bool {
    requested.wall_seconds <= available.wall_seconds
        && requested.tool_calls <= available.tool_calls
        && requested.tokens <= available.tokens
}

fn budget_held(reason: BudgetTaskHoldReason, message: impl Into<String>) -> BudgetTaskAdmission {
    BudgetTaskAdmission::Held {
        reason,
        message: message.into(),
    }
}

impl ExecutionAdmission {
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    pub const fn rejection_reason(&self) -> Option<AdmissionRejectionReason> {
        match self {
            Self::Allowed { .. } => None,
            Self::Rejected { reason, .. } => Some(*reason),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionRejectionReason {
    InvalidExecutionPolicy,
    InvalidTaskContract,
    UnknownRole,
    ReadScopeLimitExceeded,
    WriteScopeLimitExceeded,
    InvalidScope,
    AuthoritativeStateUnavailable,
    ActiveAgentLimitExceeded,
    ReaderLimitExceeded,
    WriterLimitExceeded,
    PermissionExpansion,
    UnsafeOperation,
    ApprovalRequired,
    OverlappingWriteScope,
    ReadWriteScopeConflict,
    WriterSerializationRequired,
    InvalidActiveScope,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionDiagnostic {
    pub field: String,
    pub message: String,
}

impl ExecutionDiagnostic {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

pub fn validate_execution_policy(policy: &ExecutionPolicy) -> Vec<ExecutionDiagnostic> {
    let mut diagnostics = Vec::new();
    if policy.max_read_scope_entries == 0 {
        diagnostics.push(ExecutionDiagnostic::new(
            "max_read_scope_entries",
            "must be at least 1",
        ));
    }
    if policy.max_write_scope_entries == 0 {
        diagnostics.push(ExecutionDiagnostic::new(
            "max_write_scope_entries",
            "must be at least 1",
        ));
    }
    if policy.roles.is_empty() {
        diagnostics.push(ExecutionDiagnostic::new(
            "roles",
            "must declare at least one role",
        ));
    }
    for (role, permissions) in &policy.roles {
        if !valid_identifier(role) {
            diagnostics.push(ExecutionDiagnostic::new(
                format!("roles.{role}"),
                "role id must use letters, digits, '.', '_', or '-'",
            ));
        }
        validate_roots(
            &mut diagnostics,
            role,
            "read_roots",
            &permissions.filesystem.read_roots,
        );
        validate_roots(
            &mut diagnostics,
            role,
            "write_roots",
            &permissions.filesystem.write_roots,
        );
        for write_root in &permissions.filesystem.write_roots {
            if !permissions
                .filesystem
                .read_roots
                .iter()
                .any(|read_root| scope_contains(read_root, write_root))
            {
                diagnostics.push(ExecutionDiagnostic::new(
                    format!("roles.{role}.filesystem.write_roots"),
                    format!("write root `{write_root}` must be contained by a read root"),
                ));
            }
        }
        for command in &permissions.commands {
            if let Err(reason) = classify_command(command) {
                diagnostics.push(ExecutionDiagnostic::new(
                    format!("roles.{role}.commands"),
                    format!("unsafe command is forbidden: {reason}"),
                ));
            }
        }
        for reference in &permissions.secret_references {
            if !valid_identifier(reference) {
                diagnostics.push(ExecutionDiagnostic::new(
                    format!("roles.{role}.secret_references"),
                    "secret references must be names, never values",
                ));
            }
        }
    }
    diagnostics
}

fn validate_roots(
    diagnostics: &mut Vec<ExecutionDiagnostic>,
    role: &str,
    field: &str,
    roots: &BTreeSet<String>,
) {
    for root in roots {
        if let Err(message) = validate_scope(root) {
            diagnostics.push(ExecutionDiagnostic::new(
                format!("roles.{role}.filesystem.{field}"),
                message,
            ));
        }
    }
}

pub fn preview_permission_diff(
    grant: &RolePermissions,
    contract: &TaskContract,
    request: &PermissionRequest,
) -> PermissionDiff {
    PermissionDiff {
        read_scope: outside_roots(&contract.read_scope, &grant.filesystem.read_roots),
        write_scope: outside_roots(&contract.write_scope, &grant.filesystem.write_roots),
        network_hosts: set_difference(&request.network_hosts, &grant.network_hosts),
        tools: set_difference(&request.tools, &grant.tools),
        mcp_servers: set_difference(&request.mcp_servers, &grant.mcp_servers),
        commands: set_difference(&request.commands, &grant.commands),
        environment: set_difference(&request.environment, &grant.environment),
        hooks: set_difference(&request.hooks, &grant.hooks),
        secret_references: set_difference(&request.secret_references, &grant.secret_references),
        overwrite_existing: request.overwrite_existing && !grant.filesystem.allow_overwrite,
    }
}

pub fn admit_execution(
    policy: &ExecutionPolicy,
    usage: &UsageLimits,
    request: &ExecutionAdmissionRequest,
    concurrency: ConcurrencySnapshot,
    active: &[ActiveExecution],
) -> ExecutionAdmission {
    let policy_diagnostics = validate_execution_policy(policy);
    if let Some(diagnostic) = policy_diagnostics.first() {
        return rejected(
            AdmissionRejectionReason::InvalidExecutionPolicy,
            format!("{}: {}", diagnostic.field, diagnostic.message),
            "fix .planr/policy.toml before delegation",
            false,
            PermissionDiff::default(),
        );
    }
    let contract_diagnostics = validate_task_contract(&request.contract);
    if let Some(diagnostic) = contract_diagnostics.first() {
        return invalid_contract(diagnostic);
    }
    if request.contract.read_scope.len() > policy.max_read_scope_entries as usize {
        return rejected(
            AdmissionRejectionReason::ReadScopeLimitExceeded,
            "task read scope exceeds the policy entry limit",
            "split the task into smaller bounded contracts",
            false,
            PermissionDiff::default(),
        );
    }
    if request.contract.write_scope.len() > policy.max_write_scope_entries as usize {
        return rejected(
            AdmissionRejectionReason::WriteScopeLimitExceeded,
            "task write scope exceeds the policy entry limit",
            "split the task into smaller exclusive write scopes",
            false,
            PermissionDiff::default(),
        );
    }
    for scope in request
        .contract
        .read_scope
        .iter()
        .chain(&request.contract.write_scope)
    {
        if let Err(message) = validate_scope(scope) {
            return rejected(
                AdmissionRejectionReason::InvalidScope,
                message,
                "use a normalized repository-relative scope without '.', '..', or wildcards",
                true,
                PermissionDiff::default(),
            );
        }
    }
    let Some(grant) = policy.roles.get(&request.role) else {
        return rejected(
            AdmissionRejectionReason::UnknownRole,
            format!("execution role `{}` is not declared", request.role),
            "declare the role in execution.roles or choose an existing role",
            true,
            PermissionDiff::default(),
        );
    };

    let diff = preview_permission_diff(grant, &request.contract, &request.permissions);
    if !request.permissions.destructive_operations.is_empty()
        || request
            .permissions
            .commands
            .iter()
            .any(|command| classify_command(command).is_err())
    {
        return rejected(
            AdmissionRejectionReason::UnsafeOperation,
            "destructive or privileged work is forbidden before delegation",
            "remove the destructive operation and create a separately approved manual procedure",
            true,
            diff,
        );
    }
    if !diff.is_empty() {
        return rejected(
            AdmissionRejectionReason::PermissionExpansion,
            "requested execution permissions exceed the declared role grant",
            "review the permission diff and update policy explicitly before retrying",
            true,
            diff,
        );
    }

    if active.iter().any(|execution| !execution.scope_known) {
        return rejected(
            AdmissionRejectionReason::AuthoritativeStateUnavailable,
            "an active item has no authoritative admitted scope",
            "pause or settle the unscoped active item, then admit this task again",
            true,
            diff,
        );
    }

    let candidate_writer = !request.contract.write_scope.is_empty();
    let candidate_reader = !candidate_writer && !request.contract.read_scope.is_empty();
    if concurrency.active_agents.saturating_add(1) > usage.max_active_agents {
        return rejected(
            AdmissionRejectionReason::ActiveAgentLimitExceeded,
            "projected active agents exceed Usage Policy max_active_agents",
            "wait for an active item to settle before delegation",
            false,
            diff,
        );
    }
    if candidate_reader
        && concurrency.parallel_readers.saturating_add(1) > usage.max_parallel_readers
    {
        return rejected(
            AdmissionRejectionReason::ReaderLimitExceeded,
            "projected parallel readers exceed Usage Policy max_parallel_readers",
            "wait for a reader to settle before delegation",
            false,
            diff,
        );
    }
    if candidate_writer
        && concurrency.parallel_writers.saturating_add(1) > usage.max_parallel_writers
    {
        return rejected(
            AdmissionRejectionReason::WriterLimitExceeded,
            "projected parallel writers exceed Usage Policy max_parallel_writers",
            "wait for a writer to settle before delegation",
            false,
            diff,
        );
    }

    let required = required_approvals(grant, &request.contract, &request.permissions);
    let missing: Vec<_> = required.difference(&request.approvals).copied().collect();
    if !missing.is_empty() {
        return rejected(
            AdmissionRejectionReason::ApprovalRequired,
            format!("missing required approvals: {missing:?}"),
            "request and record the required approval before delegation",
            true,
            diff,
        );
    }

    let mut concurrent_with = Vec::new();
    for active in active {
        if active
            .read_scope
            .iter()
            .chain(&active.write_scope)
            .any(|scope| validate_scope(scope).is_err())
        {
            return rejected(
                AdmissionRejectionReason::InvalidActiveScope,
                format!("active item {} has an invalid write scope", active.item_id),
                "repair the active task contract before admitting more work",
                true,
                diff,
            );
        }
        if request.contract.write_scope.is_empty() && active.write_scope.is_empty() {
            concurrent_with.push(active.item_id.clone());
            continue;
        }
        let isolated = distinct_worktrees(&request.isolation, &active.isolation);
        if !request.contract.write_scope.is_empty()
            && !active.write_scope.is_empty()
            && scopes_overlap(&request.contract.write_scope, &active.write_scope)
        {
            return rejected(
                AdmissionRejectionReason::OverlappingWriteScope,
                format!("write scope overlaps active item {}", active.item_id),
                "serialize the writers or narrow the task contracts",
                true,
                diff,
            );
        }
        let read_write_overlap = scopes_overlap(&request.contract.read_scope, &active.write_scope)
            || scopes_overlap(&request.contract.write_scope, &active.read_scope);
        if read_write_overlap && !isolated {
            return rejected(
                AdmissionRejectionReason::ReadWriteScopeConflict,
                format!(
                    "read/write scope overlaps active item {} in shared isolation",
                    active.item_id
                ),
                "serialize the work or use distinct worktree isolation",
                true,
                diff,
            );
        }
        if !request.contract.write_scope.is_empty() && !active.write_scope.is_empty() && !isolated {
            return rejected(
                AdmissionRejectionReason::WriterSerializationRequired,
                format!(
                    "disjoint writers require distinct worktree isolation; active item {} is not isolated",
                    active.item_id
                ),
                "serialize the writers or assign distinct worktree ids",
                false,
                diff,
            );
        }
        concurrent_with.push(active.item_id.clone());
    }

    ExecutionAdmission::Allowed {
        role: request.role.clone(),
        permission_diff: diff,
        concurrent_with,
    }
}

fn invalid_contract(diagnostic: &PolicyDiagnostic) -> ExecutionAdmission {
    rejected(
        AdmissionRejectionReason::InvalidTaskContract,
        diagnostic.to_string(),
        "supply a bounded objective, inputs, outputs, scopes, verification, evidence, and stop conditions",
        false,
        PermissionDiff::default(),
    )
}

fn rejected(
    reason: AdmissionRejectionReason,
    message: impl Into<String>,
    repair: impl Into<String>,
    safety_stop: bool,
    permission_diff: PermissionDiff,
) -> ExecutionAdmission {
    ExecutionAdmission::Rejected {
        reason,
        message: message.into(),
        repair: repair.into(),
        safety_stop,
        permission_diff,
    }
}

fn required_approvals(
    grant: &RolePermissions,
    contract: &TaskContract,
    request: &PermissionRequest,
) -> BTreeSet<ApprovalKind> {
    let mut used = BTreeSet::new();
    if !contract.write_scope.is_empty() {
        used.insert(ApprovalKind::Write);
    }
    if request.overwrite_existing {
        used.insert(ApprovalKind::Overwrite);
    }
    if !request.network_hosts.is_empty() {
        used.insert(ApprovalKind::Network);
    }
    if !request.tools.is_empty() || !request.mcp_servers.is_empty() {
        used.insert(ApprovalKind::Tool);
    }
    if !request.commands.is_empty() {
        used.insert(ApprovalKind::Command);
    }
    if !request.environment.is_empty() {
        used.insert(ApprovalKind::Environment);
    }
    if !request.hooks.is_empty() {
        used.insert(ApprovalKind::Hook);
    }
    if !request.secret_references.is_empty() {
        used.insert(ApprovalKind::Secret);
    }
    grant.approvals.intersection(&used).copied().collect()
}

fn validate_scope(scope: &str) -> Result<(), String> {
    if scope.is_empty() || scope.contains(['*', '?']) {
        return Err(format!(
            "invalid scope `{scope}`: empty and wildcard scopes are forbidden"
        ));
    }
    let path = Path::new(scope);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "invalid scope `{scope}`: scope must be normalized and repository-relative"
        ));
    }
    Ok(())
}

fn outside_roots(scopes: &[String], roots: &BTreeSet<String>) -> BTreeSet<String> {
    scopes
        .iter()
        .filter(|scope| !roots.iter().any(|root| scope_contains(root, scope)))
        .cloned()
        .collect()
}

fn set_difference<T: Clone + Ord>(requested: &BTreeSet<T>, allowed: &BTreeSet<T>) -> BTreeSet<T> {
    requested.difference(allowed).cloned().collect()
}

fn scope_contains(root: &str, candidate: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn scopes_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|left_scope| {
        right.iter().any(|right_scope| {
            scope_contains(left_scope, right_scope) || scope_contains(right_scope, left_scope)
        })
    })
}

fn distinct_worktrees(left: &IsolationMode, right: &IsolationMode) -> bool {
    matches!(
        (left, right),
        (IsolationMode::Worktree { id: left }, IsolationMode::Worktree { id: right })
            if !left.trim().is_empty() && !right.trim().is_empty() && left != right
    )
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-:".contains(character))
}

fn classify_command(command: &CommandSpec) -> Result<(), String> {
    if command.program.trim().is_empty() || command.args.iter().any(|arg| arg.is_empty()) {
        return Err("program and every argument must be non-empty tokens".to_string());
    }
    if Path::new(&command.program).components().count() != 1 || command.program.contains('\\') {
        return Err("command programs must be canonical names, not executable paths".to_string());
    }
    match command.program.as_str() {
        "cargo" => classify_cargo_args(&command.args),
        "git" => classify_git_args(&command.args),
        _ => Err(format!(
            "program `{}` is not in the explicit safe command grammar",
            command.program
        )),
    }
}

fn classify_cargo_args(args: &[String]) -> Result<(), String> {
    let Some(subcommand) = args.first().map(String::as_str) else {
        return Err("cargo requires an explicitly classified subcommand".to_string());
    };
    if ["check", "clippy", "fmt", "metadata", "test", "tree"].contains(&subcommand) {
        Ok(())
    } else {
        Err(format!(
            "cargo subcommand `{subcommand}` is not in the explicit safe command grammar"
        ))
    }
}

fn classify_git_args(args: &[String]) -> Result<(), String> {
    let mut index = 0;
    while matches!(
        args.get(index).map(String::as_str),
        Some("--no-pager" | "--paginate" | "--no-replace-objects")
    ) {
        index += 1;
    }
    let Some(subcommand) = args.get(index).map(String::as_str) else {
        return Err("git requires an explicitly classified subcommand".to_string());
    };
    if !["diff", "log", "ls-files", "rev-parse", "show", "status"].contains(&subcommand) {
        return Err(format!(
            "git subcommand `{subcommand}` is not in the explicit read-only command grammar"
        ));
    }
    if args.iter().skip(index + 1).any(|arg| {
        matches!(
            arg.as_str(),
            "--ext-diff" | "--textconv" | "--no-index" | "--exec" | "--output" | "-C" | "-c"
        ) || arg.starts_with("--output=")
            || arg.starts_with("--exec=")
            || arg.starts_with("--ext-diff=")
            || arg.starts_with("--textconv=")
            || arg.starts_with("--git-dir=")
            || arg.starts_with("--work-tree=")
    }) {
        return Err("git command contains an option outside the read-only grammar".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_policy::{
        BudgetExhaustionBehavior, BudgetProvenance, FeatureRunBudgetPhase, FeatureRunPhaseReserves,
        MaterialityTrigger, MeteringMode, MeteringProvenance, RiskLevel, budget_snapshot,
    };

    fn set(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn command(program: &str, args: &[&str]) -> CommandSpec {
        CommandSpec {
            program: program.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
        }
    }

    fn usage() -> UsageLimits {
        UsageLimits {
            max_active_agents: 3,
            max_parallel_readers: 2,
            max_parallel_writers: 3,
            max_depth: 1,
            max_attempts: 3,
            max_wall_time_seconds: None,
            max_tool_calls: None,
            max_tokens: None,
            max_credits_micros: None,
            phase_reserves: crate::usage_policy::PhaseBudgetReserves::default(),
            budget_exhaustion: BudgetExhaustionBehavior::Stop,
            metering: MeteringMode::Unavailable,
        }
    }

    fn snapshot(active: &[ActiveExecution]) -> ConcurrencySnapshot {
        ConcurrencySnapshot {
            active_agents: active.len() as u32,
            parallel_readers: active
                .iter()
                .filter(|item| item.write_scope.is_empty() && !item.read_scope.is_empty())
                .count() as u32,
            parallel_writers: active
                .iter()
                .filter(|item| !item.write_scope.is_empty())
                .count() as u32,
        }
    }

    fn admit(
        execution_policy: &ExecutionPolicy,
        request: &ExecutionAdmissionRequest,
        active: &[ActiveExecution],
    ) -> ExecutionAdmission {
        admit_with_usage(execution_policy, &usage(), request, active)
    }

    fn admit_with_usage(
        execution_policy: &ExecutionPolicy,
        limits: &UsageLimits,
        request: &ExecutionAdmissionRequest,
        active: &[ActiveExecution],
    ) -> ExecutionAdmission {
        admit_execution(execution_policy, limits, request, snapshot(active), active)
    }

    pub(crate) fn policy() -> ExecutionPolicy {
        ExecutionPolicy {
            max_read_scope_entries: 4,
            max_write_scope_entries: 2,
            roles: BTreeMap::from([(
                "worker".to_string(),
                RolePermissions {
                    filesystem: FilesystemPermissions {
                        read_roots: set(&["src", "tests"]),
                        write_roots: set(&["src", "tests"]),
                        allow_overwrite: true,
                    },
                    network_hosts: set(&["docs.rs"]),
                    tools: set(&["cargo"]),
                    mcp_servers: set(&["planr"]),
                    commands: BTreeSet::from([command("cargo", &["test"])]),
                    environment: set(&["RUST_LOG"]),
                    hooks: BTreeSet::new(),
                    secret_references: set(&["registry_token"]),
                    approvals: BTreeSet::new(),
                },
            )]),
        }
    }

    fn contract(write_scope: &[&str]) -> TaskContract {
        TaskContract {
            objective: "Implement the bounded change".to_string(),
            inputs: vec!["linked plan".to_string()],
            outputs: vec!["verified source change".to_string()],
            read_scope: vec!["src".to_string(), "tests".to_string()],
            write_scope: write_scope
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            acceptance_criteria: vec!["tests pass".to_string()],
            verification: vec!["cargo test".to_string()],
            evidence_requirements: vec!["changed files".to_string()],
            max_attempts: 2,
            stop_conditions: vec!["ReviewGate opened".to_string()],
            risk: RiskLevel::Low,
            materiality_triggers: BTreeSet::<MaterialityTrigger>::new(),
            context: vec!["small context".to_string()],
            max_context_bytes: 100,
        }
    }

    fn request(write_scope: &[&str]) -> ExecutionAdmissionRequest {
        ExecutionAdmissionRequest {
            item_id: "candidate".to_string(),
            pick_token: "pick-candidate".to_string(),
            role: "worker".to_string(),
            contract: contract(write_scope),
            permissions: PermissionRequest {
                tools: set(&["cargo"]),
                commands: BTreeSet::from([command("cargo", &["test"])]),
                overwrite_existing: true,
                ..PermissionRequest::default()
            },
            isolation: IsolationMode::Shared,
            approvals: BTreeSet::new(),
        }
    }

    #[test]
    fn bounded_declared_request_is_admitted() {
        assert!(admit(&policy(), &request(&["src/app"]), &[]).is_allowed());
    }

    #[test]
    fn task_contract_requires_inputs_outputs_verification_and_stop() {
        let mut value = request(&["src/app"]);
        value.contract.inputs.clear();
        value.contract.outputs.clear();
        value.contract.verification.clear();
        value.contract.stop_conditions.clear();
        assert_eq!(
            admit(&policy(), &value, &[]).rejection_reason(),
            Some(AdmissionRejectionReason::InvalidTaskContract)
        );
    }

    #[test]
    fn permission_expansion_is_previewed_and_rejected() {
        let mut value = request(&["docs"]);
        value.permissions.network_hosts = set(&["example.com"]);
        let decision = admit(&policy(), &value, &[]);
        let ExecutionAdmission::Rejected {
            reason,
            permission_diff,
            safety_stop,
            ..
        } = decision
        else {
            panic!("scope and network expansion must be rejected")
        };
        assert_eq!(reason, AdmissionRejectionReason::PermissionExpansion);
        assert_eq!(permission_diff.write_scope, set(&["docs"]));
        assert_eq!(permission_diff.network_hosts, set(&["example.com"]));
        assert!(safety_stop);
    }

    #[test]
    fn readers_overlap_but_writers_require_isolated_disjoint_scopes() {
        let reader = request(&[]);
        let active_reader = ActiveExecution {
            item_id: "reader-1".to_string(),
            read_scope: vec!["src".to_string()],
            write_scope: Vec::new(),
            isolation: IsolationMode::Shared,
            scope_known: true,
        };
        assert!(admit(&policy(), &reader, &[active_reader]).is_allowed());

        let mut overlapping = request(&["src/app"]);
        let overlapping_writer = ActiveExecution {
            item_id: "writer-1".to_string(),
            read_scope: vec!["src".to_string()],
            write_scope: vec!["src/app/policy.rs".to_string()],
            isolation: IsolationMode::Worktree {
                id: "one".to_string(),
            },
            scope_known: true,
        };
        overlapping.isolation = IsolationMode::Worktree {
            id: "two".to_string(),
        };
        assert_eq!(
            admit(&policy(), &overlapping, &[overlapping_writer]).rejection_reason(),
            Some(AdmissionRejectionReason::OverlappingWriteScope)
        );

        let mut disjoint = request(&["tests"]);
        disjoint.contract.read_scope = vec!["tests".to_string()];
        let mut disjoint_writer = ActiveExecution {
            item_id: "writer-1".to_string(),
            read_scope: vec!["src".to_string()],
            write_scope: vec!["src/app".to_string()],
            isolation: IsolationMode::Shared,
            scope_known: true,
        };
        assert_eq!(
            admit(&policy(), &disjoint, &[disjoint_writer.clone()]).rejection_reason(),
            Some(AdmissionRejectionReason::WriterSerializationRequired)
        );
        disjoint.isolation = IsolationMode::Worktree {
            id: "two".to_string(),
        };
        disjoint_writer.isolation = IsolationMode::Worktree {
            id: "one".to_string(),
        };
        assert!(admit(&policy(), &disjoint, &[disjoint_writer]).is_allowed());
    }

    #[test]
    fn shared_read_write_overlap_conflicts_but_distinct_worktrees_are_safe() {
        let active_writer = ActiveExecution {
            item_id: "writer-1".to_string(),
            read_scope: vec!["src".to_string()],
            write_scope: vec!["src/app".to_string()],
            isolation: IsolationMode::Shared,
            scope_known: true,
        };
        let reader = request(&[]);
        assert_eq!(
            admit(&policy(), &reader, std::slice::from_ref(&active_writer)).rejection_reason(),
            Some(AdmissionRejectionReason::ReadWriteScopeConflict)
        );

        let active_reader = ActiveExecution {
            item_id: "reader-1".to_string(),
            read_scope: vec!["src/app".to_string()],
            write_scope: Vec::new(),
            isolation: IsolationMode::Shared,
            scope_known: true,
        };
        let writer = request(&["src/app"]);
        assert_eq!(
            admit(&policy(), &writer, &[active_reader]).rejection_reason(),
            Some(AdmissionRejectionReason::ReadWriteScopeConflict)
        );

        let mut isolated_reader = reader;
        isolated_reader.isolation = IsolationMode::Worktree {
            id: "reader-tree".to_string(),
        };
        let mut isolated_writer = active_writer;
        isolated_writer.isolation = IsolationMode::Worktree {
            id: "writer-tree".to_string(),
        };
        assert!(admit(&policy(), &isolated_reader, &[isolated_writer]).is_allowed());
    }

    #[test]
    fn usage_limits_and_unknown_authoritative_scopes_fail_closed() {
        let reader = |id: &str| ActiveExecution {
            item_id: id.to_string(),
            read_scope: vec!["src".to_string()],
            write_scope: Vec::new(),
            isolation: IsolationMode::Shared,
            scope_known: true,
        };
        let active_readers = [reader("reader-1"), reader("reader-2")];
        assert_eq!(
            admit(&policy(), &request(&[]), &active_readers).rejection_reason(),
            Some(AdmissionRejectionReason::ReaderLimitExceeded)
        );

        let mut active_limits = usage();
        active_limits.max_active_agents = 2;
        active_limits.max_parallel_readers = 3;
        assert_eq!(
            admit_with_usage(&policy(), &active_limits, &request(&[]), &active_readers)
                .rejection_reason(),
            Some(AdmissionRejectionReason::ActiveAgentLimitExceeded)
        );

        let unknown = ActiveExecution {
            item_id: "unscoped".to_string(),
            read_scope: Vec::new(),
            write_scope: Vec::new(),
            isolation: IsolationMode::Shared,
            scope_known: false,
        };
        assert_eq!(
            admit(&policy(), &request(&[]), &[unknown]).rejection_reason(),
            Some(AdmissionRejectionReason::AuthoritativeStateUnavailable)
        );

        let mut writer_limits = usage();
        writer_limits.max_parallel_writers = 1;
        let active_writer = ActiveExecution {
            item_id: "writer-1".to_string(),
            read_scope: vec!["src".to_string()],
            write_scope: vec!["src/app".to_string()],
            isolation: IsolationMode::Worktree {
                id: "writer-tree".to_string(),
            },
            scope_known: true,
        };
        let mut candidate = request(&["tests"]);
        candidate.isolation = IsolationMode::Worktree {
            id: "candidate-tree".to_string(),
        };
        assert_eq!(
            admit_with_usage(&policy(), &writer_limits, &candidate, &[active_writer])
                .rejection_reason(),
            Some(AdmissionRejectionReason::WriterLimitExceeded)
        );
    }

    #[test]
    fn unsafe_destructive_work_stops_before_delegation() {
        for operation in [
            DestructiveOperation::DeletePath,
            DestructiveOperation::ForcePush,
            DestructiveOperation::HardReset,
            DestructiveOperation::PrivilegedCommand,
            DestructiveOperation::GlobalConfigurationWrite,
        ] {
            let mut value = request(&["src/app"]);
            value.permissions.destructive_operations.insert(operation);
            assert_eq!(
                admit(&policy(), &value, &[]).rejection_reason(),
                Some(AdmissionRejectionReason::UnsafeOperation)
            );
        }
    }

    #[test]
    fn absolute_traversal_and_wildcard_scopes_fail_closed() {
        for scope in ["/tmp/out", "../outside", "src/../outside", "src/**"] {
            let value = request(&[scope]);
            assert_eq!(
                admit(&policy(), &value, &[]).rejection_reason(),
                Some(AdmissionRejectionReason::InvalidScope),
                "scope {scope}"
            );
        }
    }

    #[test]
    fn approvals_are_explicit_and_do_not_expand_permissions() {
        let mut execution_policy = policy();
        execution_policy
            .roles
            .get_mut("worker")
            .unwrap()
            .approvals
            .insert(ApprovalKind::Command);
        let mut value = request(&["src/app"]);
        assert_eq!(
            admit(&execution_policy, &value, &[]).rejection_reason(),
            Some(AdmissionRejectionReason::ApprovalRequired)
        );
        value.approvals.insert(ApprovalKind::Command);
        assert!(admit(&execution_policy, &value, &[]).is_allowed());
    }

    #[test]
    fn policy_validation_rejects_dangerous_commands_and_invalid_roots() {
        let mut value = policy();
        let role = value.roles.get_mut("worker").unwrap();
        role.commands
            .insert(command("git", &["reset", "--hard", "HEAD"]));
        role.filesystem.write_roots.insert("../outside".to_string());
        let diagnostics = validate_execution_policy(&value);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("unsafe command"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("repository-relative"))
        );
    }

    #[test]
    fn structured_command_classifier_blocks_common_bypass_forms() {
        let unsafe_commands = [
            command("/bin/rm", &["-rf", "src"]),
            command("/usr/bin/git", &["--no-pager", "reset", "HEAD", "--hard"]),
            command("git", &["-c", "color.ui=false", "clean", "-fd"]),
            command("git", &["--no-pager", "push", "origin", "main", "-f"]),
            command(
                "git",
                &["push", "--force-with-lease=refs/heads/main", "origin"],
            ),
            command("sh", &["-c", "git reset --hard"]),
            command("env", &["git", "push", "--force"]),
            command("/usr/bin/sudo", &["cargo", "test"]),
            command("python3", &["-c", "import shutil; shutil.rmtree('src')"]),
            command("node", &["-e", "require('fs').rmSync('src')"]),
            command("perl", &["-e", "unlink 'src/file'"]),
            command("find", &["src", "-delete"]),
            command("truncate", &["-s", "0", "src/main.rs"]),
            command("cargo", &["run", "--bin", "cleanup"]),
            command("/tmp/cargo", &["test"]),
            command("git", &["diff", "--output=outside.patch"]),
            command("git", &["diff", "--output", "outside.patch"]),
            command("git", &["show", "--textconv", "HEAD:file"]),
        ];
        for unsafe_command in unsafe_commands {
            assert!(
                classify_command(&unsafe_command).is_err(),
                "command unexpectedly allowed: {unsafe_command:?}"
            );
        }

        for safe_command in [
            command("cargo", &["test"]),
            command("cargo", &["clippy", "--all-targets"]),
            command("git", &["status", "--short"]),
            command("git", &["--no-pager", "log", "-1"]),
        ] {
            assert_eq!(classify_command(&safe_command), Ok(()));
        }
    }

    #[test]
    fn execution_policy_wire_shape_contains_no_route_or_model_fields() {
        let json = serde_json::to_string(&policy()).unwrap();
        assert!(!json.contains("model"));
        assert!(!json.contains("fallback"));
        assert!(!json.contains("cost_tier"));
    }

    #[test]
    fn budget_admission_deterministically_protects_starvation_deadlines_and_state_integrity() {
        const STARTED_AT_UNIX_MS: u64 = 1_700_000_000_000;
        let amounts = |value| BudgetAmounts {
            wall_seconds: value,
            tool_calls: value,
            tokens: value,
        };
        let provenance = BudgetProvenance {
            wall_seconds: MeteringProvenance::Trusted,
            tool_calls: MeteringProvenance::Trusted,
            tokens: MeteringProvenance::Trusted,
        };
        let contract = FeatureRunBudgetContract::bounded(
            "run-admission-matrix",
            STARTED_AT_UNIX_MS,
            amounts(100),
            FeatureRunPhaseReserves {
                maker: amounts(40),
                verification: amounts(20),
                review: amounts(20),
                repair: amounts(10),
                release: amounts(10),
            },
            provenance,
        )
        .expect("bounded contract");
        let snapshot = budget_snapshot(
            &contract,
            FeatureRunBudgetPhase::Maker,
            BudgetAmounts::ZERO,
            BudgetAmounts::ZERO,
            provenance,
            None,
        )
        .expect("maker snapshot");

        assert_eq!(
            admit_budget_task(&contract, &snapshot, Some(amounts(41)), STARTED_AT_UNIX_MS,)
                .hold_reason(),
            Some(BudgetTaskHoldReason::DownstreamReserveProtected),
            "maker work cannot consume any later-phase reserve"
        );
        assert_eq!(
            admit_budget_task(
                &contract,
                &snapshot,
                Some(amounts(1)),
                STARTED_AT_UNIX_MS + 100_000,
            )
            .hold_reason(),
            Some(BudgetTaskHoldReason::RunDeadlineExceeded),
            "the exact persisted run deadline is closed"
        );
        assert_eq!(
            admit_budget_task(
                &contract,
                &snapshot,
                Some(amounts(11)),
                STARTED_AT_UNIX_MS + 90_000,
            )
            .hold_reason(),
            Some(BudgetTaskHoldReason::TaskDeadlineExceeded),
            "a task may not extend past the persisted run deadline"
        );

        let mut corrupt = snapshot.clone();
        corrupt.contract_digest = "sha256:corrupt".to_string();
        assert_eq!(
            admit_budget_task(&contract, &corrupt, Some(amounts(1)), STARTED_AT_UNIX_MS,)
                .hold_reason(),
            Some(BudgetTaskHoldReason::InvalidBudgetState)
        );

        let unbounded = FeatureRunBudgetContract::unbounded(
            "run-unbounded-admission",
            STARTED_AT_UNIX_MS,
            provenance,
        )
        .expect("unbounded contract");
        let unbounded_snapshot = budget_snapshot(
            &unbounded,
            FeatureRunBudgetPhase::Maker,
            BudgetAmounts::ZERO,
            BudgetAmounts::ZERO,
            provenance,
            None,
        )
        .expect("unbounded snapshot");
        assert!(
            admit_budget_task(&unbounded, &unbounded_snapshot, None, STARTED_AT_UNIX_MS,)
                .is_admitted()
        );
        assert_eq!(
            admit_budget_task(
                &unbounded,
                &unbounded_snapshot,
                Some(amounts(1)),
                STARTED_AT_UNIX_MS,
            )
            .hold_reason(),
            Some(BudgetTaskHoldReason::UnexpectedTaskMaxima),
            "unbounded mode stays one explicit non-numeric contract"
        );
    }
}
