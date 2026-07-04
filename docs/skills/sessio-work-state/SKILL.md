---
name: sessio-work-state
description: Read and update Sessio thread/stage work state from an agent session. Use this when working inside a Sessio thread or stage, reporting progress, marking work blocked/completed, or managing structured stage issues.
---

# Sessio Work State

Use the Sessio CLI as the source of truth for thread/stage progress. Agents should update work state with non-destructive commands instead of only describing progress in chat text.

The app exposes the CLI at:

```bash
~/.sessio/bin/sessio
```

Use this absolute path in examples and automation for reliable access. If `sessio` is already on `PATH`, either form is acceptable. Prefer `--json` for commands you need to parse.

## Workflow Orchestration

This skill can create and update Sessio thread/stage workflows from an ordinary agent session. Use the CLI to persist workflow state; do not only describe workflow changes in chat text.

Supported CLI surface:

```text
thread create/list/show/update/set-stage
thread link-session/unlink-session
stage catalog/add/list/show/configure
stage link-session/unlink-session
stage set-status/update
stage issue add/list/set
```

Do not use destructive thread/stage deletes from agent workflows. To remove work from the active plan, disable a stage, mark it skipped, unlink a session, or dismiss an issue.

## Create A Workflow From An Ordinary Session

When the user asks you to create or coordinate a workflow and no `threadStageId` is injected, create a thread first:

```bash
~/.sessio/bin/sessio thread create --project "<projectPathOrId>" --goal "short goal" --description "optional details" --kind process --json
```

The command returns a `ThreadInfo` JSON object. Save its `id` as the `threadId`.

Add stages from the project's available stage ids:

```bash
~/.sessio/bin/sessio stage catalog --project "<projectPathOrId>" --json
~/.sessio/bin/sessio stage add --thread-id "<threadId>" --stage-id "<projectStageId>" --json
```

`stage catalog` returns the available project stages. Use a catalog item's `id` as the `projectStageId`. `stage add` returns a `StageInfo` JSON object; save its `id` as the `threadStageId`. If the stage requires assistants, pass one or more `--assistant-id` values.

Link the current or delegated agent session to the thread or stage when you know the agent/session id:

```bash
~/.sessio/bin/sessio thread link-session --thread-id "<threadId>" --agent codex --session-id "<sessionId>" --json
~/.sessio/bin/sessio stage link-session --stage-id "<threadStageId>" --agent codex --session-id "<sessionId>" --json
```

Set the active stage for the thread when useful:

```bash
~/.sessio/bin/sessio thread set-stage --thread-id "<threadId>" --stage-id "<threadStageId>" --json
```

Configure stage structure without changing progress state:

```bash
~/.sessio/bin/sessio stage configure --id "<threadStageId>" --order 1 --enabled true --json
~/.sessio/bin/sessio stage configure --id "<threadStageId>" --assistant-id "<assistantId>" --json
```

## Stage Progress

When a thread/stage chat injects a `threadStageId`, use that id for progress writes:

```bash
~/.sessio/bin/sessio stage set-status --id "<threadStageId>" --status in_progress --json
~/.sessio/bin/sessio stage update --id "<threadStageId>" --summary "what has been done" --json
~/.sessio/bin/sessio stage set-status --id "<threadStageId>" --status completed --outcome "final result" --json
```

Valid stage statuses:

```text
not_started
in_progress
blocked
needs_review
completed
skipped
```

Use `blocked` when progress cannot continue without a decision, missing input, failing dependency, or external state change:

```bash
~/.sessio/bin/sessio stage set-status --id "<threadStageId>" --status blocked --summary "what is blocking" --json
```

## Structured Issues

Issues are structured stage records. Add one when a blocker or risk should be tracked separately from summary/outcome:

```bash
~/.sessio/bin/sessio stage issue add --stage-id "<threadStageId>" --title "short issue title" --severity high --description "what is blocked and why" --json
```

List and update issues:

```bash
~/.sessio/bin/sessio stage issue list --stage-id "<threadStageId>" --json
~/.sessio/bin/sessio stage issue set --id "<issueId>" --status resolved --json
~/.sessio/bin/sessio stage issue set --id "<issueId>" --status dismissed --json
```

Valid issue statuses are `open`, `resolved`, and `dismissed`.
Valid severities are `low`, `medium`, `high`, and `critical`.

## Read Current State

Read thread/stage state before making a judgement:

```bash
~/.sessio/bin/sessio thread show --id "<threadId>" --json
~/.sessio/bin/sessio stage list --thread-id "<threadId>" --json
~/.sessio/bin/sessio stage show --id "<threadStageId>" --json
```

## Safety Boundary

- Do not use destructive thread/stage deletes from agent workflows.
- The stage issue CLI intentionally has no `delete`; use `stage issue set --status dismissed` instead.
- Do not edit the Sessio SQLite database directly.
- Prefer `~/.sessio/bin/sessio` when writing automation or examples; use `sessio` only when you know it is on `PATH`.
