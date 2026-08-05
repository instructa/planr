# ADRs

## ADR-001: Build Planr As A Self-Owned Product

Status: Accepted

### Context

Planr needs to combine durable Markdown planning with executable graph coordination under one owned product surface.

### Decision

Planr will be a new codebase and brand. Implementation, docs, assets, command names, and product vocabulary must be original unless explicitly retained under compatible license obligations.

### Alternatives Considered

- Build only Markdown plans: preserves readable context but lacks graph concurrency.
- Build only a graph engine: strong execution state but weak product and implementation context.
- Build hosted SaaS first: too much scope for V1.

### Consequences

- More initial engineering work.
- Cleaner product ownership.
- Better ability to design graph + Markdown as one system.

### Risks

- Rebuilding core graph behavior can introduce bugs.
- Public contracts must be explicit before release.

### Follow-Up Tasks

- TASK-FND-001
- TASK-DATA-001

## ADR-002: Graph State In SQLite, Rich Context In Markdown

Status: Accepted

### Context

Map item state needs atomic picks and link-based readiness. Human and agent context needs readable, versionable documents.

### Decision

Use SQLite as the authoritative graph state store and `.planr/*.md` as the rich context layer.

### Alternatives Considered

- Markdown only: simple, but no atomic concurrent picks.
- Database only: robust state, but poor narrative handoff.
- Hosted database: not local-first.

### Consequences

- Requires reconciliation between graph state and plan documents.
- Gives users both machine reliability and readable context.

### Risks

- Agents may treat Markdown checkboxes as state unless prompts and APIs are explicit.

### Follow-Up Tasks

- TASK-DATA-001
- TASK-FND-003

## ADR-003: MCP Is The Primary Cross-Agent Integration Surface

Status: Accepted

### Context

Codex, Claude Code, Cursor, and explicitly opted-in Grok Build all have MCP integration paths, while their native skill/plugin systems differ.

### Decision

Expose Planr through MCP tools, resources, and prompts first. Add client-specific wrappers where useful.

### Alternatives Considered

- Separate plugin per client: more native but higher maintenance.
- CLI-only: universal but too prompt-dependent.

### Consequences

- MCP schema design becomes part of the stable product contract.
- Prompts can expose Planr workflows as user-invoked commands.

### Risks

- MCP clients vary in supported capabilities and approval behavior.

### Follow-Up Tasks

- TASK-BE-003
- TASK-AI-002

## ADR-004: Durable ReviewGate Loop Is A Product Primitive

Status: Accepted

### Context

Agent work needs scoped review, logs, and honest status. FeatureRuns and ReviewGates encode this without workflow-only map rows.

### Decision

Every material outcome settles through a FeatureRun. Materiality or structured escalation creates one durable ReviewGate whose attempts, findings, resolution, and re-review remain attached to that gate.

### Alternatives Considered

- Close code items immediately after implementation.
- Use external PR review only.

### Consequences

- Better completion quality.
- Durable review history without inflating the executable map.

### Risks

- Small items may feel over-modeled unless lightweight defaults exist.

### Follow-Up Tasks

- TASK-DATA-002
- TASK-AI-003

## ADR-005: No Cloud Or Account System In V1

Status: Accepted

### Context

The product must be useful locally and not create privacy or deployment complexity.

### Decision

V1 has no Planr cloud account, billing, or hosted sync.

### Alternatives Considered

- Hosted dashboard first.
- Optional sync in V1.

### Consequences

- Simpler security model.
- Users own their data.

### Risks

- Team collaboration beyond shared Git remains limited.

### Follow-Up Tasks

- TASK-SEC-001
- TASK-REL-001
