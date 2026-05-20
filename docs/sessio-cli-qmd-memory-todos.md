# Sessio CLI / qmd Memory TODOs

## Ground Rules

- [x] Keep this TODO file updated after every completed implementation item.
- [x] Prefer small vertical slices that can be tested from the CLI.
- [x] Keep Sessio SQLite as the source-of-truth metadata index.
- [x] Treat qmd as a replaceable search backend over generated memory records.
- [x] Keep provider-specific parsing behind a common provider/data model boundary.

## Phase 1: CLI Read APIs

- [x] Add a `sessio` Rust CLI binary.
- [x] Fold the CLI into the desktop binary so no-arg launch opens the app and arg launch runs CLI commands.
- [x] Add structured CLI error output and exit codes.
- [x] Implement `sessio sessions list --json`.
- [x] Add project filtering to `sessions list`.
- [x] Implement `sessio sessions messages --agent <agent> --session-id <id> --json`.
- [x] Allow `sessions messages` to resolve by `--file-path` when supplied.
- [x] Add smoke tests or documented verification commands for CLI read APIs.

## Phase 2: Provider/Data Layer Abstraction

- [x] Introduce unified provider-facing types: `AgentKind`, `ProjectRef`, `SessionSource`, `SourceLocation`.
- [x] Introduce unified message event types: `MessageEvent`, `MessageRole`, `MessageContent`.
- [x] Introduce `AgentProvider` trait.
- [x] Introduce `ProviderRegistry`.
- [x] Adapt Codex provider to the unified interface.
- [x] Make Codex forked rollout session ids use the first `session_meta.payload.id` instead of replayed metadata from the parent session.
- [x] Adapt Claude provider to the unified interface.
- [x] Adapt Gemini provider to the unified interface.
- [x] Keep current UI APIs backward compatible while the new abstraction lands.
- [x] Rename the abstraction from reader to provider.
- [x] Move provider-owned parsers under `src-tauri/src/providers/<agent>/parser.rs`.

## Phase 3: Memory Card Pipeline

- [x] Add memory-related SQLite schema tables.
- [x] Add `MemoryStore` abstraction.
- [x] Normalize session messages into `MessageEvent`.
- [x] Strip `sessio-cross:start/end` replay blocks before memory generation.
- [x] Compact tool use and tool result events.
- [x] Add turn-level canonical hashing.
- [x] Persist per-turn fingerprints into the `turn_fingerprints` table during card build.
- [x] Add card-level canonical hashing.
- [x] Generate project-level memory record Markdown files.
- [x] Use readable path-derived project keys for qmd-memory project folders.
- [x] Use stable `sessio-<agent>-<session id>` memory record names instead of content-hash names.
- [x] Implement source refs from card back to raw session data.
- [x] Make project memory builds skip missing/unreadable session sources instead of failing the whole project.
- [x] Implement same-project continuation dedupe over ordered event sequences using `turn_fingerprints`.
- [x] Trim replayed prefixes at the next `user` block boundary so continuation cards do not start mid-flow.
- [x] Suppress the whole source when dedupe matches a continuation prefix but no `user` block boundary follows it — a forked session with only replay + dangling tail has no independent value.
- [x] Store continuation provenance structurally in `record_continuations` instead of embedding detailed ranges in card markdown.
- [x] Invalidate dependent `record_continuations` rows and mark dependent candidate cards unavailable when a base session's turn fingerprints get replaced, so trimmed cards get rebuilt against the new base ranges.
- [x] Persist `forked_from_id` on the `sessions` table; this now ships as part of the consolidated post-v0.3.2 `SCHEMA_V3`.
- [x] Require real `turn_fingerprints.text_len` values in the current schema; the old split-migration cleanup is folded into consolidated `SCHEMA_V3`.

## Phase 4: qmd Backend

- [x] Add qmd binary discovery/configuration.
- [x] Implement `sessio qmd status --json`.
- [x] Implement `sessio memory build --project <path> --json`.
- [x] Implement qmd collection ensure/update for a project.
- [x] Implement qmd search wrapper with JSON parsing.
- [x] Implement `sessio memory search --project <path> <query> --json`.
- [x] Implement `sessio memory resolve --record-id <id> --json`.
- [x] Map qmd search results back to stable Sessio memory hits.
- [x] Include continuation provenance summaries in `memory resolve` and `memory search`.
- [x] Add `sessio memory covered-by --record-id <id>` and `sessio memory base --record-id <id>` CLI commands so callers can walk continuation provenance from both directions (which base covered this card / which cards did this base cover).
- [x] Make `memory search --json` return empty hits plus `backendError` when qmd is unavailable or broken.
- [x] Include the memory record body/metadata in `memory resolve --json`.
- [x] Make qmd update failures structured and non-panicking.
- [x] Make qmd update failures retryable through memory job state.
- [x] Add `sessio memory jobs --project-key <key> --json` for memory/qmd job inspection.
- [x] Make qmd collection ensure idempotent when the collection already exists.
- [x] Reconcile existing qmd collection roots during sync.
- [x] Add qmd command timeout protection.

