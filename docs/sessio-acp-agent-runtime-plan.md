# Sessio ACP Agent Runtime Plan

## Goal

Sessio should keep its current role as the unified session index and memory layer, and add a separate runtime layer for real agent interaction. The runtime layer should use ACP as the primary protocol so Sessio can talk to agents through structured JSON-RPC events instead of scraping terminal output.

The target shape is:

- Sessio indexes Codex, Claude, and Gemini historical sessions as it does today.
- Sessio can also start, prompt, stream, cancel, and resume live agent runs.
- ACP is the preferred transport for live interaction.
- Agent-specific CLI stream modes remain fallback transports where ACP is unavailable.
- The UI consumes one Sessio-owned event model, not raw ACP messages or raw CLI output.

## Why ACP First

Plain CLI interaction is fragile because it treats the agent like a terminal program. Sessio can pipe stdin and read stdout, but it has to infer whether output is assistant text, reasoning, tool use, permission requests, or errors. That breaks easily when CLI formatting changes.

ACP gives Sessio a structured client/server boundary:

```text
Sessio
  as ACP client
    initialize
    session/new or session/load
    session/prompt
    session/cancel
      ↓
Agent process
  as ACP server
    session/update events
    permission requests
    completion/error state
```

This is a better fit for Sessio because the runtime can preserve agent-native context management while still exposing stable UI events.

## Architecture

Add an `agent-runtime` layer beside the existing agents/sources, indexer, and memory layers:

```text
Sessio UI
  ↓ Tauri commands + events
agent-runtime
  ├─ AcpTransport                primary path
  ├─ CliStreamJsonTransport      fallback for structured CLI output
  └─ PlainCliTransport           last-resort fallback
      ↓
Codex / Claude / Gemini

agents/sources
  parse historical session files

indexer + memory
  maintain searchable project history
```

The current source layer should not become responsible for live runs. Agent sources parse historical sessions; runtimes drive active agent processes.

## Design Review

The overall implementation direction is sound. The most important boundary is already correct: historical `agents/sources` keep owning file discovery and parsing, while the new runtime layer owns live process control and streaming interaction. This matches Sessio's current architecture, where SQLite and memory records are the searchable source-of-truth metadata layer, not a replacement for agent-owned transcripts.

The plan is reasonable if the first implementation is kept intentionally narrow:

- Start with runtime infrastructure, fake runtime tests, and one real transport slice.
- Keep ACP as a protocol adapter, not as Sessio's public frontend model.
- Persist recovery/linking metadata only; let the existing source/indexer pipeline discover transcripts after the agent writes them.
- Treat CLI fallbacks as capability-reduced adapters, not as equivalent ACP implementations.

The main risks are around protocol drift, process lifecycle, and state reconciliation:

- ACP method names, capabilities, and update payloads must come from the official Rust SDK/schema (`agent-client-protocol`), rather than copied from examples or hand-written JSON-RPC structs.
- `agent-client-protocol-tokio` is not required for the current SDK slice: `agent-client-protocol 0.12.1` already exposes `AcpAgent`/`Stdio` subprocess transports, while the separate tokio helper crate currently tracks older SDK versions and could introduce duplicate protocol types.
- `session/cancel` is an ACP notification. `cancel_turn` can still return `Result<()>`, but that result should mean Sessio successfully sent cancellation and updated local state; completion is confirmed later by the pending `session/prompt` response or terminal runtime state.
- A single Sessio runtime session id should be distinct from the agent/ACP session id. Otherwise UI state, SQLite rows, and agent-native ids will become hard to reconcile.
- Permission requests are client-side ACP methods. The runtime manager needs to route them through Tauri/UI and reply exactly once, including cancelled/expired cases.
- Fallback transports cannot reliably emit all fine-grained events. The UI should surface capability flags so unsupported features do not look broken.

### Runtime Interface

Define one runtime trait around Sessio's concepts, not ACP's exact wire schema:

```rust
trait AgentRuntime: Send + Sync {
    fn agent(&self) -> AgentKind;
    fn transport_kind(&self) -> RuntimeTransportKind;
    async fn status(&self) -> Result<RuntimeStatus>;
    async fn start_session(&self, req: StartAgentSession) -> Result<AgentSessionHandle>;
    async fn ensure_session(&self, req: EnsureAgentRuntimeSession) -> Result<AgentSessionHandle>;
    async fn send_input(&self, session_id: &str, req: AgentInput) -> Result<AgentTurnHandle>;
    async fn cancel_turn(&self, session_id: &str, turn_id: &str) -> Result<()>;
    async fn load_agent_native_session(&self, session_id: &str) -> Result<AgentSessionHandle>;
}
```

