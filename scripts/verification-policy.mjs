import { createHash } from "node:crypto";

export const POLICY_VERSION = "1.1.0";

export const GATES = Object.freeze({
  "docs-content": "Validate generated docs content and links",
  "docs-typecheck": "Type-check the documentation application",
  "docs-lint": "Lint the documentation application",
  "docs-build": "Build the documentation application once",
  "docs-artifact": "Verify the existing documentation artifact",
  "rust-fmt": "Check Rust formatting",
  "rust-clippy": "Run strict Rust lints",
  "rust-test": "Run the Rust test suite",
  "generated-reference": "Check generated CLI and MCP reference pages",
  "github-actions": "Verify GitHub Actions contracts",
  "release-contract": "Verify release and packaging contracts",
  "linux-portability": "Verify portable Linux release artifacts",
  "release-evaluation": "Verify the synthetic release evaluation contract",
});

const PROFILES = deepFreeze({
  "focused-docs": ["docs-content", "docs-typecheck", "docs-lint", "docs-build", "docs-artifact"],
  docs: ["docs-content", "docs-typecheck", "docs-lint", "docs-build", "docs-artifact"],
  core: [
    "rust-fmt", "rust-clippy", "rust-test", "generated-reference",
  ],
  "release-critical": [
    "github-actions", "release-contract", "linux-portability",
    "release-evaluation",
  ],
});

const FULL_GATES = Object.freeze([...new Set(Object.values(PROFILES).flat())]);

export const POLICY_RULES = deepFreeze([
  {
    id: "policy",
    description: "Classifier, package graph, and policy fixtures",
    profile: "full",
    sensitive: true,
    detail: "The verification policy or command graph changed.",
    matchers: [
      { source: "^(scripts\\/(?:verification-(?:policy|runner)|classify-changes|test-verification-(?:policy|runner)|ci-router|test-ci-router)\\.mjs|scripts\\/fixtures\\/verification-policy\\/|package\\.json$|pnpm-workspace\\.yaml$)", flags: "u" },
    ],
  },
  {
    id: "lockfile",
    description: "Dependency lockfiles",
    profile: "full",
    sensitive: true,
    detail: "A dependency lockfile changed.",
    matchers: [{ source: "^(?:Cargo\\.lock|pnpm-lock\\.yaml)$", flags: "u" }],
  },
  {
    id: "generated-reference",
    description: "Generated reference output",
    profile: "full",
    sensitive: true,
    detail: "Generated reference output changed and must be checked against its owning source.",
    matchers: [{ source: "^apps\\/docs\\/content\\/docs\\/reference\\/(?:cli-generated|mcp-schemas-generated)\\.mdx$", flags: "u" }],
  },
  {
    id: "release-critical",
    description: "Workflows, release, packaging, and local security tooling",
    profile: "release-critical",
    sensitive: true,
    detail: "Release, workflow, packaging, contract, or security infrastructure changed.",
    matchers: [
      { source: "^(?:\\.github\\/|npm\\/|docs\\/contracts\\/|docs\\/RELEASE\\.md$|CHANGELOG\\.md$|scripts\\/(?:release|build-release|build-linux-release|prepare-release-candidate|verify-linux-release-artifact|verify-release|test-release|verify-github-actions|test-verify-github-actions|security-local|check-repository-privacy|install|generate-formula|verify-changelog-release-links))", flags: "u" },
    ],
  },
  {
    id: "rust",
    description: "Rust sources, manifests, and tests",
    profile: "core",
    sensitive: false,
    detail: "Rust product code, tests, or its manifest changed.",
    matchers: [{ source: "^(?:src\\/.*\\.rs|tests\\/.*\\.rs|Cargo\\.toml)$", flags: "u" }],
  },
  {
    id: "docs-interactive",
    description: "Interactive documentation application code",
    profile: "docs",
    sensitive: false,
    liveBrowser: true,
    detail: "Interactive documentation behavior changed; request one focused live browser oracle outside automatic CI.",
    matchers: [
      { source: "^apps/docs/(?:app|components)/.*[.](?:js|jsx|mjs|ts|tsx)$", flags: "u" },
    ],
  },
  {
    id: "docs-app",
    description: "Documentation application and verification code",
    profile: "docs",
    sensitive: false,
    detail: "Documentation application or verification code changed.",
    matchers: [
      { source: "^apps\\/docs\\/(?:app|components|lib|scripts|public)\\/", flags: "u" },
      { source: "^apps\\/docs\\/(?:package\\.json|next\\.config\\.mjs|eslint\\.config\\.mjs|tsconfig\\.json|source\\.config\\.ts|postcss\\.config\\.mjs|wrangler\\.jsonc|alchemy\\.run\\.ts)$", flags: "u" },
    ],
  },
  {
    id: "docs-content",
    description: "Documentation prose and static content",
    profile: "focused-docs",
    sensitive: false,
    detail: "Documentation content changed.",
    matchers: [{ source: "^(?:apps\\/docs\\/content\\/|docs\\/(?!contracts\\/)|README\\.md$|LICENSE\\.md$)", flags: "u" }],
  },
  {
    id: "unknown",
    description: "Paths without a policy owner",
    profile: "full",
    sensitive: true,
    detail: "No verification policy rule owns this path.",
    matchers: [],
  },
]);

