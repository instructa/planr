//! Provider-neutral Usage Policy v1 schema and pure enforcement rules.
//!
//! This module deliberately knows nothing about provider model ids or host
//! dispatch. Host bindings map the abstract policy decisions to concrete
//! clients later; execution permissions live in their own service.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const POLICY_RELATIVE_PATH: &str = ".planr/policy.toml";
pub const USAGE_POLICY_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsagePolicyV1 {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub usage: UsageLimits,
    pub transitions: TransitionPolicy,
    pub materiality: MaterialityPolicy,
    pub execution: crate::execution_policy::ExecutionPolicy,
}

pub const LEGACY_ALPHA2_POLICY_SHAPE: &str = "planr.policy.v1@v1.10.0-alpha.2";
pub const CURRENT_POLICY_SHAPE: &str = "planr.policy.v1.current";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PolicyUpgradePreview {
    pub from_shape: String,
    pub to_shape: String,
    pub path: String,
    pub changes: Vec<String>,
    pub canonical_toml: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyAlpha2Policy {
    schema_version: u32,
    id: String,
    version: String,
    usage: LegacyAlpha2UsageLimits,
    transitions: TransitionPolicy,
    materiality: LegacyAlpha2MaterialityPolicy,
    execution: crate::execution_policy::ExecutionPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyAlpha2UsageLimits {
    max_active_agents: u32,
    max_parallel_readers: u32,
    max_parallel_writers: u32,
    max_depth: u8,
    max_attempts: u32,
    #[serde(default)]
    max_wall_time_seconds: Option<u64>,
    #[serde(default)]
    max_tool_calls: Option<u64>,
    #[serde(default)]
    max_tokens: Option<u64>,
    #[serde(default)]
    max_credits_micros: Option<u64>,
    review_reserve_percent: u8,
    budget_exhaustion: BudgetExhaustionBehavior,
    metering: MeteringMode,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyAlpha2MaterialityPolicy {
    #[serde(default)]
    changed_files_threshold: Option<u32>,
    #[serde(default)]
    changed_lines_threshold: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageLimits {
    pub max_active_agents: u32,
    pub max_parallel_readers: u32,
    pub max_parallel_writers: u32,
    pub max_depth: u8,
    pub max_attempts: u32,
    #[serde(default)]
    pub max_wall_time_seconds: Option<u64>,
    #[serde(default)]
    pub max_tool_calls: Option<u64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub max_credits_micros: Option<u64>,
    pub phase_reserves: PhaseBudgetReserves,
    pub budget_exhaustion: BudgetExhaustionBehavior,
    pub metering: MeteringMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseBudgetReserves {
    #[serde(default)]
    pub verification_percent: u8,
    #[serde(default)]
    pub review_percent: u8,
    #[serde(default)]
    pub repair_percent: u8,
}

impl PhaseBudgetReserves {
    pub const fn total_percent(self) -> u16 {
        self.verification_percent as u16 + self.review_percent as u16 + self.repair_percent as u16
    }

    pub const fn protected_percent_for(self, phase: BudgetPhase) -> u16 {
        match phase {
            BudgetPhase::Implementation => self.total_percent(),
            BudgetPhase::Verification => self.review_percent as u16 + self.repair_percent as u16,
            BudgetPhase::Review => self.repair_percent as u16,
            BudgetPhase::Repair => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPhase {
    Implementation,
    Verification,
    Review,
    Repair,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeteringMode {
    Unavailable,
    Estimated,
    Trusted,
}

pub const FEATURE_RUN_BUDGET_CONTRACT_SCHEMA: &str = "planr.feature_run_budget_contract.v2";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRunBudgetMode {
    Bounded,
    Unbounded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRunBudgetPhase {
    Maker,
    Verification,
    Review,
    Repair,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetClockBasis {
    UtcUnixMilliseconds,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeteringProvenance {
    Unavailable,
    Estimated,
    Trusted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetAmounts {
    pub wall_seconds: u64,
    pub tool_calls: u64,
    pub tokens: u64,
}

impl BudgetAmounts {
    pub const ZERO: Self = Self {
        wall_seconds: 0,
        tool_calls: 0,
        tokens: 0,
    };

    pub fn checked_add(self, other: Self) -> Result<Self, BudgetArithmeticError> {
        Ok(Self {
            wall_seconds: self.wall_seconds.checked_add(other.wall_seconds).ok_or(
                BudgetArithmeticError::ReserveOverflow(BudgetContractDimension::WallSeconds),
            )?,
            tool_calls: self.tool_calls.checked_add(other.tool_calls).ok_or(
                BudgetArithmeticError::ReserveOverflow(BudgetContractDimension::ToolCalls),
            )?,
            tokens: self.tokens.checked_add(other.tokens).ok_or(
                BudgetArithmeticError::ReserveOverflow(BudgetContractDimension::Tokens),
            )?,
        })
    }

    pub const fn saturating_sub(self, other: Self) -> Self {
        Self {
            wall_seconds: self.wall_seconds.saturating_sub(other.wall_seconds),
            tool_calls: self.tool_calls.saturating_sub(other.tool_calls),
            tokens: self.tokens.saturating_sub(other.tokens),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetContractDimension {
    WallSeconds,
    ToolCalls,
    Tokens,
}

impl BudgetContractDimension {
    const fn as_str(self) -> &'static str {
        match self {
            Self::WallSeconds => "wall_seconds",
            Self::ToolCalls => "tool_calls",
            Self::Tokens => "tokens",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetArithmeticError {
    ZeroMaximum(BudgetContractDimension),
    ReserveOverflow(BudgetContractDimension),
    InvalidClockAnchor,
    DeadlineOverflow,
    IncompleteBoundedContract,
    UnexpectedBoundedValues,
}

impl fmt::Display for BudgetArithmeticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroMaximum(dimension) => {
                write!(
                    formatter,
                    "{} maximum must be at least 1",
                    dimension.as_str()
                )
            }
            Self::ReserveOverflow(dimension) => {
                write!(
                    formatter,
                    "{} reserve arithmetic overflowed",
                    dimension.as_str()
                )
            }
            Self::InvalidClockAnchor => {
                formatter.write_str("task admission requires a UTC clock anchor")
            }
            Self::DeadlineOverflow => formatter.write_str("task deadline arithmetic overflowed"),
            Self::IncompleteBoundedContract => {
                formatter.write_str("bounded budget contract is missing limits or phase reserves")
            }
            Self::UnexpectedBoundedValues => formatter
                .write_str("unbounded budget contract cannot contain limits or phase reserves"),
        }
    }
}

impl std::error::Error for BudgetArithmeticError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRunPhaseReserves {
    pub maker: BudgetAmounts,
    pub verification: BudgetAmounts,
    pub review: BudgetAmounts,
    pub repair: BudgetAmounts,
    pub release: BudgetAmounts,
}

impl FeatureRunPhaseReserves {
    pub fn total(self) -> Result<BudgetAmounts, BudgetArithmeticError> {
        self.maker
            .checked_add(self.verification)?
            .checked_add(self.review)?
            .checked_add(self.repair)?
            .checked_add(self.release)
    }

    pub fn protected_after(
        self,
        phase: FeatureRunBudgetPhase,
    ) -> Result<BudgetAmounts, BudgetArithmeticError> {
        match phase {
            FeatureRunBudgetPhase::Maker => self
                .verification
                .checked_add(self.review)?
                .checked_add(self.repair)?
                .checked_add(self.release),
            FeatureRunBudgetPhase::Verification => self
                .review
                .checked_add(self.repair)?
                .checked_add(self.release),
            FeatureRunBudgetPhase::Review => self.repair.checked_add(self.release),
            FeatureRunBudgetPhase::Repair => Ok(self.release),
            FeatureRunBudgetPhase::Release => Ok(BudgetAmounts::ZERO),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetProvenance {
    pub wall_seconds: MeteringProvenance,
    pub tool_calls: MeteringProvenance,
    pub tokens: MeteringProvenance,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRunBudgetContract {
    pub schema: String,
    pub run_id: String,
    pub started_at_unix_ms: u64,
    pub clock_basis: BudgetClockBasis,
    pub mode: FeatureRunBudgetMode,
    pub limits: Option<BudgetAmounts>,
    pub phase_reserves: Option<FeatureRunPhaseReserves>,
    pub metering_requirements: BudgetProvenance,
    pub digest: String,
}

impl FeatureRunBudgetContract {
    pub fn bounded(
        run_id: impl Into<String>,
        started_at_unix_ms: u64,
        limits: BudgetAmounts,
        phase_reserves: FeatureRunPhaseReserves,
        metering_requirements: BudgetProvenance,
    ) -> Result<Self, PolicyDiagnostics> {
        seal_feature_run_budget_contract(Self {
            schema: FEATURE_RUN_BUDGET_CONTRACT_SCHEMA.to_string(),
            run_id: run_id.into(),
            started_at_unix_ms,
            clock_basis: BudgetClockBasis::UtcUnixMilliseconds,
            mode: FeatureRunBudgetMode::Bounded,
            limits: Some(limits),
            phase_reserves: Some(phase_reserves),
            metering_requirements,
            digest: String::new(),
        })
    }

    pub fn unbounded(
        run_id: impl Into<String>,
        started_at_unix_ms: u64,
        metering_requirements: BudgetProvenance,
    ) -> Result<Self, PolicyDiagnostics> {
        seal_feature_run_budget_contract(Self {
            schema: FEATURE_RUN_BUDGET_CONTRACT_SCHEMA.to_string(),
            run_id: run_id.into(),
            started_at_unix_ms,
            clock_basis: BudgetClockBasis::UtcUnixMilliseconds,
            mode: FeatureRunBudgetMode::Unbounded,
            limits: None,
            phase_reserves: None,
            metering_requirements,
            digest: String::new(),
        })
    }
}

pub fn feature_run_budget_contract_from_policy(
    run_id: impl Into<String>,
    started_at_unix_ms: u64,
    policy: Option<&UsagePolicyV1>,
) -> Result<FeatureRunBudgetContract, PolicyDiagnostics> {
    let run_id = run_id.into();
    let metering_requirements = BudgetProvenance {
        wall_seconds: MeteringProvenance::Trusted,
        tool_calls: policy
            .map(|value| MeteringProvenance::from(value.usage.metering))
            .unwrap_or(MeteringProvenance::Unavailable),
        tokens: policy
            .map(|value| MeteringProvenance::from(value.usage.metering))
            .unwrap_or(MeteringProvenance::Unavailable),
    };
    let Some(policy) = policy else {
        return FeatureRunBudgetContract::unbounded(
            run_id,
            started_at_unix_ms,
            metering_requirements,
        );
    };
    let configured = [
        policy.usage.max_wall_time_seconds,
        policy.usage.max_tool_calls,
        policy.usage.max_tokens,
    ];
    if configured.iter().all(Option::is_none) {
        return FeatureRunBudgetContract::unbounded(
            run_id,
            started_at_unix_ms,
            metering_requirements,
        );
    }
    if configured.iter().any(Option::is_none) {
        return Err(PolicyDiagnostics::single(PolicyDiagnostic::validation(
            "usage",
            "bounded FeatureRuns require wall-seconds, tool-call, and token limits together",
        )));
    }
    let limits = BudgetAmounts {
        wall_seconds: configured[0].expect("complete bounded limits"),
        tool_calls: configured[1].expect("complete bounded limits"),
        tokens: configured[2].expect("complete bounded limits"),
    };
    FeatureRunBudgetContract::bounded(
        run_id,
        started_at_unix_ms,
        limits,
        exact_feature_run_phase_reserves(limits, policy.usage.phase_reserves),
        metering_requirements,
    )
}

impl From<MeteringMode> for MeteringProvenance {
    fn from(value: MeteringMode) -> Self {
        match value {
            MeteringMode::Unavailable => Self::Unavailable,
            MeteringMode::Estimated => Self::Estimated,
            MeteringMode::Trusted => Self::Trusted,
        }
    }
}

fn exact_feature_run_phase_reserves(
    limits: BudgetAmounts,
    percentages: PhaseBudgetReserves,
) -> FeatureRunPhaseReserves {
    let percent = |amounts: BudgetAmounts, value: u8| BudgetAmounts {
        wall_seconds: percentage_amount(amounts.wall_seconds, value),
        tool_calls: percentage_amount(amounts.tool_calls, value),
        tokens: percentage_amount(amounts.tokens, value),
    };
    let verification = percent(limits, percentages.verification_percent);
    let review = percent(limits, percentages.review_percent);
    let repair = percent(limits, percentages.repair_percent);
    let assigned = verification
        .checked_add(review)
        .and_then(|value| value.checked_add(repair))
        .expect("validated phase percentages cannot overflow their limits");
    FeatureRunPhaseReserves {
        maker: limits.saturating_sub(assigned),
        verification,
        review,
        repair,
        release: BudgetAmounts::ZERO,
    }
}

const fn percentage_amount(limit: u64, percent: u8) -> u64 {
    let percent = percent as u64;
    (limit / 100) * percent + ((limit % 100) * percent) / 100
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBudget {
    pub max_wall_seconds: u64,
    pub max_tool_calls: u64,
    pub max_tokens: u64,
    pub deadline_unix_ms: u64,
}

impl ExecutionBudget {
    pub fn new(
        admitted_at_unix_ms: u64,
        maxima: BudgetAmounts,
    ) -> Result<Self, BudgetArithmeticError> {
        require_positive_amounts(maxima)?;
        Ok(Self {
            max_wall_seconds: maxima.wall_seconds,
            max_tool_calls: maxima.tool_calls,
            max_tokens: maxima.tokens,
            deadline_unix_ms: deadline_unix_ms(admitted_at_unix_ms, maxima.wall_seconds)?,
        })
    }

    pub const fn maxima(self) -> BudgetAmounts {
        BudgetAmounts {
            wall_seconds: self.max_wall_seconds,
            tool_calls: self.max_tool_calls,
            tokens: self.max_tokens,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetSnapshot {
    pub mode: FeatureRunBudgetMode,
    pub consumed: BudgetAmounts,
    pub reserved: BudgetAmounts,
    pub remaining: Option<BudgetAmounts>,
    pub protected: BudgetAmounts,
    pub available: Option<BudgetAmounts>,
    pub released_through: FeatureRunBudgetPhase,
    pub metering: BudgetProvenance,
    pub task_deadline_unix_ms: Option<u64>,
    pub contract_digest: String,
}

pub fn deadline_unix_ms(
    admitted_at_unix_ms: u64,
    max_wall_seconds: u64,
) -> Result<u64, BudgetArithmeticError> {
    if admitted_at_unix_ms == 0 {
        return Err(BudgetArithmeticError::InvalidClockAnchor);
    }
    if max_wall_seconds == 0 {
        return Err(BudgetArithmeticError::ZeroMaximum(
            BudgetContractDimension::WallSeconds,
        ));
    }
    let duration_ms = max_wall_seconds
        .checked_mul(1_000)
        .ok_or(BudgetArithmeticError::DeadlineOverflow)?;
    admitted_at_unix_ms
        .checked_add(duration_ms)
        .ok_or(BudgetArithmeticError::DeadlineOverflow)
}

pub fn feature_run_budget_contract_digest(
    contract: &FeatureRunBudgetContract,
) -> anyhow::Result<String> {
    let value = serde_json::to_value(contract)?;
    crate::canonical_json::sha256_json_digest_without_top_level_field(&value, "digest")
}

pub fn validate_feature_run_budget_contract(
    contract: &FeatureRunBudgetContract,
) -> Vec<PolicyDiagnostic> {
    let mut diagnostics = validate_feature_run_budget_contract_body(contract);
    if !is_sha256_digest(&contract.digest) {
        diagnostics.push(PolicyDiagnostic::validation(
            "budget_contract.digest",
            "must be sha256:<64 lowercase hex>",
        ));
    } else {
        match feature_run_budget_contract_digest(contract) {
            Ok(actual) if actual != contract.digest => {
                diagnostics.push(PolicyDiagnostic::validation(
                    "budget_contract.digest",
                    "does not match the canonical contract body",
                ))
            }
            Ok(_) => {}
            Err(error) => diagnostics.push(PolicyDiagnostic::validation(
                "budget_contract.digest",
                format!("cannot canonicalize contract: {error}"),
            )),
        }
    }
    diagnostics
}

pub fn validate_execution_budget(budget: &ExecutionBudget) -> Vec<PolicyDiagnostic> {
    let mut diagnostics = Vec::new();
    for (field, value) in [
        ("execution_budget.max_wall_seconds", budget.max_wall_seconds),
        ("execution_budget.max_tool_calls", budget.max_tool_calls),
        ("execution_budget.max_tokens", budget.max_tokens),
    ] {
        if value == 0 {
            diagnostics.push(PolicyDiagnostic::validation(field, "must be at least 1"));
        }
    }
    let minimum_duration_ms = budget.max_wall_seconds.checked_mul(1_000);
    if minimum_duration_ms.is_none_or(|duration| budget.deadline_unix_ms <= duration) {
        diagnostics.push(PolicyDiagnostic::validation(
            "execution_budget.deadline_unix_ms",
            "must be an absolute UTC deadline after the declared wall duration",
        ));
    }
    diagnostics
}

pub fn budget_snapshot(
    contract: &FeatureRunBudgetContract,
    released_through: FeatureRunBudgetPhase,
    consumed: BudgetAmounts,
    reserved: BudgetAmounts,
    metering: BudgetProvenance,
    execution_budget: Option<ExecutionBudget>,
) -> Result<BudgetSnapshot, BudgetArithmeticError> {
    let (remaining, protected, available) = match contract.mode {
        FeatureRunBudgetMode::Bounded => {
            let limits = contract
                .limits
                .ok_or(BudgetArithmeticError::IncompleteBoundedContract)?;
            let phase_reserves = contract
                .phase_reserves
                .ok_or(BudgetArithmeticError::IncompleteBoundedContract)?;
            let remaining = limits.saturating_sub(consumed);
            let protected = phase_reserves.protected_after(released_through)?;
            let available = remaining.saturating_sub(reserved).saturating_sub(protected);
            (Some(remaining), protected, Some(available))
        }
        FeatureRunBudgetMode::Unbounded => {
            if contract.limits.is_some() || contract.phase_reserves.is_some() {
                return Err(BudgetArithmeticError::UnexpectedBoundedValues);
            }
            (None, BudgetAmounts::ZERO, None)
        }
    };
    Ok(BudgetSnapshot {
        mode: contract.mode,
        consumed,
        reserved,
        remaining,
        protected,
        available,
        released_through,
        metering,
        task_deadline_unix_ms: execution_budget.map(|budget| budget.deadline_unix_ms),
        contract_digest: contract.digest.clone(),
    })
}

fn seal_feature_run_budget_contract(
    mut contract: FeatureRunBudgetContract,
) -> Result<FeatureRunBudgetContract, PolicyDiagnostics> {
    let diagnostics = validate_feature_run_budget_contract_body(&contract);
    if !diagnostics.is_empty() {
        return Err(PolicyDiagnostics { diagnostics });
    }
    contract.digest = feature_run_budget_contract_digest(&contract).map_err(|error| {
        PolicyDiagnostics::single(PolicyDiagnostic::validation(
            "budget_contract.digest",
            format!("cannot canonicalize contract: {error}"),
        ))
    })?;
    Ok(contract)
}

fn validate_feature_run_budget_contract_body(
    contract: &FeatureRunBudgetContract,
) -> Vec<PolicyDiagnostic> {
    let mut diagnostics = Vec::new();
    if contract.schema != FEATURE_RUN_BUDGET_CONTRACT_SCHEMA {
        diagnostics.push(PolicyDiagnostic::validation(
            "budget_contract.schema",
            format!("must be {FEATURE_RUN_BUDGET_CONTRACT_SCHEMA}"),
        ));
    }
    require_nonempty(&mut diagnostics, "budget_contract.run_id", &contract.run_id);
    if contract.started_at_unix_ms == 0 {
        diagnostics.push(PolicyDiagnostic::validation(
            "budget_contract.started_at_unix_ms",
            "must be a persisted UTC run-start anchor",
        ));
    }
    match contract.mode {
        FeatureRunBudgetMode::Bounded => {
            if contract.limits.is_none() {
                diagnostics.push(PolicyDiagnostic::validation(
                    "budget_contract.limits",
                    "bounded mode requires all limits",
                ));
            }
            if contract.phase_reserves.is_none() {
                diagnostics.push(PolicyDiagnostic::validation(
                    "budget_contract.phase_reserves",
                    "bounded mode requires all phase reserves",
                ));
            }
            if let (Some(limits), Some(reserves)) = (contract.limits, contract.phase_reserves) {
                for (field, value) in [
                    ("budget_contract.limits.wall_seconds", limits.wall_seconds),
                    ("budget_contract.limits.tool_calls", limits.tool_calls),
                    ("budget_contract.limits.tokens", limits.tokens),
                ] {
                    if value == 0 {
                        diagnostics.push(PolicyDiagnostic::validation(field, "must be at least 1"));
                    }
                }
                match reserves.total() {
                    Ok(total) => {
                        for (field, actual, expected) in [
                            (
                                "budget_contract.phase_reserves.wall_seconds",
                                total.wall_seconds,
                                limits.wall_seconds,
                            ),
                            (
                                "budget_contract.phase_reserves.tool_calls",
                                total.tool_calls,
                                limits.tool_calls,
                            ),
                            (
                                "budget_contract.phase_reserves.tokens",
                                total.tokens,
                                limits.tokens,
                            ),
                        ] {
                            if actual != expected {
                                diagnostics.push(PolicyDiagnostic::validation(
                                    field,
                                    format!(
                                        "phase allocations total {actual}; expected {expected}"
                                    ),
                                ));
                            }
                        }
                    }
                    Err(error) => diagnostics.push(PolicyDiagnostic::validation(
                        "budget_contract.phase_reserves",
                        error.to_string(),
                    )),
                }
                if deadline_unix_ms(contract.started_at_unix_ms, limits.wall_seconds).is_err() {
                    diagnostics.push(PolicyDiagnostic::validation(
                        "budget_contract.limits.wall_seconds",
                        "overflows the absolute run deadline",
                    ));
                }
            }
            if contract.metering_requirements.wall_seconds != MeteringProvenance::Trusted {
                diagnostics.push(PolicyDiagnostic::validation(
                    "budget_contract.metering_requirements.wall_seconds",
                    "bounded wall time requires trusted UTC clock provenance",
                ));
            }
        }
        FeatureRunBudgetMode::Unbounded => {
            if contract.limits.is_some() {
                diagnostics.push(PolicyDiagnostic::validation(
                    "budget_contract.limits",
                    "unbounded mode cannot contain numeric limits",
                ));
            }
            if contract.phase_reserves.is_some() {
                diagnostics.push(PolicyDiagnostic::validation(
                    "budget_contract.phase_reserves",
                    "unbounded mode cannot contain numeric phase reserves",
                ));
            }
        }
    }
    diagnostics
}

fn require_positive_amounts(amounts: BudgetAmounts) -> Result<(), BudgetArithmeticError> {
    for (dimension, value) in [
        (BudgetContractDimension::WallSeconds, amounts.wall_seconds),
        (BudgetContractDimension::ToolCalls, amounts.tool_calls),
        (BudgetContractDimension::Tokens, amounts.tokens),
    ] {
        if value == 0 {
            return Err(BudgetArithmeticError::ZeroMaximum(dimension));
        }
    }
    Ok(())
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetExhaustionBehavior {
    Stop,
    DowngradeNoncritical,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionPolicy {
    pub retry: RetryPolicy,
    pub availability_fallback: AvailabilityFallbackPolicy,
    pub quality_escalation: QualityEscalationPolicy,
    pub quota_downgrade: QuotaDowngradePolicy,
    pub safety_stop: SafetyStopPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    pub max_same_route_retries: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvailabilityFallbackPolicy {
    pub max_fallbacks: u32,
    pub require_same_capability_class: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityEscalationPolicy {
    pub max_escalations: u32,
    pub require_verification_evidence: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaDowngradePolicy {
    pub enabled: bool,
    pub max_downgrades: u32,
    pub noncritical_only: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafetyStopPolicy {
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialityPolicy {
    pub protected_risks: BTreeSet<MaterialityTrigger>,
    #[serde(default)]
    pub changed_files_threshold: Option<u32>,
    #[serde(default)]
    pub changed_lines_threshold: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Moderate,
    High,
    Critical,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialityTrigger {
    SecurityOrAuth,
    SecretsOrCrypto,
    SchemaOrMigration,
    InfrastructureOrDeploy,
    PublicApi,
    Billing,
    ConcurrencyOrTransaction,
    LargeDependencyChange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Code,
    Documentation,
    Formatting,
    TestsOnly,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeSummary {
    pub risk: RiskLevel,
    #[serde(default)]
    pub triggers: BTreeSet<MaterialityTrigger>,
    pub changed_files: u32,
    pub changed_lines: u32,
    pub kind: ChangeKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRequirement {
    None,
    IndependentHighSignal,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssuranceDepth {
    Standard,
    Expanded,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialityDecision {
    pub material: bool,
    pub review: ReviewRequirement,
    pub assurance_depth: AssuranceDepth,
    pub reasons: Vec<String>,
}

/// Classifies materiality without consulting a host or model catalog.
pub fn classify_materiality(
    policy: &MaterialityPolicy,
    change: &ChangeSummary,
) -> MaterialityDecision {
    let mut reasons: Vec<String> = change
        .triggers
        .iter()
        .map(|trigger| {
            if policy.protected_risks.contains(trigger) {
                format!("material_trigger:{}", trigger.as_str())
            } else {
                format!("assurance_trigger:{}", trigger.as_str())
            }
        })
        .collect();

    if change.risk >= RiskLevel::High {
        reasons.push(format!("risk:{}", change.risk.as_str()));
    }
    if policy
        .changed_files_threshold
        .is_some_and(|limit| change.changed_files >= limit)
    {
        reasons.push(format!("changed_files_threshold:{}", change.changed_files));
    }
    if policy
        .changed_lines_threshold
        .is_some_and(|limit| change.changed_lines >= limit)
    {
        reasons.push(format!("changed_lines_threshold:{}", change.changed_lines));
    }

    let protected_risk = change
        .triggers
        .iter()
        .any(|trigger| policy.protected_risks.contains(trigger));
    let material = protected_risk;
    let expanded_assurance = material
        || change.risk >= RiskLevel::High
        || !change.triggers.is_empty()
        || change
            .triggers
            .contains(&MaterialityTrigger::LargeDependencyChange)
        || policy
            .changed_files_threshold
            .is_some_and(|limit| change.changed_files >= limit)
        || policy
            .changed_lines_threshold
            .is_some_and(|limit| change.changed_lines >= limit);
    MaterialityDecision {
        material,
        review: if material {
            ReviewRequirement::IndependentHighSignal
        } else {
            ReviewRequirement::None
        },
        assurance_depth: if expanded_assurance {
            AssuranceDepth::Expanded
        } else {
            AssuranceDepth::Standard
        },
        reasons,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewEscalationReason {
    UserRequested,
    PolicyRequired,
    ProtectedRiskDiscovered,
    ExternalSideEffect,
    DataIntegrityRisk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationSource {
    User,
    Policy,
    MakerFinding,
    ReviewerFinding,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewEscalation {
    pub reason: ReviewEscalationReason,
    pub source: EscalationSource,
    pub reference: String,
    pub explanation: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationalGapReason {
    MissingEvidence,
    VerifierFailure,
    AdapterDrift,
    SandboxRestriction,
    Uncertainty,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewInterruptRequest {
    StructuredEscalation {
        escalation: ReviewEscalation,
    },
    OperationalGap {
        reason: OperationalGapReason,
    },
    ChangeSize {
        changed_files: u32,
        changed_lines: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewInterruptRejectionReason {
    MissingReference,
    MissingExplanation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReviewInterruptDecision {
    OpenCheckpoint {
        escalation: ReviewEscalation,
    },
    ContinueWithExpandedAssurance,
    Rejected {
        reason: ReviewInterruptRejectionReason,
    },
    RejectedOperationalGap {
        gap: OperationalGapReason,
    },
}

pub fn admit_review_interrupt(request: &ReviewInterruptRequest) -> ReviewInterruptDecision {
    match request {
        ReviewInterruptRequest::StructuredEscalation { escalation } => {
            if escalation.reference.trim().is_empty() {
                return ReviewInterruptDecision::Rejected {
                    reason: ReviewInterruptRejectionReason::MissingReference,
                };
            }
            if escalation.explanation.trim().is_empty() {
                return ReviewInterruptDecision::Rejected {
                    reason: ReviewInterruptRejectionReason::MissingExplanation,
                };
            }
            ReviewInterruptDecision::OpenCheckpoint {
                escalation: escalation.clone(),
            }
        }
        ReviewInterruptRequest::OperationalGap { reason } => {
            ReviewInterruptDecision::RejectedOperationalGap { gap: *reason }
        }
        ReviewInterruptRequest::ChangeSize { .. } => {
            ReviewInterruptDecision::ContinueWithExpandedAssurance
        }
    }
}

impl MaterialityTrigger {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SecurityOrAuth => "security_or_auth",
            Self::SecretsOrCrypto => "secrets_or_crypto",
            Self::SchemaOrMigration => "schema_or_migration",
            Self::InfrastructureOrDeploy => "infrastructure_or_deploy",
            Self::PublicApi => "public_api",
            Self::Billing => "billing",
            Self::ConcurrencyOrTransaction => "concurrency_or_transaction",
            Self::LargeDependencyChange => "large_dependency_change",
        }
    }
}

impl RiskLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

/// Provider-neutral task contract. Filesystem admission is intentionally not
/// performed here; the execution-policy service owns that boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContract {
    pub objective: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    #[serde(default)]
    pub read_scope: Vec<String>,
    #[serde(default)]
    pub write_scope: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub verification: Vec<String>,
    pub evidence_requirements: Vec<String>,
    pub max_attempts: u32,
    pub stop_conditions: Vec<String>,
    pub risk: RiskLevel,
    #[serde(default)]
    pub materiality_triggers: BTreeSet<MaterialityTrigger>,
    #[serde(default)]
    pub context: Vec<String>,
    pub max_context_bytes: u64,
}

pub fn validate_task_contract(contract: &TaskContract) -> Vec<PolicyDiagnostic> {
    let mut diagnostics = Vec::new();
    require_nonempty(&mut diagnostics, "task.objective", &contract.objective);
    require_nonempty_list(&mut diagnostics, "task.inputs", &contract.inputs);
    require_nonempty_list(&mut diagnostics, "task.outputs", &contract.outputs);
    require_nonempty_list(
        &mut diagnostics,
        "task.acceptance_criteria",
        &contract.acceptance_criteria,
    );
    require_nonempty_list(
        &mut diagnostics,
        "task.verification",
        &contract.verification,
    );
    require_nonempty_list(
        &mut diagnostics,
        "task.evidence_requirements",
        &contract.evidence_requirements,
    );
    require_nonempty_list(
        &mut diagnostics,
        "task.stop_conditions",
        &contract.stop_conditions,
    );
    if contract.max_attempts == 0 {
        diagnostics.push(PolicyDiagnostic::validation(
            "task.max_attempts",
            "must be at least 1",
        ));
    }
    if contract.max_context_bytes == 0 {
        diagnostics.push(PolicyDiagnostic::validation(
            "task.max_context_bytes",
            "must be at least 1",
        ));
    } else {
        let actual = contract.context.iter().map(String::len).sum::<usize>() as u64;
        if actual > contract.max_context_bytes {
            diagnostics.push(PolicyDiagnostic::validation(
                "task.context",
                format!(
                    "serialized context is {actual} bytes, above max_context_bytes {}",
                    contract.max_context_bytes
                ),
            ));
        }
    }
    diagnostics
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyLoad {
    Missing,
    Invalid(PolicyDiagnostics),
    Loaded(Box<UsagePolicyV1>),
}

pub fn policy_path(root: &Path) -> PathBuf {
    root.join(POLICY_RELATIVE_PATH)
}

/// Missing policy is an explicit non-error state so existing projects retain
/// advisory routing behavior. Existing malformed policy always fails closed.
pub fn load_policy(root: &Path) -> PolicyLoad {
    let path = policy_path(root);
    match std::fs::read_to_string(&path) {
        Ok(text) => match parse_policy(&text) {
            Ok(policy) => PolicyLoad::Loaded(Box::new(policy)),
            Err(diagnostics) => PolicyLoad::Invalid(diagnostics),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => PolicyLoad::Missing,
        Err(err) => PolicyLoad::Invalid(PolicyDiagnostics::single(PolicyDiagnostic::io(
            path.display().to_string(),
            err.to_string(),
        ))),
    }
}

pub fn parse_policy(text: &str) -> Result<UsagePolicyV1, PolicyDiagnostics> {
    let policy = toml::from_str::<UsagePolicyV1>(text)
        .map_err(|err| PolicyDiagnostics::single(PolicyDiagnostic::parse(err.to_string())))?;
    let diagnostics = validate_policy(&policy);
    if diagnostics.is_empty() {
        Ok(policy)
    } else {
        Err(PolicyDiagnostics { diagnostics })
    }
}

pub fn preview_policy_upgrade(
    root: &Path,
) -> Result<Option<PolicyUpgradePreview>, PolicyDiagnostics> {
    let path = policy_path(root);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(PolicyDiagnostics::single(PolicyDiagnostic::io(
                path.display().to_string(),
                error.to_string(),
            )));
        }
    };
    preview_policy_upgrade_text(&text, &path)
}

pub fn apply_policy_upgrade(root: &Path) -> Result<PolicyUpgradePreview, PolicyDiagnostics> {
    let path = policy_path(root);
    let preview = preview_policy_upgrade(root)?.ok_or_else(|| {
        PolicyDiagnostics::single(PolicyDiagnostic::validation(
            "policy",
            "no supported legacy policy upgrade is available",
        ))
    })?;
    parse_policy(&preview.canonical_toml)?;
    let parent = path.parent().ok_or_else(|| {
        PolicyDiagnostics::single(PolicyDiagnostic::io(
            path.display().to_string(),
            "policy path has no parent directory".to_string(),
        ))
    })?;
    let temporary = parent.join(format!(
        ".policy.toml.upgrade-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(preview.canonical_toml.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temporary);
        return Err(PolicyDiagnostics::single(PolicyDiagnostic::io(
            path.display().to_string(),
            error.to_string(),
        )));
    }
    Ok(preview)
}

fn preview_policy_upgrade_text(
    text: &str,
    path: &Path,
) -> Result<Option<PolicyUpgradePreview>, PolicyDiagnostics> {
    if parse_policy(text).is_ok() {
        return Ok(None);
    }
    let current_diagnostics = parse_policy(text).expect_err("current policy was already rejected");
    let raw = toml::from_str::<toml::Value>(text)
        .map_err(|error| PolicyDiagnostics::single(PolicyDiagnostic::parse(error.to_string())))?;
    let usage = raw.get("usage").and_then(toml::Value::as_table);
    let materiality = raw.get("materiality").and_then(toml::Value::as_table);
    let has_legacy_reserve =
        usage.is_some_and(|value| value.contains_key("review_reserve_percent"));
    let has_current_reserves = usage.is_some_and(|value| value.contains_key("phase_reserves"));
    let has_current_protected =
        materiality.is_some_and(|value| value.contains_key("protected_risks"));
    if has_legacy_reserve && (has_current_reserves || has_current_protected) {
        return Err(PolicyDiagnostics::single(PolicyDiagnostic::validation(
            "policy",
            "ambiguous mixed legacy/current policy shape",
        )));
    }
    if !has_legacy_reserve {
        return Err(current_diagnostics);
    }
    let legacy = toml::from_str::<LegacyAlpha2Policy>(text).map_err(|error| {
        PolicyDiagnostics::single(PolicyDiagnostic::parse(format!(
            "unsupported or lossy alpha.2 policy: {error}"
        )))
    })?;
    let policy = UsagePolicyV1 {
        schema_version: legacy.schema_version,
        id: legacy.id,
        version: legacy.version,
        usage: UsageLimits {
            max_active_agents: legacy.usage.max_active_agents,
            max_parallel_readers: legacy.usage.max_parallel_readers,
            max_parallel_writers: legacy.usage.max_parallel_writers,
            max_depth: legacy.usage.max_depth,
            max_attempts: legacy.usage.max_attempts,
            max_wall_time_seconds: legacy.usage.max_wall_time_seconds,
            max_tool_calls: legacy.usage.max_tool_calls,
            max_tokens: legacy.usage.max_tokens,
            max_credits_micros: legacy.usage.max_credits_micros,
            phase_reserves: PhaseBudgetReserves {
                verification_percent: 0,
                review_percent: legacy.usage.review_reserve_percent,
                repair_percent: 0,
            },
            budget_exhaustion: legacy.usage.budget_exhaustion,
            metering: legacy.usage.metering,
        },
        transitions: legacy.transitions,
        materiality: MaterialityPolicy {
            protected_risks: canonical_interrupting_risks(),
            changed_files_threshold: legacy.materiality.changed_files_threshold,
            changed_lines_threshold: legacy.materiality.changed_lines_threshold,
        },
        execution: legacy.execution,
    };
    let diagnostics = validate_policy(&policy);
    if !diagnostics.is_empty() {
        return Err(PolicyDiagnostics { diagnostics });
    }
    let canonical_toml = toml::to_string_pretty(&policy).map_err(|error| {
        PolicyDiagnostics::single(PolicyDiagnostic::parse(format!(
            "canonical policy serialization failed: {error}"
        )))
    })?;
    parse_policy(&canonical_toml)?;
    Ok(Some(PolicyUpgradePreview {
        from_shape: LEGACY_ALPHA2_POLICY_SHAPE.to_string(),
        to_shape: CURRENT_POLICY_SHAPE.to_string(),
        path: path.display().to_string(),
        changes: vec![
            "usage.review_reserve_percent -> usage.phase_reserves.review_percent".to_string(),
            "materiality.protected_risks -> canonical_interrupting_risks".to_string(),
        ],
        canonical_toml,
    }))
}

fn canonical_interrupting_risks() -> BTreeSet<MaterialityTrigger> {
    BTreeSet::from([
        MaterialityTrigger::SecurityOrAuth,
        MaterialityTrigger::SecretsOrCrypto,
        MaterialityTrigger::SchemaOrMigration,
        MaterialityTrigger::InfrastructureOrDeploy,
        MaterialityTrigger::PublicApi,
        MaterialityTrigger::Billing,
        MaterialityTrigger::ConcurrencyOrTransaction,
    ])
}

pub fn validate_policy(policy: &UsagePolicyV1) -> Vec<PolicyDiagnostic> {
    let mut diagnostics = Vec::new();
    if policy.schema_version != USAGE_POLICY_SCHEMA_VERSION {
        diagnostics.push(PolicyDiagnostic::validation(
            "schema_version",
            format!(
                "unsupported schema version {}; expected {}",
                policy.schema_version, USAGE_POLICY_SCHEMA_VERSION
            ),
        ));
    }
    require_nonempty(&mut diagnostics, "id", &policy.id);
    require_nonempty(&mut diagnostics, "version", &policy.version);
    if policy.usage.max_active_agents == 0 {
        diagnostics.push(PolicyDiagnostic::validation(
            "usage.max_active_agents",
            "must be at least 1",
        ));
    }
    if policy.usage.max_parallel_readers > policy.usage.max_active_agents {
        diagnostics.push(PolicyDiagnostic::validation(
            "usage.max_parallel_readers",
            "cannot exceed usage.max_active_agents",
        ));
    }
    if policy.usage.max_parallel_writers > policy.usage.max_active_agents {
        diagnostics.push(PolicyDiagnostic::validation(
            "usage.max_parallel_writers",
            "cannot exceed usage.max_active_agents",
        ));
    }
    if policy.usage.max_depth != 1 {
        diagnostics.push(PolicyDiagnostic::validation(
            "usage.max_depth",
            "Usage Policy v1 requires max_depth = 1",
        ));
    }
    if policy.usage.max_attempts == 0 {
        diagnostics.push(PolicyDiagnostic::validation(
            "usage.max_attempts",
            "must be at least 1",
        ));
    }
    if policy.usage.phase_reserves.total_percent() > 100 {
        diagnostics.push(PolicyDiagnostic::validation(
            "usage.phase_reserves",
            "verification, review, and repair reserves must total at most 100 percent",
        ));
    }
    validate_optional_limit(
        &mut diagnostics,
        "usage.max_wall_time_seconds",
        policy.usage.max_wall_time_seconds,
    );
    validate_optional_limit(
        &mut diagnostics,
        "usage.max_tool_calls",
        policy.usage.max_tool_calls,
    );
    validate_optional_limit(
        &mut diagnostics,
        "usage.max_tokens",
        policy.usage.max_tokens,
    );
    validate_optional_limit(
        &mut diagnostics,
        "usage.max_credits_micros",
        policy.usage.max_credits_micros,
    );
    validate_optional_u32(
        &mut diagnostics,
        "materiality.changed_files_threshold",
        policy.materiality.changed_files_threshold,
    );
    validate_optional_u32(
        &mut diagnostics,
        "materiality.changed_lines_threshold",
        policy.materiality.changed_lines_threshold,
    );
    if policy
        .materiality
        .protected_risks
        .contains(&MaterialityTrigger::LargeDependencyChange)
    {
        diagnostics.push(PolicyDiagnostic::validation(
            "materiality.protected_risks",
            "large_dependency_change is assurance-only and cannot be a protected interrupting risk",
        ));
    }
    for diagnostic in crate::execution_policy::validate_execution_policy(&policy.execution) {
        diagnostics.push(PolicyDiagnostic::validation(
            format!("execution.{}", diagnostic.field),
            diagnostic.message,
        ));
    }

    let max_transitions = policy.usage.max_attempts.saturating_sub(1);
    if policy.transitions.retry.max_same_route_retries > max_transitions {
        diagnostics.push(PolicyDiagnostic::validation(
            "transitions.retry.max_same_route_retries",
            "cannot exceed usage.max_attempts - 1",
        ));
    }
    if policy.transitions.availability_fallback.max_fallbacks > max_transitions {
        diagnostics.push(PolicyDiagnostic::validation(
            "transitions.availability_fallback.max_fallbacks",
            "cannot exceed usage.max_attempts - 1",
        ));
    }
    if !policy
        .transitions
        .availability_fallback
        .require_same_capability_class
    {
        diagnostics.push(PolicyDiagnostic::validation(
            "transitions.availability_fallback.require_same_capability_class",
            "must be true in Usage Policy v1",
        ));
    }
    if policy.transitions.quality_escalation.max_escalations > max_transitions {
        diagnostics.push(PolicyDiagnostic::validation(
            "transitions.quality_escalation.max_escalations",
            "cannot exceed usage.max_attempts - 1",
        ));
    }
    if !policy
        .transitions
        .quality_escalation
        .require_verification_evidence
    {
        diagnostics.push(PolicyDiagnostic::validation(
            "transitions.quality_escalation.require_verification_evidence",
            "must be true in Usage Policy v1",
        ));
    }
    if policy.transitions.quota_downgrade.max_downgrades > max_transitions {
        diagnostics.push(PolicyDiagnostic::validation(
            "transitions.quota_downgrade.max_downgrades",
            "cannot exceed usage.max_attempts - 1",
        ));
    }
    if policy.transitions.quota_downgrade.enabled
        && !policy.transitions.quota_downgrade.noncritical_only
    {
        diagnostics.push(PolicyDiagnostic::validation(
            "transitions.quota_downgrade.noncritical_only",
            "must be true when quota downgrade is enabled",
        ));
    }
    if !policy.transitions.safety_stop.enabled {
        diagnostics.push(PolicyDiagnostic::validation(
            "transitions.safety_stop.enabled",
            "must be true in Usage Policy v1",
        ));
    }
    diagnostics
}

fn require_nonempty(diagnostics: &mut Vec<PolicyDiagnostic>, field: &str, value: &str) {
    if value.trim().is_empty() {
        diagnostics.push(PolicyDiagnostic::validation(field, "must not be empty"));
    }
}

fn require_nonempty_list(diagnostics: &mut Vec<PolicyDiagnostic>, field: &str, values: &[String]) {
    if values.is_empty() || values.iter().any(|value| value.trim().is_empty()) {
        diagnostics.push(PolicyDiagnostic::validation(
            field,
            "must contain at least one non-empty value and no empty values",
        ));
    }
}

fn validate_optional_limit(
    diagnostics: &mut Vec<PolicyDiagnostic>,
    field: &str,
    value: Option<u64>,
) {
    if value == Some(0) {
        diagnostics.push(PolicyDiagnostic::validation(field, "must be at least 1"));
    }
}

fn validate_optional_u32(diagnostics: &mut Vec<PolicyDiagnostic>, field: &str, value: Option<u32>) {
    if value == Some(0) {
        diagnostics.push(PolicyDiagnostic::validation(field, "must be at least 1"));
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyDiagnostics {
    pub diagnostics: Vec<PolicyDiagnostic>,
}

impl PolicyDiagnostics {
    fn single(diagnostic: PolicyDiagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
        }
    }
}

impl fmt::Display for PolicyDiagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            write!(formatter, "{diagnostic}")?;
        }
        Ok(())
    }
}

impl std::error::Error for PolicyDiagnostics {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyDiagnostic {
    pub code: String,
    pub field: Option<String>,
    pub message: String,
}

impl PolicyDiagnostic {
    fn parse(message: String) -> Self {
        Self {
            code: "policy_parse_error".to_string(),
            field: None,
            message,
        }
    }

    fn validation(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: "policy_validation_error".to_string(),
            field: Some(field.into()),
            message: message.into(),
        }
    }

    fn io(field: String, message: String) -> Self {
        Self {
            code: "policy_io_error".to_string(),
            field: Some(field),
            message,
        }
    }
}

impl fmt::Display for PolicyDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(field) = &self.field {
            write!(formatter, "{}: {}", field, self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryTrigger {
    TransientToolFailure,
    Timeout,
    RateLimited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityTrigger {
    HostUnavailable,
    ModelUnavailable,
    RateLimited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityTrigger {
    VerificationFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaTrigger {
    BudgetThreshold,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyTrigger {
    PermissionDenied,
    PolicyViolation,
    UnsafeOperation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransitionRequest {
    Retry {
        trigger: RetryTrigger,
    },
    AvailabilityFallback {
        trigger: AvailabilityTrigger,
        same_capability_class: bool,
    },
    QualityEscalation {
        trigger: QualityTrigger,
        verification_evidence: bool,
    },
    QuotaDowngrade {
        trigger: QuotaTrigger,
        remaining_usage_evidence: bool,
    },
    SafetyStop {
        trigger: SafetyTrigger,
        policy_rule: String,
    },
}

impl TransitionRequest {
    pub const fn kind(&self) -> TransitionKind {
        match self {
            Self::Retry { .. } => TransitionKind::Retry,
            Self::AvailabilityFallback { .. } => TransitionKind::AvailabilityFallback,
            Self::QualityEscalation { .. } => TransitionKind::QualityEscalation,
            Self::QuotaDowngrade { .. } => TransitionKind::QuotaDowngrade,
            Self::SafetyStop { .. } => TransitionKind::SafetyStop,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    Retry,
    AvailabilityFallback,
    QualityEscalation,
    QuotaDowngrade,
    SafetyStop,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageObservation {
    pub metering: MeteringMode,
    #[serde(default)]
    pub tool_calls: Option<u64>,
    #[serde(default)]
    pub tokens: Option<u64>,
    #[serde(default)]
    pub credits_micros: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionState {
    pub current_depth: u8,
    pub projected_active_agents: u32,
    pub projected_parallel_readers: u32,
    pub projected_parallel_writers: u32,
    pub attempts_started: u32,
    pub same_route_retries: u32,
    pub availability_fallbacks: u32,
    pub quality_escalations: u32,
    pub quota_downgrades: u32,
    pub elapsed_seconds: u64,
    pub usage: UsageObservation,
    pub risk: RiskLevel,
    pub material: bool,
    pub budget_phase: BudgetPhase,
    #[serde(default)]
    pub pending_safety_stop: Option<SafetyTrigger>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetDimension {
    WallTime,
    ToolCalls,
    Tokens,
    Credits,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetAssessment {
    pub dimension: BudgetDimension,
    pub metering: MeteringMode,
    pub observed: Option<u64>,
    pub configured_limit: u64,
    pub effective_limit: u64,
    pub exhausted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TransitionDecision {
    Allowed {
        transition: TransitionKind,
        requires_independent_review: bool,
        budget: Vec<BudgetAssessment>,
    },
    Rejected {
        transition: TransitionKind,
        reason: TransitionRejectionReason,
        message: String,
        budget: Vec<BudgetAssessment>,
    },
}

impl TransitionDecision {
    pub fn rejection_reason(&self) -> Option<TransitionRejectionReason> {
        match self {
            Self::Allowed { .. } => None,
            Self::Rejected { reason, .. } => Some(*reason),
        }
    }

    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionRejectionReason {
    InvalidPolicy,
    SafetyStopRequired,
    AttemptBudgetExhausted,
    DepthLimitExceeded,
    ActiveAgentLimitExceeded,
    ReaderLimitExceeded,
    WriterLimitExceeded,
    WallTimeBudgetExhausted,
    ToolCallBudgetExhausted,
    TokenBudgetExhausted,
    CreditBudgetExhausted,
    RequiredPhaseReserveProtected,
    RetryExhausted,
    AvailabilityFallbackExhausted,
    AvailabilityRetryRequired,
    CapabilityClassMismatch,
    QualityEscalationExhausted,
    VerificationEvidenceRequired,
    BudgetExhaustionRequiresStop,
    QuotaDowngradeDisabled,
    QuotaDowngradeExhausted,
    MaterialWorkCannotDowngrade,
    CriticalWorkCannotDowngrade,
    UsageEvidenceRequired,
    SafetyStopDisabled,
    PolicyRuleRequired,
}

/// Resolves one proposed transition. It does not mutate counters or choose a
/// concrete route: callers persist an allowed decision and then perform the
/// host-specific action.
pub fn resolve_transition(
    policy: &UsagePolicyV1,
    state: &TransitionState,
    request: &TransitionRequest,
) -> TransitionDecision {
    let kind = request.kind();
    let policy_diagnostics = validate_policy(policy);
    if !policy_diagnostics.is_empty() {
        return rejected(
            kind,
            TransitionRejectionReason::InvalidPolicy,
            PolicyDiagnostics {
                diagnostics: policy_diagnostics,
            }
            .to_string(),
            Vec::new(),
        );
    }

    if let TransitionRequest::SafetyStop { policy_rule, .. } = request {
        if !policy.transitions.safety_stop.enabled {
            return rejected(
                kind,
                TransitionRejectionReason::SafetyStopDisabled,
                "safety stop is disabled",
                Vec::new(),
            );
        }
        if policy_rule.trim().is_empty() {
            return rejected(
                kind,
                TransitionRejectionReason::PolicyRuleRequired,
                "safety stop requires a policy rule or repair reference",
                Vec::new(),
            );
        }
        return allowed(kind, state.material, Vec::new());
    }

    if let Some(trigger) = state.pending_safety_stop {
        return rejected(
            kind,
            TransitionRejectionReason::SafetyStopRequired,
            format!(
                "{} requires safety_stop; retry, fallback, escalation, and downgrade are forbidden",
                trigger.as_str()
            ),
            Vec::new(),
        );
    }

    if state.current_depth > policy.usage.max_depth {
        return rejected(
            kind,
            TransitionRejectionReason::DepthLimitExceeded,
            format!(
                "current depth {} exceeds max_depth {}",
                state.current_depth, policy.usage.max_depth
            ),
            Vec::new(),
        );
    }
    if state.projected_active_agents > policy.usage.max_active_agents {
        return rejected(
            kind,
            TransitionRejectionReason::ActiveAgentLimitExceeded,
            "projected active agents exceed policy limit",
            Vec::new(),
        );
    }
    if state.projected_parallel_readers > policy.usage.max_parallel_readers {
        return rejected(
            kind,
            TransitionRejectionReason::ReaderLimitExceeded,
            "projected parallel readers exceed policy limit",
            Vec::new(),
        );
    }
    if state.projected_parallel_writers > policy.usage.max_parallel_writers {
        return rejected(
            kind,
            TransitionRejectionReason::WriterLimitExceeded,
            "projected parallel writers exceed policy limit",
            Vec::new(),
        );
    }

    let budget = assess_budgets(policy, state);
    let route_change_only = matches!(request, TransitionRequest::QuotaDowngrade { .. });
    if !route_change_only {
        if state.attempts_started >= policy.usage.max_attempts {
            return rejected(
                kind,
                TransitionRejectionReason::AttemptBudgetExhausted,
                "attempt budget is exhausted",
                budget,
            );
        }
        if let Some((reason, message)) = first_exhausted_budget(&budget) {
            return rejected(kind, reason, message, budget);
        }
    }

    match request {
        TransitionRequest::Retry { .. } => {
            if state.same_route_retries >= policy.transitions.retry.max_same_route_retries {
                rejected(
                    kind,
                    TransitionRejectionReason::RetryExhausted,
                    "same-route retry limit is exhausted",
                    budget,
                )
            } else {
                allowed(kind, state.material, budget)
            }
        }
        TransitionRequest::AvailabilityFallback {
            trigger,
            same_capability_class,
        } => {
            if matches!(trigger, AvailabilityTrigger::RateLimited)
                && state.same_route_retries < policy.transitions.retry.max_same_route_retries
            {
                return rejected(
                    kind,
                    TransitionRejectionReason::AvailabilityRetryRequired,
                    "rate-limit retry budget must be used before availability fallback",
                    budget,
                );
            }
            if state.availability_fallbacks
                >= policy.transitions.availability_fallback.max_fallbacks
            {
                return rejected(
                    kind,
                    TransitionRejectionReason::AvailabilityFallbackExhausted,
                    "availability fallback limit is exhausted",
                    budget,
                );
            }
            if policy
                .transitions
                .availability_fallback
                .require_same_capability_class
                && !same_capability_class
            {
                return rejected(
                    kind,
                    TransitionRejectionReason::CapabilityClassMismatch,
                    "availability fallback must remain in the same capability class",
                    budget,
                );
            }
            allowed(kind, state.material, budget)
        }
        TransitionRequest::QualityEscalation {
            verification_evidence,
            ..
        } => {
            if state.quality_escalations >= policy.transitions.quality_escalation.max_escalations {
                return rejected(
                    kind,
                    TransitionRejectionReason::QualityEscalationExhausted,
                    "quality escalation limit is exhausted",
                    budget,
                );
            }
            if policy
                .transitions
                .quality_escalation
                .require_verification_evidence
                && !verification_evidence
            {
                return rejected(
                    kind,
                    TransitionRejectionReason::VerificationEvidenceRequired,
                    "quality escalation requires verification-failure evidence",
                    budget,
                );
            }
            allowed(kind, state.material, budget)
        }
        TransitionRequest::QuotaDowngrade {
            remaining_usage_evidence,
            ..
        } => {
            if policy.usage.budget_exhaustion == BudgetExhaustionBehavior::Stop {
                return rejected(
                    kind,
                    TransitionRejectionReason::BudgetExhaustionRequiresStop,
                    "budget exhaustion behavior is stop; quota downgrade is forbidden",
                    budget,
                );
            }
            if !policy.transitions.quota_downgrade.enabled {
                return rejected(
                    kind,
                    TransitionRejectionReason::QuotaDowngradeDisabled,
                    "quota downgrade is disabled",
                    budget,
                );
            }
            if state.quota_downgrades >= policy.transitions.quota_downgrade.max_downgrades {
                return rejected(
                    kind,
                    TransitionRejectionReason::QuotaDowngradeExhausted,
                    "quota downgrade limit is exhausted",
                    budget,
                );
            }
            if state.risk >= RiskLevel::High {
                return rejected(
                    kind,
                    TransitionRejectionReason::CriticalWorkCannotDowngrade,
                    "high-risk or critical work cannot be quota-downgraded",
                    budget,
                );
            }
            if policy.transitions.quota_downgrade.noncritical_only && state.material {
                return rejected(
                    kind,
                    TransitionRejectionReason::MaterialWorkCannotDowngrade,
                    "material work cannot be quota-downgraded",
                    budget,
                );
            }
            if !remaining_usage_evidence || state.usage.metering == MeteringMode::Unavailable {
                return rejected(
                    kind,
                    TransitionRejectionReason::UsageEvidenceRequired,
                    "quota downgrade requires estimated or trusted remaining-usage evidence",
                    budget,
                );
            }
            allowed(kind, false, budget)
        }
        TransitionRequest::SafetyStop { .. } => unreachable!("handled before spend gates"),
    }
}

impl SafetyTrigger {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::PolicyViolation => "policy_violation",
            Self::UnsafeOperation => "unsafe_operation",
        }
    }
}

fn allowed(
    transition: TransitionKind,
    requires_independent_review: bool,
    budget: Vec<BudgetAssessment>,
) -> TransitionDecision {
    TransitionDecision::Allowed {
        transition,
        requires_independent_review,
        budget,
    }
}

fn rejected(
    transition: TransitionKind,
    reason: TransitionRejectionReason,
    message: impl Into<String>,
    budget: Vec<BudgetAssessment>,
) -> TransitionDecision {
    TransitionDecision::Rejected {
        transition,
        reason,
        message: message.into(),
        budget,
    }
}

pub fn assess_budgets(policy: &UsagePolicyV1, state: &TransitionState) -> Vec<BudgetAssessment> {
    let mut assessments = Vec::new();
    if let Some(limit) = policy.usage.max_wall_time_seconds {
        assessments.push(BudgetAssessment {
            dimension: BudgetDimension::WallTime,
            metering: MeteringMode::Trusted,
            observed: Some(state.elapsed_seconds),
            configured_limit: limit,
            effective_limit: reserved_limit(limit, policy, state.budget_phase),
            exhausted: state.elapsed_seconds >= reserved_limit(limit, policy, state.budget_phase),
        });
    }
    push_metered_assessment(
        &mut assessments,
        BudgetDimension::ToolCalls,
        policy.usage.max_tool_calls,
        state.usage.tool_calls,
        policy,
        state,
    );
    push_metered_assessment(
        &mut assessments,
        BudgetDimension::Tokens,
        policy.usage.max_tokens,
        state.usage.tokens,
        policy,
        state,
    );
    push_metered_assessment(
        &mut assessments,
        BudgetDimension::Credits,
        policy.usage.max_credits_micros,
        state.usage.credits_micros,
        policy,
        state,
    );
    assessments
}

fn push_metered_assessment(
    assessments: &mut Vec<BudgetAssessment>,
    dimension: BudgetDimension,
    limit: Option<u64>,
    observed: Option<u64>,
    policy: &UsagePolicyV1,
    state: &TransitionState,
) {
    let Some(limit) = limit else {
        return;
    };
    // Confidence is per dimension: a globally trustworthy meter cannot make
    // a missing token/tool/credit observation enforceable.
    let metering = if observed.is_some() {
        std::cmp::min(policy.usage.metering, state.usage.metering)
    } else {
        MeteringMode::Unavailable
    };
    let effective_limit = reserved_limit(limit, policy, state.budget_phase);
    assessments.push(BudgetAssessment {
        dimension,
        metering,
        observed,
        configured_limit: limit,
        effective_limit,
        exhausted: metering == MeteringMode::Trusted
            && observed.is_some_and(|value| value >= effective_limit),
    });
}

pub fn phase_spend_limit(limit: u64, reserves: PhaseBudgetReserves, phase: BudgetPhase) -> u64 {
    let protected = reserves.protected_percent_for(phase).min(100);
    let spend_percent = u64::from(100 - protected);
    (limit / 100) * spend_percent + ((limit % 100) * spend_percent) / 100
}

fn reserved_limit(limit: u64, policy: &UsagePolicyV1, phase: BudgetPhase) -> u64 {
    phase_spend_limit(limit, policy.usage.phase_reserves, phase)
}

pub(crate) fn first_exhausted_budget(
    budget: &[BudgetAssessment],
) -> Option<(TransitionRejectionReason, String)> {
    let assessment = budget.iter().find(|assessment| assessment.exhausted)?;
    let reserved = assessment.effective_limit < assessment.configured_limit;
    let reason = if reserved {
        TransitionRejectionReason::RequiredPhaseReserveProtected
    } else {
        match assessment.dimension {
            BudgetDimension::WallTime => TransitionRejectionReason::WallTimeBudgetExhausted,
            BudgetDimension::ToolCalls => TransitionRejectionReason::ToolCallBudgetExhausted,
            BudgetDimension::Tokens => TransitionRejectionReason::TokenBudgetExhausted,
            BudgetDimension::Credits => TransitionRejectionReason::CreditBudgetExhausted,
        }
    };
    let message = if reserved {
        format!(
            "{} budget reached the non-review limit; reserved review budget is protected",
            assessment.dimension.as_str()
        )
    } else {
        format!("{} budget is exhausted", assessment.dimension.as_str())
    };
    Some((reason, message))
}

impl BudgetDimension {
    const fn as_str(self) -> &'static str {
        match self {
            Self::WallTime => "wall_time",
            Self::ToolCalls => "tool_calls",
            Self::Tokens => "tokens",
            Self::Credits => "credits",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_policy::{ExecutionPolicy, FilesystemPermissions, RolePermissions};
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn execution_policy() -> ExecutionPolicy {
        ExecutionPolicy {
            max_read_scope_entries: 4,
            max_write_scope_entries: 2,
            roles: BTreeMap::from([(
                "worker".to_string(),
                RolePermissions {
                    filesystem: FilesystemPermissions {
                        read_roots: BTreeSet::from(["src".to_string(), "tests".to_string()]),
                        write_roots: BTreeSet::from(["src".to_string(), "tests".to_string()]),
                        allow_overwrite: true,
                    },
                    ..RolePermissions::default()
                },
            )]),
        }
    }

    fn policy() -> UsagePolicyV1 {
        UsagePolicyV1 {
            schema_version: 1,
            id: "policy-a".to_string(),
            version: "1.0.0".to_string(),
            usage: UsageLimits {
                max_active_agents: 3,
                max_parallel_readers: 2,
                max_parallel_writers: 1,
                max_depth: 1,
                max_attempts: 4,
                max_wall_time_seconds: Some(600),
                max_tool_calls: Some(100),
                max_tokens: Some(10_000),
                max_credits_micros: Some(1_000_000),
                phase_reserves: PhaseBudgetReserves {
                    verification_percent: 10,
                    review_percent: 5,
                    repair_percent: 5,
                },
                budget_exhaustion: BudgetExhaustionBehavior::DowngradeNoncritical,
                metering: MeteringMode::Trusted,
            },
            transitions: TransitionPolicy {
                retry: RetryPolicy {
                    max_same_route_retries: 1,
                },
                availability_fallback: AvailabilityFallbackPolicy {
                    max_fallbacks: 1,
                    require_same_capability_class: true,
                },
                quality_escalation: QualityEscalationPolicy {
                    max_escalations: 1,
                    require_verification_evidence: true,
                },
                quota_downgrade: QuotaDowngradePolicy {
                    enabled: true,
                    max_downgrades: 1,
                    noncritical_only: true,
                },
                safety_stop: SafetyStopPolicy { enabled: true },
            },
            materiality: MaterialityPolicy {
                protected_risks: BTreeSet::from([
                    MaterialityTrigger::SecurityOrAuth,
                    MaterialityTrigger::SecretsOrCrypto,
                    MaterialityTrigger::SchemaOrMigration,
                    MaterialityTrigger::InfrastructureOrDeploy,
                    MaterialityTrigger::PublicApi,
                    MaterialityTrigger::Billing,
                    MaterialityTrigger::ConcurrencyOrTransaction,
                ]),
                changed_files_threshold: Some(10),
                changed_lines_threshold: Some(500),
            },
            execution: execution_policy(),
        }
    }

    fn state() -> TransitionState {
        TransitionState {
            current_depth: 1,
            projected_active_agents: 1,
            projected_parallel_readers: 0,
            projected_parallel_writers: 1,
            attempts_started: 1,
            same_route_retries: 0,
            availability_fallbacks: 0,
            quality_escalations: 0,
            quota_downgrades: 0,
            elapsed_seconds: 10,
            usage: UsageObservation {
                metering: MeteringMode::Trusted,
                tool_calls: Some(5),
                tokens: Some(500),
                credits_micros: Some(10_000),
            },
            risk: RiskLevel::Low,
            material: false,
            budget_phase: BudgetPhase::Implementation,
            pending_safety_stop: None,
        }
    }

    fn v2_budget_provenance() -> BudgetProvenance {
        BudgetProvenance {
            wall_seconds: MeteringProvenance::Trusted,
            tool_calls: MeteringProvenance::Estimated,
            tokens: MeteringProvenance::Trusted,
        }
    }

    fn v2_budget_limits() -> BudgetAmounts {
        BudgetAmounts {
            wall_seconds: 100,
            tool_calls: 50,
            tokens: 1_000,
        }
    }

    fn v2_phase_reserves() -> FeatureRunPhaseReserves {
        FeatureRunPhaseReserves {
            maker: BudgetAmounts {
                wall_seconds: 50,
                tool_calls: 25,
                tokens: 500,
            },
            verification: BudgetAmounts {
                wall_seconds: 20,
                tool_calls: 10,
                tokens: 200,
            },
            review: BudgetAmounts {
                wall_seconds: 10,
                tool_calls: 5,
                tokens: 100,
            },
            repair: BudgetAmounts {
                wall_seconds: 10,
                tool_calls: 5,
                tokens: 100,
            },
            release: BudgetAmounts {
                wall_seconds: 10,
                tool_calls: 5,
                tokens: 100,
            },
        }
    }

    fn v2_bounded_contract() -> FeatureRunBudgetContract {
        FeatureRunBudgetContract::bounded(
            "run-budget-v2",
            1_700_000_000_000,
            v2_budget_limits(),
            v2_phase_reserves(),
            v2_budget_provenance(),
        )
        .expect("valid bounded contract")
    }

    fn valid_toml() -> String {
        toml::to_string(&policy()).expect("serialize policy")
    }

    #[test]
    fn missing_policy_preserves_legacy_behavior() {
        let root = tempdir().expect("tempdir");
        assert_eq!(load_policy(root.path()), PolicyLoad::Missing);
    }

    #[test]
    fn malformed_policy_has_parser_location_and_unknown_field() {
        let text = valid_toml().replace("max_depth = 1", "max_depth = 1\nmodel = \"gpt-x\"");
        let diagnostics = parse_policy(&text).expect_err("provider field must be rejected");
        let message = diagnostics.to_string();
        assert!(message.contains("unknown field `model`"), "{message}");
        assert!(message.contains("line"), "{message}");
    }

    #[test]
    fn validation_rejects_unsafe_or_unbounded_v1_configuration() {
        let mut value = policy();
        value.usage.max_depth = 2;
        value.usage.max_attempts = 0;
        value.usage.max_tokens = Some(0);
        value
            .transitions
            .availability_fallback
            .require_same_capability_class = false;
        value
            .transitions
            .quality_escalation
            .require_verification_evidence = false;
        value.transitions.safety_stop.enabled = false;
        value
            .materiality
            .protected_risks
            .insert(MaterialityTrigger::LargeDependencyChange);
        let fields: Vec<_> = validate_policy(&value)
            .into_iter()
            .filter_map(|diagnostic| diagnostic.field)
            .collect();
        assert_eq!(
            fields,
            vec![
                "usage.max_depth",
                "usage.max_attempts",
                "usage.max_tokens",
                "materiality.protected_risks",
                "transitions.retry.max_same_route_retries",
                "transitions.availability_fallback.max_fallbacks",
                "transitions.availability_fallback.require_same_capability_class",
                "transitions.quality_escalation.max_escalations",
                "transitions.quality_escalation.require_verification_evidence",
                "transitions.quota_downgrade.max_downgrades",
                "transitions.safety_stop.enabled",
            ]
        );
    }

    #[test]
    fn transition_wire_shape_cannot_confuse_failure_classes() {
        let wrong_quality = r#"{
            "kind":"quality_escalation",
            "trigger":"rate_limited",
            "verification_evidence":true
        }"#;
        assert!(serde_json::from_str::<TransitionRequest>(wrong_quality).is_err());

        let wrong_retry = r#"{
            "kind":"retry",
            "trigger":"permission_denied"
        }"#;
        assert!(serde_json::from_str::<TransitionRequest>(wrong_retry).is_err());
    }

    #[test]
    fn rate_limit_uses_retry_before_same_class_fallback() {
        let value = policy();
        let mut current = state();
        let fallback = TransitionRequest::AvailabilityFallback {
            trigger: AvailabilityTrigger::RateLimited,
            same_capability_class: true,
        };
        assert_eq!(
            resolve_transition(&value, &current, &fallback).rejection_reason(),
            Some(TransitionRejectionReason::AvailabilityRetryRequired)
        );

        let retry = TransitionRequest::Retry {
            trigger: RetryTrigger::RateLimited,
        };
        assert!(resolve_transition(&value, &current, &retry).is_allowed());
        current.same_route_retries = 1;
        assert!(resolve_transition(&value, &current, &fallback).is_allowed());
        assert_eq!(
            resolve_transition(&value, &current, &retry).rejection_reason(),
            Some(TransitionRejectionReason::RetryExhausted)
        );
    }

    #[test]
    fn verification_failure_escalates_once_and_requires_evidence() {
        let value = policy();
        let request = TransitionRequest::QualityEscalation {
            trigger: QualityTrigger::VerificationFailed,
            verification_evidence: false,
        };
        let mut current = state();
        assert_eq!(
            resolve_transition(&value, &current, &request).rejection_reason(),
            Some(TransitionRejectionReason::VerificationEvidenceRequired)
        );

        let request = TransitionRequest::QualityEscalation {
            trigger: QualityTrigger::VerificationFailed,
            verification_evidence: true,
        };
        assert!(resolve_transition(&value, &current, &request).is_allowed());
        current.quality_escalations = 1;
        assert_eq!(
            resolve_transition(&value, &current, &request).rejection_reason(),
            Some(TransitionRejectionReason::QualityEscalationExhausted)
        );
    }

    #[test]
    fn safety_triggers_forbid_every_spending_transition() {
        let value = policy();
        let requests = [
            TransitionRequest::Retry {
                trigger: RetryTrigger::TransientToolFailure,
            },
            TransitionRequest::AvailabilityFallback {
                trigger: AvailabilityTrigger::HostUnavailable,
                same_capability_class: true,
            },
            TransitionRequest::QualityEscalation {
                trigger: QualityTrigger::VerificationFailed,
                verification_evidence: true,
            },
            TransitionRequest::QuotaDowngrade {
                trigger: QuotaTrigger::BudgetThreshold,
                remaining_usage_evidence: true,
            },
        ];
        for trigger in [
            SafetyTrigger::PermissionDenied,
            SafetyTrigger::PolicyViolation,
            SafetyTrigger::UnsafeOperation,
        ] {
            let mut current = state();
            current.pending_safety_stop = Some(trigger);
            for request in &requests {
                assert_eq!(
                    resolve_transition(&value, &current, request).rejection_reason(),
                    Some(TransitionRejectionReason::SafetyStopRequired)
                );
            }
            assert!(
                resolve_transition(
                    &value,
                    &current,
                    &TransitionRequest::SafetyStop {
                        trigger,
                        policy_rule: "execution.permission".to_string(),
                    }
                )
                .is_allowed()
            );
        }
    }

    #[test]
    fn quota_downgrade_is_gated_by_usage_materiality_and_risk() {
        let value = policy();
        let request = TransitionRequest::QuotaDowngrade {
            trigger: QuotaTrigger::BudgetThreshold,
            remaining_usage_evidence: true,
        };
        let mut current = state();
        assert!(resolve_transition(&value, &current, &request).is_allowed());

        current.material = true;
        assert_eq!(
            resolve_transition(&value, &current, &request).rejection_reason(),
            Some(TransitionRejectionReason::MaterialWorkCannotDowngrade)
        );
        current.material = false;
        current.risk = RiskLevel::High;
        assert_eq!(
            resolve_transition(&value, &current, &request).rejection_reason(),
            Some(TransitionRejectionReason::CriticalWorkCannotDowngrade)
        );
        current.risk = RiskLevel::Low;
        current.usage.metering = MeteringMode::Unavailable;
        assert_eq!(
            resolve_transition(&value, &current, &request).rejection_reason(),
            Some(TransitionRejectionReason::UsageEvidenceRequired)
        );
    }

    #[test]
    fn budget_exhaustion_behavior_deterministically_controls_quota_downgrade() {
        let request = TransitionRequest::QuotaDowngrade {
            trigger: QuotaTrigger::BudgetThreshold,
            remaining_usage_evidence: true,
        };
        let current = state();

        let mut value = policy();
        value.usage.budget_exhaustion = BudgetExhaustionBehavior::Stop;
        assert_eq!(
            resolve_transition(&value, &current, &request).rejection_reason(),
            Some(TransitionRejectionReason::BudgetExhaustionRequiresStop)
        );

        value.usage.budget_exhaustion = BudgetExhaustionBehavior::DowngradeNoncritical;
        assert!(resolve_transition(&value, &current, &request).is_allowed());
    }

    #[test]
    fn count_time_and_trusted_usage_budgets_fail_closed() {
        let value = policy();
        let request = TransitionRequest::Retry {
            trigger: RetryTrigger::Timeout,
        };
        let rejects = |current: TransitionState, expected| {
            assert_eq!(
                resolve_transition(&value, &current, &request).rejection_reason(),
                Some(expected)
            );
        };
        let mut current = state();
        current.current_depth = 2;
        rejects(current, TransitionRejectionReason::DepthLimitExceeded);

        let mut current = state();
        current.projected_active_agents = 4;
        rejects(current, TransitionRejectionReason::ActiveAgentLimitExceeded);

        let mut current = state();
        current.projected_parallel_readers = 3;
        rejects(current, TransitionRejectionReason::ReaderLimitExceeded);

        let mut current = state();
        current.projected_parallel_writers = 2;
        rejects(current, TransitionRejectionReason::WriterLimitExceeded);

        let mut current = state();
        current.usage.tokens = Some(8_000);
        assert_eq!(
            resolve_transition(&value, &current, &request).rejection_reason(),
            Some(TransitionRejectionReason::RequiredPhaseReserveProtected)
        );
        current.budget_phase = BudgetPhase::Repair;
        assert!(resolve_transition(&value, &current, &request).is_allowed());
        current.usage.tokens = Some(10_000);
        assert_eq!(
            resolve_transition(&value, &current, &request).rejection_reason(),
            Some(TransitionRejectionReason::TokenBudgetExhausted)
        );
    }

    #[test]
    fn untrusted_metering_is_visible_but_not_silently_enforced() {
        let value = policy();
        let mut current = state();
        current.usage.metering = MeteringMode::Estimated;
        current.usage.tokens = Some(50_000);
        let decision = resolve_transition(
            &value,
            &current,
            &TransitionRequest::Retry {
                trigger: RetryTrigger::TransientToolFailure,
            },
        );
        let TransitionDecision::Allowed { budget, .. } = decision else {
            panic!("estimated budget must remain advisory")
        };
        let tokens = budget
            .iter()
            .find(|assessment| assessment.dimension == BudgetDimension::Tokens)
            .expect("token assessment");
        assert_eq!(tokens.metering, MeteringMode::Estimated);
        assert!(!tokens.exhausted);
    }

    #[test]
    fn metering_confidence_is_downgraded_per_missing_dimension() {
        let value = policy();
        let mut current = state();
        current.usage.metering = MeteringMode::Trusted;
        current.usage.tool_calls = Some(5);
        current.usage.tokens = None;
        current.usage.credits_micros = Some(10_000);

        let decision = resolve_transition(
            &value,
            &current,
            &TransitionRequest::Retry {
                trigger: RetryTrigger::TransientToolFailure,
            },
        );
        let TransitionDecision::Allowed { budget, .. } = decision else {
            panic!("mixed observations below their limits must be allowed")
        };
        let assessment = |dimension| {
            budget
                .iter()
                .find(|assessment| assessment.dimension == dimension)
                .expect("configured budget assessment")
        };
        assert_eq!(
            assessment(BudgetDimension::ToolCalls).metering,
            MeteringMode::Trusted
        );
        assert_eq!(
            assessment(BudgetDimension::Tokens).metering,
            MeteringMode::Unavailable
        );
        assert_eq!(assessment(BudgetDimension::Tokens).observed, None);
        assert_eq!(
            assessment(BudgetDimension::Credits).metering,
            MeteringMode::Trusted
        );

        current.usage.metering = MeteringMode::Estimated;
        let TransitionDecision::Allowed { budget, .. } = resolve_transition(
            &value,
            &current,
            &TransitionRequest::Retry {
                trigger: RetryTrigger::TransientToolFailure,
            },
        ) else {
            panic!("estimated observations below their limits must be allowed")
        };
        assert_eq!(
            budget
                .iter()
                .find(|assessment| assessment.dimension == BudgetDimension::ToolCalls)
                .expect("tool-call assessment")
                .metering,
            MeteringMode::Estimated
        );
        assert_eq!(
            budget
                .iter()
                .find(|assessment| assessment.dimension == BudgetDimension::Tokens)
                .expect("token assessment")
                .metering,
            MeteringMode::Unavailable
        );
    }

    #[test]
    fn materiality_matrix_is_deterministic_and_trivial_changes_stay_cheap() {
        let mut value = policy();
        for trigger in [
            MaterialityTrigger::SecurityOrAuth,
            MaterialityTrigger::SecretsOrCrypto,
            MaterialityTrigger::SchemaOrMigration,
            MaterialityTrigger::InfrastructureOrDeploy,
            MaterialityTrigger::PublicApi,
            MaterialityTrigger::Billing,
            MaterialityTrigger::ConcurrencyOrTransaction,
        ] {
            let decision = classify_materiality(
                &value.materiality,
                &ChangeSummary {
                    risk: RiskLevel::Low,
                    triggers: BTreeSet::from([trigger]),
                    changed_files: 1,
                    changed_lines: 1,
                    kind: ChangeKind::Code,
                },
            );
            assert!(decision.material);
            assert_eq!(decision.review, ReviewRequirement::IndependentHighSignal);
            assert_eq!(decision.assurance_depth, AssuranceDepth::Expanded);
        }

        let same_trigger = MaterialityTrigger::SecurityOrAuth;
        value.materiality.protected_risks.remove(&same_trigger);
        let unconfigured = classify_materiality(
            &value.materiality,
            &ChangeSummary {
                risk: RiskLevel::Critical,
                triggers: BTreeSet::from([same_trigger]),
                changed_files: 1,
                changed_lines: 1,
                kind: ChangeKind::Code,
            },
        );
        assert!(!unconfigured.material);
        assert_eq!(unconfigured.review, ReviewRequirement::None);
        assert_eq!(unconfigured.assurance_depth, AssuranceDepth::Expanded);

        let size_only = classify_materiality(
            &value.materiality,
            &ChangeSummary {
                risk: RiskLevel::Low,
                triggers: BTreeSet::from([MaterialityTrigger::LargeDependencyChange]),
                changed_files: 100,
                changed_lines: 10_000,
                kind: ChangeKind::Code,
            },
        );
        assert!(!size_only.material);
        assert_eq!(size_only.review, ReviewRequirement::None);
        assert_eq!(size_only.assurance_depth, AssuranceDepth::Expanded);

        for kind in [
            ChangeKind::Documentation,
            ChangeKind::Formatting,
            ChangeKind::TestsOnly,
        ] {
            let decision = classify_materiality(
                &value.materiality,
                &ChangeSummary {
                    risk: RiskLevel::Low,
                    triggers: BTreeSet::new(),
                    changed_files: 1,
                    changed_lines: 4,
                    kind,
                },
            );
            assert!(!decision.material);
            assert_eq!(decision.review, ReviewRequirement::None);
            assert_eq!(decision.assurance_depth, AssuranceDepth::Standard);
        }
    }

    #[test]
    fn review_interrupt_requires_structured_allowed_provenance() {
        let allowed = ReviewInterruptRequest::StructuredEscalation {
            escalation: ReviewEscalation {
                reason: ReviewEscalationReason::DataIntegrityRisk,
                source: EscalationSource::MakerFinding,
                reference: "finding-17".into(),
                explanation: "transaction invariant may be violated".into(),
            },
        };
        assert!(matches!(
            admit_review_interrupt(&allowed),
            ReviewInterruptDecision::OpenCheckpoint { .. }
        ));

        let no_reference = ReviewInterruptRequest::StructuredEscalation {
            escalation: ReviewEscalation {
                reason: ReviewEscalationReason::UserRequested,
                source: EscalationSource::User,
                reference: " ".into(),
                explanation: "requested explicitly".into(),
            },
        };
        assert_eq!(
            admit_review_interrupt(&no_reference),
            ReviewInterruptDecision::Rejected {
                reason: ReviewInterruptRejectionReason::MissingReference
            }
        );
    }

    #[test]
    fn operational_gaps_and_change_size_cannot_masquerade_as_escalation() {
        for reason in [
            OperationalGapReason::MissingEvidence,
            OperationalGapReason::VerifierFailure,
            OperationalGapReason::AdapterDrift,
            OperationalGapReason::SandboxRestriction,
            OperationalGapReason::Uncertainty,
        ] {
            let decision =
                admit_review_interrupt(&ReviewInterruptRequest::OperationalGap { reason });
            assert_eq!(
                decision,
                ReviewInterruptDecision::RejectedOperationalGap { gap: reason }
            );
            assert_eq!(
                serde_json::to_value(&decision).expect("gap decision wire shape"),
                serde_json::json!({
                    "status": "rejected_operational_gap",
                    "gap": reason,
                })
            );
        }
        assert_eq!(
            admit_review_interrupt(&ReviewInterruptRequest::ChangeSize {
                changed_files: u32::MAX,
                changed_lines: u32::MAX,
            }),
            ReviewInterruptDecision::ContinueWithExpandedAssurance
        );
    }

    #[test]
    fn bounded_feature_run_contract_is_complete_sealed_and_tamper_evident() {
        let contract = v2_bounded_contract();
        assert_eq!(contract.mode, FeatureRunBudgetMode::Bounded);
        assert_eq!(contract.limits, Some(v2_budget_limits()));
        assert_eq!(contract.phase_reserves, Some(v2_phase_reserves()));
        assert!(validate_feature_run_budget_contract(&contract).is_empty());
        assert_eq!(
            contract.digest,
            feature_run_budget_contract_digest(&contract).expect("canonical digest")
        );

        let mut tampered = contract;
        tampered.run_id = "run-tampered".to_string();
        let fields = validate_feature_run_budget_contract(&tampered)
            .into_iter()
            .filter_map(|diagnostic| diagnostic.field)
            .collect::<Vec<_>>();
        assert_eq!(fields, vec!["budget_contract.digest"]);
    }

    #[test]
    fn unbounded_feature_run_contract_is_explicit_and_contains_no_numeric_budget() {
        let contract = FeatureRunBudgetContract::unbounded(
            "run-unbounded",
            1_700_000_000_000,
            v2_budget_provenance(),
        )
        .expect("valid explicit unbounded contract");
        assert!(validate_feature_run_budget_contract(&contract).is_empty());
        let value = serde_json::to_value(&contract).expect("contract serializes");
        assert_eq!(value["mode"], "unbounded");
        assert!(value["limits"].is_null());
        assert!(value["phase_reserves"].is_null());

        let snapshot = budget_snapshot(
            &contract,
            FeatureRunBudgetPhase::Maker,
            BudgetAmounts::ZERO,
            BudgetAmounts::ZERO,
            v2_budget_provenance(),
            None,
        )
        .expect("unbounded projection");
        assert_eq!(snapshot.remaining, None);
        assert_eq!(snapshot.protected, BudgetAmounts::ZERO);
        assert_eq!(snapshot.available, None);
    }

    #[test]
    fn authored_policy_becomes_one_exact_feature_run_contract_snapshot() {
        let policy = policy();
        let contract = feature_run_budget_contract_from_policy(
            "run-policy-snapshot",
            1_700_000_000_000,
            Some(&policy),
        )
        .expect("complete bounded policy");
        assert_eq!(contract.mode, FeatureRunBudgetMode::Bounded);
        assert_eq!(
            contract.limits,
            Some(BudgetAmounts {
                wall_seconds: 600,
                tool_calls: 100,
                tokens: 10_000,
            })
        );
        let reserves = contract.phase_reserves.expect("bounded reserves");
        assert_eq!(
            reserves.total().expect("exact total"),
            contract.limits.unwrap()
        );
        assert_eq!(reserves.release, BudgetAmounts::ZERO);
        assert_eq!(
            contract.metering_requirements,
            BudgetProvenance {
                wall_seconds: MeteringProvenance::Trusted,
                tool_calls: MeteringProvenance::Trusted,
                tokens: MeteringProvenance::Trusted,
            }
        );
    }

    #[test]
    fn missing_policy_is_explicitly_unbounded_but_partial_limits_are_rejected() {
        let contract = feature_run_budget_contract_from_policy(
            "run-unbounded-policy",
            1_700_000_000_000,
            None,
        )
        .expect("missing authored policy maps to explicit unbounded mode");
        assert_eq!(contract.mode, FeatureRunBudgetMode::Unbounded);
        assert!(contract.limits.is_none());

        let mut partial = policy();
        partial.usage.max_tokens = None;
        let diagnostics = feature_run_budget_contract_from_policy(
            "run-partial-policy",
            1_700_000_000_000,
            Some(&partial),
        )
        .expect_err("partial bounded policy must fail closed");
        assert_eq!(diagnostics.diagnostics[0].field.as_deref(), Some("usage"));
    }

    #[test]
    fn bounded_contract_validation_rejects_incomplete_or_overallocated_dimensions() {
        let mut contract = v2_bounded_contract();
        contract
            .phase_reserves
            .as_mut()
            .expect("bounded reserves")
            .release
            .tokens += 1;
        contract.digest = feature_run_budget_contract_digest(&contract).expect("reseal fixture");
        let fields = validate_feature_run_budget_contract(&contract)
            .into_iter()
            .filter_map(|diagnostic| diagnostic.field)
            .collect::<Vec<_>>();
        assert_eq!(fields, vec!["budget_contract.phase_reserves.tokens"]);

        contract.mode = FeatureRunBudgetMode::Unbounded;
        contract.digest = feature_run_budget_contract_digest(&contract).expect("reseal fixture");
        let fields = validate_feature_run_budget_contract(&contract)
            .into_iter()
            .filter_map(|diagnostic| diagnostic.field)
            .collect::<Vec<_>>();
        assert_eq!(
            fields,
            vec!["budget_contract.limits", "budget_contract.phase_reserves"]
        );
    }

    #[test]
    fn execution_budget_requires_all_maxima_and_uses_an_absolute_checked_deadline() {
        let budget = ExecutionBudget::new(
            1_700_000_000_000,
            BudgetAmounts {
                wall_seconds: 30,
                tool_calls: 12,
                tokens: 4_000,
            },
        )
        .expect("valid execution budget");
        assert_eq!(budget.deadline_unix_ms, 1_700_000_030_000);
        assert!(validate_execution_budget(&budget).is_empty());
        assert_eq!(
            ExecutionBudget::new(
                1_700_000_000_000,
                BudgetAmounts {
                    wall_seconds: 30,
                    tool_calls: 0,
                    tokens: 4_000,
                }
            ),
            Err(BudgetArithmeticError::ZeroMaximum(
                BudgetContractDimension::ToolCalls
            ))
        );
        assert_eq!(
            deadline_unix_ms(u64::MAX - 1_000, 2),
            Err(BudgetArithmeticError::DeadlineOverflow)
        );
        assert_eq!(
            deadline_unix_ms(0, 1),
            Err(BudgetArithmeticError::InvalidClockAnchor)
        );
        assert_eq!(
            deadline_unix_ms(1, u64::MAX),
            Err(BudgetArithmeticError::DeadlineOverflow)
        );
    }

    #[test]
    fn budget_snapshot_protects_every_unreleased_later_phase() {
        let contract = v2_bounded_contract();
        let execution_budget = ExecutionBudget::new(
            1_700_000_000_000,
            BudgetAmounts {
                wall_seconds: 20,
                tool_calls: 10,
                tokens: 200,
            },
        )
        .expect("task budget");
        let snapshot = budget_snapshot(
            &contract,
            FeatureRunBudgetPhase::Maker,
            BudgetAmounts {
                wall_seconds: 20,
                tool_calls: 10,
                tokens: 200,
            },
            BudgetAmounts {
                wall_seconds: 10,
                tool_calls: 5,
                tokens: 100,
            },
            v2_budget_provenance(),
            Some(execution_budget),
        )
        .expect("bounded snapshot");
        assert_eq!(
            snapshot.remaining,
            Some(BudgetAmounts {
                wall_seconds: 80,
                tool_calls: 40,
                tokens: 800,
            })
        );
        assert_eq!(
            snapshot.protected,
            BudgetAmounts {
                wall_seconds: 50,
                tool_calls: 25,
                tokens: 500,
            }
        );
        assert_eq!(
            snapshot.available,
            Some(BudgetAmounts {
                wall_seconds: 20,
                tool_calls: 10,
                tokens: 200,
            })
        );
        assert_eq!(
            snapshot.task_deadline_unix_ms,
            Some(execution_budget.deadline_unix_ms)
        );
        assert_eq!(snapshot.contract_digest, contract.digest);
    }

    #[test]
    fn reserve_addition_reports_overflow_instead_of_wrapping() {
        let reserves = FeatureRunPhaseReserves {
            maker: BudgetAmounts {
                wall_seconds: u64::MAX,
                tool_calls: 1,
                tokens: 1,
            },
            verification: BudgetAmounts {
                wall_seconds: 1,
                tool_calls: 0,
                tokens: 0,
            },
            review: BudgetAmounts::ZERO,
            repair: BudgetAmounts::ZERO,
            release: BudgetAmounts::ZERO,
        };
        assert_eq!(
            reserves.total(),
            Err(BudgetArithmeticError::ReserveOverflow(
                BudgetContractDimension::WallSeconds
            ))
        );
    }

    #[test]
    fn phase_budget_limits_release_only_the_current_and_prior_reserves() {
        let reserves = PhaseBudgetReserves {
            verification_percent: 20,
            review_percent: 10,
            repair_percent: 10,
        };
        assert_eq!(
            phase_spend_limit(1_000, reserves, BudgetPhase::Implementation),
            600
        );
        assert_eq!(
            phase_spend_limit(1_000, reserves, BudgetPhase::Verification),
            800
        );
        assert_eq!(phase_spend_limit(1_000, reserves, BudgetPhase::Review), 900);
        assert_eq!(
            phase_spend_limit(1_000, reserves, BudgetPhase::Repair),
            1_000
        );
        assert_eq!(
            phase_spend_limit(u64::MAX, reserves, BudgetPhase::Implementation),
            (u64::MAX / 5) * 3 + ((u64::MAX % 5) * 3) / 5
        );
    }

    #[test]
    fn transition_limits_hold_for_values_around_each_boundary() {
        let value = policy();
        for count in 0..=3 {
            let mut current = state();
            current.same_route_retries = count;
            assert_eq!(
                resolve_transition(
                    &value,
                    &current,
                    &TransitionRequest::Retry {
                        trigger: RetryTrigger::Timeout,
                    }
                )
                .is_allowed(),
                count < value.transitions.retry.max_same_route_retries
            );

            current = state();
            current.availability_fallbacks = count;
            assert_eq!(
                resolve_transition(
                    &value,
                    &current,
                    &TransitionRequest::AvailabilityFallback {
                        trigger: AvailabilityTrigger::HostUnavailable,
                        same_capability_class: true,
                    }
                )
                .is_allowed(),
                count < value.transitions.availability_fallback.max_fallbacks
            );

            current = state();
            current.quality_escalations = count;
            assert_eq!(
                resolve_transition(
                    &value,
                    &current,
                    &TransitionRequest::QualityEscalation {
                        trigger: QualityTrigger::VerificationFailed,
                        verification_evidence: true,
                    }
                )
                .is_allowed(),
                count < value.transitions.quality_escalation.max_escalations
            );
        }
    }

    #[test]
    fn task_contract_rejects_unbounded_or_empty_contracts() {
        let contract = TaskContract {
            objective: " ".to_string(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            read_scope: Vec::new(),
            write_scope: Vec::new(),
            acceptance_criteria: Vec::new(),
            verification: Vec::new(),
            evidence_requirements: Vec::new(),
            max_attempts: 0,
            stop_conditions: Vec::new(),
            risk: RiskLevel::Low,
            materiality_triggers: BTreeSet::new(),
            context: vec!["too large".to_string()],
            max_context_bytes: 2,
        };
        let fields: Vec<_> = validate_task_contract(&contract)
            .into_iter()
            .filter_map(|diagnostic| diagnostic.field)
            .collect();
        assert_eq!(
            fields,
            vec![
                "task.objective",
                "task.inputs",
                "task.outputs",
                "task.acceptance_criteria",
                "task.verification",
                "task.evidence_requirements",
                "task.stop_conditions",
                "task.max_attempts",
                "task.context",
            ]
        );
    }
}
