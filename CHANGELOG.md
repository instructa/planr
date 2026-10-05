# Changelog

This file follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/).

## [2.0.0-beta.1] - 2026-10-05

### Changed

- Rebuilt planr around a Markdown task graph in a planning repository.
- Added a Node.js CLI with no runtime dependencies, an Agent Skill, a standalone board & a bb plugin.
- Added plugins for Claude Code & Codex through the planr marketplaces. Both contain only the skill.
- Approve & check by prompt or a person's standing delegation.
- Removed the stale marker from the engine and boards.
- planr 2.0 does not read 1.x data. Version 1.x remains installable as `planr@1`.
