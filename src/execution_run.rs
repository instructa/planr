//! Pure contracts for the first-class feature execution lifecycle.
//!
//! This module owns legal phase changes, batch accounting, role ownership, and
//! maker replacement provenance. It performs no persistence or host dispatch.

use serde::{Deserialize, Serialize};

pub const DEFAULT_BATCH_OUTCOME_CAP: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRunPhase {
    Implementation,
    RiskReview,
    SourceFrozen,
    Verification,
    FinalReview,
    Complete,
    Held,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRunStatus {
    Active,
    Held,
    Complete,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunRole {
    Maker,
    Verifier,
    Reviewer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleOwner {
    pub role: RunRole,
    pub worker_id: String,
    pub lease_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MakerReplacementReason {
    Unavailable,
    ContextLost,
    OwnershipIncompatible,
    BatchCapReached,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MakerReplacement {
    pub replaced_maker_worker_id: String,
    pub successor_maker_worker_id: String,
    pub reason: MakerReplacementReason,
    pub reference: String,
    pub explanation: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionBatchStatus {
    Active,
    PausedForRiskReview,
    Ended,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBatch {
    pub id: String,
    pub run_id: String,
    pub maker_worker_id: String,
    pub status: ExecutionBatchStatus,
    pub settled_outcome_ids: Vec<String>,
    pub replacement: Option<MakerReplacement>,
}

impl ExecutionBatch {
    pub fn settled_count(&self) -> u32 {
        u32::try_from(self.settled_outcome_ids.len()).unwrap_or(u32::MAX)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRunTerminalReason {
    Completed,
    UserCancelled,
    PolicyCancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRun {
    pub id: String,
    pub plan_id: String,
    pub status: FeatureRunStatus,
    pub phase: FeatureRunPhase,
    pub policy_digest: String,
    pub source_revision: Option<String>,
    pub active_batch_id: Option<String>,
    pub role_owners: Vec<RoleOwner>,
    pub outcomes_settled: u32,
    pub batch_outcome_count: u32,
    pub held_from_phase: Option<FeatureRunPhase>,
    pub hold_reason: Option<FeatureRunHoldReason>,
    pub terminal_reason: Option<FeatureRunTerminalReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRunHoldReason {
    Budget,
    Capability,
}

/// Persisted budget-contract compatibility as diagnosed by the application
/// boundary before any FeatureRun work is dispatched.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRunBudgetContractCompatibility {
    Compatible,
    Missing,
    Invalid,
    DigestMismatch,
}

impl FeatureRunBudgetContractCompatibility {
    pub fn is_incompatible(self) -> bool {
        self != Self::Compatible
    }
}

/// Explicit operator reason accepted by the incompatible-run restart
/// lifecycle. Healthy-run cancellation is intentionally a separate contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeatureRunRestartReason {
    IncompatibleBudget,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRunRestartRequest {
    pub plan_id: String,
    pub reason: FeatureRunRestartReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRunRestartDisposition {
    Retired,
    AlreadyRetired,
}

/// Pure transition output. The application repository applies the run,
/// active-batch, and role-lease effects in one transaction; later ordinary
/// pick logic remains the only owner allowed to create a successor run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRunRestartTransition {
    pub request: FeatureRunRestartRequest,
    pub incompatibility: FeatureRunBudgetContractCompatibility,
    pub disposition: FeatureRunRestartDisposition,
    pub previous_phase: FeatureRunPhase,
    pub ended_batch_id: Option<String>,
    pub released_role_owners: Vec<RoleOwner>,
    pub retired_run: FeatureRun,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRunBudgetHoldResolutionDisposition {
    Resumed,
    AlreadyResumed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRunBudgetHoldResolutionCause {
    ActiveReservationsRevalidated,
    TransientContentionCleared,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRunBudgetHoldResolutionRequest {
    pub plan_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRunBudgetHoldResolutionTransition {
    pub request: FeatureRunBudgetHoldResolutionRequest,
    pub disposition: FeatureRunBudgetHoldResolutionDisposition,
    pub cause: FeatureRunBudgetHoldResolutionCause,
    pub previous_phase: FeatureRunPhase,
    pub active_reservation_ids: Vec<String>,
    pub resumed_run: FeatureRun,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseTransitionCause {
    ProtectedRiskDiscovered,
    RiskCheckpointAccepted,
    ImplementationSettled,
    SourceInvalidated,
    VerificationStarted,
    VerificationPassed,
    ProductFinding,
    FinalReviewAccepted,
    BudgetHold,
    CapabilityHold,
    HoldResolved,
    UserCancelled,
    PolicyCancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseTransition {
    pub to: FeatureRunPhase,
    pub cause: PhaseTransitionCause,
    pub reference: String,
    pub owner: Option<RoleOwner>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunContractViolation {
    EmptyIdentity,
    InvalidStatusForPhase,
    InvalidTerminalReason,
    InvalidHeldOrigin,
    InvalidHoldReason,
    MissingTransitionReference,
    MissingSourceRevision,
    IllegalPhaseTransition,
    MissingActiveBatch,
    MissingRoleOwner,
    UnexpectedRoleOwner,
    DuplicateRoleOwner,
    InvalidLeaseGeneration,
    TransitionOwnerRequired,
    UnexpectedTransitionOwner,
    BatchNotActive,
    BatchOutcomeCapReached,
    BatchNotAtCap,
    BatchOwnerMismatch,
    DuplicateOutcome,
    ReplacementReasonRequired,
    ReplacementReferenceRequired,
    ReplacementExplanationRequired,
    ReplacementBeforeBatchCap,
    ReplacementSourceMismatch,
    SameWorkerReplacement,
    RestartPlanMismatch,
    RestartBudgetContractCompatible,
    RestartRunTerminal,
    BudgetHoldResolutionPlanMismatch,
    BudgetHoldResolutionNotBudgetHeld,
    BudgetHoldResolutionOwnerMismatch,
}

pub const ALL_FEATURE_RUN_PHASES: [FeatureRunPhase; 8] = [
    FeatureRunPhase::Implementation,
    FeatureRunPhase::RiskReview,
    FeatureRunPhase::SourceFrozen,
    FeatureRunPhase::Verification,
    FeatureRunPhase::FinalReview,
    FeatureRunPhase::Complete,
    FeatureRunPhase::Held,
    FeatureRunPhase::Cancelled,
];

pub const ALL_PHASE_TRANSITION_CAUSES: [PhaseTransitionCause; 13] = [
    PhaseTransitionCause::ProtectedRiskDiscovered,
    PhaseTransitionCause::RiskCheckpointAccepted,
    PhaseTransitionCause::ImplementationSettled,
    PhaseTransitionCause::SourceInvalidated,
    PhaseTransitionCause::VerificationStarted,
    PhaseTransitionCause::VerificationPassed,
    PhaseTransitionCause::ProductFinding,
    PhaseTransitionCause::FinalReviewAccepted,
    PhaseTransitionCause::BudgetHold,
    PhaseTransitionCause::CapabilityHold,
    PhaseTransitionCause::HoldResolved,
    PhaseTransitionCause::UserCancelled,
    PhaseTransitionCause::PolicyCancelled,
];

pub fn is_legal_phase_transition(
    from: FeatureRunPhase,
    to: FeatureRunPhase,
    cause: PhaseTransitionCause,
) -> bool {
    matches!(
        (from, to, cause),
        (
            FeatureRunPhase::Implementation,
            FeatureRunPhase::RiskReview,
            PhaseTransitionCause::ProtectedRiskDiscovered
        ) | (
            FeatureRunPhase::RiskReview,
            FeatureRunPhase::Implementation,
            PhaseTransitionCause::RiskCheckpointAccepted
        ) | (
            FeatureRunPhase::Implementation,
            FeatureRunPhase::SourceFrozen,
            PhaseTransitionCause::ImplementationSettled
        ) | (
            FeatureRunPhase::SourceFrozen,
            FeatureRunPhase::Implementation,
            PhaseTransitionCause::SourceInvalidated
        ) | (
            FeatureRunPhase::SourceFrozen,
            FeatureRunPhase::Verification,
            PhaseTransitionCause::VerificationStarted
        ) | (
            FeatureRunPhase::Verification,
            FeatureRunPhase::Implementation,
            PhaseTransitionCause::ProductFinding
        ) | (
            FeatureRunPhase::Verification,
            FeatureRunPhase::SourceFrozen,
            PhaseTransitionCause::VerificationPassed | PhaseTransitionCause::SourceInvalidated
        ) | (
            FeatureRunPhase::Verification,
            FeatureRunPhase::FinalReview,
            PhaseTransitionCause::VerificationPassed
        ) | (
            FeatureRunPhase::SourceFrozen,
            FeatureRunPhase::FinalReview,
            PhaseTransitionCause::VerificationPassed
        ) | (
            FeatureRunPhase::FinalReview,
            FeatureRunPhase::Implementation,
            PhaseTransitionCause::ProductFinding
        ) | (
            FeatureRunPhase::FinalReview,
            FeatureRunPhase::Complete,
            PhaseTransitionCause::FinalReviewAccepted
        ) | (
            FeatureRunPhase::Implementation
                | FeatureRunPhase::RiskReview
                | FeatureRunPhase::SourceFrozen
                | FeatureRunPhase::Verification
                | FeatureRunPhase::FinalReview,
            FeatureRunPhase::Held,
            PhaseTransitionCause::BudgetHold | PhaseTransitionCause::CapabilityHold
        ) | (
            FeatureRunPhase::Held,
            FeatureRunPhase::Implementation
                | FeatureRunPhase::RiskReview
                | FeatureRunPhase::SourceFrozen
                | FeatureRunPhase::Verification
                | FeatureRunPhase::FinalReview,
            PhaseTransitionCause::HoldResolved
        ) | (
            FeatureRunPhase::Implementation
                | FeatureRunPhase::RiskReview
                | FeatureRunPhase::SourceFrozen
                | FeatureRunPhase::Verification
                | FeatureRunPhase::FinalReview
                | FeatureRunPhase::Held,
            FeatureRunPhase::Cancelled,
            PhaseTransitionCause::UserCancelled | PhaseTransitionCause::PolicyCancelled
        )
    )
}

pub fn required_roles_for_phase(
    phase: FeatureRunPhase,
    held_from_phase: Option<FeatureRunPhase>,
) -> &'static [RunRole] {
    const NONE: &[RunRole] = &[];
    const MAKER: &[RunRole] = &[RunRole::Maker];
    const MAKER_REVIEWER: &[RunRole] = &[RunRole::Maker, RunRole::Reviewer];
    const VERIFIER: &[RunRole] = &[RunRole::Verifier];
    const REVIEWER: &[RunRole] = &[RunRole::Reviewer];
    match phase {
        FeatureRunPhase::Implementation => MAKER,
        FeatureRunPhase::RiskReview => MAKER_REVIEWER,
        FeatureRunPhase::Verification => VERIFIER,
        FeatureRunPhase::FinalReview => REVIEWER,
        FeatureRunPhase::Held => match held_from_phase {
            Some(FeatureRunPhase::Implementation) => MAKER,
            Some(FeatureRunPhase::RiskReview) => MAKER_REVIEWER,
            Some(FeatureRunPhase::Verification) => VERIFIER,
            Some(FeatureRunPhase::FinalReview) => REVIEWER,
            _ => NONE,
        },
        FeatureRunPhase::SourceFrozen | FeatureRunPhase::Complete | FeatureRunPhase::Cancelled => {
            NONE
        }
    }
}

fn requires_owner(phase: FeatureRunPhase, role: RunRole) -> bool {
    matches!(
        (phase, role),
        (FeatureRunPhase::Implementation, RunRole::Maker)
            | (
                FeatureRunPhase::RiskReview,
                RunRole::Maker | RunRole::Reviewer
            )
            | (FeatureRunPhase::Verification, RunRole::Verifier)
            | (FeatureRunPhase::FinalReview, RunRole::Reviewer)
    )
}

pub fn owner_for_role(run: &FeatureRun, role: RunRole) -> Option<&RoleOwner> {
    run.role_owners.iter().find(|owner| owner.role == role)
}

pub fn validate_feature_run(run: &FeatureRun) -> Result<(), RunContractViolation> {
    if [
        run.id.as_str(),
        run.plan_id.as_str(),
        run.policy_digest.as_str(),
    ]
    .into_iter()
    .any(|value| value.trim().is_empty())
    {
        return Err(RunContractViolation::EmptyIdentity);
    }
    let expected_status = match run.phase {
        FeatureRunPhase::Complete => FeatureRunStatus::Complete,
        FeatureRunPhase::Cancelled => FeatureRunStatus::Cancelled,
        FeatureRunPhase::Held => FeatureRunStatus::Held,
        _ => FeatureRunStatus::Active,
    };
    if run.status != expected_status {
        return Err(RunContractViolation::InvalidStatusForPhase);
    }
    match (run.phase, run.terminal_reason) {
        (FeatureRunPhase::Complete, Some(FeatureRunTerminalReason::Completed))
        | (
            FeatureRunPhase::Cancelled,
            Some(
                FeatureRunTerminalReason::UserCancelled | FeatureRunTerminalReason::PolicyCancelled,
            ),
        ) => {}
        (FeatureRunPhase::Complete | FeatureRunPhase::Cancelled, _) => {
            return Err(RunContractViolation::InvalidTerminalReason);
        }
        (_, Some(_)) => return Err(RunContractViolation::InvalidTerminalReason),
        _ => {}
    }
    if (run.phase == FeatureRunPhase::Held) != run.held_from_phase.is_some() {
        return Err(RunContractViolation::InvalidHeldOrigin);
    }
    if (run.phase == FeatureRunPhase::Held) != run.hold_reason.is_some() {
        return Err(RunContractViolation::InvalidHoldReason);
    }
    if run.held_from_phase == Some(FeatureRunPhase::Held)
        || run.held_from_phase == Some(FeatureRunPhase::Complete)
        || run.held_from_phase == Some(FeatureRunPhase::Cancelled)
    {
        return Err(RunContractViolation::InvalidHeldOrigin);
    }
    if run.batch_outcome_count > 0 && run.active_batch_id.is_none() {
        return Err(RunContractViolation::MissingActiveBatch);
    }
    let allowed_roles = required_roles_for_phase(run.phase, run.held_from_phase);
    for (index, owner) in run.role_owners.iter().enumerate() {
        if owner.worker_id.trim().is_empty() {
            return Err(RunContractViolation::EmptyIdentity);
        }
        if owner.lease_generation == 0 {
            return Err(RunContractViolation::InvalidLeaseGeneration);
        }
        if !allowed_roles.contains(&owner.role) {
            return Err(RunContractViolation::UnexpectedRoleOwner);
        }
        if run.role_owners[..index]
            .iter()
            .any(|existing| existing.role == owner.role)
        {
            return Err(RunContractViolation::DuplicateRoleOwner);
        }
    }
    let effective_phase = if run.phase == FeatureRunPhase::Held {
        run.held_from_phase.expect("held origin validated")
    } else {
        run.phase
    };
    for role in allowed_roles {
        if requires_owner(effective_phase, *role) && owner_for_role(run, *role).is_none() {
            return Err(RunContractViolation::MissingRoleOwner);
        }
    }
    if matches!(
        effective_phase,
        FeatureRunPhase::SourceFrozen
            | FeatureRunPhase::Verification
            | FeatureRunPhase::FinalReview
            | FeatureRunPhase::Complete
    ) && run
        .source_revision
        .as_deref()
        .is_none_or(|revision| revision.trim().is_empty())
    {
        return Err(RunContractViolation::MissingSourceRevision);
    }
    Ok(())
}

pub fn apply_phase_transition(
    run: &FeatureRun,
    transition: &PhaseTransition,
) -> Result<FeatureRun, RunContractViolation> {
    validate_feature_run(run)?;
    if transition.reference.trim().is_empty() {
        return Err(RunContractViolation::MissingTransitionReference);
    }
    let from = run.phase;
    if !is_legal_phase_transition(from, transition.to, transition.cause) {
        return Err(RunContractViolation::IllegalPhaseTransition);
    }
    if from == FeatureRunPhase::Held
        && transition.cause == PhaseTransitionCause::HoldResolved
        && run.held_from_phase != Some(transition.to)
    {
        return Err(RunContractViolation::InvalidHeldOrigin);
    }

    let mut next = run.clone();
    next.phase = transition.to;
    next.status = match transition.to {
        FeatureRunPhase::Complete => FeatureRunStatus::Complete,
        FeatureRunPhase::Cancelled => FeatureRunStatus::Cancelled,
        FeatureRunPhase::Held => FeatureRunStatus::Held,
        _ => FeatureRunStatus::Active,
    };
    next.held_from_phase = if transition.to == FeatureRunPhase::Held {
        Some(from)
    } else {
        None
    };
    next.hold_reason = match (transition.to, transition.cause) {
        (FeatureRunPhase::Held, PhaseTransitionCause::BudgetHold) => {
            Some(FeatureRunHoldReason::Budget)
        }
        (FeatureRunPhase::Held, PhaseTransitionCause::CapabilityHold) => {
            Some(FeatureRunHoldReason::Capability)
        }
        _ => None,
    };
    next.terminal_reason = match (transition.to, transition.cause) {
        (FeatureRunPhase::Complete, _) => Some(FeatureRunTerminalReason::Completed),
        (FeatureRunPhase::Cancelled, PhaseTransitionCause::UserCancelled) => {
            Some(FeatureRunTerminalReason::UserCancelled)
        }
        (FeatureRunPhase::Cancelled, PhaseTransitionCause::PolicyCancelled) => {
            Some(FeatureRunTerminalReason::PolicyCancelled)
        }
        _ => None,
    };
    let expected_new_role = match (from, transition.to, transition.cause) {
        (FeatureRunPhase::Held, _, PhaseTransitionCause::HoldResolved) => None,
        (_, FeatureRunPhase::RiskReview, _) | (_, FeatureRunPhase::FinalReview, _) => {
            Some(RunRole::Reviewer)
        }
        (_, FeatureRunPhase::Verification, _) => Some(RunRole::Verifier),
        (
            FeatureRunPhase::SourceFrozen
            | FeatureRunPhase::Verification
            | FeatureRunPhase::FinalReview,
            FeatureRunPhase::Implementation,
            _,
        ) => Some(RunRole::Maker),
        _ => None,
    };
    match (expected_new_role, transition.owner.as_ref()) {
        (Some(expected), Some(owner)) if owner.role == expected => {}
        (Some(_), _) => return Err(RunContractViolation::TransitionOwnerRequired),
        (None, Some(_)) => return Err(RunContractViolation::UnexpectedTransitionOwner),
        (None, None) => {}
    }
    next.role_owners = if from == FeatureRunPhase::Held
        && transition.cause == PhaseTransitionCause::HoldResolved
    {
        run.role_owners.clone()
    } else {
        match transition.to {
            FeatureRunPhase::RiskReview => {
                let mut owners = run.role_owners.clone();
                owners.retain(|owner| owner.role == RunRole::Maker);
                owners.push(transition.owner.clone().expect("reviewer owner validated"));
                owners
            }
            FeatureRunPhase::Implementation
                if from == FeatureRunPhase::RiskReview || from == FeatureRunPhase::Held =>
            {
                run.role_owners
                    .iter()
                    .filter(|owner| owner.role == RunRole::Maker)
                    .cloned()
                    .collect()
            }
            FeatureRunPhase::Implementation => {
                vec![transition.owner.clone().expect("maker owner validated")]
            }
            FeatureRunPhase::Verification | FeatureRunPhase::FinalReview => {
                vec![transition.owner.clone().expect("phase owner validated")]
            }
            FeatureRunPhase::Held => run.role_owners.clone(),
            FeatureRunPhase::SourceFrozen
            | FeatureRunPhase::Complete
            | FeatureRunPhase::Cancelled => Vec::new(),
        }
    };
    if transition.cause == PhaseTransitionCause::ImplementationSettled {
        next.source_revision = Some(transition.reference.clone());
    }
    if transition.cause == PhaseTransitionCause::ProductFinding
        || (transition.cause == PhaseTransitionCause::SourceInvalidated
            && transition.to == FeatureRunPhase::Implementation)
    {
        next.source_revision = None;
    }
    validate_feature_run(&next)?;
    Ok(next)
}

pub fn retire_incompatible_feature_run(
    run: &FeatureRun,
    request: &FeatureRunRestartRequest,
    compatibility: FeatureRunBudgetContractCompatibility,
) -> Result<FeatureRunRestartTransition, RunContractViolation> {
    validate_feature_run(run)?;
    if request.plan_id.trim().is_empty() {
        return Err(RunContractViolation::EmptyIdentity);
    }
    if request.plan_id != run.plan_id {
        return Err(RunContractViolation::RestartPlanMismatch);
    }
    if !compatibility.is_incompatible() {
        return Err(RunContractViolation::RestartBudgetContractCompatible);
    }

    if run.phase == FeatureRunPhase::Cancelled
        && run.terminal_reason == Some(FeatureRunTerminalReason::PolicyCancelled)
    {
        return Ok(FeatureRunRestartTransition {
            request: request.clone(),
            incompatibility: compatibility,
            disposition: FeatureRunRestartDisposition::AlreadyRetired,
            previous_phase: run.phase,
            ended_batch_id: None,
            released_role_owners: Vec::new(),
            retired_run: run.clone(),
        });
    }
    if matches!(
        run.status,
        FeatureRunStatus::Complete | FeatureRunStatus::Cancelled
    ) {
        return Err(RunContractViolation::RestartRunTerminal);
    }

    let previous_phase = run.phase;
    let ended_batch_id = run.active_batch_id.clone();
    let released_role_owners = run.role_owners.clone();
    let reference = match compatibility {
        FeatureRunBudgetContractCompatibility::Missing => "budget_contract_missing",
        FeatureRunBudgetContractCompatibility::Invalid => "budget_contract_invalid",
        FeatureRunBudgetContractCompatibility::DigestMismatch => "budget_contract_digest_mismatch",
        FeatureRunBudgetContractCompatibility::Compatible => unreachable!("checked above"),
    };
    let mut retired_run = apply_phase_transition(
        run,
        &PhaseTransition {
            to: FeatureRunPhase::Cancelled,
            cause: PhaseTransitionCause::PolicyCancelled,
            reference: reference.into(),
            owner: None,
        },
    )?;
    retired_run.active_batch_id = None;
    retired_run.batch_outcome_count = 0;
    validate_feature_run(&retired_run)?;

    Ok(FeatureRunRestartTransition {
        request: request.clone(),
        incompatibility: compatibility,
        disposition: FeatureRunRestartDisposition::Retired,
        previous_phase,
        ended_batch_id,
        released_role_owners,
        retired_run,
    })
}

pub fn resolve_budget_held_feature_run(
    run: &FeatureRun,
    request: &FeatureRunBudgetHoldResolutionRequest,
    worker_id: &str,
) -> Result<FeatureRun, RunContractViolation> {
    validate_feature_run(run)?;
    if request.plan_id.trim().is_empty() || worker_id.trim().is_empty() {
        return Err(RunContractViolation::EmptyIdentity);
    }
    if request.plan_id != run.plan_id {
        return Err(RunContractViolation::BudgetHoldResolutionPlanMismatch);
    }
    if run.phase != FeatureRunPhase::Held
        || run.status != FeatureRunStatus::Held
        || run.hold_reason != Some(FeatureRunHoldReason::Budget)
    {
        return Err(RunContractViolation::BudgetHoldResolutionNotBudgetHeld);
    }
    let previous_phase = run
        .held_from_phase
        .ok_or(RunContractViolation::InvalidHeldOrigin)?;
    let owner_role = match previous_phase {
        FeatureRunPhase::Implementation => RunRole::Maker,
        FeatureRunPhase::RiskReview | FeatureRunPhase::FinalReview => RunRole::Reviewer,
        FeatureRunPhase::SourceFrozen | FeatureRunPhase::Verification => RunRole::Verifier,
        FeatureRunPhase::Held | FeatureRunPhase::Complete | FeatureRunPhase::Cancelled => {
            return Err(RunContractViolation::InvalidHeldOrigin);
        }
    };
    if owner_for_role(run, owner_role).is_none_or(|owner| owner.worker_id != worker_id) {
        return Err(RunContractViolation::BudgetHoldResolutionOwnerMismatch);
    }
    apply_phase_transition(
        run,
        &PhaseTransition {
            to: previous_phase,
            cause: PhaseTransitionCause::HoldResolved,
            reference: "budget_hold:active_reservations_revalidated".to_string(),
            owner: None,
        },
    )
}

pub fn settle_batch_outcome(
    batch: &ExecutionBatch,
    outcome_id: &str,
    cap: u32,
) -> Result<ExecutionBatch, RunContractViolation> {
    if batch.status != ExecutionBatchStatus::Active {
        return Err(RunContractViolation::BatchNotActive);
    }
    if cap == 0 || batch.settled_count() >= cap {
        return Err(RunContractViolation::BatchOutcomeCapReached);
    }
    let outcome_id = outcome_id.trim();
    if outcome_id.is_empty() {
        return Err(RunContractViolation::EmptyIdentity);
    }
    if batch
        .settled_outcome_ids
        .iter()
        .any(|existing| existing == outcome_id)
    {
        return Err(RunContractViolation::DuplicateOutcome);
    }
    let mut next = batch.clone();
    next.settled_outcome_ids.push(outcome_id.to_string());
    Ok(next)
}

pub fn pause_batch_for_risk_review(
    batch: &ExecutionBatch,
) -> Result<ExecutionBatch, RunContractViolation> {
    if batch.status != ExecutionBatchStatus::Active {
        return Err(RunContractViolation::BatchNotActive);
    }
    let mut next = batch.clone();
    next.status = ExecutionBatchStatus::PausedForRiskReview;
    Ok(next)
}

pub fn resume_batch_after_risk_review(
    batch: &ExecutionBatch,
    maker_worker_id: &str,
) -> Result<ExecutionBatch, RunContractViolation> {
    if batch.status != ExecutionBatchStatus::PausedForRiskReview {
        return Err(RunContractViolation::BatchNotActive);
    }
    if batch.maker_worker_id != maker_worker_id {
        return Err(RunContractViolation::ReplacementReasonRequired);
    }
    let mut next = batch.clone();
    next.status = ExecutionBatchStatus::Active;
    Ok(next)
}

pub fn roll_batch_for_same_maker(
    batch: &ExecutionBatch,
    maker_worker_id: &str,
    cap: u32,
) -> Result<ExecutionBatch, RunContractViolation> {
    if batch.status != ExecutionBatchStatus::Active {
        return Err(RunContractViolation::BatchNotActive);
    }
    if batch.maker_worker_id != maker_worker_id {
        return Err(RunContractViolation::BatchOwnerMismatch);
    }
    if cap == 0 || batch.settled_count() != cap {
        return Err(RunContractViolation::BatchNotAtCap);
    }
    let mut next = batch.clone();
    next.status = ExecutionBatchStatus::Ended;
    next.replacement = None;
    Ok(next)
}

pub fn replace_batch_maker(
    batch: &ExecutionBatch,
    replacement: Option<MakerReplacement>,
    cap: u32,
) -> Result<ExecutionBatch, RunContractViolation> {
    if batch.status == ExecutionBatchStatus::Ended {
        return Err(RunContractViolation::BatchNotActive);
    }
    let replacement = replacement.ok_or(RunContractViolation::ReplacementReasonRequired)?;
    if replacement.reference.trim().is_empty() {
        return Err(RunContractViolation::ReplacementReferenceRequired);
    }
    if replacement.explanation.trim().is_empty() {
        return Err(RunContractViolation::ReplacementExplanationRequired);
    }
    if replacement.reason == MakerReplacementReason::BatchCapReached && batch.settled_count() < cap
    {
        return Err(RunContractViolation::ReplacementBeforeBatchCap);
    }
    if replacement.replaced_maker_worker_id.trim().is_empty()
        || replacement.successor_maker_worker_id.trim().is_empty()
    {
        return Err(RunContractViolation::EmptyIdentity);
    }
    if replacement.replaced_maker_worker_id != batch.maker_worker_id {
        return Err(RunContractViolation::ReplacementSourceMismatch);
    }
    if replacement.replaced_maker_worker_id == replacement.successor_maker_worker_id {
        return Err(RunContractViolation::SameWorkerReplacement);
    }
    let mut next = batch.clone();
    next.status = ExecutionBatchStatus::Ended;
    next.replacement = Some(replacement);
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(role: RunRole, worker_id: &str, lease_generation: u64) -> RoleOwner {
        RoleOwner {
            role,
            worker_id: worker_id.into(),
            lease_generation,
        }
    }

    fn run() -> FeatureRun {
        FeatureRun {
            id: "run-a".into(),
            plan_id: "plan-a".into(),
            status: FeatureRunStatus::Active,
            phase: FeatureRunPhase::Implementation,
            policy_digest: "sha256:a".into(),
            source_revision: None,
            active_batch_id: Some("batch-a".into()),
            role_owners: vec![owner(RunRole::Maker, "maker-a", 1)],
            outcomes_settled: 0,
            batch_outcome_count: 0,
            held_from_phase: None,
            hold_reason: None,
            terminal_reason: None,
        }
    }

    fn batch() -> ExecutionBatch {
        ExecutionBatch {
            id: "batch-a".into(),
            run_id: "run-a".into(),
            maker_worker_id: "maker-a".into(),
            status: ExecutionBatchStatus::Active,
            settled_outcome_ids: Vec::new(),
            replacement: None,
        }
    }

    fn transition(
        to: FeatureRunPhase,
        cause: PhaseTransitionCause,
        owner: Option<RoleOwner>,
    ) -> PhaseTransition {
        PhaseTransition {
            to,
            cause,
            reference: "event-a".into(),
            owner,
        }
    }

    fn restart_request() -> FeatureRunRestartRequest {
        FeatureRunRestartRequest {
            plan_id: "plan-a".into(),
            reason: FeatureRunRestartReason::IncompatibleBudget,
        }
    }

    fn state_for_phase_and_terminal(
        phase: FeatureRunPhase,
        held_from_phase: Option<FeatureRunPhase>,
        cancelled_reason: Option<FeatureRunTerminalReason>,
    ) -> FeatureRun {
        let effective = held_from_phase.unwrap_or(phase);
        let role_owners = match effective {
            FeatureRunPhase::Implementation => vec![owner(RunRole::Maker, "maker-a", 1)],
            FeatureRunPhase::RiskReview => vec![
                owner(RunRole::Maker, "maker-a", 1),
                owner(RunRole::Reviewer, "reviewer-a", 2),
            ],
            FeatureRunPhase::Verification => vec![owner(RunRole::Verifier, "verifier-a", 3)],
            FeatureRunPhase::FinalReview => vec![owner(RunRole::Reviewer, "reviewer-a", 4)],
            _ => Vec::new(),
        };
        FeatureRun {
            id: "run-a".into(),
            plan_id: "plan-a".into(),
            status: match phase {
                FeatureRunPhase::Complete => FeatureRunStatus::Complete,
                FeatureRunPhase::Cancelled => FeatureRunStatus::Cancelled,
                FeatureRunPhase::Held => FeatureRunStatus::Held,
                _ => FeatureRunStatus::Active,
            },
            phase,
            policy_digest: "sha256:a".into(),
            source_revision: matches!(
                effective,
                FeatureRunPhase::SourceFrozen
                    | FeatureRunPhase::Verification
                    | FeatureRunPhase::FinalReview
                    | FeatureRunPhase::Complete
            )
            .then(|| "sha256:source".into()),
            active_batch_id: matches!(
                effective,
                FeatureRunPhase::Implementation | FeatureRunPhase::RiskReview
            )
            .then(|| "batch-a".into()),
            role_owners,
            outcomes_settled: 2,
            batch_outcome_count: if matches!(
                effective,
                FeatureRunPhase::Implementation | FeatureRunPhase::RiskReview
            ) {
                2
            } else {
                0
            },
            held_from_phase,
            hold_reason: (phase == FeatureRunPhase::Held).then_some(FeatureRunHoldReason::Budget),
            terminal_reason: match phase {
                FeatureRunPhase::Complete => Some(FeatureRunTerminalReason::Completed),
                FeatureRunPhase::Cancelled => cancelled_reason,
                _ => None,
            },
        }
    }

    fn owner_for_transition(
        from: FeatureRunPhase,
        to: FeatureRunPhase,
        cause: PhaseTransitionCause,
    ) -> Option<RoleOwner> {
        if from == FeatureRunPhase::Held && cause == PhaseTransitionCause::HoldResolved {
            return None;
        }
        match to {
            FeatureRunPhase::RiskReview | FeatureRunPhase::FinalReview => {
                Some(owner(RunRole::Reviewer, "reviewer-next", 7))
            }
            FeatureRunPhase::Verification => Some(owner(RunRole::Verifier, "verifier-next", 7)),
            FeatureRunPhase::Implementation
                if matches!(
                    from,
                    FeatureRunPhase::SourceFrozen
                        | FeatureRunPhase::Verification
                        | FeatureRunPhase::FinalReview
                ) =>
            {
                Some(owner(RunRole::Maker, "maker-next", 7))
            }
            _ => None,
        }
    }

    #[test]
    fn lifecycle_accepts_only_canonical_progression_and_finding_return() {
        let mut value = run();
        for (to, cause, owner) in [
            (
                FeatureRunPhase::SourceFrozen,
                PhaseTransitionCause::ImplementationSettled,
                None,
            ),
            (
                FeatureRunPhase::Verification,
                PhaseTransitionCause::VerificationStarted,
                Some(owner(RunRole::Verifier, "verifier-a", 2)),
            ),
            (
                FeatureRunPhase::FinalReview,
                PhaseTransitionCause::VerificationPassed,
                Some(owner(RunRole::Reviewer, "reviewer-a", 3)),
            ),
            (
                FeatureRunPhase::Implementation,
                PhaseTransitionCause::ProductFinding,
                Some(owner(RunRole::Maker, "maker-a", 4)),
            ),
            (
                FeatureRunPhase::SourceFrozen,
                PhaseTransitionCause::ImplementationSettled,
                None,
            ),
            (
                FeatureRunPhase::Verification,
                PhaseTransitionCause::VerificationStarted,
                Some(owner(RunRole::Verifier, "verifier-b", 5)),
            ),
            (
                FeatureRunPhase::FinalReview,
                PhaseTransitionCause::VerificationPassed,
                Some(owner(RunRole::Reviewer, "reviewer-b", 6)),
            ),
            (
                FeatureRunPhase::Complete,
                PhaseTransitionCause::FinalReviewAccepted,
                None,
            ),
        ] {
            value =
                apply_phase_transition(&value, &transition(to, cause, owner)).expect("legal phase");
        }
        assert_eq!(value.status, FeatureRunStatus::Complete);
        assert_eq!(
            value.terminal_reason,
            Some(FeatureRunTerminalReason::Completed)
        );
        assert_eq!(
            apply_phase_transition(
                &run(),
                &transition(
                    FeatureRunPhase::FinalReview,
                    PhaseTransitionCause::VerificationPassed,
                    Some(owner(RunRole::Reviewer, "reviewer-a", 2)),
                )
            ),
            Err(RunContractViolation::IllegalPhaseTransition)
        );
    }

    #[test]
    fn incompatible_budget_retirement_is_typed_and_preserves_run_history() {
        for incompatibility in [
            FeatureRunBudgetContractCompatibility::Missing,
            FeatureRunBudgetContractCompatibility::Invalid,
            FeatureRunBudgetContractCompatibility::DigestMismatch,
        ] {
            let mut active = run();
            active.outcomes_settled = 7;
            active.batch_outcome_count = 2;
            active.source_revision = Some("sha256:historical-source".into());
            let transition =
                retire_incompatible_feature_run(&active, &restart_request(), incompatibility)
                    .expect("incompatible run retires");

            assert_eq!(transition.request, restart_request());
            assert_eq!(transition.incompatibility, incompatibility);
            assert_eq!(
                transition.disposition,
                FeatureRunRestartDisposition::Retired
            );
            assert_eq!(transition.previous_phase, FeatureRunPhase::Implementation);
            assert_eq!(transition.ended_batch_id.as_deref(), Some("batch-a"));
            assert_eq!(transition.released_role_owners, active.role_owners);
            assert_eq!(transition.retired_run.status, FeatureRunStatus::Cancelled);
            assert_eq!(transition.retired_run.phase, FeatureRunPhase::Cancelled);
            assert_eq!(
                transition.retired_run.terminal_reason,
                Some(FeatureRunTerminalReason::PolicyCancelled)
            );
            assert_eq!(transition.retired_run.active_batch_id, None);
            assert_eq!(transition.retired_run.role_owners, Vec::new());
            assert_eq!(transition.retired_run.outcomes_settled, 7);
            assert_eq!(transition.retired_run.batch_outcome_count, 0);
            assert_eq!(
                transition.retired_run.source_revision.as_deref(),
                Some("sha256:historical-source")
            );
        }
    }

    #[test]
    fn incompatible_budget_retirement_rejects_healthy_wrong_plan_and_terminal_runs() {
        assert_eq!(
            retire_incompatible_feature_run(
                &run(),
                &restart_request(),
                FeatureRunBudgetContractCompatibility::Compatible,
            ),
            Err(RunContractViolation::RestartBudgetContractCompatible)
        );

        let mut wrong_plan = restart_request();
        wrong_plan.plan_id = "plan-other".into();
        assert_eq!(
            retire_incompatible_feature_run(
                &run(),
                &wrong_plan,
                FeatureRunBudgetContractCompatibility::Missing,
            ),
            Err(RunContractViolation::RestartPlanMismatch)
        );

        for terminal in [
            state_for_phase_and_terminal(
                FeatureRunPhase::Complete,
                None,
                Some(FeatureRunTerminalReason::Completed),
            ),
            state_for_phase_and_terminal(
                FeatureRunPhase::Cancelled,
                None,
                Some(FeatureRunTerminalReason::UserCancelled),
            ),
        ] {
            assert_eq!(
                retire_incompatible_feature_run(
                    &terminal,
                    &restart_request(),
                    FeatureRunBudgetContractCompatibility::Invalid,
                ),
                Err(RunContractViolation::RestartRunTerminal)
            );
        }
    }

    #[test]
    fn incompatible_budget_retirement_is_idempotent_after_policy_retirement() {
        let first = retire_incompatible_feature_run(
            &run(),
            &restart_request(),
            FeatureRunBudgetContractCompatibility::Missing,
        )
        .expect("first retirement");
        let repeated = retire_incompatible_feature_run(
            &first.retired_run,
            &restart_request(),
            FeatureRunBudgetContractCompatibility::Missing,
        )
        .expect("repeated retirement");

        assert_eq!(
            repeated.disposition,
            FeatureRunRestartDisposition::AlreadyRetired
        );
        assert_eq!(repeated.retired_run, first.retired_run);
        assert_eq!(repeated.ended_batch_id, None);
        assert!(repeated.released_role_owners.is_empty());
    }

    #[test]
    fn risk_review_and_budget_hold_resume_exact_prior_phase() {
        let original = run();
        let reviewed = apply_phase_transition(
            &original,
            &transition(
                FeatureRunPhase::RiskReview,
                PhaseTransitionCause::ProtectedRiskDiscovered,
                Some(owner(RunRole::Reviewer, "reviewer-a", 2)),
            ),
        )
        .expect("risk review");
        let resumed = apply_phase_transition(
            &reviewed,
            &transition(
                FeatureRunPhase::Implementation,
                PhaseTransitionCause::RiskCheckpointAccepted,
                None,
            ),
        )
        .expect("checkpoint accepted");
        assert_eq!(
            owner_for_role(&resumed, RunRole::Maker),
            owner_for_role(&original, RunRole::Maker)
        );

        let held = apply_phase_transition(
            &resumed,
            &transition(
                FeatureRunPhase::Held,
                PhaseTransitionCause::BudgetHold,
                None,
            ),
        )
        .expect("held");
        assert_eq!(held.held_from_phase, Some(FeatureRunPhase::Implementation));
        assert_eq!(held.hold_reason, Some(FeatureRunHoldReason::Budget));
        assert_eq!(
            apply_phase_transition(
                &held,
                &transition(
                    FeatureRunPhase::Verification,
                    PhaseTransitionCause::HoldResolved,
                    None,
                )
            ),
            Err(RunContractViolation::InvalidHeldOrigin)
        );

        let restored = apply_phase_transition(
            &held,
            &transition(
                FeatureRunPhase::Implementation,
                PhaseTransitionCause::HoldResolved,
                None,
            ),
        )
        .expect("resume exact phase");
        assert_eq!(restored.hold_reason, None);
        let request = FeatureRunBudgetHoldResolutionRequest {
            plan_id: held.plan_id.clone(),
        };
        assert_eq!(
            resolve_budget_held_feature_run(&held, &request, "maker-other"),
            Err(RunContractViolation::BudgetHoldResolutionOwnerMismatch)
        );
        assert_eq!(
            resolve_budget_held_feature_run(&held, &request, "maker-a").unwrap(),
            restored
        );

        let capability_held = apply_phase_transition(
            &resumed,
            &transition(
                FeatureRunPhase::Held,
                PhaseTransitionCause::CapabilityHold,
                None,
            ),
        )
        .expect("capability held");
        assert_eq!(
            capability_held.hold_reason,
            Some(FeatureRunHoldReason::Capability)
        );
    }

    #[test]
    fn review_pause_preserves_maker_and_does_not_consume_batch_capacity() {
        let batch = settle_batch_outcome(&batch(), "outcome-a", 3).expect("first outcome");
        let count = batch.settled_count();
        let paused = pause_batch_for_risk_review(&batch).expect("pause");
        let resumed = resume_batch_after_risk_review(&paused, "maker-a").expect("resume");
        assert_eq!(resumed.maker_worker_id, "maker-a");
        assert_eq!(resumed.settled_count(), count);
        assert_eq!(
            resume_batch_after_risk_review(&paused, "maker-b"),
            Err(RunContractViolation::ReplacementReasonRequired)
        );
    }

    #[test]
    fn every_phase_owner_shape_round_trips_and_validates_after_resume() {
        let mut states = ALL_FEATURE_RUN_PHASES
            .into_iter()
            .filter(|phase| *phase != FeatureRunPhase::Held)
            .map(|phase| {
                state_for_phase_and_terminal(
                    phase,
                    None,
                    (phase == FeatureRunPhase::Cancelled)
                        .then_some(FeatureRunTerminalReason::UserCancelled),
                )
            })
            .collect::<Vec<_>>();
        states.push(state_for_phase_and_terminal(
            FeatureRunPhase::Cancelled,
            None,
            Some(FeatureRunTerminalReason::PolicyCancelled),
        ));
        for held_from in [
            FeatureRunPhase::Implementation,
            FeatureRunPhase::RiskReview,
            FeatureRunPhase::SourceFrozen,
            FeatureRunPhase::Verification,
            FeatureRunPhase::FinalReview,
        ] {
            states.push(state_for_phase_and_terminal(
                FeatureRunPhase::Held,
                Some(held_from),
                None,
            ));
        }

        for state in states {
            validate_feature_run(&state).expect("canonical state");
            let json = serde_json::to_string(&state).expect("serialize");
            let restored: FeatureRun = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(restored, state);
            validate_feature_run(&restored).expect("resumed state");
        }
    }

    #[test]
    fn complete_legal_transition_matrix_accepts_only_canonical_edges() {
        let mut legal_count = 0;
        for from in ALL_FEATURE_RUN_PHASES {
            for to in ALL_FEATURE_RUN_PHASES {
                for cause in ALL_PHASE_TRANSITION_CAUSES {
                    let legal = is_legal_phase_transition(from, to, cause);
                    let held_from = (from == FeatureRunPhase::Held).then_some(
                        if legal && cause == PhaseTransitionCause::HoldResolved {
                            to
                        } else {
                            FeatureRunPhase::Implementation
                        },
                    );
                    let state = state_for_phase_and_terminal(
                        from,
                        held_from,
                        (from == FeatureRunPhase::Cancelled)
                            .then_some(FeatureRunTerminalReason::UserCancelled),
                    );
                    let result = apply_phase_transition(
                        &state,
                        &transition(to, cause, owner_for_transition(from, to, cause)),
                    );
                    if legal {
                        legal_count += 1;
                        result.expect("legal matrix edge must apply");
                    } else {
                        assert_eq!(result, Err(RunContractViolation::IllegalPhaseTransition));
                    }
                }
            }
        }
        assert_eq!(legal_count, 39);
    }

    #[test]
    fn phase_owner_invariants_reject_missing_wrong_duplicate_and_generation_zero() {
        let mut value = run();
        value.role_owners.clear();
        assert_eq!(
            validate_feature_run(&value),
            Err(RunContractViolation::MissingRoleOwner)
        );

        value = run();
        value.role_owners[0].role = RunRole::Verifier;
        assert_eq!(
            validate_feature_run(&value),
            Err(RunContractViolation::UnexpectedRoleOwner)
        );

        value = run();
        value.role_owners.push(owner(RunRole::Maker, "maker-b", 2));
        assert_eq!(
            validate_feature_run(&value),
            Err(RunContractViolation::DuplicateRoleOwner)
        );

        value = run();
        value.role_owners[0].lease_generation = 0;
        assert_eq!(
            validate_feature_run(&value),
            Err(RunContractViolation::InvalidLeaseGeneration)
        );
    }

    #[test]
    fn every_replacement_reason_records_both_identities_and_rejects_invalid_state() {
        let mut value = batch();
        for id in ["one", "two", "three"] {
            value = settle_batch_outcome(&value, id, 3).expect("within cap");
        }
        assert_eq!(
            settle_batch_outcome(&value, "four", 3),
            Err(RunContractViolation::BatchOutcomeCapReached)
        );
        assert_eq!(
            replace_batch_maker(&value, None, 3),
            Err(RunContractViolation::ReplacementReasonRequired)
        );
        for reason in [
            MakerReplacementReason::Unavailable,
            MakerReplacementReason::ContextLost,
            MakerReplacementReason::OwnershipIncompatible,
            MakerReplacementReason::BatchCapReached,
        ] {
            let source = if reason == MakerReplacementReason::BatchCapReached {
                value.clone()
            } else {
                batch()
            };
            let provenance = MakerReplacement {
                replaced_maker_worker_id: "maker-a".into(),
                successor_maker_worker_id: "maker-b".into(),
                reason,
                reference: format!("replacement:{reason:?}"),
                explanation: "canonical replacement".into(),
            };
            let replaced = replace_batch_maker(&source, Some(provenance.clone()), 3)
                .expect("proven replacement");
            assert_eq!(replaced.status, ExecutionBatchStatus::Ended);
            assert_eq!(replaced.replacement, Some(provenance));
        }

        let same_worker = MakerReplacement {
            replaced_maker_worker_id: "maker-a".into(),
            successor_maker_worker_id: "maker-a".into(),
            reason: MakerReplacementReason::Unavailable,
            reference: "worker:a".into(),
            explanation: "not actually a replacement".into(),
        };
        assert_eq!(
            replace_batch_maker(&batch(), Some(same_worker), 3),
            Err(RunContractViolation::SameWorkerReplacement)
        );

        let mut ended = batch();
        ended.status = ExecutionBatchStatus::Ended;
        assert_eq!(
            replace_batch_maker(
                &ended,
                Some(MakerReplacement {
                    replaced_maker_worker_id: "maker-a".into(),
                    successor_maker_worker_id: "maker-b".into(),
                    reason: MakerReplacementReason::Unavailable,
                    reference: "worker:a".into(),
                    explanation: "ended batch".into(),
                }),
                3,
            ),
            Err(RunContractViolation::BatchNotActive)
        );
    }

    #[test]
    fn same_maker_roll_requires_exact_cap_and_preserves_non_replacement_provenance() {
        let mut value = batch();
        assert_eq!(
            roll_batch_for_same_maker(&value, "maker-a", 3),
            Err(RunContractViolation::BatchNotAtCap)
        );
        for id in ["one", "two", "three"] {
            value = settle_batch_outcome(&value, id, 3).expect("within cap");
        }
        assert_eq!(
            roll_batch_for_same_maker(&value, "maker-b", 3),
            Err(RunContractViolation::BatchOwnerMismatch)
        );
        let ended = roll_batch_for_same_maker(&value, "maker-a", 3).expect("roll capped batch");
        assert_eq!(ended.status, ExecutionBatchStatus::Ended);
        assert_eq!(ended.maker_worker_id, "maker-a");
        assert_eq!(ended.replacement, None);
        assert_eq!(
            roll_batch_for_same_maker(&ended, "maker-a", 3),
            Err(RunContractViolation::BatchNotActive)
        );
    }
}
