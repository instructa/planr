//! Durable three-stage route observations for one recorded run.
//!
//! Requested, host-resolved, and effective values remain independent. In
//! particular, a requested value is never copied into the effective stage as
//! proof when the host cannot report what actually executed.

use crate::usage_policy::MeteringMode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteObservation {
    pub requested: RouteStage,
    pub resolved: RouteStage,
    pub effective: RouteStage,
    pub transition: RouteTransition,
    pub policy: VersionReference,
    pub binding: VersionReference,
    pub metering: RouteMetering,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteStage {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub client: Option<String>,
    pub agent_type: StringDimension,
    pub model: StringDimension,
    pub effort: StringDimension,
    pub context_fork: ForkDimension,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StringDimension {
    #[serde(default)]
    pub value: Option<String>,
    pub enforcement: EnforcementState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceSource>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForkDimension {
    #[serde(default)]
    pub value: Option<ContextForkMode>,
    pub enforcement: EnforcementState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceSource>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ContextForkMode {
    None,
    Partial { turns: u32 },
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementState {
    Verified,
    RequestedOnly,
    Estimated,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    Policy,
    Binding,
    HostReport,
    TelemetryReceipt,
    ProcessExit,
    LocalObservation,
    UserReported,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteTransition {
    pub kind: RouteTransitionKind,
    pub reason: String,
    #[serde(default)]
    pub evidence: Vec<EvidenceSource>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteTransitionKind {
    Initial,
    Retry,
    AvailabilityFallback,
    QualityEscalation,
    QuotaDowngrade,
    SafetyStop,
}

impl RouteTransitionKind {
    pub const fn event_type(self) -> &'static str {
        match self {
            Self::Initial => "route_initial_resolved",
            Self::Retry => "route_retry_scheduled",
            Self::AvailabilityFallback => "route_availability_fallback",
            Self::QualityEscalation => "route_quality_escalated",
            Self::QuotaDowngrade => "route_quota_downgraded",
            Self::SafetyStop => "route_safety_stopped",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionReference {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteMetering {
    pub wall_time_seconds: MeteredDimension,
    pub tool_calls: MeteredDimension,
    pub tokens: MeteredDimension,
    pub credits_micros: MeteredDimension,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeteredDimension {
    #[serde(default)]
    pub value: Option<u64>,
    pub confidence: MeteringMode,
}

pub fn load_route_observation(path: &Path) -> anyhow::Result<RouteObservation> {
    let observation: RouteObservation = serde_json::from_str(&std::fs::read_to_string(path)?)
        .map_err(|error| anyhow::anyhow!("route audit parse failed: {error}"))?;
    validate_route_observation(&observation)
        .map_err(|error| anyhow::anyhow!("route audit validation failed: {error}"))?;
    Ok(observation)
}

pub fn parse_route_observation(value: serde_json::Value) -> anyhow::Result<RouteObservation> {
    let observation: RouteObservation = serde_json::from_value(value)
        .map_err(|error| anyhow::anyhow!("route audit parse failed: {error}"))?;
    validate_route_observation(&observation)
        .map_err(|error| anyhow::anyhow!("route audit validation failed: {error}"))?;
    Ok(observation)
}

pub fn validate_route_observation(observation: &RouteObservation) -> Result<(), String> {
    validate_reference("policy", &observation.policy)?;
    validate_reference("binding", &observation.binding)?;
    if observation.transition.reason.trim().is_empty() {
        return Err("transition.reason must not be empty".to_string());
    }
    if observation.transition.evidence.is_empty() {
        return Err("transition.evidence must name at least one evidence source".to_string());
    }
    validate_stage(RouteStageKind::Requested, &observation.requested)?;
    validate_stage(RouteStageKind::Resolved, &observation.resolved)?;
    validate_stage(RouteStageKind::Effective, &observation.effective)?;
    if observation.requested.client.as_deref() == Some("codex")
        && observation.requested.agent_type.value.is_none()
    {
        return Err("requested.agent_type is required for Codex routing".to_string());
    }
    for (field, dimension) in [
        (
            "metering.wall_time_seconds",
            &observation.metering.wall_time_seconds,
        ),
        ("metering.tool_calls", &observation.metering.tool_calls),
        ("metering.tokens", &observation.metering.tokens),
        (
            "metering.credits_micros",
            &observation.metering.credits_micros,
        ),
    ] {
        match (dimension.value, dimension.confidence) {
            (None, MeteringMode::Unavailable) => {}
            (None, _) => {
                return Err(format!(
                    "{field} has no value and must have unavailable confidence"
                ));
            }
            (Some(_), MeteringMode::Unavailable) => {
                return Err(format!(
                    "{field} has a value but has unavailable confidence"
                ));
            }
            (Some(_), _) => {}
        }
    }
    Ok(())
}

impl crate::app::App {
    pub(crate) fn record_route_observation_events(
        &self,
        item_id: &str,
        run_id: &str,
        observation: &RouteObservation,
    ) -> anyhow::Result<()> {
        let provenance = json!({
            "run_id": run_id,
            "policy": &observation.policy,
            "binding": &observation.binding,
            "metering": &observation.metering,
        });
        for (event_type, stage) in [
            ("route_requested", &observation.requested),
            ("route_resolved", &observation.resolved),
            ("route_effective_observed", &observation.effective),
        ] {
            self.record_event(
                event_type,
                Some(item_id),
                json!({"route": stage, "provenance": &provenance}),
            )?;
        }
        self.record_event(
            observation.transition.kind.event_type(),
            Some(item_id),
            json!({
                "transition": &observation.transition,
                "provenance": provenance,
            }),
        )
    }
}

fn validate_reference(field: &str, reference: &VersionReference) -> Result<(), String> {
    if reference.id.trim().is_empty() || reference.version.trim().is_empty() {
        return Err(format!("{field}.id and {field}.version must not be empty"));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum RouteStageKind {
    Requested,
    Resolved,
    Effective,
}

impl RouteStageKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Resolved => "resolved",
            Self::Effective => "effective",
        }
    }
}

fn validate_stage(kind: RouteStageKind, stage: &RouteStage) -> Result<(), String> {
    let field = kind.name();
    for (name, value) in [
        ("role", stage.role.as_deref()),
        ("profile", stage.profile.as_deref()),
        ("client", stage.client.as_deref()),
        ("thread_id", stage.thread_id.as_deref()),
        ("status", stage.status.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(format!("{field}.{name} must not be blank"));
        }
    }
    validate_dimension(
        &format!("{field}.agent_type"),
        stage.agent_type.value.as_deref(),
        stage.agent_type.enforcement,
        stage.agent_type.evidence,
        kind,
    )?;
    validate_dimension(
        &format!("{field}.model"),
        stage.model.value.as_deref(),
        stage.model.enforcement,
        stage.model.evidence,
        kind,
    )?;
    validate_dimension(
        &format!("{field}.effort"),
        stage.effort.value.as_deref(),
        stage.effort.enforcement,
        stage.effort.evidence,
        kind,
    )?;
    if matches!(
        stage.context_fork.value,
        Some(ContextForkMode::Partial { turns: 0 })
    ) {
        return Err(format!(
            "{field}.context_fork partial turns must be positive"
        ));
    }
    validate_dimension(
        &format!("{field}.context_fork"),
        stage.context_fork.value.as_ref().map(|_| "present"),
        stage.context_fork.enforcement,
        stage.context_fork.evidence,
        kind,
    )
}

/// Whether the single durable route observation proves that the effective
/// native route matched what Planr requested. Missing effective dimensions do
/// not make the observation unparsable, but they can never verify a route.
pub(crate) fn effective_route_matches_requested(observation: &RouteObservation) -> bool {
    let requested = &observation.requested;
    let resolved = &observation.resolved;
    let effective = &observation.effective;
    let requested_values_resolved = requested.role == resolved.role
        && requested.profile == resolved.profile
        && requested.client == resolved.client
        && requested.agent_type.value == resolved.agent_type.value
        && requested.model.value == resolved.model.value
        && requested.effort.value == resolved.effort.value
        && requested.context_fork.value == resolved.context_fork.value;
    let effective_role_matches = effective.role.as_ref().is_none_or(|role| {
        requested.role.as_ref() == Some(role) || requested.agent_type.value.as_ref() == Some(role)
    });
    let effective_values_match = requested.model.value == effective.model.value
        && requested.effort.value == effective.effort.value
        && requested.context_fork.value == effective.context_fork.value
        && requested.agent_type.value == effective.agent_type.value
        && effective_role_matches;
    requested_values_resolved && effective_values_match
}

fn validate_dimension(
    field: &str,
    value: Option<&str>,
    enforcement: EnforcementState,
    evidence: Option<EvidenceSource>,
    stage: RouteStageKind,
) -> Result<(), String> {
    if value.is_some_and(|value| value.trim().is_empty()) {
        return Err(format!("{field}.value must not be blank"));
    }
    match (value.is_some(), enforcement) {
        (false, EnforcementState::Unavailable) => {}
        (false, _) => return Err(format!("{field} has no value and must be unavailable")),
        (true, EnforcementState::Unavailable) => {
            return Err(format!("{field} has a value but is marked unavailable"));
        }
        _ => {}
    }
    if matches!(stage, RouteStageKind::Effective) && enforcement == EnforcementState::RequestedOnly
    {
        return Err(format!(
            "{field} cannot use requested_only as effective execution proof"
        ));
    }
    match (stage, enforcement, evidence) {
        (
            RouteStageKind::Effective,
            EnforcementState::Verified,
            Some(
                EvidenceSource::HostReport
                | EvidenceSource::TelemetryReceipt
                | EvidenceSource::ProcessExit
                | EvidenceSource::LocalObservation,
            ),
        ) => {}
        (RouteStageKind::Effective, EnforcementState::Verified, _) => {
            return Err(format!(
                "{field} is verified effective execution but lacks host_report, telemetry_receipt, process_exit, or local_observation evidence"
            ));
        }
        (_, EnforcementState::Verified | EnforcementState::Estimated, None) => {
            return Err(
                format!("{field} is {enforcement:?} but has no evidence source").to_lowercase(),
            );
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(value: Option<&str>, enforcement: EnforcementState) -> StringDimension {
        StringDimension {
            value: value.map(ToOwned::to_owned),
            enforcement,
            evidence: (enforcement == EnforcementState::Verified)
                .then_some(EvidenceSource::HostReport),
        }
    }

    fn stage(enforcement: EnforcementState) -> RouteStage {
        RouteStage {
            role: Some("worker".to_string()),
            profile: Some("coder".to_string()),
            client: Some("codex".to_string()),
            agent_type: field(Some("planr-terra-high"), enforcement),
            model: field(Some("gpt-5.6-terra"), enforcement),
            effort: field(Some("high"), enforcement),
            context_fork: ForkDimension {
                value: Some(ContextForkMode::None),
                enforcement,
                evidence: (enforcement == EnforcementState::Verified)
                    .then_some(EvidenceSource::HostReport),
            },
            thread_id: None,
            status: None,
        }
    }

    fn fixture() -> RouteObservation {
        RouteObservation {
            requested: stage(EnforcementState::RequestedOnly),
            resolved: stage(EnforcementState::Verified),
            effective: stage(EnforcementState::Verified),
            transition: RouteTransition {
                kind: RouteTransitionKind::Initial,
                reason: "initial policy route".to_string(),
                evidence: vec![EvidenceSource::Policy],
            },
            policy: VersionReference {
                id: "balanced".to_string(),
                version: "1.0.0".to_string(),
            },
            binding: VersionReference {
                id: "codex-openai".to_string(),
                version: "2.0.0".to_string(),
            },
            metering: RouteMetering {
                wall_time_seconds: MeteredDimension {
                    value: Some(4),
                    confidence: MeteringMode::Trusted,
                },
                tool_calls: MeteredDimension {
                    value: Some(2),
                    confidence: MeteringMode::Trusted,
                },
                tokens: MeteredDimension {
                    value: Some(100),
                    confidence: MeteringMode::Trusted,
                },
                credits_micros: MeteredDimension {
                    value: None,
                    confidence: MeteringMode::Unavailable,
                },
            },
        }
    }

    #[test]
    fn verified_observation_round_trips() {
        let observation = fixture();
        validate_route_observation(&observation).unwrap();
        assert!(effective_route_matches_requested(&observation));
        let value = serde_json::to_value(&observation).unwrap();
        assert_eq!(parse_route_observation(value).unwrap(), observation);
    }

    #[test]
    fn codex_route_verification_requires_requested_and_effective_values_to_match() {
        let mut observation = fixture();
        observation.effective.model.value = Some("gpt-5.6-sol".to_string());
        assert!(!effective_route_matches_requested(&observation));

        let mut observation = fixture();
        observation.effective.effort = field(None, EnforcementState::Unavailable);
        assert!(!effective_route_matches_requested(&observation));

        let mut observation = fixture();
        observation.effective.agent_type.value = Some("planr-sol-high".to_string());
        assert!(!effective_route_matches_requested(&observation));

        let mut observation = fixture();
        observation.effective.agent_type = field(None, EnforcementState::Unavailable);
        assert!(!effective_route_matches_requested(&observation));

        let mut observation = fixture();
        observation.effective.role = Some("planr-sol-high".to_string());
        assert!(!effective_route_matches_requested(&observation));

        let mut observation = fixture();
        observation.requested.agent_type = field(None, EnforcementState::Unavailable);
        assert_eq!(
            validate_route_observation(&observation).unwrap_err(),
            "requested.agent_type is required for Codex routing"
        );
    }

    #[test]
    fn unknown_effective_dimensions_remain_explicitly_unavailable() {
        let mut observation = fixture();
        observation.effective.model = field(None, EnforcementState::Unavailable);
        observation.effective.effort = field(None, EnforcementState::Unavailable);
        validate_route_observation(&observation).unwrap();
    }

    #[test]
    fn requested_values_cannot_masquerade_as_effective_proof() {
        let mut observation = fixture();
        observation.effective.model = field(Some("gpt-5.6-terra"), EnforcementState::RequestedOnly);
        assert!(
            validate_route_observation(&observation)
                .unwrap_err()
                .contains("cannot use requested_only")
        );
    }

    #[test]
    fn effective_evidence_matrix_rejects_disguised_requested_values() {
        for evidence in [
            EvidenceSource::Policy,
            EvidenceSource::Binding,
            EvidenceSource::UserReported,
        ] {
            let mut observation = fixture();
            observation.effective.model.evidence = Some(evidence);
            assert!(
                validate_route_observation(&observation)
                    .unwrap_err()
                    .contains(
                        "lacks host_report, telemetry_receipt, process_exit, or local_observation evidence"
                    )
            );
        }

        for evidence in [
            EvidenceSource::HostReport,
            EvidenceSource::TelemetryReceipt,
            EvidenceSource::ProcessExit,
            EvidenceSource::LocalObservation,
        ] {
            let mut observation = fixture();
            observation.effective.model.evidence = Some(evidence);
            validate_route_observation(&observation).unwrap();
        }

        let mut observation = fixture();
        observation.effective.model = field(Some("gpt-5.6-terra"), EnforcementState::Estimated);
        assert!(
            validate_route_observation(&observation)
                .unwrap_err()
                .contains("is estimated but has no evidence source")
        );

        observation.effective.model.evidence = Some(EvidenceSource::Policy);
        validate_route_observation(&observation).unwrap();
    }

    #[test]
    fn values_and_evidence_states_fail_closed() {
        let mut observation = fixture();
        observation.effective.model = field(None, EnforcementState::Verified);
        assert!(validate_route_observation(&observation).is_err());

        let mut observation = fixture();
        observation.effective.model.evidence = None;
        assert!(validate_route_observation(&observation).is_err());

        let mut observation = fixture();
        observation.effective.context_fork.value = Some(ContextForkMode::Partial { turns: 0 });
        assert!(validate_route_observation(&observation).is_err());

        let mut observation = fixture();
        observation.metering.tokens.value = None;
        assert!(validate_route_observation(&observation).is_err());
    }
}