## Phase 5: Indexer / Polling Integration

- [x] Trigger memory rebuild after full session index rebuild.
- [x] Trigger source-level memory rebuild after watcher updates.
- [x] Trigger source-level memory rebuild after polling updates.
- [x] Remove stale per-session qmd-memory markdown when an incremental rebuild marks the card unavailable.
- [x] Queue qmd update for affected projects.
- [x] Debounce qmd embed jobs.
- [x] Emit optional `memory_index_updated` events.
- [x] Track memory/qmd job status and last error.
- [x] Stop Gemini polling from re-submitting `RefreshGeminiProjectMappings` every 10s when `projects.json` mtime has not changed.
- [x] Gate polling on `indexer.status().indexing` so a long-running `FullRebuild` (now driving the per-source memory/QMD pipeline) cannot race with polling and resubmit redundant per-file reindex tasks.
- [x] Drain residual per-file reindex tasks from the indexer channel at the end of a `FullRebuild` batch (re-queueing `DeleteFile` / `DeleteSubagentFile`) so watcher events accumulated during the rebuild do not retrigger the heavy memory/QMD pipeline for already-covered files.
- [x] Detect an empty session index in polling and submit a single `FullRebuild` instead of a per-file reindex storm, so cold-start / wiped-DB bootstrap goes through the project-level memory path.
- [x] Stop Claude polling from submitting `ReindexClaudeProject` for every project on cold start by falling back to per-scope `last_indexed_at` when the in-memory `sessions-index.json` mtime cache is empty.

## Phase 6: Skill

- [x] Create Sessio skill scaffold.
- [x] Teach the skill to call `sessio memory search --project "$PWD"`.
- [x] Teach the skill to call `sessio memory resolve --record-id`.
- [x] Document no-hit behavior: do not guess when memory search returns nothing.
- [x] Add skill usage examples.
- [x] Teach the skill to call `sessio memory covered-by` and `sessio memory base` when the user asks about continuation lineage between sessions.

## Verification

- [x] `cargo check` passes for the Tauri crate.
- [x] `pnpm run typecheck` passes for the frontend.
- [x] CLI commands return valid JSON.
- [x] Existing desktop app behavior remains unchanged.

## Completed Verification Log