The concrete ACP implementation owns:

- process launch
- JSON-RPC request ids
- protocol initialization
- ACP session ids
- incoming update dispatch
- permission response routing
- process shutdown and recovery

## Event Model

Expose one unified event stream from Rust to the frontend:

```rust
enum AgentRuntimeEvent {
    SessionStarted { agent, session_id, transport },
    TurnStarted { session_id, turn_id },
    TextDelta { session_id, turn_id, text },
    ReasoningDelta { session_id, turn_id, text },
    ToolStarted { session_id, turn_id, tool_id, name, input },
    ToolInputDelta { session_id, turn_id, tool_id, delta },
    ToolOutputDelta { session_id, turn_id, tool_id, delta },
    PermissionRequested { session_id, turn_id, request_id, tool_name, input },
    TurnCompleted { session_id, turn_id, result },
    TurnError { session_id, turn_id, error },
    SessionEnded { session_id },
}
```

ACP messages should be converted into this model at the runtime boundary. The frontend should not depend on ACP method names. This leaves room for CLI fallback transports to emit the same events.

## Context Ownership

For ACP sessions, the agent owns short-term conversation context. Sessio should not replay the entire transcript on every turn.

Sessio remains responsible for orchestration context:

- choosing when to create a new session vs load an existing one
- injecting project memory into the initial prompt or explicit context messages
- building cross-agent continuation prompts
- remembering the mapping between Sessio session records and runtime session ids
- recovering from disconnected runtimes
- exposing permission decisions through the UI

This keeps agent-native context behavior intact while still making Sessio the cross-agent control plane.

## Transport Fallbacks

The preferred order should be:

1. `AcpTransport`
   - Used when the agent exposes ACP directly or through a wrapper.
   - Supports structured session updates and permission flow.
2. `CliStreamJsonTransport`
   - Used when an agent has stable structured CLI streaming.
   - For example, Claude/Gemini-style `stream-json` modes can be parsed line by line.
3. `PlainCliTransport`
   - Used only as a compatibility escape hatch.
   - Emits coarse text/error events and should not be treated as a full feature runtime.

The desktop-cc-gui project is a useful reference for fallback behavior: it uses structured CLI streaming for Claude and Gemini, while Codex is driven through an app-server runtime rather than plain terminal scraping.

## Tauri API

Add runtime commands without changing existing session index commands:

```text
start_agent_session(agent, workspace_path, initial_prompt?, options?) -> AgentSessionHandle
send_agent_input(session_id, text, attachments?, options?) -> AgentTurnHandle
cancel_agent_turn(session_id, turn_id) -> void
ensure_agent_runtime_session(agent, sessio_runtime_session_id, workspace_path, agent_runtime_session_id?) -> AgentSessionHandle
load_agent_session(agent, runtime_session_id, workspace_path) -> AgentSessionHandle  # compatibility wrapper for ensure without ACP-native load
get_agent_runtime_status(agent) -> RuntimeStatus
```

`ensure_agent_runtime_session` is intentionally not the same as ACP `session/load`.
It binds an existing Sessio chat window/session id to an active runtime worker. When an
ACP-native `agent_runtime_session_id` is provided, the ACP worker may start with
`session/load`; otherwise it starts a fresh ACP `session/new` under the existing Sessio
runtime id. This keeps the UI id stable while avoiding the earlier ambiguity where
`load_agent_session` sounded like it always loaded agent-owned history.

Runtime events should be pushed with Tauri events, for example:

```text
agent-runtime-event
```

The existing commands such as `list_sessions`, `get_session_messages`, `search_project_memory`, and `write_cross_prompt` should remain read/index APIs.

## Chat UI Data Flow

The chat view should become a composed timeline: historical messages still come from indexed agent session files, while live runtime turns are appended from Sessio-owned runtime events until the indexer catches up.

