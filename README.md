# planr

A small planning layer for agent-built products, using Markdown files in Git.

When several coding agents work on one product, planr shows what is ready,
blocked, waiting on you & what changed. Tasks & their dependencies live in a
planning repository, usually separate from your code & often private.
No database, daemon or MCP server.

planr says what is next. Your project's orchestrator, such as
[split-orchestrator](https://github.com/regenrek/split-orchestrator), handles the build.

## Get started

Paste this prompt into your coding agent. It checks Node.js & Git, asks where to keep the plan
& sets up the planning files & plugin.

```text
Set up planr for this project. Follow https://planr.so/docs/setup.md
```

To set it up yourself, follow the [quickstart](docs/quickstart.md). planr needs Node.js 20 or
newer & Git. The CLI has no runtime dependencies.

## Plugins & CLI

- [Claude Code](docs/plugins/claude-code.md). Call the skill with `/planr:planr`.
- [Codex](docs/plugins/codex.md). Call the skill with `$planr:planr`.
- [bb](docs/plugins/bb.md). See the board beside your threads & approve or check work.
- Other agents. Install the skill with `npx skills add instructa/planr`.
- [CLI](docs/cli.md). Check files, find ready work & update status. Install it with `npm install -g planr`
  or use `npx planr`.

## More detail

- [Concepts](docs/concepts.md) for how the plan, tasks, goals & approvers fit together.
- [The board](docs/board.md) for progress & history.
- [File format](skills/planr/references/format.md) & [technical contract](docs/contract.md).

## License

[MIT](LICENSE). Bundled fonts use the SIL Open Font License 1.1.
See the [font licenses](skills/planr/assets/board/fonts/).
