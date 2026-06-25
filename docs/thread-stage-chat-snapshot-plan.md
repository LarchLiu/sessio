# Thread/Stage Work-State Snapshot Plan

## Summary

Expand the snapshot feature from "chat context" into a full thread/stage work-state snapshot. A snapshot captures the thread goal, every stage's current state, what is complete or incomplete, stage issues/blockers, related chats, and stable detail-access references.

New thread/stage chats can still receive concise context, but the stored snapshot becomes the source of truth for the thread's overall situation at that moment.

## Goals

- Preserve the overall situation of a thread or stage when a new chat is created.
- Represent stage progress explicitly instead of relying only on the current active stage pointer.
- Capture completed stages, unfinished stages, blocked stages, unresolved issues, and review needs.
- Keep detail access traceable by returning source indexes and focused detail APIs rather than duplicating every raw record.
- Continue to support chat context handoff through concise markdown context attachments and existing `session_history_snapshots`.
- Let agents update stage status and issues themselves through Sessio CLI commands while they work.

## Key Changes

- Add explicit per-thread-stage work state instead of inferring status only from stage order.
- Introduce stage statuses:
  - `not_started`
  - `in_progress`
  - `blocked`
  - `needs_review`
  - `completed`
  - `skipped`
- Add per-stage issue records with `title`, optional `description`, `status`, `severity`, and timestamps.
- Keep `threads.stage_id` as the active/current stage pointer, but do not treat it as the only completion signal.
- Rename the conceptual snapshot model from `ThreadStageChatSnapshot` to `ThreadWorkSnapshot`.
- Preserve the existing new-chat behavior, while enriching thread/stage chats with work-state context.
- Treat Sessio CLI as the stable agent-facing interface for reading thread/stage state and writing stage progress.

## Data Model

Add store-backed state for thread/stage work tracking:

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

Recommended issue statuses:

- `open`
- `resolved`
- `dismissed`

Recommended severities:

- `low`
- `medium`
- `high`
- `critical`

## APIs

Add Tauri commands and matching frontend API wrappers:

- `get_thread_work_state(threadId)`
  - Returns the current thread overview, ordered stages, stage state, issues, linked sessions, and detail refs.
- `update_thread_stage_state(threadStageId, patch)`
  - Edits status, summary, and outcome.
- `create_thread_stage_issue(threadStageId, input)`
  - Adds a stage issue/blocker.
- `update_thread_stage_issue(issueId, patch)`
  - Edits issue title, description, status, or severity.
- `delete_thread_stage_issue(issueId)`
  - Removes an issue.
- `save_thread_work_snapshot(childAgent, childSessionId, snapshot)`
  - Saves the work-state snapshot for the newly created chat session.
- `get_thread_work_snapshot(childAgent, childSessionId)`
  - Loads the stored snapshot for a chat.
- `get_thread_work_snapshot_sources(childAgent, childSessionId)`
  - Returns source indexes and detail-access references.

## CLI Surface

Sessio already has a CLI entrypoint for `sessions`, `memory`, and `config`. Extend that same binary with `thread` and `stage` command groups so supported agents can update workflow state without depending on the GUI.

All commands should support `--json` and use the same SQLite store as the desktop app.

Read commands:

- `sessio thread list --project <path> --json`
  - Lists project threads with stage rollups and active stage ids.
- `sessio thread show --id <threadId> --json`
  - Shows one thread with ordered stages, current state, issues, linked sessions, and detail refs.
- `sessio stage list --thread-id <threadId> --json`
  - Lists stages for one thread.
- `sessio stage show --id <threadStageId> --json`
  - Shows one thread stage, including state, summary, outcome, issues, assistants, and linked sessions.

State update commands:

- `sessio stage set-status --id <threadStageId> --status <not_started|in_progress|blocked|needs_review|completed|skipped> --json`
- `sessio stage set-summary --id <threadStageId> --summary <text> --json`
- `sessio stage set-outcome --id <threadStageId> --outcome <text> --json`
- `sessio stage update --id <threadStageId> [--status <status>] [--summary <text>] [--outcome <text>] --json`

Issue commands:

- `sessio stage issue list --stage-id <threadStageId> --json`
- `sessio stage issue add --stage-id <threadStageId> --title <text> [--description <text>] [--severity <low|medium|high|critical>] --json`
- `sessio stage issue set --id <issueId> [--title <text>] [--description <text>] [--status <open|resolved|dismissed>] [--severity <low|medium|high|critical>] --json`
- `sessio stage issue delete --id <issueId> --json`

CLI write commands should emit the updated stage or issue object. The GUI should pick up changes through existing refresh/event paths; no separate agent-only state should exist.

## Snapshot Contents

`ThreadWorkSnapshot` should include:

- Thread metadata:
  - thread id
  - project id
  - goal
  - description
  - active stage id
  - created/updated timestamps
- Ordered stage state:
  - thread stage id
  - project stage id
  - name/kind/icon/description
  - status
  - summary
  - outcome
  - assistants and selected agent config
  - linked session refs
  - issue refs and issue summaries
