# Thread/Stage Work-State: Target Architecture and MVP Execution Plan

## Purpose

This document fuses the two existing thread/stage plans:

- `thread-stage-chat-snapshot-plan.md`: stronger target architecture, with explicit stage state, issues, `ThreadWorkSnapshot`, and detail/source APIs.
- `thread-chat-stage-scalable-stonebraker.md`: stronger execution skeleton, with concrete phases, known code paths, and the existing CLI/session snapshot mechanics.

The product direction is: a Sessio thread is a workflow container, each stage has real maintainable state, agents can update that state through Sessio CLI, and new thread/stage chats receive a frozen work-state snapshot as context.

## Target Architecture

Thread/stage state should be first-class workflow data, not inferred UI decoration.

- `threads.stage_id` remains the active/current focus pointer.
- Stage progress is stored separately and explicitly.
- Stage issues/blockers are structured records, not only freeform notes.
- Chat excerpts remain in `session_history_snapshots`.
- Overall thread/stage situation is stored in `thread_work_snapshots`.
- Details are reached through source indexes and focused APIs, not by embedding all raw detail in every response.

Recommended stage statuses:

- `not_started`
- `in_progress`
- `blocked`
- `needs_review`
- `completed`
- `skipped`

Recommended issue statuses:

- `open`
- `resolved`
- `dismissed`

Recommended issue severities:

- `low`
- `medium`
- `high`
- `critical`

Core data model:

```sql
thread_stage_states(
  thread_stage_id TEXT PRIMARY KEY,
  status TEXT NOT NULL,
  summary TEXT,
  outcome TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
)
```

```sql
thread_stage_issues(
  id TEXT PRIMARY KEY,
  thread_stage_id TEXT NOT NULL,
  title TEXT NOT NULL,
  description TEXT,
  status TEXT NOT NULL,
  severity TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
)
```

```sql
thread_work_snapshots(
  child_agent TEXT NOT NULL,
  child_session_id TEXT NOT NULL,
  thread_id TEXT NOT NULL,
  stage_id TEXT,
  snapshot_json TEXT NOT NULL,
  version INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(child_agent, child_session_id)
)
```

## Public Interfaces

Tauri/frontend APIs:

- `get_thread_work_state(threadId)`
- `update_thread_stage_state(threadStageId, patch)`
- `create_thread_stage_issue(threadStageId, input)`
- `update_thread_stage_issue(issueId, patch)`
- `delete_thread_stage_issue(issueId)`
- `save_thread_work_snapshot(childAgent, childSessionId, snapshot)`
- `get_thread_work_snapshot(childAgent, childSessionId)`
- `get_thread_work_snapshot_sources(childAgent, childSessionId)`

CLI APIs for agents:

- `sessio thread list --project <path> --json`
- `sessio thread show --id <threadId> --json`
- `sessio stage list --thread-id <threadId> --json`
- `sessio stage show --id <threadStageId> --json`
- `sessio stage set-status --id <threadStageId> --status <status> --json`
- `sessio stage update --id <threadStageId> [--status <status>] [--summary <text>] [--outcome <text>] --json`
- `sessio stage issue list --stage-id <threadStageId> --json`
- `sessio stage issue add --stage-id <threadStageId> --title <text> [--description <text>] [--severity <level>] --json`
- `sessio stage issue set --id <issueId> [--status <status>] [--severity <level>] [--title <text>] [--description <text>] --json`

MVP CLI should not expose destructive thread/stage deletion to agents. Keep destructive CRUD in the GUI/API unless a later permission model is added.

## MVP Execution Skeleton

### Phase 1: Stage State and Issues

Add explicit state and issue persistence first. This replaces the old UI-only inference where previous/current/future stages are derived from `threads.stage_id` and order.

- Add Rust/TS models for stage status, stage state, and stage issue.
- Add store methods and SQLite migrations.
- Keep old threads readable by deriving defaults:
  - stages before active stage -> `completed`
  - active stage -> `in_progress`
  - stages after active stage -> `not_started`
  - no active stage -> all `not_started`