```text
Chat composer
  submit text / attachments / options
    ↓
frontend runtime controller
  optimistic user message
  call start_agent_session or send_agent_input
    ↓ Tauri command
RuntimeManager
  creates/loads Sessio runtime session
  selects ACP transport
  writes runtime metadata
    ↓
AcpTransport
  session/new or session/load
  session/prompt
    ↓ JSON-RPC over stdio/socket
Agent ACP server
    ↓ session/update notifications + prompt response
AcpTransport
  converts ACP updates to AgentRuntimeEvent
    ↓ Tauri event: agent-runtime-event
frontend runtime reducer
  appends text deltas to the active assistant bubble
  appends reasoning/tool/permission state to the same turn
  finalizes turn on completion/error/cancel
    ↓
Chat window
  renders historical messages + live overlay
    ↓ later
agents/sources + indexer
  observes agent-owned transcript file
  updates historical SessionDetail data
    ↓
frontend reconciliation
  replaces matching live completed turns with indexed messages
  keeps any still-live turns in the overlay
```

Important rules:

- The composer should never write directly into historical session files.
- Streaming deltas should update an in-memory live message buffer keyed by Sessio runtime session id and turn id.
- The renderer should support incremental Markdown text without requiring every partial delta to be valid complete Markdown.
- A new assistant bubble should be created on `TurnStarted` or the first assistant delta, then mutated in place as `TextDelta` arrives.
- Tool, reasoning, and permission updates should attach to the current turn rather than becoming unrelated top-level messages unless the UX intentionally separates them.
- Auto-scroll should follow the stream only when the user is already near the bottom; reading older content should not be interrupted by incoming deltas.
- Auto-scroll has two separate timing hazards:
  - Initial session load cannot rely on a single `scrollIntoView` call because Markdown, code blocks, and media can change height after the first layout. Use a shared "scroll to bottom" helper that retries across animation frames and a short timeout.
  - Live streaming cannot let programmatic scroll events disable follow mode. Setting `scrollTop` also fires `onScroll`, so the live/streaming flag must be updated synchronously with render state before scroll handlers run; otherwise an intermediate non-bottom position can set `followLiveStream=false` and stop user/fake/agent messages from following the bottom.
- Once the indexer observes the persisted agent transcript, reconciliation should dedupe by runtime metadata and source refs so completed live bubbles do not appear twice.
- If ACP disconnects before the transcript is indexed, the live overlay should remain visible as a recovered/disconnected runtime turn.

## Persistence

Persist only runtime metadata needed for recovery and linking:

- `agent`
- `transport_kind`
- `runtime_session_id`
- `workspace_path`
- `started_at`
- `updated_at`
- `last_turn_id`
- `status`
- `source_session_id` when created from an indexed historical session

Do not duplicate full live transcripts into Sessio's runtime tables. Historical truth should still come from agent session files and the existing agents/sources and indexer pipeline. Runtime metadata exists to reconnect UI state to agent-owned sessions.

## Implementation Phases

### Phase 1: Runtime Shell

- Add runtime traits, shared event types, and Tauri event dispatch.
- Add an in-process fake runtime for tests.
- Keep UI minimal: start, send input, stream deltas, cancel.

### Phase 2: ACP Transport

- Implement ACP process launch and JSON-RPC initialization.
- Support session create/load, prompt, update stream, cancel, and permission responses.
- Add timeout and reconnect handling.

### Phase 3: Agent Adapters

- Add per-agent runtime config for binary path, args, environment, and workspace root.
- Prefer ACP where available.
- Add CLI stream-json fallback only for agents without ACP support.

### Phase 4: Sessio Integration

- Connect cross prompt generation to `start_agent_session`.
- Allow memory search results to be injected into new sessions.
- Link runtime sessions back to indexed session records after agent sources observe the new files.

## Testing

Test at three layers:

- Runtime unit tests with fake ACP server messages.
- Transport tests for JSON-RPC request/response routing and session update conversion.
- Integration tests using fake CLI binaries that emit ACP-like or stream-json events.

Required scenarios:

- start session and receive `SessionStarted`
- send input and stream multiple `TextDelta` events
- receive tool and permission events
- approve or reject a permission request
- cancel an active turn
- runtime process exits unexpectedly
- reconnect/load an existing session
- fallback transport emits the same unified event types

## Open Questions

- Which installed agents expose ACP directly today, and which require wrappers?
- Should Sessio provide its own ACP wrapper for non-ACP CLIs, or only support native ACP plus documented fallbacks?
- Should runtime metadata live in the existing SQLite database or a separate runtime database?
- How much memory context should Sessio inject automatically versus only on explicit user action?

The recommended default is ACP-first with CLI stream-json fallback, runtime metadata in the existing SQLite store, and explicit memory injection for the first version.

## Implementation TODOs

