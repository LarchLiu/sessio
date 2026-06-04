# Sessio Astra Plan

## Summary

Sessio should use Astra as an internal thread-level orchestration intelligence built on Pi, not as a fourth user-facing runtime agent beside Codex, Claude, and Gemini.

The intended shape is:

- Astra acts as Sessio's orchestration brain.
- Sessio Rust remains the owner of runtime state, ACP sessions, permissions, thread/stage state, memory access, and historical indexing.
- ACP agents remain the execution workers.
- The first product entry point is a Thread-level "Astra" action.
- Clicking Astra grants one run-level authorization immediately; Astra may then plan, dispatch, observe results, and continue until it decides to complete, cancel, or error.
- Astra runs as a Node sidecar: source/dev mode during development and bundled binary mode for release.

The core boundary is:

```text
User / Thread / Stage
  -> Sessio context, memory, work state
  -> Astra sidecar
  -> Sessio tool bridge
  -> Sessio RuntimeManager
  -> ACP agents
```

Astra must not directly spawn or own Codex, Claude, Gemini, or future external ACP agents. It can only request orchestration actions through Sessio tools, and Sessio decides whether those actions are valid and allowed.

## Goals And Non-Goals

V1 must deliver one closed orchestration loop. Each goal maps to a concrete mechanism described later in this document:

1. **Delegate by context** — read thread/stage/memory via `sessio.project.snapshot` / `sessio.memory.search` and dispatch a task to an ACP agent.
2. **Receive task results** — `sessio.agent.dispatch_task` returns an `AstraTaskResult` when the delegated ACP turn reaches a terminal state.
3. **Decide stage updates** — judge the result and submit an `AstraStageDecision` (a request, not an applied mutation).
4. **Receive update results** — Sessio validates and applies the decision, then returns an `AstraStageMutationResult` (success or structured error).
5. **Re-dispatch by stage state** — refresh the snapshot and choose the next task from updated stage state.
6. **Close the loop** — repeat until Astra decides the run is complete; Astra emits `complete`, and Sessio records that terminal decision.
7. **Retry the same stage** — when unsatisfied with a result, request another delegated task for the same stage.
8. **Bounded retries** — Sessio counts per-stage attempts and enforces a configurable retry limit, returning `retryLimitReached` to force a new decision.

Non-Goals (V1):

- Astra does not spawn ACP agents, run shell, write project files, or invoke the `sessio` CLI directly.
- Astra does not count its own retries or enforce limits — Sessio does.
- Astra is not added to the indexed historical `Agent` enum.
- No automatic resume of an interrupted run; state is preserved and inspectable, continuation is manual.
- At most one concurrent orchestration run per thread.

## Name And Product Meaning

**Astra** comes from Latin and means **stars / celestial bodies**: stars, stellar bodies, and the wider sky of navigable relationships.

The name fits Sessio's internal intelligence because:

- Astra sees the whole project map rather than a single message. It reads thread, stage, memory, and agent session state as one connected field.
- Astra is a navigator. Like ancient star navigation, it helps decide where the project should go next, which task should move first, and which path carries risk.
- Astra coordinates orbits. Each ACP agent runs in its own lane; Astra keeps them moving around the same thread goal without collisions or drift.
- Astra has a small amount of mystery without becoming vague. It feels more like direction, order, and spatial awareness than a generic coordinator label.

Product semantics:

> **Astra is Sessio's orchestration intelligence: a thread-level navigator that turns project context into coordinated agent work.**

中文语义：

> **Astra 是 Sessio 的线程导航智能：它从复杂上下文中识别方向，并协调多个 agent 向同一个目标推进。**

## Architecture

Add a dedicated orchestration subsystem instead of extending `RuntimeTransportKind` with an Astra transport.

```text
Sessio UI
  -> Tauri commands
  -> AstraService
  -> sidecars/sessio-astra
      -> @earendil-works/pi-ai
      -> @earendil-works/pi-agent-core
      -> stdio JSONL protocol
  -> AstraService tool bridge
  -> RuntimeManager / SessionStore / MemoryService
  -> ACP agents
```

Astra is an internal service because it coordinates other runtimes rather than serving as a normal chat runtime. This avoids mixing orchestration state with the existing Codex/Claude/Gemini runtime event model.

