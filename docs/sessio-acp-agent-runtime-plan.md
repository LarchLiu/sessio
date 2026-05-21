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

### Runtime Interface

Define one runtime trait around Sessio's concepts, not ACP's exact wire schema:

```rust
trait AgentRuntime: Send + Sync {
    fn agent(&self) -> AgentKind;
    fn transport_kind(&self) -> RuntimeTransportKind;
    async fn status(&self) -> Result<RuntimeStatus>;
    async fn start_session(&self, req: StartAgentSession) -> Result<AgentSessionHandle>;
    async fn send_input(&self, session_id: &str, req: AgentInput) -> Result<AgentTurnHandle>;
    async fn cancel_turn(&self, session_id: &str, turn_id: &str) -> Result<()>;
    async fn load_session(&self, session_id: &str) -> Result<AgentSessionHandle>;
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
load_agent_session(agent, runtime_session_id, workspace_path) -> AgentSessionHandle
get_agent_runtime_status(agent) -> RuntimeStatus
```

Runtime events should be pushed with Tauri events, for example:

```text
agent-runtime-event
```

The existing commands such as `list_sessions`, `get_session_messages`, `search_project_memory`, and `write_cross_prompt` should remain read/index APIs.

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
