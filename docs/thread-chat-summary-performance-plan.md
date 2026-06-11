# Thread Chat Summary Performance Fix Plan

## Summary

The current high-CPU path is caused by rebuilding thread chat summaries through a full session-table scan. The sampled stack was:

```text
refresh_thread_chat_summaries
  -> ThreadChatSummaryCache::refresh_project
  -> build_project_summaries
  -> session_lookup
  -> SessionStore::list_all_sessions
  -> SqliteStore::load_sessions
  -> is_codex_guardian_index_row
  -> serde_json::from_str
```

The expensive part is not the thread count. Even a project with only a few threads can force Sessio to load every indexed session, open many Codex JSONL files, and parse `session_meta` lines just to build a lookup for a small set of referenced sessions.

This plan fixes all known contributors:

- Replace full-session lookups with key-scoped batch session loading.
- Remove JSONL guardian detection from read hot paths.
- Apply the same scoped lookup fix to thread replay, not only thread summaries.
- Split `astra_runs.internal_planner_session_ids_json` into a structured association table.
- Coalesce duplicate frontend/backend summary refreshes after indexing events.

## Current Behavior

### Thread Summary Build

`src-tauri/src/thread_chat_summary.rs` builds `ThreadChatSummaryInfo` from these sources:

- `thread.sessions`: already loaded as full `SessionInfo`.
- `thread.stages[*].sessions`: already loaded as full `SessionInfo`.
- `plan_rounds[*].tasks[*].sessions`: only `PlanTaskSessionInfo`, containing `agent` and `session_id`.
- `astra_runs[*].internal_planner_session_ids_json`: JSON array of session IDs, with agent inferred from `planner_backend`.

The last two sources need full `SessionInfo`, so the current code builds a global lookup by calling `list_all_sessions()`.

### Thread Replay

`SessionStore::get_thread_replay()` has the same full lookup pattern. It loads the thread, plan rounds, Astra runs, then calls `list_all_sessions()` so plan task and Astra internal references can be enriched with full session metadata.

### Guardian Filtering

`load_sessions()` filters Codex guardian sessions with `is_codex_guardian_index_row()`. That helper opens each Codex session file and parses JSONL until it finds `session_meta`.

The parser already knows when a Codex file is a guardian session and returns `None`, but existing DB rows can still be present, so read paths repeat the file parse defensively.

### Astra Internal Planner Sessions

`astra_runs.internal_planner_session_ids_json` stores planner sessions as an untyped JSON array. Consumers must parse JSON, infer agent from `planner_backend`, and cannot query these links directly.

This is workable for UI payloads but weak as a relational source of truth.

## Problems

1. `thread_chat_summary::session_lookup()` scans all sessions for a small subset of references.

2. `SessionStore::get_thread_replay()` repeats the same full scan.

3. `load_sessions()` performs file IO and JSON parsing through `is_codex_guardian_index_row()` on read. Any caller that lists sessions can pay this cost.

4. `astra_runs.internal_planner_session_ids_json` makes planner-session references opaque to SQLite. It prevents direct joins, scoped lookups, and indexed cleanup.

5. `sessions_index_updated` currently fans out to more than one thread-summary refresh path in the frontend, so one index completion can enqueue repeated backend refresh work.

## Target Model

Session metadata remains single-source in `sessions`. Association tables store references, not duplicated `SessionInfo`.

Thread summary and replay should follow this pattern:

1. Load the relevant project/thread graph.
2. Add already-loaded direct/stage sessions immediately.
3. Collect only unresolved session references from plan tasks and Astra run sessions.
4. Batch-load those session keys.
5. Build summaries/replay from the merged direct sessions and resolved references.

Guardian detection should be an indexed/session-ingest concern, not a list/read concern.

## Schema Changes

Keep the database schema version at v5. This work has not shipped yet, so new changes should be folded into the v5 target schema instead of adding `SCHEMA_V6` or preserving compatibility with the current unreleased v5 shape.

