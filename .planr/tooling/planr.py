#!/usr/bin/env python3
from __future__ import annotations

import argparse
import errno
import fcntl
import hashlib
import json
import sys
import tempfile
import time
import unicodedata
from contextlib import contextmanager
from datetime import date, datetime, timezone
from pathlib import Path
from typing import Any, Iterable, Iterator


STATUS_FILE = Path(".planr/status/current.json")
PLANS_DIR = Path(".planr/plans")
DONE_PLANS_DIR = PLANS_DIR / "done"
PROJECT_DIR = Path(".planr/project")
STATUS_LOCK_TIMEOUT_SECONDS = 10.0
STATUS_LOCK_POLL_INTERVAL_SECONDS = 0.05
VALID_SCOPE_STATUSES = {"pending", "in_progress", "completed", "blocked", "cancelled"}
VALID_CHECKLIST_STATUSES = VALID_SCOPE_STATUSES
VALID_BLOCKER_STATUSES = {"blocked", "unverified"}
VALID_VERIFICATION_STATUSES = {"passed", "failed", "blocked", "not_run"}
OPEN_SCOPE_STATUSES = {"pending", "in_progress", "blocked"}
NON_PASSED_VERIFICATION_STATUSES = VALID_VERIFICATION_STATUSES - {"passed"}
PROJECT_FILE_ORDER = [
    "product.md",
    "ownership.md",
    "flows.md",
    "state-ssot.md",
    "constraints.md",
    "quality-gates.md",
]
PROJECT_INIT_NEXT_STEP = (
    "Inspect the target codebase and rewrite `.planr/project/*.md` to match the repo's actual product, "
    "ownership boundaries, flows, state sources of truth, and quality gates before trusting architecture "
    "or ownership decisions."
)
DEFAULT_STATUS_NOTES = [
    "`.planr/status/current.json` is the live summary for this repo. Execution contracts live in `.planr/plans/*.plan.md`.",
    "Run `./.planr/tooling/planr project init`, then inspect and rewrite `.planr/project/*.md` for the target repo before trusting ownership or architecture decisions.",
]
PROJECT_TEMPLATE_TEXTS = {
    "product.md": """# Product

Durable product context for planr-style planning in **this** repository.

Fill this in for the codebase you install these skills into:

- what the software is
- what “good” looks like for users or consumers
- what outcomes count as improvement

## Product Summary

*(Replace with your project: e.g. a library, CLI, service, or app.)*

## Core Goal

Help contributors and agents work with:

- clear scope and owned paths
- explicit plans and verification
- honest status in `.planr/status/current.json`

## Non-Goals

Avoid optimizing for vague scope, unchecked plan boxes without proof, or duplicate canonical workflows for the same task.

## Plan Notes

Prefer linking to this repo’s `README.md`, `CONTRIBUTING.md`, or architecture docs when they exist.
""",
    "ownership.md": """# Ownership

Define **your** codebase’s layers so `planr-plan` / `planr-review` can place work correctly.

Replace the placeholders below with real paths (examples: `src/`, `packages/foo/`, `internal/`, `cmd/`).

## Suggested layers (customize)

| Layer | Path pattern | Owns |
|-------|----------------|------|
| Entry / UI | *(e.g. `apps/web/`, `cli/`) | User-facing surface, presentation |
| Application / API | *(e.g. `server/`, `api/`) | Use-cases, orchestration |
| Domain / core | *(e.g. `lib/`, `domain/`) | Shared rules, types, pure logic |
| Integrations | *(e.g. `adapters/`, `integrations/`) | External systems, not global product policy |

## Ownership rules

- **Runtime owner**: where the behavior runs today.
- **First-fix owner**: where the bug or duplication lives now.
- **Canonical owner**: where the logic should live after cleanup.

Do not collapse these into one vague answer.

## Wrong defaults (typical smells)

- UI owning durable server-side truth
- Thin transport layers owning domain policy
- Duplicated rules across packages without a single SSOT

## Plan Notes

Update from your repo’s architecture or CONTRIBUTING docs when available.
""",
    "flows.md": """# Flows

High-level flows planning work should respect. Adapt examples to **your** stack (web, CLI, mobile, backend, etc.).

## Change flow

1. User or ticket defines scope.
2. Read `.planr/project/` context when architecture matters.
3. Add or update a scoped plan under `.planr/plans/*.plan.md` when the work needs a contract.
4. Implement against owned paths only.
5. Record verification and update `.planr/status/current.json` when tracking execution.

## Plan → execute → verify

- **Plan**: explicit scope, ownership, phases, acceptance criteria.
- **Fix**: concrete changes + tests/docs.
- **Review**: Git-scoped evidence that the contract is satisfied.

## Status vs review vs fix

- **Status**: smallest honest verdict right now.
- **Review**: did the implementation satisfy the task correctly?
- **Fix**: what concrete work closes the gap?

Do not merge these flows silently.

## Boundaries

Prefer one canonical path per concern, explicit handoffs between layers, and a single owner per policy decision.

## Plan Notes

Derive from your repo’s docs (e.g. architecture overview, ADRs) when present.
""",
    "state-ssot.md": """# State SSOT

Default expectations for **where truth lives** in this repo. Customize for your persistence and UI model.

## Principles

- Persisted or authoritative state has one canonical owner; UI mirrors derive from it.
- Feature-local UI state stays near the feature.
- Derived state is recomputed from SSOT, not duplicated with fragile sync.

## Planning questions

Before state changes, answer:

- What is the current source of truth?
- What competing source should be removed or narrowed?
- Which layer should derive instead of store?

## Anti-patterns

- Two parallel owners for the same rule
- “Helpful” fallbacks in transport that become the real behavior
- Dead config or schema fields that no longer affect behavior

## Plan Notes

Align with your repo’s data model and API contracts when documented.
""",
    "constraints.md": """# Constraints

This file captures non-negotiable planning and implementation constraints for planr workflows.

## Hard-Cut Product Policy

Default stance:

- keep one canonical current-state implementation
- delete or narrow fallback, compatibility, shim, bridge, adapter, and dual-path behavior when the task calls for a hard cut
- when only one meaningful current-state behavior remains, delete the surrounding concept end-to-end instead of preserving a one-value enum, one-option selector, dead config key, or pass-through field
- prefer fail-fast diagnostics and explicit recovery steps
- use invalid-state diagnostics only on boundaries that still legitimately exist; do not keep deleted concepts around solely to reject their former values

Do not introduce migration glue or second current-state paths unless the user explicitly asks for transition support.

## Scope Discipline

- keep owned scope explicit
- do not silently widen into adjacent cleanup
- do not treat unrelated dirty files as part of the task
- if mixed authorship makes scope ambiguous, stop and clarify

## Owner Discipline

- thin shells stay thin
- reusable policy should not get stranded in transport or glue layers
- integration modules own external wiring, not global domain policy
- UI-only state should not become the SSOT for server or shared domain truth

## Fresh-Repo Bootstrap Rule

If a repo does not already have equivalent durable context for:

- product direction
- ownership boundaries
- critical flows
- state ownership
- quality gates

then `planr-plan` must create or request the `.planr/project/` pack before making strong architecture or ownership decisions.

## Documentation Bias

Prefer:

- durable repo-local docs
- explicit contracts
- explicit verification notes

over:

- ephemeral chat context
- implied assumptions
- checklist-only claims

## Plan Notes

Align with repo policy docs (e.g. `AGENTS.md`, contributing guides, or a local “hard cut / no dual paths” rule) when present.
""",
    "quality-gates.md": """# Quality Gates

This file defines the minimum bar for claiming progress or completion in planr workflows.

## Completion Contract

Do not report success until every requested item is:

- completed
- explicitly blocked
- or explicitly unverified

Status files and plan todos are summary state, not proof.

## Verification Rules

- run the smallest relevant verification first
- record the exact command and actual result
- if a broader command is blocked by unrelated failures, say so explicitly
- do not convert blocked or skipped verification into a silent pass

## Evidence Rules

Strong evidence includes:

- exact test or verification commands
- exact pass, fail, blocked, or unverified results
- scoped diff evidence
- exact path-scoped Git comparison commands for review verdicts
- searches that prove the competing path is gone
- for hard-cut removals, proof that the dead concept itself is gone from live contracts and owner layers, not only that an old value is rejected

Weak evidence includes:

- checked boxes without proof
- "looks done"
- one nearby happy-path test
- large diffs without scoped verification
- a review verdict without an exact path-scoped Git comparison command
- a reject-test for a deleted legacy value when the surrounding one-value setting, enum, or binding still exists

## Default Validation Commands

Use the smallest affected scope first, then broaden when warranted.

Customize this section for the target repository. Good defaults are:

- the smallest relevant unit or integration test command for the owned scope
- the narrowest typecheck, compile, or build command that validates the changed surface
- the repo's linter or static-analysis command for the affected package, module, or file set
- the repo's format check when formatting is part of the gate
- a targeted smoke check or manual repro command when the change is user-visible or operational

Record the exact commands and outcomes in the plan, status, or review artifact instead of assuming another repo's stack.

## Status Tracking Rules

- `.planr/status/current.json` is the canonical live planr status source
- `.planr/plans/*.plan.md` contains scoped execution contracts
- material persisted reviews may live under `.planr/review/*.review.md`, but `current.json` remains the summary layer rather than a duplicate full review log
- after each substantial step, update the live status source honestly
- after interruption or resume, read `.planr/status/current.json` and `git status` before continuing

## Plan Notes

This file is derived from:

- `AGENTS.md`
- repo user rules for test execution
""",
}