The sidecar owns only transient Astra agent state:

- current orchestration prompt
- local Astra transcript
- pending Astra tool calls
- Astra-side planning context

Sessio owns durable state:

- thread and stage records
- stage issue records
- memory records
- runtime session handles
- ACP permission decisions
- pending and completed delegated agent sessions
- persisted orchestration run metadata

## Key Changes

Add a new Node/TypeScript sidecar project:

```text
sidecars/sessio-astra
```

The sidecar should:

- depend on `@earendil-works/pi-ai` and `@earendil-works/pi-agent-core`
- expose a stdio JSONL worker
- create an Astra `Agent` with a Sessio-specific orchestration system prompt
- stream planning, status, tool, error, and completion events to Rust
- call only the tools exposed by Sessio over the JSONL protocol

Add a Rust service layer:

```text
AstraService
```

The service should:

- spawn and monitor the sidecar
- maintain JSONL request ids and pending responses
- manage active orchestration runs by thread id
- expose Tauri commands for starting, cancelling, and listing orchestration runs
- route Astra tool calls to `SessionStore`, `MemoryService`, and `RuntimeManager`
- emit frontend orchestration events

Add Tauri sidecar integration:

- add `tauri-plugin-shell = "2"` to `src-tauri/Cargo.toml`
- initialize the shell plugin in `run()`
- add `bundle.externalBin = ["binaries/sessio-astra"]` to `src-tauri/tauri.conf.json`
- add a shell sidecar permission in `src-tauri/capabilities/default.json`
- in debug builds, allow launching the source sidecar command for faster development
- in release builds, launch only the bundled sidecar binary

Add Bun-based sidecar tooling:

- keep the Sessio root project on `pnpm`, Vite, and Tauri
- use Bun only inside `sidecars/sessio-astra`
- use `bun run src/main.ts --stdio` for sidecar development
- use `bun build --compile` to produce release sidecar binaries
- commit a sidecar-local lockfile and package metadata without changing the root `pnpm-lock.yaml`

The sidecar package should use this dependency split:

```json
{
  "dependencies": {
    "@earendil-works/pi-ai": "0.78.0",
    "@earendil-works/pi-agent-core": "0.78.0"
  },
  "devDependencies": {
    "@types/bun": "latest",
    "typescript": "^5.7.3"
  }
}
```

Suggested scripts:

```json
{
  "scripts": {
    "dev": "bun run src/main.ts --stdio",
    "typecheck": "tsc --noEmit",
    "build": "bun build src/main.ts --target=bun --outdir=dist",
    "compile:mac-arm64": "bun build --compile --target=bun-darwin-arm64 src/main.ts --outfile ../../src-tauri/binaries/sessio-astra-aarch64-apple-darwin",
    "compile:mac-x64": "bun build --compile --target=bun-darwin-x64 src/main.ts --outfile ../../src-tauri/binaries/sessio-astra-x86_64-apple-darwin",
    "compile:win-x64": "bun build --compile --target=bun-windows-x64 src/main.ts --outfile ../../src-tauri/binaries/sessio-astra-x86_64-pc-windows-msvc.exe"
  }
}
```

Before relying on Bun for release packaging, run a smoke test that starts the compiled sidecar, performs the JSONL handshake, initializes the Astra agent, resolves a model with `getModel(...)`, and executes a fake provider or no-network planning path. This catches Node compatibility issues in Pi providers before they reach Tauri packaging.

Add a Thread-level UI entry:

- show an "Astra" action on a thread
- display Astra's proposed task plan before dispatch
- allow approving the full plan or a selected subset once before automatic execution
- show orchestration progress and delegated agent session links
- expose cancel for the active orchestration run

## Interfaces And Flow

### Tauri Commands

Add frontend-facing commands:

```text
start_thread_astra(threadId, prompt?) -> AstraHandle
cancel_thread_astra(runId) -> void
```

Emit frontend events on:

```text
thread-astra-event
```

The frontend should treat these events as orchestration activity, not normal chat message deltas.

### Rust Data Types

Use explicit protocol and runtime structs:

```text
AstraRun
AstraHandle
AstraTaskProposal
AstraTaskApproval
AstraEvent
AstraProtocolMessage
AstraProtocolRequest
AstraProtocolResponse
AstraProtocolEvent
AstraToolCall
AstraToolResult
AstraTaskResult
AstraStageDecision
AstraStageMutationResult
```