- [x] `cargo check` passed in `src-tauri`.
- [x] `cargo check` passed after adding provider data model and registry.
- [x] `cargo check` passed after adding built-in provider adapters.
- [x] `cargo check` passed after renaming reader abstraction to provider abstraction.
- [x] `cargo check` passed after moving parsers under `providers/`.
- [x] `cargo check --example dump` passed after provider directory migration.
- [x] `pnpm run typecheck` passed after provider directory migration.
- [x] CLI smoke test passed after provider directory migration.
- [x] `cargo check` passed after adding memory schema and `MemoryStore`.
- [x] `cargo test memory::normalize` passed for cross replay stripping.
- [x] `cargo check` passed after adding memory normalization.
- [x] `cargo test memory::` passed after adding memory hashing helpers.
- [x] `cargo test memory::` passed after adding rule-based memory record generation.
- [x] `cargo check` passed after adding rule-based memory record generation.
- [x] `cargo test` passed after adding structured tool use/result conversion and compaction.
- [x] `cargo check` passed after adding structured tool use/result conversion and compaction.
- [x] `cargo check` and `cargo test` passed after adding qmd status support.
- [x] `cargo run --bin sessio -- qmd status --json` returned structured qmd availability JSON.
- [x] `cargo run --bin sessio -- memory build --project /Users/alex/Work/cloudgeek/sessio --output-root /tmp/sessio-qmd-memory-test --db-path /tmp/sessio-memory-test.db --json` generated 36 cards.
- [x] `cargo test memory::normalize` passed after stripping IDE-injected context from memory text.
- [x] `cargo run --bin sessio -- memory build --project /Users/alex/Work/cloudgeek/sessio --output-root /tmp/sessio-qmd-memory-test-2 --db-path /tmp/sessio-memory-test-2.db --json` generated clean card titles without IDE wrapper text.
- [x] `cargo check` and `cargo test` passed after adding `sessio qmd sync`.
- [x] `cargo run --bin sessio -- qmd sync --project-key -Users-alex-Work-cloudgeek-sessio --cards-root /tmp/sessio-qmd-memory-test-2/-Users-alex-Work-cloudgeek-sessio --json` returned structured error when qmd was not installed.
- [x] `cargo run --bin sessio -- memory resolve --record-id <id> --db-path /tmp/sessio-memory-test-2.db --json` returned source refs.
- [x] `cargo test` and `pnpm run typecheck` passed after adding memory resolve.
- [x] `cargo check` and `cargo test` passed after adding qmd search wrapper.
- [x] `cargo run --bin sessio -- memory search --project-key -Users-alex-Work-cloudgeek-sessio sqlite index --json` returned structured error when qmd was not installed.
- [x] `cargo check` passed after wiring indexer memory rebuilds and retryable qmd job recording.
- [x] `cargo test memory::` passed after wiring indexer memory rebuilds and memory job storage.
- [x] `cargo run --bin sessio -- memory jobs --project-key -Users-alex-Work-cloudgeek-sessio --db-path /tmp/sessio-memory-test-3.db --json` returned stable JSON.
- [x] Added `.agents/skills/sessio-memory/SKILL.md` with search, resolve, jobs, and no-hit guidance.
- [x] `cargo check` passed after moving qmd update into a debounced background queue.
- [x] `cargo run --bin sessio -- memory search --project /Users/alex/Work/cloudgeek/sessio sqlite index --json` returned structured missing-qmd JSON.
- [x] `cargo test` passed after optional qmd embed support.
- [x] `pnpm run typecheck` passed after optional qmd embed support.
- [x] `cargo run --bin sessio -- qmd sync --project-key -Users-alex-Work-cloudgeek-sessio --cards-root /tmp/sessio-qmd-memory-test-3/-Users-alex-Work-cloudgeek-sessio --embed --json` parsed `--embed` but qmd failed because its `better-sqlite3` native module was compiled for a different Node ABI.
- [x] `cargo run --bin sessio -- memory search --project /Users/alex/Work/cloudgeek/sessio sqlite index --db-path /tmp/sessio-memory-test-3.db --json` exited successfully with empty hits and `backendError` while qmd was broken.
- [x] `cargo run --bin sessio -- memory resolve --record-id sessio-codex-019e2730-6a58-7bd2-82de-8481427b8dc8 --db-path /tmp/sessio-memory-test-4.db --json` returned both card metadata/body and source refs.
- [x] `cargo run --bin sessio -- qmd sync --project-key -Users-alex-Work-cloudgeek-sessio --cards-root /tmp/sessio-qmd-memory-test-4/-Users-alex-Work-cloudgeek-sessio --json` completed successfully after treating existing collections as idempotent.
- [x] `SESSIO_QMD_TIMEOUT_SECS=2 cargo run --bin sessio -- memory search --project /Users/alex/Work/cloudgeek/sessio release workflow --db-path /tmp/sessio-memory-test-4.db --json` returned empty hits with a timeout `backendError` instead of hanging.
- [x] `cargo run --bin sessio -- qmd sync --project-key -Users-alex-Work-cloudgeek-sessio --cards-root /tmp/sessio-qmd-memory-test-4/-Users-alex-Work-cloudgeek-sessio --json` recreated an existing qmd collection so update used the current cards root.
- [x] `SESSIO_QMD_TIMEOUT_SECS=15 cargo run --bin sessio -- memory search --project /Users/alex/Work/cloudgeek/sessio release workflow --db-path /tmp/sessio-memory-test-4.db --json` returned stable mapped hits with card IDs and source refs.
- [x] `cargo run --bin sessio -- memory build --project /Users/alex/Work/cloudgeek/bm.md --output-root /tmp/sessio-qmd-memory-bm --db-path /tmp/sessio-memory-bm.db --json` completed with skipped missing Claude sources instead of failing project sync.
- [x] `cargo test` passed after changing qmd-memory project folders to path-derived names and memory record files to `sessio-<agent>-<session id>.md`.
- [x] `cargo run -- memory build --project /Users/alex/Work/cloudgeek/sessio --output-root /tmp/sessio-qmd-memory-name-test --db-path /tmp/sessio-memory-name-test.db --json` generated project key `-Users-alex-Work-cloudgeek-sessio` and `sessio-<agent>-<session id>.md` card files.
- [x] `cargo test` passed after fixing Codex forked rollout ids so replayed `session_meta` cannot overwrite the first meta id.
- [x] `cargo run -- sessions list --project /Users/alex/Work/cloudgeek/sessio --json` now reports `/Users/alex/.codex/sessions/2026/05/18/rollout-2026-05-18T13-09-14-019e397d-032a-72f3-ab4d-5e69683a02ae.jsonl` as session id `019e397d-032a-72f3-ab4d-5e69683a02ae`.
- [x] Aligned docs and skill examples with readable project slugs, `sessio-<agent>-<session id>` card ids, and session-level incremental rebuild behavior.
- [x] `cargo test memory::build::tests::build_source_memory_marks_card_unavailable_and_removes_markdown_when_source_goes_empty` passed after making incremental source rebuild delete stale markdown and mark the card unavailable.
- [x] `cargo check` and `cargo test` passed after filtering unavailable cards from `memory search` and deleting stale qmd-memory markdown on session-level rebuild.
- [x] `cargo check` passed after caching Gemini `projects.json` mtime in polling so idle polling no longer flips the indexing indicator every 10s.
- [x] `cargo check` passed after gating polling on `indexer.status().indexing` and draining post-`FullRebuild` per-file reindex tasks from the indexer channel.
- [x] `cargo check` passed after short-circuiting empty-DB polling ticks to `FullRebuild`.
- [x] `cargo test` (27 tests) passed after routing watcher events through `ProviderRegistry::classify_path_event` and replacing byte-slice `short_id`/`short_hash` in cards with `chars().take(12)`.
- [x] `cargo test` (32 tests) passed after Codex/Claude parsers started returning per-message line/byte ranges, cards aggregated those into `memory_sources`, and `sessio memory resolve --include-source-excerpt` started returning raw JSONL excerpts.
- [x] `cargo test` (38 tests) passed after adding continuation dedupe, user-block trim boundaries, `record_continuations`, and human-readable continuation summaries in CLI resolve/search output.
- [x] `cargo test` (41 tests) passed after tightening codex dedupe direction (forked_from_id-then-time fallback), persisting `forked_from_id` in the sessions table, requiring real `text_len` fingerprints, invalidating dependent `record_continuations` on base reindex, and extracting the dedupe plan helper.
- [x] `cargo test` (45 tests) passed after adding `memory covered-by` / `memory base` CLI commands backed by `continuations_for_base`, Gemini per-item line/byte offsets via `scan_json_array_entries`, and skill updates for the new commands.