### Phase 0: Protocol and Capability Spike

- [x] Pin the ACP SDK/schema version used by Sessio and record the source URL or vendored schema location. Sessio now depends on `agent-client-protocol = 0.12.1`, which uses `agent-client-protocol-schema = 0.13.2`; source: https://github.com/agentclientprotocol/rust-sdk.
- [ ] Verify current ACP capabilities for Codex, Claude, and Gemini installations: native ACP, wrapper needed, structured CLI only, or plain CLI only.
- [ ] Map ACP update variants into `AgentRuntimeEvent`, including agent message chunks, thought/reasoning chunks, tool call lifecycle, plan updates, mode updates, and permission requests. Initial SDK-backed mapping covers `SessionNotification` message chunks, thought chunks, tool calls, tool output, and `RequestPermissionRequest`; fake runtime now emits SDK schema types instead of hand-written ACP JSON.
- [ ] Decide how Sessio exposes runtime capabilities to the UI, for example `supportsCancel`, `supportsPermissions`, `supportsToolDeltas`, `supportsResume`, `supportsAttachments`, and `supportsModes`.
- [ ] Define a stable id strategy: Sessio runtime session id, ACP session id, turn id, tool id, and permission request id must be separate fields.
- [x] Split current-window runtime binding from ACP-native session loading. `ensure_agent_runtime_session` owns the Sessio runtime id binding, while ACP `session/load` is only attempted when a distinct agent runtime session id is supplied.
- [ ] Decide whether `AgentKind` from `agents/sources/types.rs` should become the runtime-facing agent id immediately, or whether runtime v1 should bridge from the existing `models::Agent` enum.

### Phase 1: Runtime Shell

- [x] Add `src-tauri/src/agents/runtime/` with `types.rs`, `registry.rs`, `fake.rs`, and transport-specific modules.
- [x] Define serializable request/response types: `StartAgentSession`, `AgentSessionHandle`, `AgentInput`, `AgentTurnHandle`, `RuntimeStatus`, `RuntimeCapabilitySet`, and `RuntimeError`.
- [x] Define `AgentRuntimeEvent` with enough metadata for UI ordering: monotonic sequence number, wall-clock timestamp, runtime session id, agent session id, and optional turn/tool ids.
- [x] Define frontend-facing live chat entities: `LiveRuntimeSession`, `LiveTurn`, `LiveMessagePart`, `LiveToolCall`, `LivePermissionRequest`, and `LiveRuntimeStatus`.
- [x] Add a `RuntimeManager` that owns active sessions, dispatches events, serializes sends per session, and prevents two active prompt turns on the same runtime session unless a transport explicitly supports it.
- [x] Add an in-process fake runtime that can script deltas, tool calls, permission requests, errors, cancellation, and process exits.
- [x] Add Tauri commands and `agent-runtime-event` dispatch behind the manager without changing current list/detail/memory commands.
- [x] Add frontend API types in `src/api.ts` that mirror the Rust event model, but keep UI changes minimal for this phase.
- [x] Add a frontend runtime reducer that consumes `agent-runtime-event`, appends deltas to existing live turns, and emits immutable state updates for React rendering.
- [ ] Make runtime commands return only short-lived acknowledgements/handles; all long-running prompt progress, completion, and errors must be delivered through `agent-runtime-event`, not through pending invoke callbacks.
- [ ] Add command/request ids to runtime events so the frontend can correlate start/send acknowledgements with event-stream state without keeping long invoke callbacks alive.

### Phase 2: Runtime Persistence

- [ ] Add SQLite migration for `runtime_sessions` with `sessio_runtime_session_id`, `agent`, `transport_kind`, `agent_runtime_session_id`, `workspace_path`, `source_session_id`, `status`, timestamps, and last error.
- [ ] Add optional reconciliation fields for `indexed_agent`, `indexed_session_id`, `indexed_file_path`, and `indexed_at` once the source/indexer finds the transcript.
- [ ] Add a `RuntimeStore` trait rather than folding runtime writes into `SessionStore`.
- [ ] Persist status transitions for starting, active, idle, cancelling, errored, disconnected, and ended states.
- [ ] Store only recovery/linking metadata; do not store full transcript text in runtime tables.
- [ ] Add cleanup rules for stale active sessions found on app startup.
- [ ] Add a reconciliation task that links runtime sessions to indexed `sessions` rows after the source/indexer observes the agent's persisted transcript.
- [ ] Add frontend reconciliation logic that hides completed live turns once their indexed equivalents are loaded into `SessionDetail`.

