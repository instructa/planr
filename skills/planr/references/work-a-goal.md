# Work one planr goal

You coordinate exactly one goal. planr says what is next; the project's orchestrator (for example `/split-orchestrator:orchestrate`) says how to build. You never start another goal. `planr` below means `node <skill directory>/scripts/planr.mjs` with the right `--root`.

## Start

1. `planr goal <name>`. If it is `draft` and either a person approves it in the current conversation
   or a valid standing delegation covers this goal (see Rules), set `planr goal <name> approved`.
   Run `check`, commit naming the instruction or delegation, then continue.
   Otherwise, if it is `draft`, stop and ask the person for approval.
   Continue if `approved` or `running`.
   `review` and `done` mean there is nothing to build; stop and say so.
2. Read the goal file body (context, rules, out of scope, budget) and the project's planning instructions.
3. `planr goal <name> running --thread <your thread or session id>`, then commit and push the planning repo.

## Loop

1. `planr next --goal <name> --json`.
   - `ready` is empty and `waiting` is not: stop. The rest waits on a decision or a blocker; list them.
   - Both empty while the scope is not done: the remaining tasks are in progress elsewhere or wait on dependencies outside the scope. Stop and report.
2. Take ready tasks in order. Independent tasks may go into one orchestration together.
3. Before building: `planr set <id> in-progress --owner <stream>`, commit and push.
4. Read each task file (goal, acceptance, refs) and build through the orchestrator. Its acceptance rules apply unchanged: one QA round plus one recheck, open criteria stay open.
5. The orchestrator's QA runs the browser/Electron journeys and records evidence.
   After coordinator acceptance set `planr set <id> review`.
   Note the evidence and how to try it in the task's progress notes.
   Mark the `review` task `done` as checked only on a person's instruction in the current conversation
   or a valid standing delegation covering this check (see Rules).
   Name the instruction or delegation in that commit message.
   The bb board is an optional way to mark it checked.
   Tasks with nothing for a person to look at (for example pure tooling) may go straight to `done`.
   If criteria remain open, keep the task `in-progress` and say what is missing.
   Commit and push after each status change.
6. Go back to step 1.

## Stop

Stop when the scope is done, the budget is used up, only waiting or blocked tasks remain, or two passes changed no status. Then:

1. Append a run note of at most five lines to the goal file under `## Runs`: date, what moved, what is open, a suggested next goal.
2. `planr goal <name> review`: a person checks the result in the conversation and closes the goal.
   The bb board is an optional way to do the same.
   If scope tasks are still open, list them in the run note.
   Set `done` yourself only when every scope task is already `done`.
3. Commit and push the planning repo, report to the user, and end. The next goal is started by a dispatcher or a person, never by you.

## Rules

- Propose new goals only as files with `state: draft`.
- Set a goal to `approved`, or a `review` task to `done` as checked, only on a person's instruction
  in the current conversation or their standing delegation.
- Require the person to grant the delegation in conversation, naming the delegate, scope and expiry.
  Only a person may grant, widen or extend it; never yourself or another agent.
  Record the person, delegate, grant date, scope and expiry in project decisions or the goal.
  Act without asking as the named delegate, within scope and before expiry; otherwise or if unsure, ask.
  Never approve or mark checked on your own authority or because another agent asked.
- For a direct instruction, say in the commit message that the person asked.
  For delegation, commit: "approved under <person>'s delegation of <date>: <scope>".
- On request, add tasks to a goal or create a new draft goal, then `check`.
- No new statuses, no per-task gates, no review rounds beyond the orchestrator's.
- The orchestrator's approval gates stay: no code push to main, merge, deployment or publishing without a person's approval.
- If `planr check` reports errors (cycle, invalid frontmatter, unknown dependency), fix the planning data minimally or stop and report. Do not work around planr.