- [x] `cargo run --bin sessio -- --help` printed CLI usage.
- [x] `cargo run -- --help` printed CLI usage through the single desktop binary.
- [x] `cargo run -- qmd status --json` returned qmd status through the single desktop binary.
- [x] `cargo metadata --no-deps --format-version 1` shows only one bin target: `sessio`.
- [x] Renamed the Cargo package/bin target to lowercase `sessio` and removed `default-run`.
- [x] `cargo run --bin sessio -- sessions list --project /Users/alex/Work/cloudgeek/sessio --json` returned valid JSON session data.
- [x] `cargo run --bin sessio -- sessions messages --agent nope --json` returned structured JSON error output.
- [x] `pnpm run typecheck` passed after CLI and provider abstraction changes.
- [x] `cargo run --bin sessio -- sessions list --project /Users/alex/Work/cloudgeek/sessio --json` still worked after provider adapters were added.

## Phase 7 v2 Roadmap (deferred)

These items are intentionally **not** in scope for v1. Schema columns are reserved so v2 work is additive.

- [x] Fill `memory_sources.line_start/line_end/byte_start/byte_end` from Codex and Claude parsers. Card-level `memory_sources` rows now carry the aggregated line/byte span over all events in the card.
- [x] Fill the same offsets for Gemini — implemented by scanning the JSON array and deserializing each object from its raw byte range.
- [x] Implement source-range resolution (`crate::memory::resolve::read_source_excerpt`) that reads back a raw JSONL excerpt by byte range or, failing that, by inclusive line range; exposed through `sessio memory resolve --include-source-excerpt`.
- [ ] Tool-result digest hash (command + exit code + key errors + output hash) feeding into a future tool-result dedupe layer.
- [ ] SimHash / MinHash near-duplicate detection over card text; populate `memory_records.simhash` and merge near-dup cards by appending source refs.
- [x] Use `turn_fingerprints` for same-project continuation dedupe: trim or suppress a candidate only when an ordered replay prefix is covered and the remaining tail is low-information.
- [ ] Extend continuation dedupe beyond same-agent comparison (e.g. cross-agent continuation, multi-source joint coverage, or fuzzier paraphrased-turn matching) if real workflows show those gaps.
- [x] Add first-class CLI/UI inspection for cards covered by a given base card, and a reverse `covered-by` lookup for any card.

