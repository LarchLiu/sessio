# Thread Chat Summary Performance Validation

Date: 2026-06-10

## Automated checks

- `cargo test summary_keeps_thread_chat_entry_without_sessions --manifest-path src-tauri/Cargo.toml`
- `cargo test summary_resolves_direct_stage_plan_and_astra_sessions --manifest-path src-tauri/Cargo.toml`
- `cargo test thread_replay_aggregates_and_dedupes_session_sources --manifest-path src-tauri/Cargo.toml`
- `cargo test astra_run_persistence_and_recovery --manifest-path src-tauri/Cargo.toml`
- `cargo test astra_plan_task_write_through_records_round_sessions_and_results --manifest-path src-tauri/Cargo.toml`
- `cargo test skipped_placeholder_stays_hidden_after_real_path_update --manifest-path src-tauri/Cargo.toml`
- `cargo test guardian --manifest-path src-tauri/Cargo.toml`
- `pnpm test`
- `pnpm run typecheck`
- `pnpm run build`
- `cargo run --example thread_summary_perf --manifest-path src-tauri/Cargo.toml -- --iterations 5`

All commands passed locally.

## Coverage notes

- Thread summary cache still preserves thread-chat entries when resolved sessions are absent.
- Thread summaries still merge direct, stage, plan-task, and Astra internal session references while keeping missing refs in `session_keys`.
- Thread replay resolves and dedupes scoped session references without falling back to `list_all_sessions()`.
- Astra run persistence still round-trips structured internal planner session links.
- Guardian sessions stay hidden from session lists while their skipped rows remain persisted in SQLite.
- Frontend summary consumers build successfully after removing duplicate sidebar refreshes.

## Local benchmark snapshot

`thread_summary_perf` now measures summary refresh and replay against a temporary copy of the
current Sessio DB so the benchmark does not mutate the real database.

Current local sample on the default DB:

- project: `project-b7442013f87c27bc`
- thread count in project: `2`
- replay thread: `thread-5cb9a7a6c6367fbb`
- iterations: `5`
- `cache.warm`: avg `1 ms`, best `0 ms`, worst `5 ms`
- `refresh_all`: avg `3 ms`, best `3 ms`, worst `4 ms`
- `refresh_project`: avg `3 ms`, best `3 ms`, worst `4 ms`
- `list_project`: avg `0 ms`, best `0 ms`, worst `0 ms`
- `get_thread_replay`: avg `0 ms`, best `0 ms`, worst `0 ms`

## Manual profiling follow-up

The desktop CPU sampling steps from the plan were not captured in this terminal session.
They still need a manual run against a real app session for startup, `sessions_index_updated`,
and thread chat open flows.
