# Sessio CLI / qmd Memory TODOs

## Ground Rules

- [x] Keep this TODO file updated after every completed implementation item.
- [x] Prefer small vertical slices that can be tested from the CLI.
- [x] Keep Sessio SQLite as the source-of-truth metadata index.
- [x] Treat qmd as a replaceable search backend over generated memory cards.
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
- [ ] Persist per-turn fingerprints into the `turn_fingerprints` table during card build (substrate for future continuation dedupe).
- [x] Add card-level canonical hashing.
- [x] Generate project-level memory card Markdown files.
- [x] Use readable path-derived project keys for qmd-memory project folders.
- [x] Use stable `sessio-<agent>-<session id>` memory card names instead of content-hash names.
- [x] Implement source refs from card back to raw session data.
- [x] Make project memory builds skip missing/unreadable session sources instead of failing the whole project.

## Phase 4: qmd Backend

- [x] Add qmd binary discovery/configuration.
- [x] Implement `sessio qmd status --json`.
- [x] Implement `sessio memory build --project <path> --json`.
- [x] Implement qmd collection ensure/update for a project.
- [x] Implement qmd search wrapper with JSON parsing.
- [x] Implement `sessio memory search --project <path> <query> --json`.
- [x] Implement `sessio memory resolve --card-id <id> --json`.
- [x] Map qmd search results back to stable Sessio memory hits.
- [x] Make `memory search --json` return empty hits plus `backendError` when qmd is unavailable or broken.
- [x] Include the memory card body/metadata in `memory resolve --json`.
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

## Phase 6: Skill

- [x] Create Sessio skill scaffold.
- [x] Teach the skill to call `sessio memory search --project "$PWD"`.
- [x] Teach the skill to call `sessio memory resolve --card-id`.
- [x] Document no-hit behavior: do not guess when memory search returns nothing.
- [x] Add skill usage examples.

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
- [x] `cargo test memory::` passed after adding rule-based memory card generation.
- [x] `cargo check` passed after adding rule-based memory card generation.
- [x] `cargo test` passed after adding structured tool use/result conversion and compaction.
- [x] `cargo check` passed after adding structured tool use/result conversion and compaction.
- [x] `cargo check` and `cargo test` passed after adding qmd status support.
- [x] `cargo run --bin sessio -- qmd status --json` returned structured qmd availability JSON.
- [x] `cargo run --bin sessio -- memory build --project /Users/alex/Work/cloudgeek/sessio --output-root /tmp/sessio-qmd-memory-test --db-path /tmp/sessio-memory-test.db --json` generated 36 cards.
- [x] `cargo test memory::normalize` passed after stripping IDE-injected context from memory text.
- [x] `cargo run --bin sessio -- memory build --project /Users/alex/Work/cloudgeek/sessio --output-root /tmp/sessio-qmd-memory-test-2 --db-path /tmp/sessio-memory-test-2.db --json` generated clean card titles without IDE wrapper text.
- [x] `cargo check` and `cargo test` passed after adding `sessio qmd sync`.
- [x] `cargo run --bin sessio -- qmd sync --project-key -Users-alex-Work-cloudgeek-sessio --cards-root /tmp/sessio-qmd-memory-test-2/-Users-alex-Work-cloudgeek-sessio --json` returned structured error when qmd was not installed.
- [x] `cargo run --bin sessio -- memory resolve --card-id <id> --db-path /tmp/sessio-memory-test-2.db --json` returned source refs.
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
- [x] `cargo run --bin sessio -- memory resolve --card-id sessio-codex-019e2730-6a58-7bd2-82de-8481427b8dc8 --db-path /tmp/sessio-memory-test-4.db --json` returned both card metadata/body and source refs.
- [x] `cargo run --bin sessio -- qmd sync --project-key -Users-alex-Work-cloudgeek-sessio --cards-root /tmp/sessio-qmd-memory-test-4/-Users-alex-Work-cloudgeek-sessio --json` completed successfully after treating existing collections as idempotent.
- [x] `SESSIO_QMD_TIMEOUT_SECS=2 cargo run --bin sessio -- memory search --project /Users/alex/Work/cloudgeek/sessio release workflow --db-path /tmp/sessio-memory-test-4.db --json` returned empty hits with a timeout `backendError` instead of hanging.
- [x] `cargo run --bin sessio -- qmd sync --project-key -Users-alex-Work-cloudgeek-sessio --cards-root /tmp/sessio-qmd-memory-test-4/-Users-alex-Work-cloudgeek-sessio --json` recreated an existing qmd collection so update used the current cards root.
- [x] `SESSIO_QMD_TIMEOUT_SECS=15 cargo run --bin sessio -- memory search --project /Users/alex/Work/cloudgeek/sessio release workflow --db-path /tmp/sessio-memory-test-4.db --json` returned stable mapped hits with card IDs and source refs.
- [x] `cargo run --bin sessio -- memory build --project /Users/alex/Work/cloudgeek/bm.md --output-root /tmp/sessio-qmd-memory-bm --db-path /tmp/sessio-memory-bm.db --json` completed with skipped missing Claude sources instead of failing project sync.
- [x] `cargo test` passed after changing qmd-memory project folders to path-derived names and memory card files to `sessio-<agent>-<session id>.md`.
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
- [ ] Fill the same offsets for Gemini — requires a streaming JSON scanner for `logs.json` (array of objects) since `serde_json::from_str` does not surface per-item byte positions.
- [x] Implement source-range resolution (`crate::memory::resolve::read_source_excerpt`) that reads back a raw JSONL excerpt by byte range or, failing that, by inclusive line range; exposed through `sessio memory resolve --include-source-excerpt`.
- [ ] Tool-result digest hash (command + exit code + key errors + output hash) feeding into a future tool-result dedupe layer.
- [ ] SimHash / MinHash near-duplicate detection over card text; populate `memory_cards.simhash` and merge near-dup cards by appending source refs.
- [ ] Use `turn_fingerprints` for cross-session continuation dedupe: suppress new cards when a candidate session's turn set is fully covered by an existing card.

## Known Follow-Ups

- [x] Investigate why writing memory tables to default `~/.sessio/db-data/sessio-index.db` returned `attempt to write a readonly database` during CLI smoke testing.
- [x] Ensure qmd wrapper uses the Node runtime from the same directory as the discovered qmd binary to avoid native module ABI mismatches.
- [x] Reconcile existing qmd collection roots when `sessio qmd sync` is pointed at a new cards root.
- [x] Confirmed default DB readonly was sandbox-specific: the same default `memory build` succeeded with escalated filesystem access.