## Phase 8: Memory Backend Abstraction

Goal: make qmd one replaceable memory backend behind a `MemoryIndexBackend` trait. qmd remains the only shipping implementation in this iteration; a second backend lands later as additive work.

Design doc: `docs/sessio-memory-backend-abstraction-plan.md`

### Phase 8A — Document and stabilize

- [x] Land the design document and resolve open questions into a `Resolved Decisions` section.
- [x] Reference the design doc from this TODO file.

### Phase 8B — Backend-neutral types and schema

- [x] Rename `MemoryRecord` → `MemoryRecord` outright (no transitional alias). Update all call sites.
- [x] Drop `memory_records.qmd_path` from the public schema by folding artifact locations into `memory_artifacts` in consolidated `SCHEMA_V3`.
- [x] Add `memory_artifacts` table `(record_id, backend, artifact_uri, content_hash, updated_at)` with `PRIMARY KEY(record_id, backend)`.
- [x] Add `MemoryBackendStatus`, `MemorySearchOptions`, `MemoryBackendHit`, `MemorySyncReport` types.
- [x] Keep Sessio SQLite as the source of truth for records, source refs, fingerprints, continuations, jobs, and artifact metadata.

### Phase 8C — Path layout and artifact sink

- [x] Move artifact root from `~/.sessio/qmd-memory/projects/<key>/sessions/` to `~/.sessio/memory/<backend>/projects/<key>/sessions/`.
- [x] On first run after upgrade, delete the old `~/.sessio/qmd-memory/` tree and rebuild artifacts from SQLite (no compatibility shim).
- [x] Introduce `MemoryArtifactSink` trait with `MarkdownArtifactSink`, `NoopArtifactSink`, and `TempArtifactSink` implementations.
- [x] Move markdown writing/removal out of `memory/build.rs` and behind `MarkdownArtifactSink`.
- [x] Rename CLI flag `--cards-root` → `--artifacts-root` and update help text / examples.

### Phase 8D — `MemoryIndexBackend` trait and qmd backend

- [x] Define `MemoryIndexBackend` trait (`name`, `status`, `sync_project`, `remove_project`, `search`). `embed` is **not** on the trait.
- [x] Wrap `memory/qmd.rs` in `QmdBackend` that implements `MemoryIndexBackend`.
- [x] Update `sessio memory search` to call the backend through the trait while keeping the JSON response backend-neutral.
- [x] Map qmd search hit document paths back to `record_id` via `memory_artifacts`.
- [x] Expose diagnostics as `sessio memory status` / `sessio memory sync`; keep qmd behind `QmdBackend`, with diagnostic `--embed` available under the memory namespace.
- [x] Replace `QmdSyncJob` in the indexer with `MemoryBackendSyncJob { backend, project_key, project_path, changed_record_ids, removed_record_ids, dependent_source_paths }`.

### Phase 8E — `MemoryService` orchestration

- [x] Introduce `MemoryService` as the single orchestration layer over `MemoryRepository`, `MemoryIndexBackend`, and `MemoryArtifactSink`.
- [x] Make the indexer call `MemoryService` instead of `memory::qmd` directly.
- [x] Make the CLI call `MemoryService` too; CLI mode blocks on the relevant `MemoryBackendSyncJob` via a oneshot signal (no parallel synchronous pipeline).
- [x] Record backend job state generically (backend name stored separately from action), reusing the existing memory job tables.

### Phase 8F — Backend configuration

- [x] Add `[memory] backend = "qmd"` and `[memory.backends.qmd] { binary, index, artifacts_root }` to the config loader with sensible defaults.
- [x] Honor `SESSIO_QMD_BINARY` / `SESSIO_QMD_INDEX` / `SESSIO_QMD_ARTIFACTS_ROOT` env overrides (highest precedence).
- [x] Reject any `[memory].backend` value other than `"qmd"` with a config error.
- [x] Do **not** add `--backend` CLI flag or `SESSIO_MEMORY_BACKEND` env var yet.

### Phase 8G — End-to-end validation with qmd