### Phase 3: ACP Transport

- [x] Add the official Rust SDK dependency and avoid hand-written ACP JSON-RPC framing, request id allocation, response correlation, and notification dispatch.
- [x] Add an SDK-backed ACP worker scaffold using `AcpAgent`, `Client.builder()`, typed `InitializeRequest`, `NewSessionRequest`, `PromptRequest`, `CancelNotification`, `SessionNotification`, and `RequestPermissionRequest`.
- [x] Keep `agent-client-protocol-tokio` out for now because the main SDK crate provides the needed subprocess/stdin/stdout transport and the separate tokio helper crate is version-skewed with `agent-client-protocol 0.12.1`.
- [x] Implement ACP `initialize` and capability negotiation before exposing a runtime as ready for explicitly requested ACP sessions.
- [x] Implement the ACP worker start-mode split for `session/new` versus `session/load`; `session/load` checks the SDK-reported `agent_capabilities.load_session` flag and returns a startup error when unsupported.
- [ ] Persist and pass real agent-native session ids from indexed/history records so existing historical sessions can choose ACP `session/load` instead of starting `session/new`.
- [x] Start ACP prompt work in a detached runtime task after `send_agent_input` has returned an `AgentTurnHandle`, so hot reloads cannot strand Tauri invoke callback ids while the agent is still streaming.
- [x] Treat `session/cancel` as fire-and-follow-up: send the typed SDK `CancelNotification`, mark local turn cancelled, answer pending permission requests as cancelled, and let prompt completion reconcile final state.
- [x] Implement `session/request_permission` routing through the runtime manager and Tauri event stream; ensure every request receives exactly one approve/reject/cancel response. Fake and real ACP paths share the same manager-side permission waiter.
- [x] Convert ACP `session/update` notifications into `AgentRuntimeEvent` at the transport boundary using SDK `SessionNotification` / `SessionUpdate` types.
- [ ] Handle agent-side JSON-RPC errors with structured `RuntimeError` values that preserve code, message, and optional data.
- [ ] Add process supervision: startup timeout, prompt timeout, idle timeout, unexpected exit handling, restart policy, and reconnect/load behavior.
- [ ] Add logging with redaction for prompts, environment variables, file contents, and permission payloads.

### Phase 4: Agent Runtime Config

- [ ] Extend `~/.sessio/config.toml` with an `[agents.runtime.<agent>]` shape for binary path, args, environment, cwd/workspace behavior, preferred transport, and disabled transports.
- [x] Add initial `[agents.runtime.<agent>]` config parsing for explicit `transport` and `command`, with per-call runtime options still able to override config.
- [ ] Add binary discovery/status checks for each built-in agent.
- [ ] Make workspace paths absolute and validate they are allowed before launching any runtime process.
- [ ] Define per-agent default launch commands separately from user overrides.
- [ ] Add status diagnostics that explain why ACP is unavailable and which fallback, if any, will be used.
- [ ] Keep runtime config independent from memory backend config.

### Phase 5: CLI Stream Fallbacks

- [ ] Implement `CliStreamJsonTransport` only for agents with stable machine-readable streaming output.
- [ ] Define a per-agent parser contract for stream-json events and map unsupported fields to capability flags.
- [ ] Implement `PlainCliTransport` as text/error only, with no permission/tool guarantees.
- [ ] Add fallback selection rules: configured transport first, ACP if available, structured stream-json next, plain CLI only when explicitly allowed or when the UI accepts reduced capability.
- [ ] Add tests proving fallback transports emit the same required envelope fields even when they omit fine-grained event variants.

### Phase 6: UI Integration