class CliError(RuntimeError):
    pass


def utc_timestamp() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def slugify(value: str) -> str:
    normalized = unicodedata.normalize("NFKD", value).encode("ascii", "ignore").decode("ascii")
    cleaned = []
    previous_was_sep = False
    for ch in normalized.lower():
        if ch.isalnum():
            cleaned.append(ch)
            previous_was_sep = False
            continue
        if not previous_was_sep:
            cleaned.append("_")
            previous_was_sep = True
    slug = "".join(cleaned).strip("_")
    return slug or "plan"


def title_hash(title: str) -> str:
    normalized = " ".join(title.strip().split()).lower()
    return hashlib.sha1(normalized.encode("utf-8")).hexdigest()[:8]


def plan_filename(title: str, slug: str | None = None) -> str:
    effective_slug = slugify(slug or title)
    return f"{effective_slug}_{title_hash(title)}.plan.md"


def plan_relative_path(title: str, slug: str | None = None) -> str:
    return (PLANS_DIR / plan_filename(title, slug)).as_posix()


def parse_archive_date(raw_value: str) -> date:
    try:
        return datetime.strptime(raw_value, "%Y-%m-%d").date()
    except ValueError as exc:
        raise CliError("`--archive-date` must use the form YYYY-MM-DD.") from exc


def archive_bucket_relative_path(archive_date: date) -> str:
    return (DONE_PLANS_DIR / archive_date.strftime("%d-%m")).as_posix()


def path_is_within_prefix(raw_path: str, prefix: Path) -> bool:
    path_parts = Path(raw_path).parts
    prefix_parts = prefix.parts
    return len(path_parts) >= len(prefix_parts) and path_parts[: len(prefix_parts)] == prefix_parts