Each run should include:

- `runId`
- `threadId`
- `projectId`
- `projectPath`
- `status`
- `createdAt`
- `updatedAt`
- proposed tasks
- run-level authorization metadata retained for compatibility, not per-task approval
- delegated `sessioRuntimeSessionId` values
- `mode` such as `auto`
- current loop position, such as `currentStageId`
- completed task ids
- delegated task results
- stage attempt counts keyed by stage id
- retry limit configuration, defaulting to a small bounded value such as 3

`AstraTaskResult` should describe the terminal result of one delegated ACP turn:

- `taskId`
- `threadStageId`
- `sessioRuntimeSessionId`
- `status`, such as `completed`, `failed`, `cancelled`, or `errored`
- `summary` or `finalMessage`
- `error`
- `attemptCount`
- `retryLimitReached`

`AstraStageDecision` is Astra's requested state mutation, not proof that the mutation happened:

- `threadStageId`
- desired `status`
- `summary`
- `outcome`
- optional issue change
- source `taskId` or `sessioRuntimeSessionId`
- decision reason

`AstraStageMutationResult` is created by Sessio after validation and execution:

- `ok`
- updated stage or issue
- structured error
- `appliedAt`

### Rust To Astra JSONL

Each JSONL message is one complete JSON object followed by `\n`.

Request:

```json
{
  "protocolVersion": 1,
  "id": "1",
  "method": "astra/start",
  "params": {
    "runId": "run-...",
    "thread": {},
    "snapshot": {},
    "prompt": "optional user instruction"
  }
}
```

Response:

```json
{
  "protocolVersion": 1,
  "id": "1",
  "result": {}
}
```

Error:

```json
{
  "protocolVersion": 1,
  "id": "1",
  "error": {
    "code": "invalid_request",
    "message": "threadId is required"
  }
}
```

Event:

```json
{
  "protocolVersion": 1,
  "method": "event",
  "params": {
    "runId": "run-...",
    "type": "plan",
    "data": {}
  }
}
```

Astra tool call:

```json
{
  "protocolVersion": 1,
  "id": "tool-1",
  "method": "tool/call",
  "params": {
    "runId": "run-...",
    "name": "sessio.agent.dispatch_task",
    "args": {}
  }
}
```

The `sessio.agent.dispatch_task` response should resolve only after the delegated ACP turn reaches a terminal state:

```json
{
  "protocolVersion": 1,
  "id": "tool-1",
  "result": {
    "taskId": "task-...",
    "threadStageId": "stage-...",
    "sessioRuntimeSessionId": "session-...",
    "status": "completed",
    "summary": "The delegated agent finished the requested work.",
    "attemptCount": 1,
    "retryLimitReached": false
  }
}
```

The `sessio.stage.update` and `sessio.stage.issue.add_or_update` responses should be Sessio-authored mutation results:

```json
{
  "protocolVersion": 1,
  "id": "tool-2",
  "result": {
    "ok": true,
    "stage": {},
    "appliedAt": "2026-06-03T00:00:00Z"
  }
}
```

An alternative future shape is non-blocking dispatch plus Rust-to-Astra `task-completed` and `stage-update-result` events. V1 should prefer blocking dispatch so one long-lived Astra run can make the next decision from the previous tool result.

### Sessio Tools Exposed To Astra

Expose a minimal, controlled tool surface:

```text
sessio.project.snapshot
sessio.memory.search
sessio.agent.plan_task
sessio.agent.dispatch_task
sessio.stage.update
sessio.stage.issue.add_or_update
```

Tool behavior:

- `sessio.project.snapshot` returns thread, stage, linked session, issue, project path, and active stage state.
- `sessio.memory.search` returns bounded project memory search results.
- `sessio.agent.plan_task` records a proposed task but never starts an ACP agent.
- `sessio.agent.dispatch_task` dispatches a task that Astra has planned for the active run, waits for the delegated ACP turn to reach a terminal state, and returns an `AstraTaskResult`.
- `sessio.stage.update` accepts Astra's stage mutation decision/request; Sessio validates and applies status, summary, and outcome through existing store APIs, then returns the updated stage or a structured error.
- `sessio.stage.issue.add_or_update` accepts Astra's issue mutation decision/request; Sessio validates and applies blocker or review finding changes through existing issue APIs, then returns the updated issue or a structured error.