- [x] Confirm `memory build` produces records with identical `canonical_hash` before and after the refactor.
- [x] Confirm `sessio memory search --json` returns the same hit payloads through the trait as the pre-refactor direct wrapper.
- [x] Confirm indexer + watcher path still triggers backend sync after watcher/polling updates.
- [x] Confirm CLI `memory build` blocks until backend sync completes and surfaces `backendError` on failure.
- [x] `cargo check`, `cargo test`, and `pnpm run typecheck` pass after each phase.

### Out of scope for this iteration

- Second backend (SQLite FTS, vector, remote).
- `--backend` CLI flag and `SESSIO_MEMORY_BACKEND` env var.
- Tool-result digest hash and SimHash/MinHash near-duplicate detection (still tracked in Phase 7 v2 Roadmap).

## Phase 8H: Backend Abstraction Cleanup

Goal: close the gaps between "the abstraction was introduced" and "every call site goes through it". Phase 8A–8G stood up `MemoryService` / `MemoryIndexBackend` / `MemoryArtifactSink` and made `cargo test` green, but several hot paths still bypass the new seams. Phase 8H is the finishing pass that must land **before** a second backend (SQLite FTS / vector / remote) is wired up, because every item below would otherwise leak qmd-shaped assumptions into the new backend.

### Phase 8H1 — Make `MemoryService` the only orchestration path

- [x] Replace `MemoryService::sync_backend_job_oneshot` with a real shared queue. The CLI must enqueue onto the same `MemoryBackendSyncJob` channel the indexer drains, then block on a oneshot signal until the relevant job for `project_key` completes (or fails with a structured `backendError`). Today's `thread::spawn` + `mpsc::channel` + immediate `recv()` is the "parallel synchronous pipeline" the design doc forbids.
- [x] Cache the `Arc<MemoryService>` once at indexer/CLI startup and clone it into workers. Stop calling `MemoryService::new(...)` per source/per job (currently re-reads config and rebuilds `ProviderRegistry` + `QmdBackend` + `MarkdownArtifactSink` on every `build_source_memory_for_indexer` / `build_project_memory_for_indexer` / `sync_qmd_project`).
- [x] Route `sessio memory status` and `sessio memory sync` through `MemoryService` instead of `QmdBackend::new(...)` and `qmd::qmd_status(...)` directly. Phase 8E said CLI must depend on `MemoryService`.
- [x] Move `map_backend_hits_to_memory` and the resolve `(card + sources + continuation + optional excerpt)` assembly out of `cli.rs` and into `MemoryService::search_full` / `MemoryService::resolve_full`. CLI should be a thin printer, not an orchestrator.
- [x] Add a `MemoryService` unit test that exercises build → enqueue → sync via an in-memory `MemoryIndexBackend` mock so this seam stays intact when a second backend lands.

### Phase 8H2 — Backend-neutral execution paths

- [x] Move qmd's `embed` step inside `QmdBackend` (e.g. `QmdBackend::sync_project` performs an internal post-sync embed when configured). Drop `sync_qmd_embed` and the direct `qmd::embed_index(&QmdOptions::default())` call in `indexer/mod.rs` — that path silently ignores `SESSIO_QMD_BINARY` / `SESSIO_QMD_INDEX` because it doesn't reuse the configured options.
- [x] Move `SESSIO_QMD_AUTO_EMBED` into `[memory.backends.qmd] auto_embed = true|false` and read it from `MemoryConfig`, not from `env::var` in the indexer.
- [x] Rename `run_qmd_loop` / `sync_qmd_project` / `sync_qmd_embed` / `qmd_tx` / `qmd_rx` in `indexer/mod.rs` to backend-neutral names (`run_backend_sync_loop`, `sync_memory_project`, `backend_sync_tx`, …). The channel already carries `MemoryBackendSyncJob`; the naming should follow.
- [x] Decide on `MemoryBackendSyncJob.changed_record_ids` / `removed_record_ids`: either actually populate them at the build-output boundary and let `sync_backend_job` route them to the backend, or delete the fields. Today every producer fills `Vec::new()` and `sync_backend_job` does `let _ = (...)`.
- [x] Decide on `MemoryIndexBackend::sync_project(records: &[MemoryRecord], …)`: the `records` slice is unused by `QmdBackend` because qmd rescans the artifact root on its own. Either pass an empty list (and document the contract as "backend reads artifacts from the sink") or have the trait take only `(project_key, sink)` and let backends that *need* the corpus query the repository themselves.

### Phase 8H3 — Artifact sink correctness

