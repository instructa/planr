# Quickstart

Set up planr for one project, then plan your first goal. You need Node.js 20 or newer, Git &
a coding agent.

Choose your agent, who sets it up & where the plan lives. On planr.so, only the steps for
your choices appear. On GitHub, all steps are listed.

<!-- show: setup=agent -->
## Let your agent set it up

Paste the prompt below into your agent. It checks Node.js & Git, creates the plan, installs
planr & offers to draft a goal. It asks before changing your project's instructions.
Restart the agent session after setup to load the plugin.
The agent follows the [setup guide](setup.md).

<!-- show: where=own -->
### Plan in its own repository

```text
Set up planr for this project with the plan in its own repository. Follow https://planr.so/docs/setup.md
```

<!-- show: where=in-repo -->
### Plan inside your code repository

```text
Set up planr for this project with the plan inside this repository. Follow https://planr.so/docs/setup.md
```

<!-- show: setup=you -->
## Set it up yourself

### 1. Create the plan

`init` creates task instructions in `tasks/README.md`, an example draft goal & `planr.config.json`.

<!-- show: where=own -->
#### In its own repository

Keep the plan beside your project, for example `budget-planning/` next to `budget-app/`.
This is the recommended setup & lets the plan stay private.

```sh
mkdir budget-planning
cd budget-planning
git init
npx planr init
```

<!-- show: where=in-repo -->
#### Inside your code repository

Run this from the root of your code repository. Tasks & goals go in `.planr/`.

```sh
npx planr init --in-repo
```

### 2. Install the plugin

Install planr for your agent, then restart the session to load it.

<!-- show: agent=claude-code -->
#### Claude Code

Run these commands in Claude Code.

```text
/plugin marketplace add instructa/planr
/plugin install planr@planr
```

<!-- show: agent=codex -->
#### Codex

Run these commands in your shell.

```sh
codex plugin marketplace add instructa/planr
codex plugin add planr@planr
```

<!-- show: agent=bb -->
#### bb

A bb thread runs Claude Code or Codex. Install the planr plugin for the agent your threads use
([Claude Code](plugins/claude-code.md), [Codex](plugins/codex.md)), then add the
[bb plugin](plugins/bb.md) for the board beside your threads.

<!-- show: agent=other -->
#### Other agents

Run this from your project folder in your shell.

```sh
npx skills add instructa/planr
```

This installs the planr skill for agents in this project. Add `-g` for all projects.

### 3. Tell your agent where the plan lives

In your project's instructions (`AGENTS.md`, `CLAUDE.md` or the file your agent reads), name
the planning location, who approves goals & which orchestrator builds the product.
Agents need access to both the plan & the code. With `--in-repo`, the plan lives in `.planr/`.

```markdown
## Planning
The plan lives in ../budget-planning (planr). Agents read & update it; people approve goals
& check finished work. Build through this project's orchestrator.
```

## Plan your first goal

Ask your agent for a plan. It writes a file for each task, adds dependencies & proposes a draft
goal. Tell your agent when you approve the goal or have checked the work. Your orchestrator
handles the build. The prompts use the sample goal `mobile-app`; use your own goal's name.

<!-- show: agent=claude-code -->
### Prompts in Claude Code

```text
/planr:planr Plan a mobile budgeting app. Add tasks, dependencies & a draft goal.
```

Read the draft goal, then tell your agent to approve it.

```text
/planr:planr Approve goal mobile-app.
```

Ask your agent to start.

```text
/planr:planr Work on goal mobile-app with our orchestrator.
```

Ask what is ready & what needs you.

```text
/planr:planr Show what's ready & what's waiting on me.
```

Check the work, then ask your agent to mark it checked.

```text
/planr:planr MOB-04 looks good, mark it checked.
```

Add tasks to a goal, or plan the next one.

```text
/planr:planr Add Face ID sign-in to goal mobile-app. It depends on MOB-02.
```

```text
/planr:planr Plan shared budgets for households as the next goal.
```

<!-- show: agent=codex -->
### Prompts in Codex

```text
$planr:planr Plan a mobile budgeting app. Add tasks, dependencies & a draft goal.
```

Read the draft goal, then tell your agent to approve it.

```text
$planr:planr Approve goal mobile-app.
```

Ask your agent to start.

```text
$planr:planr Work on goal mobile-app with our orchestrator.
```

Ask what is ready & what needs you.

```text
$planr:planr Show what's ready & what's waiting on me.
```

Check the work, then ask your agent to mark it checked.

```text
$planr:planr MOB-04 looks good, mark it checked.
```

Add tasks to a goal, or plan the next one.

```text
$planr:planr Add Face ID sign-in to goal mobile-app. It depends on MOB-02.
```

```text
$planr:planr Plan shared budgets for households as the next goal.
```

<!-- show: agent=bb -->
### Prompts in bb

In a bb thread, use `/planr:planr` for Claude Code or `$planr:planr` for Codex.
You can also ask in plain words.

```text
Use planr to plan a mobile budgeting app. Add tasks, dependencies & a draft goal.
```

Read the draft goal, then tell your agent to approve it.

```text
Approve goal mobile-app in planr.
```

Ask your agent to start.

```text
Use planr to work on goal mobile-app with our orchestrator.
```

Ask what is ready & what needs you.

```text
Use planr to show what's ready & what's waiting on me.
```

Check the work, then ask your agent to mark it checked.

```text
MOB-04 looks good, mark it checked in planr.
```

Add tasks to a goal, or plan the next one.

```text
Use planr to add Face ID sign-in to goal mobile-app. It depends on MOB-02.
```

```text
Use planr to plan shared budgets for households as the next goal.
```

<!-- show: agent=other -->
### Prompts in other agents

Ask in plain words, or use your agent's skill command, often `/planr` or `$planr`.

```text
Use planr to plan a mobile budgeting app. Add tasks, dependencies & a draft goal.
```

Read the draft goal, then tell your agent to approve it.

```text
Approve goal mobile-app in planr.
```

Ask your agent to start.

```text
Use planr to work on goal mobile-app with our orchestrator.
```

Ask what is ready & what needs you.

```text
Use planr to show what's ready & what's waiting on me.
```

Check the work, then ask your agent to mark it checked.

```text
MOB-04 looks good, mark it checked in planr.
```

Add tasks to a goal, or plan the next one.

```text
Use planr to add Face ID sign-in to goal mobile-app. It depends on MOB-02.
```

```text
Use planr to plan shared budgets for households as the next goal.
```

### Approve & check

Your agent approves a goal or marks work checked only on your instruction in the current
conversation. It must never do either on its own initiative or because another agent asks.
Its commit message records that you asked. Built & tested tasks wait in `review` until you check them.

The [bb board](plugins/bb.md) also has optional Approve & Mark checked buttons. Your agent can use
`npx planr goal mobile-app approved` & `npx planr set MOB-04 done` in the planning folder.

## Open the board

Run this in the planning folder, or in the repository root for `--in-repo`.

```sh
npx planr board --serve
```

Open <http://127.0.0.1:4173>. The board is read-only & reloads when the plan changes.
See [the board](board.md) for its views, or [concepts](concepts.md) for tasks, goals & approvers.