These mutation tools are decision-submission interfaces, not CLI or shell execution paths. Astra does not execute the `sessio` CLI internally; Sessio is the component that mutates state and reports the result back to Astra.

### Orchestration Loop

1. User clicks Thread "Astra".
2. Rust loads the current thread work state and related memory context.
3. Rust starts or reuses the Astra sidecar.
4. Rust sends `astra/start` to Astra.
5. Astra inspects the snapshot and calls read-only tools as needed.
6. Astra emits each plan round as an event and records planned tasks through Sessio.
7. UI displays task cards with target stage, target agent, prompt, expected output, and risk as historical orchestration activity.
8. Astra dispatches the planned task queue through Sessio.
9. Sessio `RuntimeManager` owns the ACP session, handles normal permission UI, and returns an `AstraTaskResult` when the delegated turn reaches a terminal state.
10. Astra judges the task result and submits a stage or issue mutation decision to Sessio.
11. Sessio validates the decision, applies or rejects the mutation, and returns an `AstraStageMutationResult` to Astra.
12. Astra refreshes the snapshot, plans again, chooses the next task, and repeats the loop until it decides to complete, cancel, or error.
13. When Astra emits a terminal event, Sessio records and emits that orchestration state; Rust does not independently decide that the thread is complete.

If Astra is dissatisfied with a result, it may request another delegated task for the same stage. Sessio, not Astra, tracks attempt counts by stage id and enforces the retry limit. When the limit is reached, Sessio refuses another direct dispatch for that stage and returns `retryLimitReached`, forcing Astra to choose a different strategy such as changing agent, changing prompt, splitting the task, marking the stage blocked, or ending the run.

## Boundary And Failure Rules

### State Authority

- Sessio store is the source of truth for threads, stages, issues, linked sessions, and snapshots.
- Astra internal transcript is temporary orchestration context only.
- Astra must use ids returned by Sessio tools.
- Astra cannot invent stage ids, session ids, issue ids, or completion state.
- Astra may propose stage and issue mutations, but Sessio is the only executor and authority for applying them.
- Astra orchestration logs should not be treated as historical agent sessions in V1.

### Permission Boundary

- Astra cannot execute shell commands directly.
- Astra cannot call the `sessio` CLI or any shell path to mutate state.
- Astra cannot write project files directly.
- Astra cannot spawn Codex, Claude, Gemini, or future ACP agents directly.
- All mutation decisions must go through the Rust `AstraService` tool bridge, where Sessio validates ids, active run state, and thread ownership before applying any state change.
- `sessio.agent.dispatch_task` must only run for tasks Astra has registered on the active run.
- Automatic dispatch is limited to the run-level authorization created by the user's Astra click, the originating thread, and the current run id.
- Sessio owns stage attempt counting and retry-limit enforcement; Astra cannot reset or bypass those counters.
- Existing ACP permission requests from delegated agents continue to use the current Sessio permission UI.

### Concurrency

- V1 allows at most one active orchestration run per thread.
- Starting a second run for the same thread should return the active run or surface a UI action to cancel it.
- An Astra run may delegate multiple ACP tasks, but each delegated task must be tied to the originating `runId`.
- Cancelling an Astra run cancels only delegated sessions launched by that run.
- Cancelling an Astra run must not cancel unrelated user-started ACP sessions.

### Sidecar Failure

- If the sidecar fails to start, return an unavailable status and do not create delegated tasks.
- If the sidecar exits mid-run, mark the run as `errored`.
- Already-started ACP sessions remain visible and controllable in Sessio.
- The UI should show that orchestration stopped, not that delegated agent work disappeared.

### Memory Failure

- If the memory backend is unavailable, `sessio.memory.search` returns an explicit tool error.
- Astra should continue with project/thread/stage snapshot context.
- The orchestration run should not fail solely because memory search failed.

### Recovery

Persist minimal run metadata:

- run id
- thread id
- project id
- status
- mode
- proposed tasks
- run-level authorization metadata retained for compatibility
- delegated session ids
- delegated task results
- current stage id
- completed task ids
- stage attempt counts
- retry limit configuration
- created and updated timestamps

