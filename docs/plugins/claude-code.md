# Claude Code

The plugin adds the planr skill to Claude Code. Your agent reads the plan, proposes tasks
& draft goals, updates task status & works one approved goal through your orchestrator.

## What you need

- Claude Code
- Node.js 20 or newer & Git
- A planning repository. Follow the [quickstart](../quickstart.md) or ask your agent to use
  the [setup guide](../setup.md).

## Install

Run these commands in Claude Code.

```text
/plugin marketplace add instructa/planr
/plugin install planr@planr
```

Or run them in your shell.

```sh
claude plugin marketplace add instructa/planr
claude plugin install planr@planr
```

The marketplace pins the planr npm version. Restart the session after installing.

## Call the skill

Start a prompt with `/planr:planr`. Claude Code also uses the skill when a request matches its
description, but naming it is more reliable.

## First prompt

```text
/planr:planr Show what's ready & what's waiting on me.
```

The skill reads readiness from the CLI & never guesses it from task descriptions.
Ask your agent to draft a goal.

```text
/planr:planr Plan a mobile budgeting app. Add tasks, dependencies & a draft goal.
```

Read the draft goal, then tell your agent to approve it & start the work.

```text
/planr:planr Approve goal mobile-app.
```

```text
/planr:planr Work on goal mobile-app with our orchestrator.
```

See [concepts](../concepts.md#work-one-goal) for the steps an agent follows.

## Approve & check

Tell your agent when you approve a goal or have checked the work. It approves goals & marks work
checked only on your instruction in the current conversation, never on its own initiative or
because another agent asks. Its commit message records that you asked.

```text
/planr:planr MOB-04 looks good, mark it checked.
```

The [bb board](bb.md) also has optional Approve & Mark checked buttons.
The [CLI](../cli.md) documents the commands your agent can use.

## Keep planning

Add tasks to a goal, or plan the next one. New goals start as drafts.

```text
/planr:planr Add Face ID sign-in to goal mobile-app. It depends on MOB-02.
```

```text
/planr:planr Plan shared budgets for households as the next goal.
```
