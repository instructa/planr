# Safety, Privacy, And Security

## Data Inventory

### Project Metadata

- Classification: INTERNAL.
- Collected from: repo path, user commands.
- Stored where: SQLite.
- Sent to: nowhere by default.
- Retention: until project is deleted.
- Analytics allowed: local aggregate only.
- Logging allowed: yes, without source content.

### Map Graph

- Classification: INTERNAL, may become SENSITIVE if item text includes private details.
- Stored where: SQLite.
- Sent to: MCP clients only when requested by local user/agent.
- Retention: until deleted/exported.
- Analytics allowed: counts only.
- Logging allowed: metadata only.

### Plans

- Classification: INTERNAL or SENSITIVE depending on repo content.
- Stored where: `.planr/`.
- Sent to: local MCP clients when requested.
- Retention: Git/repo controlled.
- Analytics allowed: no content analytics.
- Logging allowed: path/hash only.

### Log

- Classification: INTERNAL or SENSITIVE depending on content.
- Stored where: SQLite.
- Sent to: local clients.
- Retention: until deleted/exported.
- Analytics allowed: status/counts only.
- Logging allowed: command metadata, not command output by default.

### Secrets

- Classification: SECRET.
- Stored where: never in Planr.
- Sent to: never intentionally.
- Logging allowed: no.

## Data Classification

- PUBLIC: public docs, release metadata.
- INTERNAL: item ids, statuses, event types, local metrics.
- PERSONAL: username, machine path if it identifies a person.
- SENSITIVE: private code plans, private tickets, prompts, transcripts, review content.
- SECRET: tokens, API keys, credentials, signing keys.

## Local Data

REQ-SEC-001: Planr stores V1 data locally under the repository or configured Planr home.

REQ-SEC-002: Planr must respect filesystem permissions and avoid writing outside configured paths except explicit install/config commands.

## Server Data

No server-side Planr storage in V1.

## Third-Party/Provider Data

Planr does not call AI providers by default. Agent clients may send Planr-provided context to their providers according to those clients' policies. Planr must minimize context and allow users to inspect what is returned to agents.

## Analytics Data

Allowed local diagnostics:

- command name;
- status code;
- duration;
- db schema version;
- number of items;
- event type;
- client integration type.

Forbidden analytics:

- source code;
- prompt/response text;
- plan body content;
- command output containing source or secrets;
- secrets, tokens, env vars;
- file contents.

## Consent And Disclosure Requirements

- REQ-SEC-010: Transcript capture requires explicit opt-in.
- REQ-SEC-011: Remote HTTP mode, if added, must require explicit opt-in and authentication.
- REQ-SEC-012: Install commands must show what files they will create or modify.

## Retention Policy

- SQLite map graph: retained until `planr project delete` or manual file removal.
- `.planr` Markdown: retained under user Git/repo policy.
- Debug logs: bounded retention; default 14 days if enabled.
- Transcript capture: disabled by default; retention user-configurable.

## Export And Deletion Policy

- `planr export` must produce map graph, plans, contexts, and logs.
- `planr project delete` must remove local database records and optionally `.planr` files with explicit confirmation.
- `planr scrub` must detect likely secrets in contexts and logs.

## Logging Policy

Allowed logs:

- item id;
- project id;
- worker id;
- command name;
- exit code;
- duration;
- status transition.

Forbidden logs:

- API keys and tokens;
- env var values;
- full prompts/responses;
- source file content;
- private plan body content by default.

## Security Controls

- REQ-SEC-020: SQLite writes must use parameterized queries.
- REQ-SEC-021: MCP mutation tools must validate schemas and item state transitions.
- REQ-SEC-022: HTTP server binds to localhost by default.
- REQ-SEC-023: Shell/agent runner commands must be explicit and auditable.
- REQ-SEC-024: Destructive operations require preview or confirmation.
- REQ-SEC-025: Database schema upgrades must be tested against existing schemas.

## Abuse Prevention

Planr is local-first, so abuse risk is mostly local command execution and data exfiltration through agent clients. Mitigations:

- separate read and mutation tools;
- no implicit shell execution from plan files;
- no hidden remote sync;
- content minimization in MCP responses;
- optional policy to deny mutation tools in review-only clients.

## Safety Risk Taxonomy

Planr is generally S1/S2:

- S1: wrong item state may waste developer time.
- S2: private code or item text may be exposed if sent to agent providers.

Planr must not make medical, legal, financial, or regulated-domain decisions.

## Compliance Notes

- V1 does not claim SOC 2, GDPR compliance, or enterprise compliance.
- Privacy policy and security documentation are required before any hosted service.

## Legal/Platform Review Checklist

- Review license obligations for any retained code, docs, or assets.
- Review MCP tool permissions and security copy.
- Review privacy disclosures for transcript capture.
- Review package-manager install scripts for supply-chain risk.
