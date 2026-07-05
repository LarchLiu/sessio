# Astra Continuation Run Plan

## Summary

When a multi-session chat thread is interrupted and the user clicks the Astra button again, the current backend creates a new run. The new run does not receive enough structured history about the thread's completed Astra work, so the planner can start from the beginning.

This plan keeps the frontend behavior simple: the same Astra button still calls `createAstraRun(threadId, prompt)`. The backend creates a continuation run and injects thread progress into the planner prompt so the planner continues from current state instead of redoing completed work.

This plan can later be strengthened by the [Astra Canonical Artifact Memory Plan](./astra-canonical-artifact-memory-plan.md), which gives long-lived artifacts such as an outline or plan a stable current identity.

V1 scope is teamwork planner continuation. Process, brainstorm, and debate have dedicated backend semantics. They may keep the same run creation behavior, but shared teamwork `threadProgress` context must not be blindly injected into debate lanes or other backends with different visibility rules.

## Root Cause

The current restart path works like this:

1. `create_astra_run` checks for an active run on the thread.
2. If an active run exists but no worker is registered, the backend marks it `interrupted`.
3. The backend creates a fresh run with empty `run_diagnostics`.
4. The planner receives the new run's empty recent completion context.

The backend can correctly read the thread via `get_thread_work_state`, but the new planner prompt does not include the prior Astra run and plan task progress in a structured continuation context.

## Continuation Run Semantics

Use continuation semantics rather than resurrecting the interrupted run.

Reasons:

- The in-memory wave scheduler and current completion batch cannot be reconstructed exactly.
- A new run is safer and easier to reason about.
- Thread state, plan task rows, run records, and artifacts are already persisted and can be used as source of truth.

Behavior:

- Reconcile all active runs for the thread before creating a new run. The normal invariant is at most one active run per thread, but recovery must not leave older active rows behind if that invariant was violated.
- If any active run still has a registered worker, return the existing live run handle. Do not create a continuation run while the current run is still live.
- If active runs have no worker, mark them `interrupted`.
- The no-worker path must use the same persisted cleanup contract as startup recovery:
  - active run statuses become `interrupted`;
  - running tasks for those runs become terminal with a recovery diagnostic;
  - planned/running rounds for those runs become terminal;
  - placeholder-only partial sessions are detached or archived so replay can show a missing/partial state instead of a live task.
- Create a new run with `continuedFromRunId` set to the primary interrupted run id, using the most recently updated interrupted run when multiple active rows were recovered.
- If no interrupted run exists, `continuedFromRunId` is null.
- The frontend button and API call stay unchanged.
- The new run must plan from the existing thread progress, not from an empty run.

When `prompt` is empty on a continuation run, it means "continue unresolved thread progress from persisted state." The planner must not treat an empty continuation prompt as an instruction that there is no work to do.

Task interruption is a planner context category, not a new persisted task status. Persisted plan tasks continue to use the existing status set. Recovery should represent interrupted work as `errored` or `cancelled` plus explicit recovery diagnostics and summaries.

Round index naming:

- `runRoundIndex` is the worker-local planner loop index. It starts at 0 for every new run, including continuation runs, and is what the planner sees as the current run's immediate planning step.
- `threadRoundIndex` is `thread_plan_rounds.round_index`. It is thread-global, unique for the thread, and continues across runs.
- Continuation code must never assume these two values are equal. Planner context should expose both names when both are present.

Round budget:

- Continuation must have an explicit round budget policy.
- Use a thread-level Astra planning budget derived from persisted Astra rounds, not only the new run's worker-local `runRoundIndex`.
- A continuation run may still start with local `runRoundIndex = 0`, but it must not reset the thread's effective automatic-planning budget.
- If the thread-level budget is exhausted, the continuation run should enter a diagnostic terminal state instead of creating another automatic planning loop.

## Data Model And API

Add `continued_from_run_id` to `astra_runs`.

Expose it through:

- `AstraRunRecord`
- `AstraRun`
- `AstraHandle`
- `create_astra_run`
- `get_astra_run`
- `list_astra_runs`
- TypeScript API/binding types that mirror `AstraHandle` and run records

The value is informational and planning-relevant. It should not make the new run inherit the old run status.

Schema semantics:

- `continued_from_run_id` is a nullable lineage field.
- Do not add a self-referential foreign key in v1. The value is diagnostic/planning context, not an ownership relationship.
- Thread deletion already deletes all runs for the thread; no extra run-lineage cascade behavior is required.

Persistence invariant:

- `continued_from_run_id` must round-trip through every full-row run mapping, including `AstraRunRecord`, `AstraRun`, `run_to_record`, and `record_to_run`.
- Any code path that mutates a run by loading it, changing fields, and writing the full row back must preserve `continued_from_run_id`.
- Tests should cover that ordinary status/diagnostic mutations do not clear `continued_from_run_id`.

## Planner Context

Extend the existing `AstraPlannerContext` assembled before each planner call. Do not create a second same-named context type.

Fields:

- `continuation`
  - `continuedFromRunId`
  - interrupted run status, reason, and error summary when present
- `threadProgress`
  - plan rounds for the thread
  - each round's `astraRunId`, mode, status, summary, and `threadRoundIndex`
  - the current planner call's `runRoundIndex` separately from persisted round history
  - each task's title, status, result summary, error, risk, assistant, target agent, and timestamps
  - completed task artifact path when one can be computed
- `interruptedTasks`
  - thread-level unresolved interrupted work, not only tasks from the immediately interrupted run
  - tasks from any prior run in the thread that were running, planned, errored by recovery, failed, or cancelled and have not been superseded by later completed replacement work
  - enough information for the planner to retry, skip, or replace them deliberately

