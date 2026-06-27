# Computer Use Implementation Plan v3

> A code-verified plan for adding `computer use` to Sessio, rewritten after
> validating the current ACP SDK, Sessio runtime, and the latest official ACP
> adapters on 2026-06-28. Every claim about the local codebase is tagged with the
> file/line it was checked against. External adapter claims link to the source
> repositories that were inspected that day.

## Summary

Add a session-scoped `computer use` capability to Sessio that injects a
desktop-hosted `computer_*` tool server into compatible ACP agents at session
start, while keeping all privileged OS operations (screenshot, accessibility
inspection, input injection) inside the Sessio desktop process.

The core change from v2 is the transport choice:

- `computer use` is still feasible.
- The `unstable_mcp_over_acp` / `McpServer::Acp` path is not a viable MVP target
  for the current official adapters.
- The default ACP shape must therefore be a **desktop-owned localhost HTTP MCP
  server**, injected through `NewSessionRequest.mcp_servers` as
  `McpServer::Http`.

This keeps the product goal intact, but changes the implementation and security
shape in important ways. The biggest technical risk remains the same as before:
input injection and accessibility-tree inspection are net-new privileged
capabilities.

## Why v3 Replaces v2

v2 improved on v1 by centering ACP-native MCP injection. That part was right.
What changed after deeper verification is **which MCP transport is actually
usable today**.

### 1. ACP-native MCP injection exists, but `McpServer::Acp` is not the usable path

Sessio now depends on `agent-client-protocol = 1.0.0` with
`unstable_mcp_over_acp` enabled locally
([src-tauri/Cargo.toml:28](../src-tauri/Cargo.toml)), and the schema does expose
all four MCP server variants:

- `McpServer::Http`
- `McpServer::Sse`
- `McpServer::Acp`
- `McpServer::Stdio`

See the installed ACP schema source at
`agent-client-protocol-schema-1.1.0/src/v1/agent.rs:2658`.

So v2 was right that tool injection belongs in `session/new.mcp_servers`. The
wrong part was treating `McpServer::Acp` as the default implementation shape.

### 2. The latest official ACP adapters do not currently support MCP-over-ACP

Verified on 2026-06-28 against the current `main` branches:

- `agentclientprotocol/codex-acp`
  ([repo](https://github.com/agentclientprotocol/codex-acp))
- `agentclientprotocol/claude-agent-acp`
  ([repo](https://github.com/agentclientprotocol/claude-agent-acp))

What matters is not whether the SDK schema contains `McpServer::Acp`, but
whether the adapters:

1. advertise `mcpCapabilities.acp = true` in `initialize`, and
2. actually accept and route ACP MCP servers at `session/new`.

They do not.

`codex-acp`:

- Declares `mcpCapabilities: { acp: false, http: true, sse: false }` in its
  initialize response.
- Therefore explicitly rejects MCP-over-ACP as a capability, while allowing HTTP
  MCP.

Source:

- [CodexAcpServer.ts](https://raw.githubusercontent.com/agentclientprotocol/codex-acp/main/src/CodexAcpServer.ts)
- [package.json](https://raw.githubusercontent.com/agentclientprotocol/codex-acp/main/package.json)

`claude-agent-acp`:

- Declares HTTP/SSE MCP capability, but not `acp`.
- Its session creation path handles MCP server configuration in HTTP/SSE/stdio
  terms, not ACP transport terms.

Source:

- [acp-agent.ts](https://raw.githubusercontent.com/agentclientprotocol/claude-agent-acp/main/src/acp-agent.ts)
- [package.json](https://raw.githubusercontent.com/agentclientprotocol/claude-agent-acp/main/package.json)

This is the key reason v2 must be rewritten: the attractive "desktop process +
`McpServer::Acp` + no local network surface" story is not the shape the current
adapters actually support.

### 3. Sessio's current ACP runtime still hand-builds `session/new`

Sessio already probes ACP capabilities and now surfaces MCP injection support in
`RuntimeCapabilitySet.mcp_injection`
([src-tauri/src/agents/runtime/types.rs:33](../src-tauri/src/agents/runtime/types.rs),
[acp_transport.rs:204](../src-tauri/src/agents/runtime/acp_transport.rs:204)).

But the runtime still starts a new ACP session by:

- building `NewSessionRequest` by hand
- calling `connection.send_request(request)`

See [acp_transport.rs:470](../src-tauri/src/agents/runtime/acp_transport.rs:470) and
[acp_transport.rs:837](../src-tauri/src/agents/runtime/acp_transport.rs:837).

So even ignoring adapter support, `v2` still needed real runtime work before an
in-process ACP MCP server could survive for a full session.

### 4. Pi is a different transport and should not gate the ACP MVP

Pi does not use ACP at all. Its runtime path sends `new_session {}` and
`get_commands` over Pi RPC
([src-tauri/src/agents/runtime/pi_rpc_transport.rs:295](../src-tauri/src/agents/runtime/pi_rpc_transport.rs:295)).

That means Pi should not participate in the ACP injection decision for the first
release. A Pi-specific extension path may still make sense later, but it is a
separate design and should not block the ACP MVP.

## Chosen Direction

For ACP agents, `computer use` should ship first as a:

**desktop-owned, loopback-only HTTP MCP server with per-session auth**

The desktop process starts the server before `session/new`, injects it into
`NewSessionRequest.mcp_servers` as `McpServer::Http`, and handles all tool calls
itself.

This is the most practical route because:

- `Http` is supported by the current official `codex-acp` and `claude-agent-acp`
  adapters.
- It keeps privileged logic in the desktop process.
- It avoids a helper sidecar binary in the MVP.
- It does not depend on `unstable_mcp_over_acp`.

What changes compared with v2 is the security model:

- this is no longer "free" in-process dispatch
- there is now a real localhost server surface
- the server must be loopback-only and authenticated per session

That added local auth complexity is still cheaper than building a separate
sidecar process plus broker for the initial ACP rollout.

## Why HTTP, Not ACP/SSE/Stdio

| Transport | MVP status | Why |
| --- | --- | --- |
| `McpServer::Http` | Use by default | Supported by current official adapters and can be hosted directly inside Sessio desktop |
| `McpServer::Acp` | Do not use for MVP | Current official adapters do not advertise `mcpCapabilities.acp = true` |
| `McpServer::Sse` | Do not use for MVP | `codex-acp` does not advertise SSE support |
| `McpServer::Stdio` | Keep as fallback only | Would force a spawned helper process or a broker shape immediately |

This comparison is the architectural heart of v3.

## MVP Scope

### In scope

- Desktop chat
- macOS first
- ACP agents that advertise HTTP MCP support
- Session-scoped opt-in `computerUse`
- Observe/inspect/control tool family
- Desktop-owned localhost HTTP MCP server

### Out of scope for MVP

- Pi runtime support
- IM bridge
- scheduled tasks
- thread orchestration
- Linux parity
- raw pixel click/drag primitives
- unattended background control

## Architecture

```text
Chat UI
  ↓ session-scoped runtime option (computerUse)
RuntimeManager
  ├─ starts an ACP session only when requested + eligible
  ├─ starts a localhost HTTP MCP server in the desktop process
  ├─ injects it via NewSessionRequest.mcp_servers
  └─ keeps Sessio session ↔ MCP auth token ↔ local port mapping
      ↓
ACP agent runtime
  ├─ initialize → advertises mcpCapabilities
  ├─ session/new with McpServer::Http
  └─ tool calls over loopback HTTP
      ↓
Sessio desktop-owned HTTP MCP server
  ├─ validates loopback + bearer token + session scope
  ├─ exposes computer_* tool schema
  └─ dispatches directly into the computer_use host module
      ↓
Sessio desktop host (computer_use module)
  ├─ desktop-control permission checks
  ├─ screenshot / snapshot capture
  ├─ accessibility / element-tree inspection
  ├─ input injection
  └─ approvals + foreground takeover UI
```

## Why This Architecture Is Better Than v2

### It matches what the adapters actually support

The biggest reason is simple: we can ship against current real agents without
waiting for `McpServer::Acp` support upstream.

### It preserves the important product boundary

The critical product requirement was never "no localhost port." It was:

- privileged desktop control stays in Sessio
- the user sees a session-scoped opt-in
- the tool is injected only when the session starts
- toggling is recreate-only, never hot-plug

HTTP MCP preserves all of those.

### It avoids premature sidecar complexity

If we jumped straight to stdio sidecar or a separate `sessio-computer-use` bin,
we would pay immediately for:

- process lifecycle management
- IPC protocol design
- sidecar packaging/discovery
- broker authentication

Embedding the HTTP MCP server in the desktop process avoids most of that for the
ACP MVP. We still need auth, but not an extra executable.

## New Host-Owned Module

```text
src-tauri/src/computer_use/
  mod.rs
  host.rs         # provider selection, orchestration
  lease.rs        # lease lifecycle, snapshot staleness
  settings.rs
  permissions.rs  # consumes the shared desktop-control permission layer
  approvals.rs    # session + app approvals
  provider.rs     # capture / inspect / control behind one interface
  mcp_http.rs     # localhost MCP server, auth, request routing
```

## Tool Model

Stateful and conservative. Public surface:

- `computer_status`
- `computer_list_apps`
- `computer_start`
- `computer_get_app_state`
- `computer_click_element`
- `computer_type_text`
- `computer_press_key`
- `computer_scroll`
- `computer_stop`

Flow:

1. the agent opens a lease
2. the agent requests app state
3. Sessio returns screenshot + display metadata + elements + snapshot id + allowed actions
4. actions must reference the latest snapshot id

No raw coordinate tools in the MVP.

## Permission Model

Generalize the existing Appshot permission layer into a shared desktop-control
layer.

What already exists and should be reused:

- macOS screenshot permission preflight via `ScreenCaptureAccess.preflight()`
  ([src-tauri/src/lib.rs:4628](../src-tauri/src/lib.rs:4628))
- macOS accessibility trust check via `AXIsProcessTrustedWithOptions`
  ([src-tauri/src/lib.rs:4640](../src-tauri/src/lib.rs:4640))
- native permission onboarding panel
  ([src-tauri/src/lib.rs:4688](../src-tauri/src/lib.rs:4688))
- shared derived desktop-control state
  ([src-tauri/src/lib.rs:4603](../src-tauri/src/lib.rs:4603))

Required shared status:

```ts
interface DesktopControlPermissionStatus {
  platform: "macos" | "windows" | "linux" | "other" | string;
  requiresPermission: boolean;
  screenshots: { granted: boolean; supported: boolean };
  accessibility: { granted: boolean; supported: boolean };
  canObserve: boolean;
  canInspect: boolean;
  canControl: boolean;
}
```

Why this matters more in v3:

- the HTTP MCP server must not expose "control" tools when policy or platform
  says only observation is allowed
- Appshot and `computer use` now share one permission source, but consume it with
  different semantics

## Net-New Privileged Capabilities

These remain the hardest part of the project:

- **Input injection (`canControl`)**
  Existing dependencies still stop at `core-graphics` / `core-foundation`; there
  is no input provider yet
  ([src-tauri/Cargo.toml:20](../src-tauri/Cargo.toml:20)).
- **AX element-tree inspection**
  Existing accessibility code only checks trust; it does not enumerate elements.
- **Snapshot capture**
  The exact reusable macOS capture path still needs to be isolated for repeated
  session snapshots.

This is still where most of the real engineering time lives.

## Runtime Integration

### Agent eligibility

Sessio already has the right place to gate this:

- `RuntimeCapabilitySet.mcp_injection`
  ([src-tauri/src/agents/runtime/types.rs:33](../src-tauri/src/agents/runtime/types.rs:33))
- `runtime_capabilities_from_acp()`
  ([src-tauri/src/agents/runtime/acp_transport.rs:204](../src-tauri/src/agents/runtime/acp_transport.rs:204))
- `RuntimeAgentMetadata.computer_use_eligible`
  ([src-tauri/src/models.rs:1583](../src-tauri/src/models.rs:1583))

For v3, ACP eligibility should be:

`mcp_injection.http == true` **and** Sessio explicitly supports the computer-use
contract for that agent/version.

`mcp_injection.acp` should not gate the MVP, because the current adapters do not
support it.

### Launch-time injection

Sessio still builds `NewSessionRequest` directly
([src-tauri/src/agents/runtime/acp_transport.rs:837](../src-tauri/src/agents/runtime/acp_transport.rs:837)).

So the injection work for v3 is:

1. start a localhost HTTP MCP server before `session/new`
2. generate an ephemeral loopback URL and bearer token
3. append `McpServer::Http(McpServerHttp::new(name, url).headers(...))`
4. send `session/new`
5. retain the server handle for the whole session lifetime
6. shut it down on session end / recreate / transport failure

### Session option semantics

`computerUse` still needs two semantics:

- transient launch option for the immediate next session
- persisted config if restore/recreate should keep it

Today Sessio persists only `model`, `effort`, and `permissionMode`
([src-tauri/src/agents/runtime/manager.rs:1616](../src-tauri/src/agents/runtime/manager.rs:1616)).

So v3 still requires an explicit persistence decision.

### No hot-plug

This remains unchanged and should stay unchanged.

Changing `computerUse` on a running session means:

- tear down current session after the turn boundary, or
- state clearly that the change applies to the next session

Never live-discover a new tool mid-session.

## HTTP MCP Security Model

This is the main new section v2 did not need.

The desktop-owned HTTP MCP server must:

- bind only to `127.0.0.1` / loopback
- listen on an ephemeral port
- require a per-session random bearer token or equivalent header
- reject requests that do not map to a live `computerUse` session
- expire tokens when the session ends
- avoid serving any non-session-global inspection endpoint without auth

Why this is required:

- the ACP agent is a separate child process, not in-process code
- localhost is not a trust boundary by itself
- multiple Sessio sessions may exist concurrently

The correct mental model is "desktop-owned local service with per-session
capability auth," not "harmless internal web server."

## UI Changes

- **Chat composer:** session-scoped `computer use` toggle, visible only when the
  selected agent is eligible
- **Settings:** `Desktop Control` section for platform support, permission tiers,
  active lease, and approval state
- **Foreground takeover:** explicit warning + reliable cancel path for actions
  that need foreground control

## Platform Strategy

- **macOS:** first and primary target
- **Windows:** follow once host/provider shape is stable
- **Linux:** disabled or observation-only initially
- **Pi:** excluded from MVP; revisit after ACP agents land

## Phase Plan

### Phase 0 — HTTP MCP spike + source matrix

This phase gates the whole project.

- Build a minimal desktop-owned HTTP MCP server exposing only
  `computer_status`.
- Inject it through `NewSessionRequest.mcp_servers` as `McpServer::Http`.
- Verify against the latest official `codex-acp` and `claude-agent-acp` that:
  - the model sees the tool
  - the model can call the tool
  - auth headers are forwarded
  - the tool disappears when `computerUse` is off
- Record each agent's advertised MCP capabilities from `initialize`.
- Confirm the practical contract:
  - tool naming
  - headers support
  - localhost URL acceptance
  - any adapter-specific limits
- Explicitly mark Pi out of MVP scope.

Exit criteria:

- at least one real ACP agent can discover and call `computer_status`
  through Sessio's own session loop
- the chosen MVP transport is confirmed as HTTP
- the per-agent support matrix is written down

### Phase 1 — Shared desktop-control permission layer

- finalize the shared `DesktopControlPermissionStatus`
- keep Appshot working unchanged as a consumer
- make `computer use` consume all three tiers

### Phase 2 — Host layer + embedded HTTP MCP server

- add `computer_use` module
- implement lease/snapshot orchestration
- implement the desktop-owned HTTP MCP server
- implement loopback binding, token auth, and session routing

### Phase 3 — Net-new privileged capabilities

- input injection
- AX tree inspection
- app-state snapshot assembly

This remains the highest-risk phase.

### Phase 4 — Runtime injection plumbing + session semantics

- add `computerUse` option
- start/stop the HTTP MCP server around session creation
- append `McpServer::Http` to `session/new`
- gate the toggle on `mcp_injection.http` + Sessio product eligibility
- finalize recreate-on-toggle semantics

### Phase 5 — Approvals + foreground UX

- session-level approvals
- app-level approvals
- cancel/abort path
- settings visibility

### Phase 6 — Broaden support

- more ACP agents
- Windows
- Linux observation-only if feasible
- Pi-specific extension path, if still justified

## Test Plan

### Runtime

- `computerUse` adds an HTTP MCP server only for eligible ACP agents
- ineligible agents never receive the toggle
- session recreation changes server URL/token cleanly
- session teardown invalidates the token and closes the port

### Security

- non-loopback connections are rejected
- missing/invalid token is rejected
- stale session token is rejected
- cross-session token reuse is rejected

### Rust host

- lease lifecycle
- snapshot staleness
- permission-tier derivation
- approval rules

### Frontend

- toggle gating by eligibility
- permission status rendering
- takeover warning/abort path
- Appshot regression check

### Manual

- start a new Codex chat with `computer use` enabled; confirm tool discovery
- start the same chat with it disabled; confirm no tool discovery
- revoke permissions; confirm actionable UI
- trigger a foreground-required action; confirm takeover UI
- close/recreate the session; confirm the old MCP endpoint no longer works

## Open Questions

- Which local HTTP stack is the best fit inside Tauri/Rust for an embedded MCP
  server with low friction and good shutdown semantics?
- Should `computerUse` persist across restored sessions, or be strictly transient?
- Do we want one MCP server instance per session, or one shared process-wide
  server with per-session routing and tokens?
- Is there any adapter-specific header restriction that would force the token into
  URL/query metadata instead of headers?

## Appendix A — Why Pi Is Not In v3 MVP

Pi is not blocked forever; it is just a different problem.

Reasons to exclude it from the MVP:

- it is not ACP, so it does not help answer the core MCP transport decision
- its current Sessio path is plain Pi RPC, not `session/new.mcp_servers`
  ([src-tauri/src/agents/runtime/pi_rpc_transport.rs:295](../src-tauri/src/agents/runtime/pi_rpc_transport.rs:295))
- supporting it well likely means a Pi-native extension path, not reusing the ACP
  injection work directly
- folding that into the same milestone would mix two independent transport
  problems and slow down the first ship

The right sequence is:

1. land ACP HTTP MCP on macOS
2. stabilize the host/provider model
3. revisit Pi as a follow-on integration