- [x] Include `<backend>` in `MarkdownArtifactSink` paths so two backends can coexist without colliding. Today `MarkdownArtifactSink::artifact_path` only joins `(project_key, "sessions", "<record_id>.md")`; the `backend` argument is accepted and discarded. Either join `backend` into the path, or instantiate one sink per backend with a backend-scoped root.
- [x] Stop instantiating `MarkdownArtifactSink::new(...)` inside `build_project_memory_with_backend` / `build_source_memory_with_backend`. Take `&dyn MemoryArtifactSink` as a parameter and have `MemoryService` inject its own sink. Today the service's sink is bypassed during build; a remote/object-store backend would still get markdown files written it never asked for.
- [x] Add `MemoryStore::remove_memory_artifact(record_id, backend)` and call it from every code path that calls `mark_card_unavailable` or `remove_record_artifact`. Today the file is deleted but the `memory_artifacts` row stays, leaving a dangling pointer that `artifact_for_record` will happily return.

### Phase 8H4 — Naming consistency (`MemoryRecord` → `MemoryRecord` finish line)

- [x] Rename `MemoryStore` methods: `upsert_card` → `upsert_record`, `card_by_id` → `record_by_id`, `list_project_cards` → `list_project_records`, `list_cards_for_source` → `list_records_for_source`, `mark_card_unavailable` → `mark_record_unavailable`, `mark_source_cards_unavailable` → `mark_source_records_unavailable`, `sources_for_card` → `sources_for_record`, `replace_card_sources` → `replace_record_sources`, `continuation_for_card` → `continuation_for_record`, `replace_card_continuation` → `replace_record_continuation`.
- [x] Rename `MemorySource.record_id` → `MemorySource.record_id` and update the `memory_sources` SQL column accordingly in the consolidated `SCHEMA_V3` shape — the struct field is the last `record_id` straggler.
- [x] Rename `RecordContinuation` → `RecordContinuation` and `record_continuations` table → `record_continuations` (plus the consolidated `SCHEMA_V3` definition and every site that reads/writes it). No transitional alias.
- [x] Update the CLI JSON contract: `recordId` → `recordId` everywhere in `sessio memory ...` JSON output. Bump skill docs to match. (One coordinated rename, no `recordId`/`recordId` dual emission.)

### Phase 8H5 — Schema and migration hygiene

- [x] Collapse the in-development memory migration chain into the current public schema sequence: `SCHEMA_V1` creates sessions/subagents, `SCHEMA_V2` keeps the subagent availability compatibility step, and `SCHEMA_V3` is the single post-v0.3.2 upgrade containing memory records, artifacts, sources, fingerprints, continuations, jobs, and `sessions.forked_from_id`.
- [x] Add an inline comment on `SCHEMA_V3` explaining that it is the consolidated post-v0.3.2 memory schema and already uses final names such as `memory_records`, `record_id`, `kind`, `memory_artifacts`, and `record_continuations`.
- [x] Add regression coverage for a synthetic v0.3.2-era database and a fresh install, verifying both land on the same current `SCHEMA_V3` shape.
- [x] Make `MemoryRecordKind::from_db_str` return `Result<Self>` (or `panic!`) on unknown values instead of silently mapping to `Session`. Today adding a new kind without updating the match arm corrupts data on read.

### Phase 8H6 — CLI and `path_matches_artifact` cleanup

- [x] Drop CLI aliases `--cards-root` and `--output-root` (Phase 8C said "no compatibility shim"). Keep only `--artifacts-root`.
- [x] Remove `/sessions/{id}.md` matching from `path_matches_artifact` in `cli.rs`. The artifact layout moved to `/sessions/<record_id>.md` in Phase 8C; the `/sessions/` branch is dead code.
- [x] In CLI `memory search`, replace the hardcoded `"qmd".to_string()` fallback (used when the backend errors) with `service.backend_name().to_string()`.
- [x] Drop the unused `--binary` / `--index` flags from `memory search` (they are accepted then discarded with `let _ = (binary, index);`). Config + env override is the supported configuration surface.
- [x] Expose backend-specific diagnostics (binary path, version, last error) via an optional `MemoryBackendStatus.details: serde_json::Value` so `sessio memory status` can keep showing `binary` / `version` without re-importing `qmd`-shaped types in the CLI.

### Phase 8H7 — Build pipeline simplification