`isContinuation` can be derived from whether `continuedFromRunId` is present. If it is emitted for convenience, it must be derived from the same value and never stored as an independent source of truth.

Context data sources:

- Build thread identity, kind, goal, description, stages, assistants, and agents from `get_thread_work_state(thread_id)`.
- Build `threadProgress` from `list_plan_rounds(thread_id)`.
- Use `thread_plan_rounds.astra_run_id` to associate persisted rounds with their originating run.
- Sort rounds by `threadRoundIndex`, then tasks by `sort_order`.
- Use run records from `list_astra_runs(thread_id)` only for run lineage, status, diagnostics, and `continuedFromRunId`; do not treat run records as the task lifecycle source of truth.

Relationship to the existing teamwork round journal:

- Existing teamwork prompts use `previousRounds` from the current run's `run_diagnostics`.
- A continuation run starts with empty `run_diagnostics`, so it must not rely on the new run's `previousRounds` as its only history source.
- `threadProgress` is the continuation history source of truth. It is built from thread-level plan rounds and tasks, then enriched when possible from prior run diagnostics journals.
- When prior run diagnostics contain teamwork round journal entries, merge their planner summaries, output excerpts, and output paths into the matching `threadProgress` rounds/tasks.
- If no matching journal entry exists, fall back to persisted plan round summaries, task result summaries, and computed task artifact paths.
- Within a single uninterrupted run, `previousRounds` may remain as a compact per-run journal, but continuation planning must receive equivalent cross-run history through `threadProgress`.

Context budget:

- The planner context must be bounded.
- Include current canonical artifacts in full.
- Include unresolved or interrupted tasks in full, within a fixed cap.
- Include the most recent N thread rounds in detail, with task summaries and artifact paths.
- Summarize older rounds using compact round summaries and task status counts.
- Use stable ordering: `threadRoundIndex` ascending for history, terminal/unresolved tasks grouped deterministically by round and sort order.
- Truncate long task summaries, errors, prompts, and artifact descriptions to fixed prompt-safe limits.
- Ordinary artifact paths that are superseded by current canonical artifacts should be omitted or summarized unless needed for audit or retry diagnostics.

Planner prompt changes:

- Include the context as `continuation` and `threadProgress`.
- Instruct the planner not to redo completed tasks.
- Instruct the planner to inspect artifact paths for completed tasks when the next decision depends on their details.
- Instruct the planner to prioritize unresolved interrupted work, but only when it is still necessary for the thread goal.

## Interaction With Artifacts

Before canonical artifact memory exists:

- Use completed plan task data and computed task artifact paths.
- Include result summaries and errors to guide retry decisions.

After canonical artifact memory exists:

- Include current canonical artifacts in the same planner context.
- Prefer current canonical artifacts such as `outline` or `plan` over older ordinary task outputs.
- Resolve canonical artifact roles through the same role catalog and alias rules as `artifactRole` and `usesArtifactRoles`.
- Continue to expose ordinary task artifacts for audit and recovery.
- Avoid duplicate prompt bloat: `threadProgress` should still include task status and summaries, but ordinary artifact paths that are superseded by a current canonical artifact should be omitted or summarized unless they are needed for audit or retry diagnostics.
- If both a canonical artifact and ordinary task artifacts cover the same role, planner instructions should treat the canonical artifact as the working source of truth and use ordinary artifacts only as provenance.

## Test Plan

- `create_astra_run` returns the existing active run when its worker is still registered.
- `create_astra_run` marks zombie active runs for the thread as `interrupted` and creates a new run with `continuedFromRunId` pointing at the primary interrupted run.
- A non-continuation run stores and returns `continuedFromRunId = null`.
- Zombie active-run recovery closes recovered runs' running tasks, planned/running rounds, and placeholder-only partial sessions using the same persisted cleanup contract as startup recovery.
- `continuedFromRunId` survives ordinary full-row run mutations such as status updates and diagnostic updates.
- Store tests cover upsert, get, and list roundtrips for `continued_from_run_id`.
- TypeScript API/binding types expose `continuedFromRunId`.
- Planner prompt includes continuation context for a continuation run.
- Planner prompt includes completed historical tasks and their artifact paths.
- Continuation planner history does not depend on the new run's empty `run_diagnostics`; prior teamwork round journal summaries/excerpts/paths are available through `threadProgress` when they exist.
- Planner prompt distinguishes `runRoundIndex` from `threadRoundIndex`.
- Planner prompt context is capped for long threads and still includes unresolved/interrupted tasks.
- Planner context includes unresolved interrupted work across a continuation chain, not only the immediate `continuedFromRunId`.
- Continuation round budget is enforced across runs and cannot be reset indefinitely by repeated interruption and continuation.
- Multiple zombie active runs on one thread are all reconciled before creating a continuation run.
- Empty continuation prompt is interpreted as "continue unresolved persisted work" rather than "no work requested".
- Planner prompt avoids duplicating ordinary artifact paths when a current canonical artifact already represents the same plan/outline/draft role.
- Planner prompt includes interrupted tasks and recovery errors.
- The prompt explicitly tells the planner not to restart completed work.
- Regression: `cargo test --manifest-path src-tauri/Cargo.toml astra`.

## Assumptions

- Continuation is thread-scoped, not run-resurrection.
- The frontend Astra button remains unchanged.
- Backend store state is the source of truth for progress.
- `sessio-work-state` is not required for planner continuation. It can remain available to stage agents, but the planner should receive structured progress directly from Sessio.
