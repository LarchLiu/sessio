# Sessio Astra Plan

## Summary

Sessio should use Astra as an internal thread-level orchestration intelligence built on Pi, not as a fourth user-facing runtime agent beside Codex, Claude, and Gemini.

The intended shape is:

- Astra acts as Sessio's orchestration brain.
- Sessio Rust remains the owner of runtime state, ACP sessions, permissions, thread/stage state, memory access, and historical indexing.
- ACP agents remain the execution workers.
- The first product entry point is a Thread-level "Astra" action.
- Astra proposes a thread plan first; one user confirmation is required before Sessio enters automatic execution for that run.
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
- expose Tauri commands for starting, confirming, and cancelling orchestration
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
confirm_thread_astra(runId, approvedTaskIds[]) -> void
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
- approved task ids
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
- `sessio.agent.dispatch_task` dispatches a planned task only after the run is confirmed, waits for the delegated ACP turn to reach a terminal state, and returns an `AstraTaskResult`.
- `sessio.stage.update` accepts Astra's stage mutation decision/request; Sessio validates and applies status, summary, and outcome through existing store APIs, then returns the updated stage or a structured error.
- `sessio.stage.issue.add_or_update` accepts Astra's issue mutation decision/request; Sessio validates and applies blocker or review finding changes through existing issue APIs, then returns the updated issue or a structured error.

These mutation tools are decision-submission interfaces, not CLI or shell execution paths. Astra does not execute the `sessio` CLI internally; Sessio is the component that mutates state and reports the result back to Astra.

### Orchestration Loop

1. User clicks Thread "Astra".
2. Rust loads the current thread work state and related memory context.
3. Rust starts or reuses the Astra sidecar.
4. Rust sends `astra/start` to Astra.
5. Astra inspects the snapshot and calls read-only tools as needed.
6. Astra emits a complete multi-stage task plan.
7. UI displays task cards with target stage, target agent, prompt, expected output, and risk.
8. User approves the full plan or a selected subset once.
9. Rust calls `confirm_thread_astra`.
10. Rust unlocks automatic dispatch for that run only.
11. Astra dispatches the next approved stage task through Sessio.
12. Sessio `RuntimeManager` owns the ACP session, handles normal permission UI, and returns an `AstraTaskResult` when the delegated turn reaches a terminal state.
13. Astra judges the task result and submits a stage or issue mutation decision to Sessio.
14. Sessio validates the decision, applies or rejects the mutation, and returns an `AstraStageMutationResult` to Astra.
15. Astra refreshes the snapshot, chooses the next task, and repeats the loop until all thread stages reach terminal state.
16. When the final stage is done, Sessio marks the Astra run `completed` and emits a `complete` orchestration event.

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
- All mutation decisions must go through the Rust `AstraService` tool bridge, where Sessio validates ids, run id, thread ownership, and approval state before applying any state change.
- `sessio.agent.dispatch_task` must fail until the run is confirmed by the user.
- After confirmation, automatic dispatch is limited to the approved plan, originating thread, and current run id.
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
- approved task ids
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

## Implementation Steps

1. Create `sidecars/sessio-astra` with TypeScript, package scripts, and a stdio JSONL entrypoint.
2. Add Astra bootstrap code using `@earendil-works/pi-ai` and `@earendil-works/pi-agent-core`.
3. Implement sidecar protocol helpers for reading JSONL, writing responses, writing events, and calling Sessio tools.
4. Add Rust protocol structs and JSONL framing helpers.
5. Add `AstraService` with sidecar startup, request routing, crash detection, timeout handling, and cancellation.
6. Add read-only tools first: project snapshot, thread/stage snapshot, and memory search.
7. Add Tauri commands and frontend API wrappers for start, confirm, and cancel.
8. Add Thread-level "Astra" UI with plan preview and task approval.
9. Add confirmed-run task dispatch through existing `RuntimeManager`, blocking each Astra tool response until the delegated ACP turn reaches a terminal state and returning `AstraTaskResult`.
10. Add the automatic orchestration loop, snapshot refresh, same-stage retry branch, and final-stage `completed` terminal condition.
11. Add stage update and issue decision-submission tools with strict Rust-side validation, Sessio-side execution, and success/failure result notification back to Astra.
12. Persist minimal orchestration run metadata, including loop position, task results, per-stage attempt counts, and retry limit configuration.
13. Add Sessio-side retry-limit enforcement so repeated dispatches for the same stage can be refused with `retryLimitReached`.
14. Add Bun sidecar scripts for dev, typecheck, JS build, and target-specific compiled binaries.
15. Add Tauri `externalBin` configuration and copy compiled binaries to `src-tauri/binaries`.
16. Add structured logs tagged with `runId`, `threadId`, `taskId`, `threadStageId`, and delegated `sessioRuntimeSessionId`.

## Test Plan

### Document Verification

After this document is written:

```bash
rg -n "Sessio Astra Plan|AstraService|confirm_thread_astra|externalBin" docs/sessio-astra-plan.md
git status --short docs/sessio-astra-plan.md
```

### Unit Tests

- JSONL parser handles partial lines, invalid JSON, unknown methods, and duplicate ids.
- Protocol decoding rejects missing `protocolVersion`.
- Tool bridge rejects unknown tool names.
- `sessio.agent.dispatch_task` rejects calls before run confirmation and rejects tasks outside the confirmed plan.
- `sessio.agent.dispatch_task` returns an `AstraTaskResult` when a delegated ACP turn completes, fails, is cancelled, or errors.
- Stage and issue mutation requests are validated and executed by Sessio, then return success/failure to Astra.
- Astra cannot bypass Sessio validation by invoking CLI or shell-based state mutation paths.
- Sessio increments stage attempt counts and returns `retryLimitReached` after the configured threshold.
- Event mapping is deterministic for plan, tool, error, and completion events.

### Integration Tests

- Starting orchestration on a thread returns a run handle and emits a plan event.
- Plan proposals contain valid thread and stage ids.
- Confirming a plan starts automatic dispatch only for approved tasks in that run.
- Cancelling before confirmation starts no ACP sessions.
- Cancelling after dispatch cancels only sessions launched by that orchestration run.
- The loop refreshes the snapshot after each task result and stage mutation result before choosing the next task.
- Successful stage mutation decisions advance to the next stage.
- Failed stage mutation decisions are reported to Astra and do not let Astra treat the stage as completed.
- Final-stage completion marks the run `completed` and emits `complete`.
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
- V1 uses auto mode: user confirmation approves the run plan once, then Astra may continue dispatching approved work within that run.
- Astra uses stdio JSONL rather than localhost HTTP.
- Astra is not added to the indexed historical `Agent` enum in V1.
- Sessio Rust remains the owner of ACP runtime lifecycle and permission routing.
- Astra does not directly execute the `sessio` CLI; internal orchestration state changes go through Sessio Rust service/tool bridge.
- Same-stage retry counts are maintained by Sessio per run and stage id, with a configurable retry limit defaulting to 3.
- When the retry limit is reached, Sessio refuses direct redispatch and notifies Astra so Astra can make a new decision.
- Release packaging uses Bun `--compile` to produce a self-contained sidecar binary so end users do not need Node or Bun installed.
- Bun is scoped to the sidecar project; the Sessio root project continues to use pnpm.
- The existing docs naming convention prefers `*-plan.md`, so this file is named `sessio-astra-plan.md`.