- Overall status rollup:
  - completed stage count
  - incomplete stage count
  - blocked stage count
  - open issue count
  - current stage label
- Related context:
  - recent chat excerpt groups from relevant linked sessions
  - related kanban/session refs when available
- Detail refs:
  - thread id
  - thread stage ids
  - issue ids
  - session ids
  - original file paths when known
  - `session_history_snapshots` group indexes

## Behavior

- New thread chat automatically links to the thread and the current active stage if one exists.
- New stage chat links to the thread and the selected stage.
- The context attachment sent to agents should summarize work state first, then include recent chat excerpts second.
- Missing or unreadable original chat files must not break work-state display; stored snapshot state and source indexes still render.
- Existing `session_history_snapshots` should still store excerpt groups for chat-turn continuity.
- `thread_work_snapshots` should store the broader work-state envelope, including progress, issues, and detail refs.

## Agent Context

Thread/stage chat context must tell the agent exactly where it is working and how to report progress back to Sessio.

Each thread/stage context attachment should include:

- `threadId`
- `threadStageId` when a stage is active or explicitly selected
- current stage status
- current summary and outcome
- open issues
- linked sessions used as context
- exact CLI examples for updating state

Example instructions embedded in context:

```text
You are working in Sessio thread stage <threadStageId>.
When you begin work, you may run:
sessio stage set-status --id <threadStageId> --status in_progress --json

When the stage is complete, run:
sessio stage set-status --id <threadStageId> --status completed --json

If blocked, run:
sessio stage set-status --id <threadStageId> --status blocked --json
sessio stage issue add --stage-id <threadStageId> --title "..." --severity high --json
```

Add a Sessio skill or project-level agent instruction that explains these commands. Agents should prefer CLI writes over describing status only in chat text.

## Initial Defaults

For old or unspecific threads that do not yet have explicit stage states:

- Stages before the active stage default to `completed`.
- The active stage defaults to `in_progress`.
- Stages after the active stage default to `not_started`.
- Threads with no active stage default all stages to `not_started`.
- `done` kind stages default to `completed` only when active/current or manually set.

These defaults should be materialized lazily when `get_thread_work_state(threadId)` is first called, or during schema migration if the implementation prefers eager migration.

## UI

- Thread detail should show a compact work-state overview:
  - completed / in progress / blocked / not started counts
  - ordered stage list with status badges
  - open issues per stage
  - linked chats and "view details" entry points
- Stage controls should allow editing:
  - status
  - summary
  - outcome
  - issue list
- New chat page should show non-editable context chips for thread/stage and include work-state context automatically.
- Chat detail should expose "snapshot sources" so users can inspect the original sessions, stages, issues, and file-backed chat records that informed the snapshot.

## Detail Access

Use source indexes plus focused detail APIs instead of embedding all raw detail in one large response.

`get_thread_work_snapshot_sources` should return:

- child session identity
- snapshot version and created time
- captured thread id and stage id
- source stage ids and issue ids
- linked session identities
- original `filePath` values when available
- snapshot excerpt group indexes
- enough labels to render a useful source list without loading every detail record

The UI can then call existing APIs such as `getSessionHistory`, plus the new thread/stage work-state APIs, to fetch detail on demand.

## Tests

- Rust store tests:
  - stage state CRUD
  - issue CRUD
  - work snapshot round-trip
  - source-index retrieval
- Migration/defaulting tests:
  - active-stage-derived initial statuses
  - no-active-stage defaults
  - existing thread/session links continue to load
- API tests:
  - `get_thread_work_state`
  - `get_thread_work_snapshot`
  - `get_thread_work_snapshot_sources`
- CLI tests:
  - `sessio thread show --json` returns thread/stage rollups.
  - `sessio stage set-status --json` persists state and returns the updated stage.
  - `sessio stage issue add/list/set/delete --json` round-trips issue data.
  - CLI-written stage state is visible through `get_thread_work_state`.
- Frontend tests or focused unit tests:
  - thread new chat creates context with active stage
  - stage new chat uses explicit stage
  - pending session links thread and stage after runtime session id resolves
  - work-state context prompt includes status, issues, and detail refs
  - work-state context prompt includes `threadStageId` and CLI update examples
  - context prompt caps chat excerpts
- Manual acceptance:
  - mark stages completed/blocked/not started and confirm snapshot preserves that state
  - add stage issues and confirm they appear in new chat context
  - have an agent run `sessio stage set-status` and confirm the GUI reflects the update
  - create thread/stage chat and confirm linked session placement
  - remove original session file and confirm snapshot overview still works
  - use detail/source view to reach original sessions when available

## Assumptions

- Stage state is explicitly stored and editable.
- Problems are modeled as per-stage issue lists.
- Detail access uses source indexes plus focused detail APIs, not one giant embedded payload.
- Old threads do not require manual migration; defaults are derived lazily or during schema migration.
- Chat excerpt snapshots remain useful, but they are only one part of the broader thread/stage work-state snapshot.
- Agents receive `threadStageId` in context before they are expected to update stage state.
- JSON CLI output is the stable integration contract for agents and skills.