On app restart:

- active runs become `interrupted`
- delegated ACP sessions remain linked through existing thread/stage session links
- delegated tasks with a `threadStageId` must link their runtime session under that stage's `sessions`; only non-stage thread tasks link at the thread top level
- users can inspect the interrupted loop position, delegated task results, and per-stage attempt counts
- users can inspect or continue the delegated sessions through normal Sessio UI

## Phased Implementation Plan

Each phase is independently shippable and ends with an explicit exit check. Phases 2–5 build the closed loop in dependency order. Step numbers in parentheses map back to the flat steps this plan replaces, so no work item is lost.

**Phase 0 — Sidecar skeleton & protocol handshake** (steps 1–5)
- Create `sidecars/sessio-astra` (TypeScript, package scripts, stdio JSONL entrypoint); Astra bootstrap on `@earendil-works/pi-ai` + `@earendil-works/pi-agent-core`; sidecar protocol helpers (read JSONL, write responses/events, call Sessio tools).
- Rust: protocol structs + JSONL framing; `AstraService` with sidecar startup, request routing, crash detection, timeout handling, and cancellation.
- Exit: JSONL parser unit tests (partial lines, invalid JSON, unknown method, duplicate id, missing `protocolVersion`); compiled-sidecar smoke test (handshake, agent init, `getModel`, no-network planning path); clean start/stop.

**Phase 1 — Read-only context, proposal & UI** (steps 6–8)
- Read-only tools first: `sessio.project.snapshot`, thread/stage snapshot, `sessio.memory.search`, and `sessio.agent.plan_task` (proposal only, no ACP start).
- Tauri commands + frontend API wrappers for start/cancel/list; Thread-level "Astra" UI with run-level start authorization and historical plan/task display.
- Exit: starting on a real thread returns a run handle and emits a `plan` event whose proposals carry valid thread/stage ids; memory failure degrades to a tool error without aborting the run.

**Phase 2 — Run-authorized blocking dispatch (Goals 1–2)** (step 9)
- Active-run task dispatch through existing `RuntimeManager`, blocking each Astra tool response until the delegated ACP turn reaches a terminal state and returning `AstraTaskResult`; timeout + cancel unblock it.
- Exit: dispatch fails for inactive runs and tasks not registered by Astra; an active run dispatches a task and returns a structured result at the turn's terminal state; a permission request mid-turn blocks then resumes; cancel kills only this run's sessions.

**Phase 3 — Stage update & issue: decide → execute → report (Goals 3–4)** (step 11)
- `sessio.stage.update` and `sessio.stage.issue.add_or_update` as decision-submission tools: strict Rust-side validation, Sessio-side execution (store APIs / `sessio` CLI stage entrypoint), and an `AstraStageMutationResult` (success or structured error) returned to Astra.
- Exit: both success and failure of a stage mutation reach Astra; invalid ids / inactive run mutations are rejected in Rust; Astra cannot treat a failed mutation as a completed stage.

**Phase 4 — Autonomous loop & termination (Goals 5–6)** (step 10)
- The automatic orchestration loop: snapshot refresh after each plan queue is consumed, queued task dispatch, and the Astra terminal decision that emits `complete`.
- Exit: after the initial run-level authorization the loop advances stages with no per-task prompts; Astra's terminal `complete` marks the run `completed`; loop progress (`currentStageId`, completed task ids) is visible in the UI.

**Phase 5 — Retry & Sessio threshold circuit breaker (Goals 7–8)** (steps 10, 13)
- Same-stage retry branch in the loop; Sessio-side retry-limit enforcement so repeated dispatches for the same stage are refused with `retryLimitReached`, forcing Astra to re-decide (switch agent / change prompt / split task / mark stage blocked / abort run).
- Exit: retrying the same stage increments Sessio's per-stage attempt count; reaching the threshold circuit-breaks and routes Astra into the re-decide branch without a stuck loop.

**Phase 6 — Persistence, recovery & observability** (steps 12, 16)
- Persist minimal run metadata (loop position, delegated task results, per-stage attempt counts, retry limit configuration); structured logs tagged with `runId`, `threadId`, `taskId`, `threadStageId`, and delegated `sessioRuntimeSessionId`.
- Exit: on restart active runs become `interrupted` with inspectable loop position/results/attempt counts; sidecar crash marks the run `errored` while delegated ACP sessions stay visible; integration tests cover crash/restart/cancel.