def require_string_list_field(parent: dict[str, Any], key: str, owner: str) -> list[str]:
    items = get_list_field(parent, key, owner)
    strings: list[str] = []
    for index, item in enumerate(items):
        if not isinstance(item, str) or not item.strip():
            raise CliError(f"`{owner}.{key}[{index}]` must be a non-empty string in `.planr/status/current.json`.")
        strings.append(item.strip())
    return strings


def resolve_repo_root(repo_root: str | None) -> Path:
    if repo_root:
        root = Path(repo_root).expanduser().resolve()
        if not root.exists():
            raise CliError(f"Repo root does not exist: {root}")
        if not (root / ".planr").exists():
            raise CliError(f"Repo root does not contain `.planr/`: {root}")
        return root

    current = Path.cwd().resolve()
    for candidate in (current, *current.parents):
        if (candidate / ".planr").exists():
            return candidate
    raise CliError("Could not find repo root containing `.planr/` from the current working directory.")


def ensure_relative_to_root(root: Path, raw_path: str) -> str:
    candidate = Path(raw_path).expanduser()
    resolved = candidate.resolve() if candidate.is_absolute() else (root / candidate).resolve()
    try:
        return resolved.relative_to(root).as_posix()
    except ValueError as exc:
        raise CliError(f"Path must stay inside the repo root: {raw_path}") from exc


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise CliError(f"Missing JSON file: {path}") from exc
    except json.JSONDecodeError as exc:
        raise CliError(f"Invalid JSON in {path}: {exc}") from exc


def atomic_write_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False, dir=path.parent) as handle:
        handle.write(content)
        temp_path = Path(handle.name)
    temp_path.replace(path)


def write_json(path: Path, data: Any) -> None:
    atomic_write_text(path, json.dumps(data, indent=2, ensure_ascii=False) + "\n")


def canonical_project_context_files() -> list[str]:
    return [f"{PROJECT_DIR.as_posix()}/{name}" for name in PROJECT_FILE_ORDER]


def default_status_payload() -> dict[str, Any]:
    return {
        "schema_version": 1,
        "updated_at": utc_timestamp(),
        "global_status": "open",
        "project_context": {
            "root": PROJECT_DIR.as_posix(),
            "files": canonical_project_context_files(),
        },
        "notes": list(DEFAULT_STATUS_NOTES),
        "scopes": [],
    }


def normalize_unique_paths(root: Path, values: Iterable[str] | None) -> list[str]:
    if not values:
        return []
    normalized = {ensure_relative_to_root(root, value) for value in values}
    return sorted(normalized)


def get_status_path(root: Path) -> Path:
    return root / STATUS_FILE


def get_status_lock_path(root: Path) -> Path:
    status_path = get_status_path(root)
    return status_path.with_name(f"{status_path.name}.lock")


def load_status(root: Path) -> dict[str, Any]:
    return read_json(get_status_path(root))


def save_status(root: Path, data: dict[str, Any]) -> None:
    data["updated_at"] = utc_timestamp()
    write_json(get_status_path(root), data)


@contextmanager
def status_write_lock(root: Path) -> Iterator[None]:
    lock_path = get_status_lock_path(root)
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+", encoding="utf-8") as handle:
        deadline = time.monotonic() + STATUS_LOCK_TIMEOUT_SECONDS
        while True:
            try:
                fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except OSError as exc:
                if exc.errno not in {errno.EACCES, errno.EAGAIN}:
                    raise CliError(f"Could not acquire status write lock: {lock_path.relative_to(root).as_posix()}") from exc
                if time.monotonic() >= deadline:
                    raise CliError(f"Timed out waiting for status write lock: {lock_path.relative_to(root).as_posix()}")
                time.sleep(STATUS_LOCK_POLL_INTERVAL_SECONDS)
        try:
            yield
        finally:
            fcntl.flock(handle.fileno(), fcntl.LOCK_UN)


@contextmanager
def status_mutation_session(root: Path) -> Iterator[dict[str, Any]]:
    with status_write_lock(root):
        status_path = get_status_path(root)
        if status_path.exists():
            data = load_status(root)
            if not isinstance(data, dict):
                raise CliError(f"Invalid status payload in {status_path}")
        else:
            data = default_status_payload()
        yield data
        save_status(root, data)


def scopes(data: dict[str, Any]) -> list[dict[str, Any]]:
    scope_list = data.get("scopes")
    if not isinstance(scope_list, list):
        raise CliError("`.planr/status/current.json` is missing a valid `scopes` array.")
    return scope_list


def require_list_field(parent: dict[str, Any], key: str, owner: str) -> list[dict[str, Any]]:
    items = parent.setdefault(key, [])
    if not isinstance(items, list):
        raise CliError(f"`{owner}.{key}` must be a list in `.planr/status/current.json`.")
    return items


def get_list_field(parent: dict[str, Any], key: str, owner: str) -> list[Any]:
    items = parent.get(key, [])
    if items is None:
        return []
    if not isinstance(items, list):
        raise CliError(f"`{owner}.{key}` must be a list in `.planr/status/current.json`.")
    return items


def get_object_list(parent: dict[str, Any], key: str, owner: str) -> list[dict[str, Any]]:
    items = get_list_field(parent, key, owner)
    for index, item in enumerate(items):
        if not isinstance(item, dict):
            raise CliError(f"`{owner}.{key}[{index}]` must be an object in `.planr/status/current.json`.")
    return items  # type: ignore[return-value]


def require_valid_status(value: Any, valid: set[str], label: str) -> str:
    if not isinstance(value, str) or value not in valid:
        raise CliError(f"{label} must be one of: {', '.join(sorted(valid))}")
    return value