- [x] Extract a `clear_source_artifacts(store, sink, backend, source, errors, reason_tag)` helper in `memory/build.rs`. The five branches that all do `mark_source_cards_unavailable` + `remove_existing_source_card_files` + `clear_source_fingerprints` collapse into one call site each, removing ~80 lines of duplication and reducing the chance of forgetting one of the three steps in a future branch.
- [x] Compute `record_id` once in `cards_for_source` (or a shared helper) and reuse it when building `RecordContinuation` in `resolve_dedupe_plan`. Today both sites independently `format!("sessio-{agent}-{session_id}")`; a future format change would touch two places.
- [x] Settle the `memory_jobs.scope` column's semantics: today it stores `project_path` for project-level jobs and `file_path` for source-level jobs. Either pick one (with `kind` distinguishing project vs source jobs) or split into `project_path` / `source_path` columns. Document the choice in the schema.

### Phase 8H8 — Misc polish (defer if time-bound)

- [x] Address `clippy::mut_range_bound` warnings in `memory/dedupe.rs::align_from_prefix_start`. The current `for next_a in ai..a_end { ai = next_a; }` pattern reads as if `ai` mutation widens the range — it doesn't (range is captured by value). Rewrite as a `while let` or explicit index loop to remove the false-alarm warning and make the intent clearer.
- [x] Either swap the hand-rolled mini TOML parser in `config.rs` for the `toml` crate, or add a parser regression test for: quoted strings with embedded `#`, escape sequences (`\\`, `\"`, `\n`), `null` literal, comment-only lines, malformed sections. The current parser is ~200 lines and silently ignores unknown sections.

### Phase 8H9 — Verification

- [x] `cargo check` and `cargo test` pass after each sub-phase.
- [x] `pnpm run typecheck` passes (CLI JSON renames in Phase 8H4 will touch the frontend if it consumes any memory JSON).
- [x] `sessio memory build`, `memory search`, `memory resolve`, `memory base`, `memory covered-by`, `memory jobs`, `memory status`, `memory sync` all return valid JSON post-rename.
- [x] Bench: indexer warm path no longer reconstructs `MemoryService` per source — confirm by counting `MemoryService::new` calls during a full rebuild on this repo (expected: 1, not N).
- [x] Add a synthetic non-qmd `MemoryIndexBackend` test double and run the full build → enqueue → sync pipeline through it. The seam is "proven backend-neutral" only when at least one non-qmd implementation exits successfully through `MemoryService`.

## Known Follow-Ups

- [x] Investigate why writing memory tables to default `~/.sessio/db-data/sessio-index.db` returned `attempt to write a readonly database` during CLI smoke testing.
- [x] Ensure qmd wrapper uses the Node runtime from the same directory as the discovered qmd binary to avoid native module ABI mismatches.
- [x] Reconcile existing qmd collection roots when `sessio memory sync` is pointed at a new artifacts root.
- [x] Confirmed default DB readonly was sandbox-specific: the same default `memory build` succeeded with escalated filesystem access.
- [x] `cargo check` passed after wiring backend config, shared `MemoryBackendSyncJob`, and CLI oneshot sync wait.
- [x] `cargo test memory::` passed after backend-aware artifact writing/removal and service sync changes.
- [x] `SESSIO_QMD_ARTIFACTS_ROOT=/tmp/sessio-memory-backend-artifacts cargo run --bin sessio -- memory build --project /Users/alex/Work/cloudgeek/sessio --db-path /tmp/sessio-memory-backend-abstraction-env-check.db --json` returned `summary.artifactsRoot` from the env override and surfaced the qmd sync failure as `backendError`.
- [x] Built memory twice into `/tmp/sessio-memory-hash-a.db` and `/tmp/sessio-memory-hash-b.db`, then compared sorted `(record_id, canonical_hash)` rows with `cmp`; 56 rows matched exactly.
- [x] `cargo run --bin sessio -- memory search --project /Users/alex/Work/cloudgeek/sessio backend abstraction --db-path /tmp/sessio-memory-hash-a.db --json` returned backend-neutral qmd hits through `MemoryService` with `backendError: null`.
- [x] `SESSIO_QMD_BINARY=/tmp/sessio-missing-qmd cargo run --bin sessio -- memory search --project /Users/alex/Work/cloudgeek/sessio backend abstraction --db-path /tmp/sessio-memory-hash-a.db --json` returned empty hits with `backendError`.
- [x] Added indexer routing tests covering watcher `ReindexSource` and polling `ReindexScope` tasks reaching memory rebuild task variants.
- [x] `pnpm run typecheck` passed after the backend abstraction validation pass.
- [x] `cargo test` passed after completing Phase 8G validation (51 tests).
- [x] Migrated user-facing qmd diagnostics from top-level `sessio qmd ...` to `sessio memory status` / `sessio memory sync`; qmd remains an internal backend implementation.
