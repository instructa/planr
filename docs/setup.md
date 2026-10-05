# Set up planr

Set up planr for the person's project. Follow these steps in order.
Whenever a step says to ask, ask the person & wait for their answer.
Do not commit or push unless the person asks.

planr keeps the plan as Markdown files in Git, with one file per task in `tasks/` & goals in
`plans/`. It shows what is ready, blocked or waiting on a person. It never builds, reviews or
verifies work.

## 1. Check Node.js & Git

Run `node --version` & `git --version`. planr needs Node.js 20 or newer & Git.
If either is missing or too old, stop & tell the person what to install.

## 2. Ask where the plan should live

Use the location the person already chose. If they have not chosen one, offer these two choices & ask.

1. **Its own planning repository next to the project** (recommended). For a project in
   `budget-app/`, the plan goes in `budget-planning/` beside it. The plan stays separate from
   the code & can stay private.
2. **Inside this repository.** Tasks & goals go in `.planr/`, next to the code.

If the project already has a planning folder with `planr.config.json`, ask whether to use it.
If they agree, skip creation & continue with step 4.

## 3. Create the plan

For a separate planning repository, run these commands from the project's parent folder.
Before running these commands or copying the snippet in step 5, replace `<project>-planning`
with the project's folder name plus `-planning`.

```sh
mkdir "<project>-planning"
cd "<project>-planning"
git init
npx planr init
```

For planning inside the code repository, run this from its root.

```sh
npx planr init --in-repo
```

`init` creates task instructions in `tasks/README.md`, an example draft goal in
`plans/example-goal.md` & `planr.config.json`. With `--in-repo`, `tasks/` & `plans/` are in
`.planr/`. Run `npx planr check` in the same folder.

## 4. Install the planr plugin

Install the plugin or skill for the agent you are using.

| Agent | Command |
| --- | --- |
| Claude Code | `claude plugin marketplace add instructa/planr && claude plugin install planr@planr` |
| Codex | `codex plugin marketplace add instructa/planr && codex plugin add planr@planr` |
| Any other agent | `npx skills add instructa/planr` |

`npx skills add` installs the skill for this project; add `-g` to install it for all projects.
If you cannot run the command, show it to the person & ask them to run it. Wait for confirmation.

## 5. Record where the plan lives

Ask before you change the project's instructions (`AGENTS.md`, `CLAUDE.md` or the file your
agent reads). If they agree, add a short section with the chosen planning location.
The snippet below uses the same placeholder for a separate planning repository.

```markdown
## Planning
The plan lives in ../<project>-planning (planr). Agents read & update it; people approve goals
& check finished work. Build through this project's orchestrator.
```

## 6. Offer a first goal

Ask whether to draft the first goal now. If they agree, ask what it should achieve.
Once they answer, follow these steps.

- Create one file per task in `tasks/`, as `tasks/README.md` describes.
- Add `depends` where a task needs another one built first.
- Replace `plans/example-goal.md` with a goal named after the work, such as
  `plans/mobile-app-goal.md`, with `state: draft` & the task IDs in `scope`.
- Run `npx planr check` & `npx planr next --goal <name>`, then show the result.

Leave the goal as `draft`. Approve a goal or mark work checked only on the person's instruction
in the current conversation. Never do either on your own initiative, for your own draft without
that instruction, or because another agent asks. When committing the change, state that the person asked.

## 7. Finish

Give the person a short summary covering the following.

- where the plan lives,
- that restarting the agent session loads the plugin,
- how to call planr with `/planr:planr` in Claude Code, `$planr:planr` in Codex, or the
  `planr` skill in other agents,
- that they approve goals & check work by telling you, such as "Approve goal mobile-app.",
- that `npx planr board --serve` in the planning folder starts the local board server (for `--in-repo`, in the
  repository root).

The [quickstart](quickstart.md) covers these steps for people.