const COMPILED_RULES = POLICY_RULES.map((rule) => ({
  rule,
  matchers: rule.matchers.map(({ source, flags }) => new RegExp(source, flags)),
}));

const POLICY_DOCUMENT = deepFreeze({
  schemaVersion: 1,
  policyVersion: POLICY_VERSION,
  profiles: PROFILES,
  rules: POLICY_RULES,
});

export const POLICY_DIGEST = digest(POLICY_DOCUMENT);

export function policyDigestForRules(rules) {
  return digest({
    schemaVersion: POLICY_DOCUMENT.schemaVersion,
    policyVersion: POLICY_VERSION,
    profiles: PROFILES,
    rules,
  });
}

const STATUS_ALIASES = Object.freeze({
  A: "added",
  added: "added",
  M: "modified",
  modified: "modified",
  D: "deleted",
  deleted: "deleted",
  T: "type_changed",
  type_changed: "type_changed",
  R: "renamed",
  renamed: "renamed",
  C: "copied",
  copied: "copied",
});

export function classifyChanges(input, { baseRevision = null, headRevision = null } = {}) {
  const normalized = normalizeChanges(input);
  const pathMatches = [];
  const escalationReasons = [...normalized.errors];

  for (const change of normalized.changes) {
    for (const path of change.paths) {
      const match = classifyPath(path);
      pathMatches.push({ path, status: change.status, ...match });
      if (match.profile === "full") {
        escalationReasons.push({ code: `${match.pathClass}_path`, path, detail: match.detail });
      }
    }
  }

  const matchedClasses = [...new Set(pathMatches.map((match) => match.pathClass))].sort();
  const sensitive = pathMatches.filter((match) => match.sensitive);
  const nonSensitive = pathMatches.filter((match) => !match.sensitive);
  if (sensitive.length > 0 && nonSensitive.length > 0) {
    escalationReasons.push({
      code: "sensitive_overlap",
      paths: [...new Set(pathMatches.map((match) => match.path))].sort(),
      detail: "Sensitive and non-sensitive path classes overlap; full verification is required.",
    });
  }

  const escalatedToFull = escalationReasons.length > 0;
  const profile = escalatedToFull ? "full" : selectProfile(pathMatches);
  const selectedGates = profile === "full" ? [...FULL_GATES] : gatesForMatches(pathMatches);
  const liveBrowserPaths = pathMatches.filter((match) => match.liveBrowser).map((match) => match.path);
  const reasons = selectedGates.map((gate) => {
    const owners = pathMatches.filter((match) => profile === "full" || PROFILES[match.profile]?.includes(gate));
    return {
      gate,
      code: profile === "full" ? "full_profile" : "path_class_match",
      detail: profile === "full"
        ? "Selected because fail-closed full verification is required."
        : GATES[gate],
      paths: [...new Set(owners.map((owner) => owner.path))].sort(),
    };
  });

  return {
    schemaVersion: 1,
    policyVersion: POLICY_VERSION,
    policyDigest: POLICY_DIGEST,
    baseRevision: safeRevision(baseRevision),
    headRevision: safeRevision(headRevision),
    changedFilesDigest: digest(normalized.changes),
    profile,
    escalatedToFull,
    escalationReasons: deduplicateReasons(escalationReasons),
    matchedPathClasses: matchedClasses,
    selectedGates,
    liveVerification: {
      browser: liveBrowserPaths.length > 0,
      paths: [...new Set(liveBrowserPaths)].sort(),
      detail: liveBrowserPaths.length > 0
        ? "Run one focused live browser oracle for the changed interaction; automatic CI remains browser-free."
        : "No browser oracle selected for text, Markdown, link, image, or non-interactive changes.",
    },
    reasons,
    changes: normalized.changes,
    pathMatches,
  };
}