- [x] Add a bottom chat composer to the existing chat view, visually matching the compact rounded panel: placeholder `Ask, Search or Chat...`, left mode button, `Auto` mode label, context usage label, and circular send button.
- [x] Make the composer support multiline input, submit-on-enter, newline-on-shift-enter, disabled/loading states, and pending send feedback.
- [x] Add a mode selector for `Auto` and future modes without blocking the v1 ACP path on implementing every mode.
- [x] Wire composer submit to `start_agent_session` when no live runtime session exists for the selected workspace, otherwise to `send_agent_input`.
- [x] Keep the composer fixed to the bottom of the chat pane while the message timeline scrolls behind/above it with enough bottom padding.
- [x] Add a runtime panel or composer controller that can start a live session from a workspace and stream events.
- [x] Compose rendered chat items from indexed `SessionMessage[]` plus live runtime overlay turns in timestamp/sequence order.
- [x] Render streaming assistant text by mutating/appending to the current assistant bubble instead of inserting one message per delta.
- [x] Reuse the existing Markdown renderer for streamed assistant text, but tolerate incomplete Markdown fences, tables, lists, and math while the turn is in progress.
- [x] Add a subtle pending indicator for active runtime turns before assistant text arrives.
- [ ] Render reasoning, tool calls, tool output, permission requests, errors, and cancellation state as nested turn blocks that can update while streaming. Initial live overlay rendering covers reasoning, tool call/result pairs, permission request status, runtime status, and errors.
- [x] Add turn state rendering for pending, streaming-before-text, cancelling, failed, and cancelled turns.
- [ ] Add permission prompt UI with approve, reject, and cancel paths. Initial live overlay supports approve/reject for fake runtime permission requests; explicit cancel/timeout states remain.
- [ ] Disable or hide unsupported controls based on runtime capability flags.
- [x] Add near-bottom auto-scroll behavior for live streams and preserve manual scroll position when the user scrolls up.
- [x] Add context usage display, initially backed by runtime status or a placeholder value when the transport cannot report token/context usage.
- [ ] Connect existing cross-agent continuation generation to `start_agent_session` as an optional launch path.
- [ ] Add explicit memory injection controls for v1 instead of automatic background injection.
- [ ] Show the link between a live runtime session and its indexed historical session after reconciliation succeeds.
- [x] Add empty/no-selection behavior for the composer: disabled with explanation when no project/workspace can be resolved, enabled when a workspace is selected.
- [x] Add keyboard focus behavior so opening a live chat session focuses the composer without stealing focus during streaming.

### Phase 7: Testing and Verification

- [ ] Unit-test runtime manager ordering, active-turn locking, cancellation state, and permission response routing.
- [ ] Test that start/send commands return promptly while the fake/ACP runtime continues to emit stream events after the invoke callback has completed.
- [ ] Unit-test the frontend runtime reducer for delta append, duplicate event ignore, out-of-order sequence handling, completion, cancellation, and permission response updates.
- [ ] Unit-test live/historical reconciliation so completed live messages do not duplicate after indexed messages reload.
- [ ] Rely on the Rust SDK for ACP JSON-RPC request/response matching and add Sessio tests for worker startup timeout, unknown notification handling, malformed payload handling, and request timeout behavior.
- [x] Unit-test ACP update-to-event conversion using SDK schema types for message chunks, tool calls, tool output, and permission requests.
- [ ] Integration-test fake ACP server flows: start, prompt, deltas, tool calls, permission approve/reject, cancellation, process exit, and load existing session.
- [ ] Integration-test fake structured CLI binaries for Claude/Gemini-style stream-json fallback.
- [ ] Add component tests or Playwright checks for composer layout at narrow and wide widths, streaming text append, auto-scroll, and manual scroll preservation.
- [ ] Add regression coverage for chat auto-scroll timing: initial historical load with late content height changes, optimistic user message append, streamed fake/agent deltas, and manual scroll-up preservation.
- [ ] Verify streaming Markdown does not break the chat window while code fences, tables, math, or lists are incomplete.
- [ ] Test runtime persistence recovery after app restart with active, errored, disconnected, and ended sessions.
- [ ] Verify `cargo test`, `cargo check`, and `pnpm run typecheck` after each vertical slice.
- [ ] Add manual smoke commands or scripts for starting one real agent session from a local workspace once a real transport lands.

### Completed Verification Log

- [x] `cargo check` passed after adding the fake agent runtime shell, Tauri runtime commands, and `agent-runtime-event` dispatch.
- [x] `cargo test` passed after adding fake runtime unit coverage.
- [x] `pnpm run typecheck` passed after adding runtime API types and the frontend live runtime reducer.
- [x] `cargo check`, `pnpm run typecheck`, and `pnpm run build` passed after wiring the bottom composer to fake runtime streaming and live chat overlay rendering.

### Deferred

- [ ] Multi-agent simultaneous orchestration from one UI workflow.
- [ ] Automatic memory/context injection policy beyond explicit user action.
- [ ] Runtime transcript mirroring or independent transcript search before the source/indexer observes agent-owned files.
- [ ] Dynamic third-party agent runtime plugins.
- [ ] Cross-device or remote runtime execution.