def find_scope(data: dict[str, Any], scope_id: str) -> dict[str, Any] | None:
    for scope in scopes(data):
        if scope.get("id") == scope_id:
            return scope
    return None


def find_scope_index(data: dict[str, Any], scope_id: str) -> int | None:
    for index, scope in enumerate(scopes(data)):
        if scope.get("id") == scope_id:
            return index
    return None


def require_scope(data: dict[str, Any], scope_id: str) -> dict[str, Any]:
    scope = find_scope(data, scope_id)
    if scope is None:
        raise CliError(f"Scope not found: {scope_id}")
    return scope


def find_item_index(items: list[dict[str, Any]], item_id: str) -> int | None:
    for index, item in enumerate(items):
        if item.get("id") == item_id:
            return index
    return None


def upsert_item(
    items: list[dict[str, Any]],
    item_id: str,
    *,
    before_id: str | None = None,
    after_id: str | None = None,
) -> dict[str, Any]:
    if before_id == item_id or after_id == item_id:
        raise CliError("An item cannot be positioned relative to itself.")

    existing_index = find_item_index(items, item_id)
    if existing_index is not None:
        item = items.pop(existing_index)
    else:
        item = {"id": item_id}

    if before_id is not None:
        anchor_index = find_item_index(items, before_id)
        if anchor_index is None:
            raise CliError(f"Anchor item not found: {before_id}")
        items.insert(anchor_index, item)
        return item

    if after_id is not None:
        anchor_index = find_item_index(items, after_id)
        if anchor_index is None:
            raise CliError(f"Anchor item not found: {after_id}")
        items.insert(anchor_index + 1, item)
        return item

    if existing_index is None:
        items.append(item)
        return item

    items.insert(existing_index, item)
    return item


def delete_item(items: list[dict[str, Any]], item_id: str, label: str) -> dict[str, Any]:
    index = find_item_index(items, item_id)
    if index is None:
        raise CliError(f"{label} not found: {item_id}")
    return items.pop(index)


def yaml_quoted(value: str) -> str:
    return json.dumps(value)


def phase_heading(todo_id: str) -> str:
    return todo_id.replace("_", " ").replace("-", " ").title()


def render_plan(title: str, overview: str, todos: list[tuple[str, str]], is_project: bool) -> str:
    lines: list[str] = [
        "---",
        f"name: {yaml_quoted(title.lower())}",
        f"overview: {yaml_quoted(overview)}",
        "todos:",
    ]
    for todo_id, content in todos:
        lines.extend(
            [
                f"  - id: {yaml_quoted(todo_id)}",
                f"    content: {yaml_quoted(content)}",
                "    status: pending",
            ]
        )
    lines.extend(
        [
            f"isProject: {'true' if is_project else 'false'}",
            "---",
            "",
            f"# {title}",
            "",
            "## Scope Decision",
            "",
            "- Define the exact requested scope here.",
            "- List the rejected scope expansions here.",
            "",
            "## Ownership Target",
            "",
            "- `Runtime owner`: ...",
            "- `First fix owner`: ...",
            "- `Canonical long-term owner`: ...",
            "- `Wrong competing owners`: ...",
            "",
            "## Existing Leverage",
            "",
            "- List the concrete files, symbols, tests, and existing behavior this plan will reuse or narrow.",
            "",
        ]
    )
    for index, (todo_id, content) in enumerate(todos, start=1):
        lines.extend(
            [
                f"## Phase {index}: {phase_heading(todo_id)}",
                "",
                "Phase goal:",
                "",
                f"- {content}",
                "",
                "Tasks:",
                "",
                f"- [ ] {content}",
                "- [ ] Reconcile the scoped diff and recorded verification before marking this phase complete.",
                "",
            ]
        )
    lines.extend(
        [
            "## Out Of Scope",
            "",
            "- Fill in the adjacent work this plan will not absorb.",
            "",
            "## Verification",
            "",
            "- `git diff -- <owned paths>`",
            "- `<focused verification command>`",
            "",
            "## Acceptance Criteria",
            "",
        ]
    )
    for _, content in todos:
        lines.append(f"- {content}")
    lines.append("")
    return "\n".join(lines)


def parse_todos(raw_todos: list[str]) -> list[tuple[str, str]]:
    todos: list[tuple[str, str]] = []
    seen: set[str] = set()
    for raw in raw_todos:
        todo_id, sep, content = raw.partition("=")
        todo_id = todo_id.strip()
        content = content.strip()
        if not sep or not todo_id or not content:
            raise CliError("Each `--todo` must use the form `id=content`.")
        if todo_id in seen:
            raise CliError(f"Duplicate todo id: {todo_id}")
        seen.add(todo_id)
        todos.append((todo_id, content))
    if not todos:
        raise CliError("At least one `--todo id=content` entry is required.")
    return todos