Compatibility requirement:

- v4 databases must upgrade cleanly to the final v5 schema.
- Existing unreleased v5 development databases can be reset or handled with best-effort developer cleanup.
- New code does not need to preserve `astra_runs.internal_planner_session_ids_json`.

### New `astra_run_sessions` Table

Add this table directly to `SCHEMA_V5`:

```sql
CREATE TABLE IF NOT EXISTS astra_run_sessions (
    run_id        TEXT NOT NULL,
    agent         TEXT NOT NULL,
    session_id    TEXT NOT NULL,
    role          TEXT NOT NULL DEFAULT 'planner',
    sort_order    INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY(run_id, agent, session_id, role),
    CHECK(role IN ('planner')),
    FOREIGN KEY(run_id) REFERENCES astra_runs(run_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_astra_run_sessions_run
    ON astra_run_sessions(run_id, sort_order, created_at);

CREATE INDEX IF NOT EXISTS idx_astra_run_sessions_session
    ON astra_run_sessions(agent, session_id);
```

For v4-to-v5 upgrades there is no released `internal_planner_session_ids_json` contract to preserve. If a local developer database already has this unreleased column, a best-effort backfill may parse it before dropping/ignoring it, but production migration logic only needs to support v4 -> final v5.

Final v5 should remove `internal_planner_session_ids_json` from the `astra_runs` table definition. `astra_run_sessions` becomes the only persistent source of truth for internal planner session links.

### Guardian Marker

Adopt a sidebar-specific marker:

```sql
ALTER TABLE sessions ADD COLUMN hidden_from_sidebar INTEGER NOT NULL DEFAULT 0;
```

Then store `hidden_from_sidebar = 1` for guardian rows and internal/delegated runtime rows that should not appear as standalone sidebar sessions. This marker is only a sidebar/session-list visibility flag; thread, stage, kanban, replay, and explicit session-ref reads should still be able to resolve these rows when linked.

## Store API Changes