function safeRevision(value) {
  return typeof value === "string" && /^[A-Za-z0-9._/@{}^~:+-]{1,200}$/u.test(value) ? value : null;
}

export function parseGitNameStatus(output) {
  if (typeof output !== "string") return [{ status: "ambiguous", path: "<invalid-git-diff>" }];
  const fields = output.split("\0");
  if (fields.at(-1) === "") fields.pop();
  const changes = [];
  for (let index = 0; index < fields.length;) {
    const rawStatus = fields[index++];
    const statusCode = rawStatus?.[0];
    if (statusCode === "R" || statusCode === "C") {
      changes.push({ status: statusCode, oldPath: fields[index++], newPath: fields[index++] });
    } else {
      changes.push({ status: rawStatus, path: fields[index++] });
    }
  }
  return changes;
}

function normalizeChanges(input) {
  if (!Array.isArray(input) || input.length === 0) {
    return {
      changes: [],
      errors: [{ code: "ambiguous_input", detail: "The changed-file set is missing or empty." }],
    };
  }

  const changes = [];
  const errors = [];
  for (const [index, value] of input.entries()) {
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      errors.push({ code: "ambiguous_change", index, detail: "A change entry is not an object." });
      continue;
    }
    const rawStatus = typeof value.status === "string" ? value.status : "";
    const status = normalizeStatus(rawStatus);
    if (!status) {
      errors.push({ code: "ambiguous_status", index, detail: `Unsupported change status: ${rawStatus || "<missing>"}.` });
      continue;
    }

    const rawPaths = status === "renamed" || status === "copied"
      ? [value.oldPath, value.newPath]
      : [value.path];
    const paths = rawPaths.map(normalizePath);
    if (paths.some((path) => path === null)) {
      errors.push({ code: "ambiguous_path", index, detail: "A changed path is missing, absolute, or unsafe." });
      continue;
    }
    changes.push({ status, paths });
  }

  changes.sort((left, right) => `${left.paths.join("\0")}\0${left.status}`.localeCompare(`${right.paths.join("\0")}\0${right.status}`));
  if (changes.length === 0 && errors.length === 0) {
    errors.push({ code: "ambiguous_input", detail: "No classifiable changes were supplied." });
  }
  return { changes, errors };
}

function normalizeStatus(rawStatus) {
  if (STATUS_ALIASES[rawStatus]) return STATUS_ALIASES[rawStatus];
  if (/^R\d{1,3}$/u.test(rawStatus)) return "renamed";
  if (/^C\d{1,3}$/u.test(rawStatus)) return "copied";
  return null;
}

function normalizePath(value) {
  if (typeof value !== "string" || value.length === 0 || value.includes("\\") || value.startsWith("/")) return null;
  const parts = value.split("/");
  if (parts.some((part) => part === "" || part === "." || part === ".." || /[\0\r\n]/u.test(part))) return null;
  return parts.join("/");
}

function classifyPath(path) {
  for (const { rule, matchers } of COMPILED_RULES) {
    if (matchers.length === 0 || matchers.some((matcher) => matcher.test(path))) {
      return pathResult(rule);
    }
  }
  throw new Error("Verification policy must end with a fallback rule.");
}

function pathResult(rule) {
  return {
    pathClass: rule.id,
    profile: rule.profile,
    sensitive: rule.sensitive,
    liveBrowser: rule.liveBrowser === true,
    detail: rule.detail,
  };
}

function selectProfile(matches) {
  if (matches.some((match) => match.profile === "release-critical")) return "release-critical";
  if (matches.some((match) => match.profile === "core")) return "core";
  if (matches.some((match) => match.profile === "docs")) return "docs";
  return "focused-docs";
}

function gatesForMatches(matches) {
  const selected = new Set();
  for (const match of matches) {
    for (const gate of PROFILES[match.profile] ?? []) selected.add(gate);
  }
  return FULL_GATES.filter((gate) => selected.has(gate));
}

function deduplicateReasons(reasons) {
  const seen = new Set();
  return reasons.filter((reason) => {
    const key = canonicalJson(reason);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function digest(value) {
  return `sha256:${createHash("sha256").update(canonicalJson(value)).digest("hex")}`;
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function deepFreeze(value) {
  if (!value || typeof value !== "object" || Object.isFrozen(value)) return value;
  for (const child of Object.values(value)) deepFreeze(child);
  return Object.freeze(value);
}