def archive_scope_plan_paths(root: Path, *, scope_id: str, archive_date: date) -> dict[str, Any]:
    bucket_relative_path = archive_bucket_relative_path(archive_date)
    moved_paths: list[tuple[Path, Path]] = []

    with status_write_lock(root):
        status_path = get_status_path(root)
        data = load_status(root) if status_path.exists() else default_status_payload()
        if not isinstance(data, dict):
            raise CliError(f"Invalid status payload in {status_path}")

        scope = require_scope(data, scope_id)
        scope_status = require_valid_status(scope.get("status"), VALID_SCOPE_STATUSES, f"`{scope_id}.status`")
        if scope_status != "completed":
            raise CliError(f"Only completed scopes can be archived: {scope_id}")

        plan_paths = require_string_list_field(scope, "plan_paths", scope_id)
        if not plan_paths:
            raise CliError(f"Scope has no plan paths to archive: {scope_id}")

        seen_destinations: set[str] = set()
        move_specs: list[tuple[str, Path, str, Path]] = []
        for plan_path in plan_paths:
            if path_is_within_prefix(plan_path, DONE_PLANS_DIR):
                raise CliError(f"Plan path is already archived: {plan_path}")
            if not path_is_within_prefix(plan_path, PLANS_DIR):
                raise CliError(f"Scope plan path is not under {PLANS_DIR.as_posix()}: {plan_path}")

            source_path = root / plan_path
            if not source_path.is_file():
                raise CliError(f"Plan file not found: {plan_path}")

            destination_relative_path = f"{bucket_relative_path}/{source_path.name}"
            if destination_relative_path in seen_destinations:
                raise CliError(f"Archive destination collision: {destination_relative_path}")
            seen_destinations.add(destination_relative_path)

            destination_path = root / destination_relative_path
            if destination_path.exists():
                raise CliError(f"Archive destination already exists: {destination_relative_path}")

            move_specs.append((plan_path, source_path, destination_relative_path, destination_path))

        try:
            for _, source_path, _, destination_path in move_specs:
                destination_path.parent.mkdir(parents=True, exist_ok=True)
                source_path.replace(destination_path)
                moved_paths.append((source_path, destination_path))

            scope["plan_paths"] = [destination_relative_path for _, _, destination_relative_path, _ in move_specs]
            save_status(root, data)
        except Exception:
            for original_path, archived_path in reversed(moved_paths):
                if archived_path.exists() and not original_path.exists():
                    original_path.parent.mkdir(parents=True, exist_ok=True)
                    archived_path.replace(original_path)
            raise

    return {
        "scope": scope_id,
        "archive_bucket": bucket_relative_path,
        "archived_plan_paths": [
            {"from": source_relative_path, "to": destination_relative_path}
            for source_relative_path, _, destination_relative_path, _ in move_specs
        ],
    }


def cmd_plan_new(args: argparse.Namespace, root: Path) -> int:
    title = args.title.strip()
    if not title:
        raise CliError("`--title` cannot be empty.")
    overview = args.overview.strip()
    if not overview:
        raise CliError("`--overview` cannot be empty.")

    todos = parse_todos(args.todo)
    plan_path = root / plan_relative_path(title, args.slug)
    if plan_path.exists() and not args.force:
        raise CliError(f"Plan already exists: {plan_path.relative_to(root).as_posix()}")

    content = render_plan(title=title, overview=overview, todos=todos, is_project=args.is_project)
    atomic_write_text(plan_path, content)
    print(plan_path.relative_to(root).as_posix())
    return 0


def cmd_plan_path(args: argparse.Namespace, root: Path) -> int:
    del root
    title = args.title.strip()
    if not title:
        raise CliError("`--title` cannot be empty.")
    print(plan_relative_path(title, args.slug))
    return 0


def cmd_plan_archive(args: argparse.Namespace, root: Path) -> int:
    archive_date = parse_archive_date(args.archive_date) if args.archive_date else datetime.now().date()
    payload = archive_scope_plan_paths(root, scope_id=args.scope, archive_date=archive_date)
    print(json.dumps(payload, indent=2, ensure_ascii=False))
    return 0


def cmd_project_init(args: argparse.Namespace, root: Path) -> int:
    project_root = root / PROJECT_DIR
    project_root.mkdir(parents=True, exist_ok=True)
    (root / PLANS_DIR).mkdir(parents=True, exist_ok=True)
    (root / STATUS_FILE.parent).mkdir(parents=True, exist_ok=True)

    written: list[str] = []
    preserved_existing: list[str] = []

    for filename in PROJECT_FILE_ORDER:
        path = project_root / filename
        content = PROJECT_TEMPLATE_TEXTS[filename]
        relative_path = path.relative_to(root).as_posix()
        if path.exists() and not args.force:
            existing = path.read_text(encoding="utf-8")
            if existing != content:
                preserved_existing.append(relative_path)
                continue
        atomic_write_text(path, content)
        written.append(relative_path)

    with status_mutation_session(root) as data:
        data["project_context"] = {
            "root": PROJECT_DIR.as_posix(),
            "files": canonical_project_context_files(),
        }

    payload = {
        "project_root": PROJECT_DIR.as_posix(),
        "written": written,
        "preserved_existing": preserved_existing,
        "project_context_files": canonical_project_context_files(),
        "status_path": STATUS_FILE.as_posix(),
        "next_step": PROJECT_INIT_NEXT_STEP,
    }
    print(json.dumps(payload, indent=2, ensure_ascii=False))
    return 0


def cmd_status_show(args: argparse.Namespace, root: Path) -> int:
    data = load_status(root)
    if args.scope:
        payload = require_scope(data, args.scope)
    else:
        payload = data
    print(json.dumps(payload, indent=2, ensure_ascii=False))
    return 0


def open_scope_sort_key(summary: dict[str, Any]) -> tuple[int, int, str]:
    status = summary["status"]
    if status == "in_progress":
        bucket = 0
    elif status == "pending":
        bucket = 1
    elif status == "blocked":
        bucket = 2
    else:
        bucket = 3
    return (bucket, summary["_order"], summary["id"])