Add a structured session reference type:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionRef<'a> {
    pub agent: Agent,
    pub session_id: &'a str,
}
```

Add a batch lookup method to `SessionStore`:

```rust
fn list_sessions_by_refs(&self, refs: &[SessionRef<'_>]) -> Result<Vec<SessionInfo>>;
```

SQLite implementation:

- Deduplicate refs before querying.
- Query by `(agent, session_id)` only for requested keys.
- Prefer rows with `available = 1`, `partial = 0`, real file paths, non-empty paths, and newest `updated_at`.
- Load subagents only for returned parent sessions, not for every session in the DB.
- Do not filter `hidden_from_sidebar`; explicit ref lookup must resolve linked/internal sessions even when they are hidden from sidebar lists.
- Do not call `is_codex_guardian_index_row()`.

CachedStore implementation:

- Forward to inner initially.
- Optional later optimization: keep a lightweight `SessionInfo` lookup cache. This is not required for the first fix.

## Astra Run Store Changes

Introduce an `AstraRunSessionRecord`:

```rust
pub struct AstraRunSessionRecord {
    pub run_id: String,
    pub agent: Agent,
    pub session_id: String,
    pub role: PlanTaskSessionRole,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}
```

Store methods:

```rust
fn replace_astra_run_sessions(
    &self,
    run_id: &str,
    sessions: &[AstraRunSessionRecord],
) -> Result<()>;

fn list_astra_run_sessions(&self, run_id: &str) -> Result<Vec<AstraRunSessionRecord>>;

fn list_astra_run_sessions_for_thread(
    &self,
    thread_id: &str,
) -> Result<Vec<AstraRunSessionRecord>>;
```

Implementation rules:

- `upsert_astra_run()` writes the run row and replaces `astra_run_sessions` in the same transaction.
- `AstraRunRecord` should stop carrying `internal_planner_session_ids_json`; either add `internal_planner_sessions: Vec<AstraRunSessionRecord>` or keep run metadata and session links as separate store reads.
- `record_to_run()` should populate `AstraRun.internal_planner_session_ids` from `astra_run_sessions`.
- Cleanup paths such as `interrupt_active_astra_runs()` must read placeholder/internal planner sessions from `astra_run_sessions`, not from JSON.

## Thread Summary Changes

Refactor `thread_chat_summary.rs` around scoped reference collection.

For project refresh:

1. Load all project threads once.
2. Load plan rounds and Astra runs for those threads.
3. Add direct/stage sessions immediately.
4. Collect unresolved refs from:
   - non-superseded plan task sessions
   - structured `astra_run_sessions`
5. Call `list_sessions_by_refs()`.
6. Build summaries with the scoped lookup.

For all-project refresh:

- Either call the project path for each project independently, or collect all project graphs first and do one global scoped lookup.
- The first version can be per-project because the number of projects is small and the key set remains bounded by actual thread references.

Important behavior to preserve:

- `session_keys` must include reference-only sessions even if the session row is missing.
- `sessions` should include only resolved full `SessionInfo`.
- Direct/stage sessions should still win over partial placeholders when they are better candidates.
- Sorting by latest activity must keep considering thread, stage, plan round, plan task, Astra run, and session timestamps.

## Thread Replay Changes

Refactor `SessionStore::get_thread_replay()` to use the same reference collection pattern.

This should not remain a trait default that calls `list_all_sessions()`.

Implementation options:

- Move replay construction into a helper that accepts `list_sessions_by_refs()`.
- Keep the trait default but replace the full lookup with scoped ref collection.
- Extract shared helpers used by both replay and thread summary to avoid drifting behavior.

Required behavior:

- Direct and stage sessions pass their full `SessionInfo` into `add_replay_session_source()`.
- Plan task sessions resolve through scoped lookup.
- Astra internal sessions resolve through `astra_run_sessions`.
- Missing sessions still produce replay entries with source metadata and `session = None`.

## Guardian Handling

Move guardian detection to indexing/upsert, then remove `is_codex_guardian_index_row()` from read paths.

Implementation steps:

1. Extend Codex parser output so it can distinguish:
   - visible parsed session
   - hidden guardian session
   - unindexable/invalid file

2. When a guardian file is seen:
   - Upsert or mark the matching `sessions` row with `hidden_from_sidebar = 1`.
   - Or delete the row if no compatibility concern remains.

3. Update listing queries:
   - User-facing `list_sessions()`: filter hidden rows in SQL.
   - `list_all_sessions()`: either filter hidden rows or rename/split into explicit `list_visible_sessions()` and true internal all-session APIs.
   - Thread/stage/kanban/session-ref queries: filter hidden rows in SQL.

4. Delete or quarantine `is_codex_guardian_index_row()` after migration/backfill proves existing rows are handled.

Backfill:

- This work is still unreleased, so no released migration path is required.
- If a local developer database needs cleanup, do a one-time developer-only pass instead of parsing JSONL on every read.

## Frontend Refresh Changes

Current frontend listeners can trigger duplicate summary refreshes after `sessions_index_updated`.

Recommended change:

- Keep one owner for global thread summary refresh, preferably `App.tsx`.
- Let `AppSidebar` consume summary state via props or request cached `listThreadChatSummaries()` without forcing a refresh on every index event.
- Add a small debounce/coalescing window for `sessions_index_updated`, for example 150-300 ms.

Backend cache guard:

- Keep `ThreadChatSummaryCache.refresh_lock`.
- Optionally add "refresh already pending" coalescing if frontend events remain noisy.

## Implementation Phases

### Phase 1: Scoped Session Lookup

- Add `SessionRef` and `list_sessions_by_refs()`.
- Implement SQLite batch lookup.
- Implement CachedStore forwarding.
- Refactor `thread_chat_summary.rs`.
- Refactor `get_thread_replay()`.
- Add focused tests for:
  - summaries do not call `list_all_sessions()`
  - replay resolves plan task sessions through scoped lookup
  - missing session refs remain represented in keys/sources

### Phase 2: Astra Run Sessions Table

- Fold `astra_run_sessions` into `SCHEMA_V5`.
- Remove `internal_planner_session_ids_json` from final v5 `astra_runs`.
- Update `upsert_astra_run()` to replace structured associations.
- Update `list_astra_runs()` / `get_astra_run()` to load structured session links.
- Update summary/replay to use structured Astra session refs.
- Update tests currently asserting `internal_planner_session_ids_json` to assert `astra_run_sessions` rows and `AstraRun.internal_planner_session_ids`.

### Phase 3: Guardian Read-Path Removal

- Add `hidden_from_sidebar`.
- Mark guardian rows and internal runtime rows with `hidden_from_sidebar = 1`.
- Update parser/indexer/store write path to mark guardian sessions.
- Update sidebar/session-list queries and project counts to filter hidden rows in SQL.
- Keep thread/stage/kanban/replay/ref reads unfiltered by `hidden_from_sidebar`.
- Remove read-time JSONL parsing from `load_sessions()`, `load_thread_sessions()`, `load_stage_sessions()`, and kanban session loading.

### Phase 4: Refresh Coalescing

- Remove duplicate forced refreshes from `AppSidebar`.
- Keep cached reads in sidebar expansion paths.
- Debounce global index-event refresh in `App.tsx`.
- Verify tray recent menu and sidebar thread chat list stay fresh.

### Phase 5: Performance Validation

- Run unit tests for store, Astra, and thread summary.
- Run app against current DB and capture a new CPU sample during:
  - app startup
  - `sessions_index_updated`
  - opening thread chats
  - opening multi-session thread chat
- Confirm no stack dominated by `is_codex_guardian_index_row` or `serde_json::from_str` under summary/replay refresh.

## Versioning Notes

- Keep `schema_migrations` at version 5. Do not introduce `SCHEMA_V6` for this work.
- Update `SCHEMA_V5` as the final unreleased schema shape.
- Ensure v4 databases still upgrade into the final v5 schema.
- Existing `AstraRun` in memory can continue using `internal_planner_session_ids: Vec<String>`; store conversion should bridge between that runtime shape and `astra_run_sessions`.
- `hidden_from_sidebar` should default to visible (`0`) unless confidently identified as hidden.
- If guardian detection fails for a single unreadable file during a one-time developer/backfill pass, do not block startup. Leave the row visible and log a warning, or mark it only when confidently detected.

## Acceptance Criteria

- Refreshing thread chat summaries no longer calls `list_all_sessions()`.
- `get_thread_replay()` no longer calls `list_all_sessions()`.
- `load_sessions()` and related session-list reads no longer open Codex JSONL files to detect guardian rows.
- Astra internal planner session links are queryable through `astra_run_sessions`.
- Astra runs round-trip to `AstraRun.internal_planner_session_ids` through `astra_run_sessions`.
- Sidebar thread chat list, tray recent menu, and thread multi-session chat still show direct, stage, plan task, and Astra internal sessions.
- Current DB startup/index refresh no longer pins `target/debug/sessio` near 100% CPU for this path.

## Risks

- Schema rewrite complexity: final v5 must be updated consistently across bootstrap schema, v4 upgrade path, tests, and store DTOs.
- Semantics drift: summary and replay must share session-ref logic or they can show different session sets.
- Sidebar visibility semantics: `hidden_from_sidebar` must stay limited to ordinary session lists/project counts; applying it to thread/stage/replay/ref reads would hide linked internal sessions.
- Frontend freshness: removing duplicate refreshes must preserve sidebar and tray recency after indexing.

## Recommended First Commit

Start with Phase 1 because it directly removes the sampled hotspot and has the smallest schema surface. Then land Phase 2 and Phase 3 as separate commits so the final v5 schema rewrite and guardian handling are easy to review and test.