- Surface state in `StageInfo` or in `get_thread_work_state`, depending on implementation ergonomics.
- Add minimal UI controls for status, summary, outcome, and issues in thread/stage detail.

Implementation should reuse Stonebraker's code-path awareness:

- `models.rs` for public Rust models.
- `store/mod.rs`, `store/sqlite.rs`, `store/cached.rs` for store contracts.
- `lib.rs` for Tauri commands and `threads_updated` emission.
- `api.ts`, `ThreadPage.tsx`, and `ProjectPage.tsx` for frontend integration.

### Phase 2: CLI State Updates

Extend the existing handwritten CLI parser with `thread` and `stage` command groups.

- Reuse the same SQLite store as the app.
- Support `--json` for every read/write command.
- Start with non-destructive commands:
  - thread/stage show/list
  - stage status/summary/outcome update
  - issue add/list/set
- Return updated stage or issue objects from write commands.
- Make the app binary reachable from agents through a stable path such as `~/.sessio/bin/sessio`.
- Add a Sessio skill or project instruction telling agents how to read and update their stage.

### Phase 3: Work-State Snapshot for New Chats

Add thread/stage chat context after state and CLI are real.

- New thread chat links to thread and active stage.
- New stage chat links to thread and selected stage.
- Agent context includes:
  - `threadId`
  - `threadStageId`
  - stage status
  - summary/outcome
  - open issues
  - linked session refs
  - CLI examples for progress updates
- Save broad work-state envelope to `thread_work_snapshots`.
- Save chat excerpt groups to existing `session_history_snapshots`.
- Keep snapshot display stable even if original chat files are later unavailable.

Example agent context:

```text
You are working in Sessio thread stage <threadStageId>.

When you begin work:
sessio stage set-status --id <threadStageId> --status in_progress --json

When complete:
sessio stage set-status --id <threadStageId> --status completed --json

If blocked:
sessio stage set-status --id <threadStageId> --status blocked --json
sessio stage issue add --stage-id <threadStageId> --title "..." --severity high --json
```

### Phase 4: Detail Sources and Polish

Round out the UX once the loop works.

- Add a source/details view for work snapshots.
- `get_thread_work_snapshot_sources` returns labels and refs for thread, stages, issues, linked sessions, file paths, and excerpt group indexes.
- Let users drill into original sessions with existing `getSessionHistory`.
- Improve rollups: completed count, blocked count, open issue count, current stage label.

## Tradeoffs and Decisions

- Do not use only `note`: it is fast, but multiple blockers, severity, resolution, filtering, and auditability all become fragile.
- Do not use only `session_history_snapshots`: they are good for chat excerpts, but semantically wrong for durable workflow state.
- Do not let agents perform full destructive CRUD in v1: status and issue writes are enough for the agent workflow loop and safer.
- Keep `threads.stage_id`: it is still useful as a focus pointer and preserves existing stage activation behavior.
- Keep `enabled` separate from status: enabled controls whether a stage participates; status describes progress.
- Prefer explicit status over inference: inference is acceptable only for migration/defaulting.

## Acceptance Criteria

- A user can set a stage to `blocked` or `completed` in the GUI.
- An agent can run `sessio stage set-status --id <threadStageId> --status completed --json` and the GUI reflects it after refresh.
- An agent can add an issue through CLI and the issue appears under that stage.
- New thread/stage chats include work-state context and CLI instructions.
- A saved snapshot preserves stage statuses, issues, summary/outcome, linked sessions, and source refs.
- Snapshot overview still renders if an original session file disappears.
- Source/detail view can lead back to original sessions, stages, and issues when available.

## Implementation Stance

Use the Work-State plan as the target architecture and the Stonebraker plan as the execution map. The MVP should be small enough to land safely but should not paint the data model into a corner.

The first useful milestone is:

1. Persistent stage state and issues.
2. CLI updates for agents.
3. Thread/stage chat context that includes `threadStageId` and CLI examples.

Dedicated `ThreadWorkSnapshot` storage and source-index detail APIs can follow immediately after, but the schema and interfaces should already be shaped for them.