def summarize_open_scope(scope: dict[str, Any], index: int) -> dict[str, Any] | None:
    raw_scope_id = scope.get("id")
    if not isinstance(raw_scope_id, str) or not raw_scope_id.strip():
        raise CliError(f"`scopes[{index}].id` must be a non-empty string in `.planr/status/current.json`.")
    scope_id = raw_scope_id.strip()

    status = require_valid_status(scope.get("status"), VALID_SCOPE_STATUSES, f"`{scope_id}.status`")
    checklist_items = get_object_list(scope, "checklist", scope_id)
    blocked_items = get_object_list(scope, "blocked_or_unverified", scope_id)
    verification_items = get_object_list(scope, "verification", scope_id)

    open_checklist_ids: list[str] = []
    for checklist_index, item in enumerate(checklist_items):
        checklist_status = require_valid_status(
            item.get("status"),
            VALID_CHECKLIST_STATUSES,
            f"`{scope_id}.checklist[{checklist_index}].status`",
        )
        if checklist_status in OPEN_SCOPE_STATUSES:
            item_id = item.get("id")
            open_checklist_ids.append(item_id if isinstance(item_id, str) and item_id else f"checklist[{checklist_index}]")

    blocked_or_unverified_ids: list[str] = []
    for blocker_index, item in enumerate(blocked_items):
        blocker_status = require_valid_status(
            item.get("status"),
            VALID_BLOCKER_STATUSES,
            f"`{scope_id}.blocked_or_unverified[{blocker_index}].status`",
        )
        if blocker_status in VALID_BLOCKER_STATUSES:
            item_id = item.get("id")
            blocked_or_unverified_ids.append(
                item_id if isinstance(item_id, str) and item_id else f"blocked_or_unverified[{blocker_index}]"
            )

    non_passed_verification_ids: list[str] = []
    for verification_index, item in enumerate(verification_items):
        verification_status = require_valid_status(
            item.get("status"),
            VALID_VERIFICATION_STATUSES,
            f"`{scope_id}.verification[{verification_index}].status`",
        )
        if verification_status in NON_PASSED_VERIFICATION_STATUSES:
            item_id = item.get("id")
            non_passed_verification_ids.append(
                item_id if isinstance(item_id, str) and item_id else f"verification[{verification_index}]"
            )

    status_drift = status in {"completed", "cancelled"} and bool(
        open_checklist_ids or blocked_or_unverified_ids or non_passed_verification_ids
    )
    is_open = status in OPEN_SCOPE_STATUSES or status_drift
    if not is_open:
        return None

    title = scope.get("title")
    plan_paths = get_list_field(scope, "plan_paths", scope_id)
    return {
        "id": scope_id,
        "title": title if isinstance(title, str) else "",
        "status": status,
        "plan_paths": plan_paths,
        "open_checklist_count": len(open_checklist_ids),
        "open_checklist_ids": open_checklist_ids,
        "blocked_or_unverified_count": len(blocked_or_unverified_ids),
        "blocked_or_unverified_ids": blocked_or_unverified_ids,
        "non_passed_verification_count": len(non_passed_verification_ids),
        "non_passed_verification_ids": non_passed_verification_ids,
        "status_drift": status_drift,
        "_order": index,
    }


def sorted_open_scope_summaries(data: dict[str, Any]) -> list[dict[str, Any]]:
    summaries: list[dict[str, Any]] = []
    for index, scope in enumerate(scopes(data)):
        summary = summarize_open_scope(scope, index)
        if summary is not None:
            summaries.append(summary)
    summaries.sort(key=open_scope_sort_key)
    return summaries


def public_scope_summary(summary: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in summary.items() if not key.startswith("_")}


def cmd_status_open(args: argparse.Namespace, root: Path) -> int:
    del args
    data = load_status(root)
    payload = [public_scope_summary(summary) for summary in sorted_open_scope_summaries(data)]
    print(json.dumps(payload, indent=2, ensure_ascii=False))
    return 0


def cmd_status_next(args: argparse.Namespace, root: Path) -> int:
    del args
    data = load_status(root)
    summaries = sorted_open_scope_summaries(data)

    actionable = [summary for summary in summaries if summary["status"] in OPEN_SCOPE_STATUSES]
    if actionable:
        payload = public_scope_summary(actionable[0])
        payload["selection_reason"] = (
            "selected the first actionable open scope by status precedence "
            "(in_progress, pending, blocked) and scope order"
        )
        print(json.dumps(payload, indent=2, ensure_ascii=False))
        return 0

    if summaries:
        payload = public_scope_summary(summaries[0])
        payload["selection_reason"] = (
            "no actionable in_progress, pending, or blocked scopes remain; "
            "returning the first closed scope with status drift"
        )
        print(json.dumps(payload, indent=2, ensure_ascii=False))
        return 0

    print("null")
    return 0


def cmd_status_ensure_scope(args: argparse.Namespace, root: Path) -> int:
    with status_mutation_session(root) as data:
        scope = find_scope(data, args.id)
        if scope is None:
            scope = {
                "id": args.id,
                "title": "",
                "status": "pending",
                "source": "",
                "plan_paths": [],
                "owned_paths": [],
                "checklist": [],
                "verification": [],
                "blocked_or_unverified": [],
            }
            scopes(data).append(scope)

        if args.title is not None:
            scope["title"] = args.title
        if args.status is not None:
            scope["status"] = args.status
        if args.source is not None:
            scope["source"] = args.source
        if args.clear_plan_paths:
            scope["plan_paths"] = []
        elif args.plan_path is not None:
            scope["plan_paths"] = normalize_unique_paths(root, args.plan_path)
        if args.clear_owned_paths:
            scope["owned_paths"] = []
        elif args.owned_path is not None:
            scope["owned_paths"] = normalize_unique_paths(root, args.owned_path)
    print(json.dumps(scope, indent=2, ensure_ascii=False))
    return 0