**Phase 7 — Packaging & release** (steps 14–15)
- Bun sidecar scripts for dev/typecheck/JS build/target-specific compiled binaries; Tauri `externalBin` configuration with compiled binaries copied to `src-tauri/binaries`; debug launches source, release launches the bundled binary.
- Exit: compiled-sidecar smoke test, `tauri build --no-bundle` with the binary present, and `pnpm check` all pass.

## Test Plan

### Document Verification

After this document is written:

```bash
rg -n "Sessio Astra Plan|AstraService|start_thread_astra|externalBin" docs/sessio-astra-plan.md
git status --short docs/sessio-astra-plan.md
```

### Unit Tests

- JSONL parser handles partial lines, invalid JSON, unknown methods, and duplicate ids.
- Protocol decoding rejects missing `protocolVersion`.
- Tool bridge rejects unknown tool names.
- `sessio.agent.dispatch_task` rejects inactive runs and tasks not registered by Astra.
- `sessio.agent.dispatch_task` returns an `AstraTaskResult` when a delegated ACP turn completes, fails, is cancelled, or errors.
- Stage and issue mutation requests are validated and executed by Sessio, then return success/failure to Astra.
- Astra cannot bypass Sessio validation by invoking CLI or shell-based state mutation paths.
- Sessio increments stage attempt counts and returns `retryLimitReached` after the configured threshold.
- Event mapping is deterministic for plan, tool, error, and completion events.

### Integration Tests

- Starting orchestration on a thread returns a run handle and emits a plan event.
- Plan proposals contain valid thread and stage ids.
- Starting a run grants automatic dispatch only for Astra-registered tasks in that run.
- Cancelling before the first dispatch starts no ACP sessions.
- Cancelling after dispatch cancels only sessions launched by that orchestration run.
- The loop refreshes the snapshot after each task result and stage mutation result before choosing the next task.
- Successful stage mutation decisions advance to the next stage.
- Failed stage mutation decisions are reported to Astra and do not let Astra treat the stage as completed.
- Astra terminal completion marks the run `completed` and emits `complete`; Rust does not independently infer completion from stage state or approved task counts.
- Unsatisfactory task results can retry the same stage until Sessio's retry threshold is reached.
- Retry-limit refusal forces Astra into a different decision branch instead of another direct dispatch.
- Task failures create or update a stage issue and do not trigger unbounded retries.
- Sidecar crash marks the run `errored` while preserving delegated ACP session visibility.
- Memory backend failure produces a tool error but does not fail the full orchestration run.

### Build And Release Checks

- `pnpm check`
- Rust tests for protocol and orchestration service behavior.
- sidecar TypeScript typecheck through Bun project scripts.
- compiled sidecar smoke test: JSONL handshake, Astra agent initialization, model resolution, and fake/no-network planning path.
- sidecar binary exists at `src-tauri/binaries/sessio-astra-<target-triple>` for release builds.
- `tauri build --no-bundle` passes when the sidecar binary is present.

## Assumptions

- V1 is triggered from a Thread-level "Astra" action.
- V1 uses auto mode: clicking Astra authorizes one orchestration run, then Astra may continue planning and dispatching registered work within that run.
- Astra uses stdio JSONL rather than localhost HTTP.
- Astra is not added to the indexed historical `Agent` enum in V1.
- Sessio Rust remains the owner of ACP runtime lifecycle and permission routing.
- Astra does not directly execute the `sessio` CLI; internal orchestration state changes go through Sessio Rust service/tool bridge.
- Same-stage retry counts are maintained by Sessio per run and stage id, with a configurable retry limit defaulting to 3.
- When the retry limit is reached, Sessio refuses direct redispatch and notifies Astra so Astra can make a new decision.
- Release packaging uses Bun `--compile` to produce a self-contained sidecar binary so end users do not need Node or Bun installed.
- Bun is scoped to the sidecar project; the Sessio root project continues to use pnpm.
- The existing docs naming convention prefers `*-plan.md`, so this file is named `sessio-astra-plan.md`.
