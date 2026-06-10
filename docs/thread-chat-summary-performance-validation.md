# Thread Chat Summary Performance Validation

Date: 2026-06-10

## Automated checks

- `cargo test summary_keeps_thread_chat_entry_without_sessions --manifest-path src-tauri/Cargo.toml`
- `cargo test thread_replay_aggregates_and_dedupes_session_sources --manifest-path src-tauri/Cargo.toml`
- `cargo test astra_run_persistence_and_recovery --manifest-path src-tauri/Cargo.toml`
- `cargo test astra_plan_task_write_through_records_round_sessions_and_results --manifest-path src-tauri/Cargo.toml`
- `cargo test skipped_placeholder_stays_hidden_after_real_path_update --manifest-path src-tauri/Cargo.toml`
- `cargo test guardian --manifest-path src-tauri/Cargo.toml`
- `pnpm run typecheck`
- `pnpm run build`

All commands passed locally.

## Coverage notes

- Thread summary cache still preserves thread-chat entries when resolved sessions are absent.
- Thread replay resolves and dedupes scoped session references without falling back to `list_all_sessions()`.
- Astra run persistence still round-trips structured internal planner session links.
- Guardian sessions stay hidden from session lists while their skipped rows remain persisted in SQLite.
- Frontend summary consumers build successfully after removing duplicate sidebar refreshes.

## Manual profiling follow-up

The desktop CPU sampling steps from the plan were not captured in this terminal session.
They still need a manual run against a real app session for startup, `sessions_index_updated`,
and thread chat open flows.