def cmd_status_delete_scope(args: argparse.Namespace, root: Path) -> int:
    with status_mutation_session(root) as data:
        scope_index = find_scope_index(data, args.id)
        if scope_index is None:
            raise CliError(f"Scope not found: {args.id}")
        scope = scopes(data).pop(scope_index)
    print(json.dumps(scope, indent=2, ensure_ascii=False))
    return 0


def cmd_status_set_checklist(args: argparse.Namespace, root: Path) -> int:
    with status_mutation_session(root) as data:
        scope = require_scope(data, args.scope)
        items = require_list_field(scope, "checklist", args.scope)
        item = upsert_item(items, args.item_id, before_id=args.before_id, after_id=args.after_id)
        if args.content is not None:
            item["content"] = args.content
        elif "content" not in item:
            raise CliError("`--content` is required when creating a new checklist item.")
        item["status"] = args.status
    print(json.dumps(item, indent=2, ensure_ascii=False))
    return 0


def cmd_status_set_blocker(args: argparse.Namespace, root: Path) -> int:
    with status_mutation_session(root) as data:
        scope = require_scope(data, args.scope)
        items = require_list_field(scope, "blocked_or_unverified", args.scope)
        item = upsert_item(items, args.item_id, before_id=args.before_id, after_id=args.after_id)
        if args.content is not None:
            item["content"] = args.content
        elif "content" not in item:
            raise CliError("`--content` is required when creating a new blocker or unverified item.")
        item["status"] = args.status
    print(json.dumps(item, indent=2, ensure_ascii=False))
    return 0


def cmd_status_set_verification(args: argparse.Namespace, root: Path) -> int:
    with status_mutation_session(root) as data:
        scope = require_scope(data, args.scope)
        items = require_list_field(scope, "verification", args.scope)
        item = upsert_item(items, args.verification_id, before_id=args.before_id, after_id=args.after_id)
        item["status"] = args.status
        item["result"] = args.result
        if args.command is not None:
            item["command"] = args.command
    print(json.dumps(item, indent=2, ensure_ascii=False))
    return 0


def cmd_status_delete_checklist(args: argparse.Namespace, root: Path) -> int:
    with status_mutation_session(root) as data:
        scope = require_scope(data, args.scope)
        items = require_list_field(scope, "checklist", args.scope)
        item = delete_item(items, args.item_id, "Checklist item")
    print(json.dumps(item, indent=2, ensure_ascii=False))
    return 0


def cmd_status_delete_blocker(args: argparse.Namespace, root: Path) -> int:
    with status_mutation_session(root) as data:
        scope = require_scope(data, args.scope)
        items = require_list_field(scope, "blocked_or_unverified", args.scope)
        item = delete_item(items, args.item_id, "Blocked or unverified item")
    print(json.dumps(item, indent=2, ensure_ascii=False))
    return 0


def cmd_status_delete_verification(args: argparse.Namespace, root: Path) -> int:
    with status_mutation_session(root) as data:
        scope = require_scope(data, args.scope)
        items = require_list_field(scope, "verification", args.scope)
        item = delete_item(items, args.verification_id, "Verification record")
    print(json.dumps(item, indent=2, ensure_ascii=False))
    return 0


