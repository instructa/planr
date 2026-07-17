const IDENTIFIER = /^[A-Za-z0-9._-]+$/;

export function safeIdentifier(value, label = "identifier") {
  if (typeof value !== "string" || !IDENTIFIER.test(value)) {
    throw new Error(`${label} is not a safe registry identifier`);
  }
  return value;
}

export function previewCommand(policyId, bindingId) {
  return `planr-routing compile ${safeIdentifier(policyId, "policy id")} --host ${safeIdentifier(bindingId, "binding id")} --output routing-bundle.json && planr routing bundle preview routing-bundle.json`;
}

export function statusLabel(status) {
  const labels = {
    experimental: "Experimental",
    verified: "Verified",
    recommended: "Recommended",
    stale: "Stale",
    deprecated: "Deprecated",
  };
  return labels[status] ?? "Unverified";
}

export function formatBasisPoints(value) {
  const basisPoints = Number(value);
  return Number.isFinite(basisPoints) ? `${(basisPoints / 100).toFixed(2)}%` : "—";
}

function findArtifact(entry, kind) {
  const artifacts = entry.artifacts.filter((artifact) => artifact.kind === kind);
  if (artifacts.length !== 1) {
    throw new Error(`registry entry must contain exactly one ${kind} artifact`);
  }
  return artifacts[0];
}

function proposedConfig(preview, kind) {
  const artifact = preview.artifacts.find((candidate) => candidate.kind === kind);
  const value = artifact?.config_diff?.proposed?.value;
  if (!value || typeof value !== "object") {
    throw new Error(`preset preview is missing proposed ${kind} configuration`);
  }
  return value;
}

function matchingCandidate(report, policyId, bindingId) {
  const candidate = report.candidates?.find(
    (value) => value.policy?.id === policyId && value.binding?.id === bindingId,
  );
  if (!candidate) {
    throw new Error(`evaluation report has no candidate for ${policyId} + ${bindingId}`);
  }
  return candidate;
}

function recommendationMatches(report, policyId, bindingId) {
  return Boolean(
    report.recommended?.some(
      (value) => value.policy === policyId && value.binding === bindingId && value.status === "recommended",
    ),
  );
}

export function projectComposition({ verified, preview, verificationEnvelope }) {
  if (!verified?.integrity_verified) {
    throw new Error("website projection requires an integrity-verified registry entry");
  }
  if (verified.recommended && (!verified.signature_verified || !verified.trusted_maintainer)) {
    throw new Error("website recommendation requires a trusted maintainer signature");
  }
  if (!verified.compatible) {
    throw new Error("website projection requires a compatible registry entry");
  }
  if (!preview?.pack?.safe) {
    throw new Error("website projection requires a canonical safe-pack preview");
  }

  const entry = verified.entry;
  const evaluation = entry.evaluation;
  if (!evaluation) {
    throw new Error("website projection requires evaluation binding metadata");
  }
  const policyId = safeIdentifier(evaluation.policy_id, "policy id");
  const bindingId = safeIdentifier(evaluation.binding_id, "binding id");
  const report = verificationEnvelope.report ?? verificationEnvelope;
  const candidate = matchingCandidate(report, policyId, bindingId);
  const reportRecommended = recommendationMatches(report, policyId, bindingId);
  if (verified.recommended && (!reportRecommended || candidate.status !== "recommended")) {
    throw new Error("registry recommendation does not match canonical evaluation evidence");
  }

  const policy = proposedConfig(preview, "active_policy");
  const agents = proposedConfig(preview, "agent_registry");
  if (policy.id !== policyId || preview.composition?.binding?.id !== bindingId) {
    throw new Error("preset preview provenance does not match registry evaluation binding");
  }
  const runs = Number(candidate.metrics?.runs ?? 0);
  const verifiedRoutes = Number(candidate.metrics?.verified_route_runs ?? 0);
  const policyArtifact = findArtifact(entry, "policy");
  const bindingArtifact = findArtifact(entry, "host-binding");
  const verificationArtifact = findArtifact(entry, "verification");

  return {
    id: `${safeIdentifier(entry.id, "entry id")}@${entry.version}`,
    entryId: entry.id,
    entryVersion: entry.version,
    status: verified.effective_status,
    statusLabel: statusLabel(verified.effective_status),
    recommended: verified.recommended,
    freshness: verified.freshness,
    lifecycle: entry.lifecycle,
    replacement: entry.replacement ?? null,
    policy: {
      id: policyId,
      version: evaluation.policy_version,
      usage: policy.usage,
      transitions: policy.transitions,
      materiality: policy.materiality,
      execution: policy.execution,
    },
    binding: {
      id: bindingId,
      version: evaluation.binding_version,
      host: preview.composition.host,
      profiles: agents.profiles,
      dispatch: preview.composition.dispatch,
    },
    compatibility: {
      hosts: entry.compatible_hosts,
      minPlanrVersion: entry.min_planr_version,
      maxPlanrVersion: entry.max_planr_version,
    },
    enforcement: [
      {
        dimension: "Policy limits",
        state: "verified",
        detail: "Planr validates count, time, concurrency, transition, and safety-stop limits before dispatch.",
      },
      {
        dimension: "Execution permissions",
        state: "verified",
        detail: "Registry-safe policy: no commands, hooks, network/MCP grants, secrets, or overwrite permission.",
      },
      {
        dimension: "Model and effort",
        state: "host_enforced",
        detail: "The host binding requests concrete routes; the host retains final execution authority.",
      },
      {
        dimension: "Effective route evidence",
        state: runs > 0 && verifiedRoutes === runs ? "verified" : "unavailable",
        detail: `${verifiedRoutes} of ${runs} evaluation runs carried verified effective-route evidence.`,
      },
    ],
    evaluation: {
      suiteId: report.suite?.id,
      suiteVersion: report.suite?.version,
      evaluatedAtUnix: report.suite?.evaluated_at_unix,
      reviewAtUnix: entry.review_at_unix,
      status: candidate.status,
      metrics: candidate.metrics,
      thresholds: candidate.threshold_results,
      resultHashes: candidate.results?.map((result) => result.result_sha256) ?? [],
      fixtureSha256: report.suite?.fixture_sha256,
    },
    registry: {
      id: verified.registry_id,
      version: verified.registry_version,
      manifestSha256: verified.manifest_sha256,
      signer: entry.signature?.signer,
      signatureVerified: verified.signature_verified,
      trustedMaintainer: verified.trusted_maintainer,
      artifacts: [policyArtifact, bindingArtifact, verificationArtifact].map((artifact) => ({
        path: artifact.path,
        kind: artifact.kind,
        sha256: artifact.sha256,
        sizeBytes: artifact.size_bytes,
      })),
    },
    command: previewCommand(policyId, bindingId),
  };
}

export function visibleCompositions(catalog, recommendedOnly = false) {
  const compositions = Array.isArray(catalog?.compositions) ? catalog.compositions : [];
  return recommendedOnly ? compositions.filter((entry) => entry.recommended) : compositions;
}
