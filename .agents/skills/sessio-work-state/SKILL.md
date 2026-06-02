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

If `sessio` is already on `PATH`, either form is acceptable. Prefer `--json` for commands you need to parse.

## Stage Progress

When a thread/stage chat injects a `threadStageId`, use that id for progress writes:

```bash
sessio stage set-status --id "<threadStageId>" --status in_progress --json
sessio stage update --id "<threadStageId>" --summary "what has been done" --json
sessio stage set-status --id "<threadStageId>" --status completed --outcome "final result" --json
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
sessio stage set-status --id "<threadStageId>" --status blocked --summary "what is blocking" --json
```

## Structured Issues

Issues are structured stage records. Add one when a blocker or risk should be tracked separately from summary/outcome:

```bash
sessio stage issue add --stage-id "<threadStageId>" --title "short issue title" --severity high --description "what is blocked and why" --json
```

List and update issues:

```bash
sessio stage issue list --stage-id "<threadStageId>" --json
sessio stage issue set --id "<issueId>" --status resolved --json
sessio stage issue set --id "<issueId>" --status dismissed --json
```

Valid issue statuses are `open`, `resolved`, and `dismissed`.
Valid severities are `low`, `medium`, `high`, and `critical`.

## Read Current State

Read thread/stage state before making a judgement:

```bash
sessio thread show --id "<threadId>" --json
sessio stage list --thread-id "<threadId>" --json
sessio stage show --id "<threadStageId>" --json
```

## Safety Boundary

- Do not use destructive thread/stage deletes from agent workflows.
- The stage issue CLI intentionally has no `delete`; use `stage issue set --status dismissed` instead.
- Do not edit the Sessio SQLite database directly.
- If a command fails because the CLI cannot be found, try `~/.sessio/bin/sessio`.