def add_relative_order_arguments(parser: argparse.ArgumentParser) -> None:
    ordering = parser.add_mutually_exclusive_group()
    ordering.add_argument("--before-id", help="Insert or move the item before the given sibling item id.")
    ordering.add_argument("--after-id", help="Insert or move the item after the given sibling item id.")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Deterministic local helper CLI for `.planr` plan and status files.")
    parser.add_argument("--repo-root", help="Repo root containing `.planr/`. Defaults to the current repo root.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    project_parser = subparsers.add_parser("project", help="Deterministic `.planr/project` helpers.")
    project_subparsers = project_parser.add_subparsers(dest="project_command", required=True)

    project_init = project_subparsers.add_parser(
        "init",
        help="Create or refresh the `.planr/project` starter pack and canonical project-context metadata.",
    )
    project_init.add_argument(
        "--force",
        action="store_true",
        help="Overwrite existing `.planr/project/*.md` files instead of preserving customized content.",
    )
    project_init.set_defaults(func=cmd_project_init)

    plan_parser = subparsers.add_parser("plan", help="Deterministic `.planr/plans` helpers.")
    plan_subparsers = plan_parser.add_subparsers(dest="plan_command", required=True)

    plan_path_parser = plan_subparsers.add_parser(
        "path",
        help="Compute the deterministic `.planr/plans/*.plan.md` path without writing scaffold content.",
    )
    plan_path_parser.add_argument("--title", required=True, help="Display title for the plan.")
    plan_path_parser.add_argument("--slug", help="Optional explicit filename slug. Defaults to a slugified title.")
    plan_path_parser.set_defaults(func=cmd_plan_path)

    plan_new = plan_subparsers.add_parser("new", help="Create a deterministic `.planr/plans/*.plan.md` scaffold.")
    plan_new.add_argument("--title", required=True, help="Display title for the plan.")
    plan_new.add_argument("--overview", required=True, help="One-paragraph overview for the plan frontmatter.")
    plan_new.add_argument("--todo", action="append", default=[], help="Todo entry in the form `id=content`.")
    plan_new.add_argument("--slug", help="Optional explicit filename slug. Defaults to a slugified title.")
    plan_new.add_argument("--project", dest="is_project", action="store_true", help="Set `isProject: true`.")
    plan_new.add_argument("--force", action="store_true", help="Overwrite an existing deterministic output path.")
    plan_new.set_defaults(func=cmd_plan_new)

    plan_archive = plan_subparsers.add_parser(
        "archive",
        help="Archive completed scope plan files into `.planr/plans/done/DD-MM/` and rewrite live scope paths.",
    )
    plan_archive.add_argument("--scope", required=True, help="Completed scope id whose plan paths should be archived.")
    plan_archive.add_argument(
        "--archive-date",
        help="Optional local archive date override in YYYY-MM-DD for deterministic tests or backfills.",
    )
    plan_archive.set_defaults(func=cmd_plan_archive)

    status_parser = subparsers.add_parser("status", help="Deterministic `.planr/status/current.json` helpers.")
    status_subparsers = status_parser.add_subparsers(dest="status_command", required=True)

    status_show = status_subparsers.add_parser("show", help="Print the current status JSON or one scope.")
    status_show.add_argument("--scope", help="Optional scope id to print only one scope.")
    status_show.set_defaults(func=cmd_status_show)

    status_open = status_subparsers.add_parser(
        "open",
        help="List open scopes, including closed scopes whose subordinate state still indicates unfinished work.",
    )
    status_open.set_defaults(func=cmd_status_open)

    status_next = status_subparsers.add_parser(
        "next",
        help="Select the next deterministic open scope, preferring actionable in-progress or pending work.",
    )
    status_next.set_defaults(func=cmd_status_next)

    ensure_scope = status_subparsers.add_parser("ensure-scope", help="Create or update a scope entry.")
    ensure_scope.add_argument("--id", required=True, help="Scope id.")
    ensure_scope.add_argument("--title", help="Scope title.")
    ensure_scope.add_argument("--status", choices=sorted(VALID_SCOPE_STATUSES), help="Scope status.")
    ensure_scope.add_argument("--source", help="Scope source note.")
    plan_paths_group = ensure_scope.add_mutually_exclusive_group()
    plan_paths_group.add_argument(
        "--clear-plan-paths",
        action="store_true",
        help="Clear the stored `plan_paths` list before any other scope updates.",
    )
    plan_paths_group.add_argument("--plan-path", action="append", help="Plan path to store on the scope. Repeatable.")
    owned_paths_group = ensure_scope.add_mutually_exclusive_group()
    owned_paths_group.add_argument(
        "--clear-owned-paths",
        action="store_true",
        help="Clear the stored `owned_paths` list before any other scope updates.",
    )
    owned_paths_group.add_argument("--owned-path", action="append", help="Owned path to store on the scope. Repeatable.")
    ensure_scope.set_defaults(func=cmd_status_ensure_scope)

    delete_scope = status_subparsers.add_parser("delete-scope", help="Delete one scope entry.")
    delete_scope.add_argument("--id", required=True, help="Scope id.")
    delete_scope.set_defaults(func=cmd_status_delete_scope)

    set_checklist = status_subparsers.add_parser("set-checklist", help="Create or update one checklist item.")
    set_checklist.add_argument("--scope", required=True, help="Scope id.")
    set_checklist.add_argument("--item-id", required=True, help="Checklist item id.")
    set_checklist.add_argument("--content", help="Checklist item content. Required when creating a new item.")
    set_checklist.add_argument("--status", required=True, choices=sorted(VALID_CHECKLIST_STATUSES), help="Checklist item status.")
    add_relative_order_arguments(set_checklist)
    set_checklist.set_defaults(func=cmd_status_set_checklist)

    delete_checklist = status_subparsers.add_parser("delete-checklist", help="Delete one checklist item.")
    delete_checklist.add_argument("--scope", required=True, help="Scope id.")
    delete_checklist.add_argument("--item-id", required=True, help="Checklist item id.")
    delete_checklist.set_defaults(func=cmd_status_delete_checklist)

    set_blocker = status_subparsers.add_parser("set-blocker", help="Create or update one blocked or unverified item.")
    set_blocker.add_argument("--scope", required=True, help="Scope id.")
    set_blocker.add_argument("--item-id", required=True, help="Blocked or unverified item id.")
    set_blocker.add_argument("--content", help="Blocked or unverified item content. Required when creating a new item.")
    set_blocker.add_argument("--status", required=True, choices=sorted(VALID_BLOCKER_STATUSES), help="Blocked or unverified item status.")
    add_relative_order_arguments(set_blocker)
    set_blocker.set_defaults(func=cmd_status_set_blocker)

    delete_blocker = status_subparsers.add_parser(
        "delete-blocker",
        help="Delete one blocked or unverified item.",
    )
    delete_blocker.add_argument("--scope", required=True, help="Scope id.")
    delete_blocker.add_argument("--item-id", required=True, help="Blocked or unverified item id.")
    delete_blocker.set_defaults(func=cmd_status_delete_blocker)

    set_verification = status_subparsers.add_parser("set-verification", help="Create or update one verification record.")
    set_verification.add_argument("--scope", required=True, help="Scope id.")
    set_verification.add_argument("--verification-id", required=True, help="Verification record id.")
    set_verification.add_argument("--status", required=True, choices=sorted(VALID_VERIFICATION_STATUSES), help="Verification status.")
    set_verification.add_argument("--result", required=True, help="Human-readable verification result.")
    set_verification.add_argument("--command", help="Optional exact command string for the verification record.")
    add_relative_order_arguments(set_verification)
    set_verification.set_defaults(func=cmd_status_set_verification)

    delete_verification = status_subparsers.add_parser("delete-verification", help="Delete one verification record.")
    delete_verification.add_argument("--scope", required=True, help="Scope id.")
    delete_verification.add_argument("--verification-id", required=True, help="Verification record id.")
    delete_verification.set_defaults(func=cmd_status_delete_verification)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        root = resolve_repo_root(args.repo_root)
        return args.func(args, root)
    except CliError as exc:
        parser.exit(2, f"error: {exc}\n")


if __name__ == "__main__":
    sys.exit(main())
