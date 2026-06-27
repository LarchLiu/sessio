# Computer Use Implementation Plan

> A code-verified plan for adding `computer use` to Sessio. Every claim about the
> current codebase is tagged with the file/line it was checked against.

## Summary

Add a session-scoped `computer use` capability to Sessio that injects a
`computer` tool into compatible agents at session start, while keeping all
privileged OS operations (screenshot, accessibility inspection, input injection)
inside the Sessio desktop process.

The shape of the implementation hinges on one existence-level question — how each
agent accepts an injected tool server — which is resolved by a spike before any
refactor is committed. The first target is desktop chat on macOS; IM bridge,
scheduled tasks, and thread orchestration are out of scope for the first
release.

## How tool injection works here

ACP carries tool injection natively. The repo uses
`agent-client-protocol = "1.0.0"`
([src-tauri/Cargo.toml:28](../src-tauri/Cargo.toml)), resolving
`agent-client-protocol-schema 1.1.0`. Its `NewSessionRequest` includes an
`mcp_servers` field, and the SDK exposes a `SessionBuilder` for attaching
Sessio-handled MCP servers. The intended flow is: the client hands the agent a
list of MCP servers at `session/new`, and the agent exposes those tools to the
model.

`McpServer` and its variants live under `schema::v1`
(`McpServer::{Http, Sse, Acp, Stdio}`, with `McpServerHttp` / `McpServerSse` /
`McpServerAcp`; see `agent-client-protocol-schema-1.1.0/src/v1/mcp.rs`). The
builder path is `connection.build_session(cwd).with_mcp_server(...)?
.start_session()` (`agent-client-protocol-1.0.0/src/session.rs:40/153/410`); it
collects the resulting `DynamicHandlerRegistration`s and ties them to the
returned `ActiveSession` for the session's lifetime.

Sessio's own `new_session_request()`
([src-tauri/src/agents/runtime/acp_transport.rs:828](../src-tauri/src/agents/runtime/acp_transport.rs))
already builds this request, but currently only fills `meta.claudeCode.options`
(model / permissionMode / effort); it never populates `mcp_servers`.

So the design choice is **in-process MCP server vs standalone MCP bin**, not "ACP
vs external server."

## Two viable shapes

A Phase 0 spike picks one. They differ sharply in cost.

### Shape A — In-process MCP server (evaluate first)

Attach a Sessio-handled MCP server through the SDK session builder —
`connection.build_session(cwd).with_mcp_server(...)?.start_session()`
(`session.rs:153`) — so the resulting `DynamicHandlerRegistration` is owned by
the `ActiveSession` for the whole session. Tool calls dispatch back **inside the
Sessio desktop process**.

This is a real change to how sessions are created today: the current path builds
a `NewSessionRequest` by hand and calls `connection.send_request(request)`
([acp_transport.rs:464](../src-tauri/src/agents/runtime/acp_transport.rs)),
bypassing the builder. The dynamic handler's lifetime matters — the SDK notes
that once the registration is dropped, the MCP server's messages are no longer
received. So Shape A must either adopt the builder (`with_mcp_server` +
`start_session`, which retains the registration), or, if it keeps hand-building
the request, store each registration until the session ends (or call
`run_indefinitely`, `session.rs:326`). Either way the integration must preserve
the existing notification, permission-request, and prompt-loop behavior of
`run_session`.

- No standalone helper binary.
- No cross-process IPC surface — so no broker auth problem and no forgeable
  session-id surface.
- No sidecar packaging/discovery.
- "Privileged OS control stays in the desktop process" is structural and free.
- Constraint: only works for agents reachable through ACP's `mcp_server`
  machinery; per-agent support must be confirmed in Phase 0.

### Shape B — Standalone MCP bin

A separate `sessio-computer-use` executable advertised to each agent as an MCP
server, talking back to a desktop broker over local IPC.

- Needed only for an agent that cannot accept the in-process form and must launch
  or connect to a process/URL itself.
- Requires, **for those agents only**: per-adapter command shaping, an
  authenticated broker channel with per-session capability tokens, and sidecar
  packaging/discovery.

### Shape C — Agent-native extension (Pi)

Pi does not use ACP, so Shapes A and B do not apply to it. Pi has its own
extension system that is a better fit than injecting an MCP server: a TypeScript
module placed in `~/.pi/agent/extensions/` (or pointed to via Pi
`settings.json` `extensions: [...]`) that calls `pi.registerTool()` to register
the `computer_*` tools directly into Pi, in-process.

Why this is Pi's best path (verified against the Pi extension docs):

- **Runtime, session-scoped registration.** `pi.registerTool()` works during load
  *and* after startup, and new tools are callable without `/reload`;
  `pi.setActiveTools()` enables/disables them at runtime. This is what makes
  per-session opt-in possible — the file-based `pi-mcp-adapter` config is
  global/project-scoped and cannot express "this session only."
- **RPC-mode compatible.** Sessio launches Pi as `pi --mode rpc`
  ([pi_rpc_transport.rs:118](../src-tauri/src/agents/runtime/pi_rpc_transport.rs)).
  In RPC mode extensions still run, `ctx.mode === "rpc"`, and `ctx.ui.confirm` /
  `ctx.ui.notify` work over the JSON protocol — but `ctx.ui.custom()` returns
  `undefined`, so the foreground-takeover overlay must be rendered by Sessio's
  frontend, not Pi's TUI (which matches the plan's UI design already).
- **Privileged ops stay in Sessio.** The extension's `execute()` runs inside the
  Pi process, but it does not perform privileged OS actions itself — it calls
  back into the Sessio desktop broker (local socket / HTTP from the Node runtime;
  `pi.exec` for one-shot needs). So the same broker that serves Shape B serves
  Pi. The broker auth requirement (per-session capability token) therefore
  **does apply to Pi**, since the extension is a separate process.
- **Lifecycle hooks.** `session_start` / `session_shutdown` bracket the broker
  connection; the docs warn against starting background resources in the factory,
  so connection setup is deferred to `session_start`.

What this still needs (Phase 0):

- How the extension reaches the broker (one-shot `pi.exec` vs a persistent local
  socket for high-frequency, binary-heavy screenshot/AX payloads).
- Whether to always-register `computer_*` and gate with `setActiveTools`, or
  register conditionally per session.
- How the extension learns "this session has computer use enabled" + its
  capability token (env at launch vs broker handshake keyed by Pi's session id).
- Deployment: write the extension into `~/.pi/agent/extensions/` vs ship it
  inside the Sessio bundle and point Pi at it via `settings.json` (preferred —
  does not pollute the user's global Pi config).

The full extension implementation thinking is in
[Appendix A](#appendix-a--pi-extension-implementation-shape-c). It is built in
Phase 4 (it is a thin broker client and cannot precede its dependencies), with a
throwaway one-tool probe in Phase 0 to de-risk the path.

**Decision rule:** ACP agents default to Shape A, falling back to Shape B only
where the spike proves Shape A impossible, isolating Shape B's broker/auth/
packaging cost to those agents. Pi uses Shape C. All three share one desktop
broker; only Shape A avoids the cross-process broker entirely.

## Architecture

```text
Chat UI
  ↓ session-scoped runtime option (computerUse)
RuntimeManager
  ├─ ACP agents: inject computer-use MCP server when requested + eligible
  ├─ Pi: ensure the sessio-computer-use extension is present + enabled
  └─ keeps runtime/session id mapping
      ↓
   ┌──────────────────────────────┬───────────────────────────────┐
ACP agent runtime                 Pi runtime (pi --mode rpc)
  ├─ session/new with mcp_servers    └─ sessio-computer-use extension
  └─ tool calls → injected server         ├─ pi.registerTool("computer_*")
      ↓                                    └─ execute() → broker (socket)
Computer-use MCP server                        ↓
  (Shape A: in-process; Shape B: bin)   ┌───────────────┘
  ├─ computer tool schema               │  (Shape B & C share the broker;
  └─ lease/snapshot/action state        │   Shape A dispatches in-process)
      ↓                                 ↓
Sessio desktop host (computer_use module)
  ├─ desktop-control permission checks
  ├─ screenshot / snapshot capture
  ├─ accessibility / element-tree inspection
  ├─ input injection
  ├─ per-session capability-token auth (Shape B & C)
  └─ session + app approvals, foreground takeover UI
```

### New host-owned module

```text
src-tauri/src/computer_use/
  mod.rs
  host.rs        # provider selection, orchestration
  lease.rs       # lease lifecycle, snapshot staleness
  settings.rs
  permissions.rs # consumes the shared desktop-control permission layer
  approvals.rs   # session + app approvals
  provider.rs    # capture / inspect / control behind one interface
```

(Shape B additionally adds `src-tauri/src/bin/sessio-computer-use.rs` and a
broker surface; Shape A does not.)

## Tool model

Stateful and conservative. Public surface (final names confirmed against each
agent in Phase 0 — flat vs dotted):

- `computer_status`
- `computer_list_apps`
- `computer_start`        (open a lease on a chosen app/window)
- `computer_get_app_state` → screenshot + display metadata + elements + snapshot id + allowed actions
- `computer_click_element`
- `computer_type_text`
- `computer_press_key`
- `computer_scroll`
- `computer_stop`

Flow: the agent opens a lease, requests app state, then acts against the latest
snapshot id. No raw pixel-level drag/click-point primitives until host policy is
ready for them.

## Permission model

Generalize the existing Appshot permission layer into a desktop-control layer.
This is a **semantic redesign**, not a rename.

What exists today and is preserved underneath:

- Real macOS checks: `appshot_screenshots_permission_granted()` →
  `ScreenCaptureAccess.preflight()`
  ([src-tauri/src/lib.rs:4621](../src-tauri/src/lib.rs)) and
  `appshot_accessibility_permission_granted()` → `AXIsProcessTrustedWithOptions`
  ([src-tauri/src/lib.rs:4632](../src-tauri/src/lib.rs)).
- A native onboarding panel (`appshot_permission_panel`, `lib.rs:4680`).

What must change: the current API ties `can_capture` directly to screenshot
permission (`appshot_permission_status()`, `lib.rs:4571`) and the frontend
renders accessibility as optional decoration —
`appshotPermissionPresentation.ts:46` maps it to
`"settings.appshot_accessibility_optional"`
([src/appshotPermissionPresentation.ts:21](../src/appshotPermissionPresentation.ts)).
For computer use, accessibility is a first-class hard dependency (element tree /
`click_element`), not optional.

New shared status with distinct capability tiers:

```ts
interface DesktopControlPermissionStatus {
  platform: "macos" | "windows" | "linux" | "other" | string;
  requiresPermission: boolean;
  screenshots: { granted: boolean; supported: boolean };
  accessibility: { granted: boolean; supported: boolean };
  canObserve: boolean;   // capture screenshots / visual state
  canInspect: boolean;   // inspect accessibility / UI hierarchy
  canControl: boolean;   // inject input under current platform/provider policy
}
```

Appshot consumes `screenshots` + `canObserve` and keeps its current copy so its
UX does not regress. Computer use consumes all tiers. OS permission state is kept
separate from product policy (session approval, app approval, provider support,
foreground takeover).

## Net-new privileged capabilities

These do not exist today and are the hardest part of the project:

- **Input injection (`canControl`).** No `enigo` / `rdev` / `autopilot` in
  `Cargo.toml`; only `core-graphics` / `core-foundation` / `objc2-*`
  ([src-tauri/Cargo.toml](../src-tauri/Cargo.toml)). macOS path is
  `CGEvent`-based and must account for accessibility trust + Secure Input.
- **AX element-tree inspection (`get_app_state.elements`).** Existing
  accessibility code only answers "is the process trusted?"; there is no element
  enumeration anywhere. `click_element` depends on this.
- **Snapshot capture.** macOS capture currently lives at `lib.rs:3183` (the
  `src-tauri/src/screenshot/` module has only `linux.rs` / `windows.rs` and a
  near-empty `mod.rs`); the exact reusable path for snapshot generation must be
  identified before relying on it.

## Runtime integration

### Agent eligibility

There is no eligibility concept today. `RuntimeAgentMetadata`
([src-tauri/src/models.rs:1591](../src-tauri/src/models.rs)) carries only
`capabilities: Option<RuntimeCapabilitySet>`, and `RuntimeCapabilitySet`
([src-tauri/src/agents/runtime/types.rs:33](../src-tauri/src/agents/runtime/types.rs))
is transport-generic (`supports_cancel`, `supports_permissions`,
`supports_tool_deltas`, …) — nothing describes "supports tool injection." The
capability probe
([src-tauri/src/agents/runtime/metadata.rs:76](../src-tauri/src/agents/runtime/metadata.rs))
only captures transport caps.

Eligibility is two layers, and the native one already exists in the protocol.
ACP exposes `AgentCapabilities.mcp_capabilities: McpCapabilities` with
`http` / `sse` / `acp` sub-capabilities
(`agent-client-protocol-schema-1.1.0/src/v1/agent.rs:3702/4325`), and each
`McpServer::{Http, Sse, Acp}` variant requires the matching capability. Sessio's
`runtime_capabilities_from_acp()`
([acp_transport.rs:204](../src-tauri/src/agents/runtime/acp_transport.rs))
currently maps prompt/session caps but **not** `mcp_capabilities`. So the work
is: first surface `mcp_capabilities.{http,sse,acp}` through the probe + metadata,
then layer Sessio's own product eligibility (does this agent/version support the
`computer use` contract we chose in Phase 0) on top of it. The toggle gates on
the combined result.

### Launch-time injection layer

There is no abstraction for shaping per-agent launch config today.
`command_from_options()`
([src-tauri/src/agents/runtime/acp_transport.rs:94](../src-tauri/src/agents/runtime/acp_transport.rs))
resolves a single command string and `spawn_acp_transport()` (same file, ~:736)
splits + spawns it. Session startup (`RuntimeManager::start_session` /
`ensure_session`,
[src-tauri/src/agents/runtime/manager.rs:395](../src-tauri/src/agents/runtime/manager.rs)
and :534) spawns the child at creation and has no facility to alter a running
session's tool set.

So injection requires a new per-agent launch-configuration step that populates
`mcp_servers` (Shape A) or registers/launches the bundled server (Shape B).

### Session option semantics

There are two distinct paths, and `computerUse` must explicitly choose:

- **Transient launch options** — opaque `RuntimeMetadata` passed at spawn.
- **Interpreted/persisted config** — `session_config_from_options()`
  ([src-tauri/src/agents/runtime/manager.rs:1616](../src-tauri/src/agents/runtime/manager.rs))
  promotes only `model` / `effort` / `permissionMode` into
  `AgentRuntimeSessionConfig`, and `RuntimeAgentSelection`
  ([src-tauri/src/store/mod.rs:204](../src-tauri/src/store/mod.rs)) persists only
  those three.

If recreate-on-toggle must survive session restore, `computerUse` needs a
persisted home alongside those three fields, not just a transient option.

### No hot-plug

Transport and child process are fixed at spawn, so a running session cannot
discover a newly added tool. Toggling `computer use` on an existing session is a
**session-recreation** boundary (recreate for the next turn, or state that it
applies next session) — never a live hot-plug.

## UI changes

- **Chat composer:** a session-scoped `computer use` toggle, visible only for
  agents flagged eligible (per the new metadata field), modifying the next live
  session's options.
- **Settings:** a `Desktop Control` section showing provider availability, the
  shared desktop-control permission status, app approvals, active lease status,
  and platform-specific guidance when unavailable.
- **Foreground takeover:** a visible warning with a reliable cancel path, scoped
  to the current session and in-flight action.

## Platform strategy

- **macOS — primary target.** Screen-capture and accessibility trust checks stay
  in the desktop host; the privileged entry points are owned by Sessio.
- **Windows — host-broker analog.** Same session-level policy and privileged
  execution boundary; provider may be helper-backed where useful.
- **Linux — no parity promise.** Disable `computer use`, or support
  observation-only if the existing screenshot stack suffices. Avoid overpromising
  Wayland/X11 control semantics.

## Phase plan

### Phase 0 — Injection spike + eligibility design (gates everything)

- Build the minimal MCP server exposing one tool (`computer_status`) and inject
  it via the SDK session builder (`with_mcp_server` + `start_session`) for each
  target agent; confirm the model sees and can call it.
- **In-process lifecycle spike:** wire that minimal handler into Sessio's actual
  ACP loop and confirm the existing session behavior survives — notifications,
  permission requests, and the prompt loop in `run_session` keep working, the
  `DynamicHandlerRegistration` stays alive for the session, and teardown is
  clean. This is Shape A's real risk, not whether `mcp_servers` can be populated.
- Produce a per-agent support matrix — for ACP agents (Claude, Codex, OpenCode):
  in-process supported? required config/CLI shape? `mcp_capabilities` advertised
  (`http`/`sse`/`acp`)? limitations? — with a cited source for each.
- **Pi (Shape C) spike:** Pi uses its own extension system, not ACP. Stand up a
  minimal `sessio-computer-use` Pi extension that registers one tool
  (`computer_status`) via `pi.registerTool()` under `pi --mode rpc`, confirm the
  model can call it, and confirm the broker callback path works from the
  extension's `execute()`. Resolve the four Shape C unknowns (broker transport,
  conditional vs always-register + `setActiveTools`, per-session token delivery,
  deployment via bundled `settings.json extensions:[...]`).
- Decide the shape per agent (ACP → A or B; Pi → C).
- Draft the eligibility data: surfacing `mcp_capabilities` through the probe,
  the new field(s) on `RuntimeAgentMetadata` / `RuntimeCapabilitySet`, and the
  product-eligibility layer on top. For Pi, eligibility is "extension present +
  enabled," not `mcp_capabilities`.
- Confirm tool naming each agent accepts (flat vs dotted).

**Exit criteria:** a working injected tool on ≥1 real agent **through Sessio's
own session loop with the handler lifetime managed**, a support matrix, and a
chosen shape. No later phase starts until this passes.

### Phase 1 — Desktop-control permission model

Semantic redesign of the permission layer (see [Permission model](#permission-model)):
define `DesktopControlPermissionStatus` with `canObserve` / `canInspect` /
`canControl`; keep the real macOS checks and onboarding panel underneath; make
Appshot a consumer that preserves its current copy; rewrite the presentation
layer so two consumers render different semantics from one source of truth.

### Phase 2 — Rust computer-use host layer

Add the `computer_use` module (host, lease, settings, approvals, provider),
independent of the injection path so it is testable early. Model lease lifecycle
and snapshot staleness; identify the reusable macOS capture path
(`lib.rs:3183`).

### Phase 3 — Net-new privileged capabilities (risk-flagged)

Implement input injection (`CGEvent` on macOS; account for accessibility trust +
Secure Input) and AX element-tree inspection (`get_app_state.elements`,
`click_element`). This is the highest-risk phase.

### Phase 4 — Injection plumbing + session option semantics

Add the per-agent launch-configuration / injection layer; add `computerUse` and
specify its path (transient vs persisted — persisted if recreate-on-toggle must
survive restore); gate the toggle on the Phase 0 eligibility field; treat toggle
changes as session recreation. **Shape B only:** authenticated broker channel
with per-session capability tokens, plus sidecar packaging via `externalBin` in
`tauri.conf.json` (no such channel exists today —
[src-tauri/tauri.conf.json:31](../src-tauri/tauri.conf.json)) and a dev/prod
discovery path. **Shape C (Pi):** build the `sessio-computer-use` Pi extension and
its deployment per
[Appendix A](#appendix-a--pi-extension-implementation-shape-c). The extension is
the *last* thing built in this phase — it is a thin client of the broker (Phase
4) which itself depends on the host (Phase 2) and privileged capabilities
(Phase 3), so it cannot be implemented earlier than its dependencies.

### Phase 5 — Approvals + foreground UX

Session-level and app-level approval records; chat approval prompts; foreground
takeover overlay with abort; settings visibility for permissions, approvals, and
active lease.

### Phase 6 — Expand agent + surface coverage

Broaden across agents Phase 0 marked supported; decide whether automation / IM
bridge / scheduled tasks may opt in; tighten unattended/background policy.

## Test plan

### Rust

- Desktop-control permission state derivation across the three tiers.
- Lease lifecycle + snapshot staleness checks.
- App approval / session eligibility rules.
- (Shape B) broker request validation rejects sessions without `computer use`
  enabled and without a valid capability token; IPC request/response encoding.

### Runtime

- Injection happens only when `computerUse` is enabled **and** the agent is
  eligible.
- Ineligible agents do not surface the toggle and fail with actionable errors if
  forced.
- Toggling after startup triggers session-recreation semantics, not hot-plug.

### Frontend

- Toggle visible only for eligible agents.
- Settings render the shared desktop-control status (three tiers) correctly.
- Takeover overlay appears only for foreground-required actions.
- App and session approvals render clearly.

### Manual

- Enable `computer use` before a new chat on an agent Phase 0 marked supported;
  confirm the model discovers the `computer` tool.
- Start the same chat without it; confirm no tool is injected.
- Revoke desktop permissions; confirm actionable UI guidance.
- Trigger a foreground-required action; confirm takeover UI appears and aborts.
- Confirm Appshot still works through the shared permission layer.

## Assumptions and limits

- Desktop-chat-first; opt-in per session; no hot-plug.
- Privileged desktop control stays in the desktop host.
- Linux may be unavailable or observation-only at first.
- The injection mechanism may vary per agent, but the host-side `computer use`
  model stays agent-agnostic.
- Input injection and AX element inspection are net-new and carry the most risk.

---

## Appendix A — Pi extension implementation (Shape C)

How Sessio gives the Pi agent the `computer` tool. This is design intent, not
code — it is implemented in **Phase 4**, after the broker (Phase 4), host (Phase
2), and privileged capabilities (Phase 3) exist. The extension is a *thin client*
of the broker; it owns no privileged operation itself.

Verified against the Pi extension docs (`./extensions.md`).

### A.1 What the extension is

A TypeScript module exporting `default function (pi: ExtensionAPI)`. It:

1. registers the `computer_*` tools with `pi.registerTool()`,
2. forwards each tool call to the Sessio desktop broker,
3. uses Pi's lifecycle hooks and UI for connection management and approvals.

It runs **in the Pi process**. The only privileged work happens back in Sessio,
reached over a local channel.

### A.2 Tool surface

One `pi.registerTool({...})` per tool in the [Tool model](#tool-model):
`computer_status`, `computer_list_apps`, `computer_start`, `computer_get_app_state`,
`computer_click_element`, `computer_type_text`, `computer_press_key`,
`computer_scroll`, `computer_stop`.

- `parameters` use `typebox` `Type.Object(...)`; string enums use `StringEnum`
  from `@earendil-works/pi-ai` (the docs note `Type.Union`/`Type.Literal` break
  Google's API).
- `promptSnippet` + `promptGuidelines` opt each tool into the system prompt only
  while active; guidelines must name the tool explicitly (no "this tool").
- Large results (screenshots, AX trees) must be truncated with `truncateHead` /
  the 50KB / 2000-line defaults, and large binaries should be referenced by
  handle/temp path rather than inlined into tool content.

### A.3 Session-scoped enablement

Pi's `pi.registerTool()` works after startup and `pi.setActiveTools()` toggles
tools at runtime — this is what makes per-session opt-in possible (the file-based
`pi-mcp-adapter` cannot). Two candidate strategies, decided in Phase 0:

- **Always register, gate with `setActiveTools`:** register `computer_*` once;
  enable them only for sessions whose broker handshake confirms computer use is
  on. Simpler registration, relies on active-tool gating.
- **Conditional registration per session:** register inside `session_start` only
  when enabled. Cleaner tool list, but must handle RPC session switches.

Either way the extension must learn, per session, (a) whether computer use is on
and (b) its capability token — see A.5.

### A.4 Broker transport (extension ↔ Sessio)

The extension calls back into the Sessio broker; it never touches the OS. Options
to settle in Phase 0:

- **Persistent local socket / HTTP** (Node `node:net` / `fetch`, available in
  extensions): preferred for high-frequency, binary-heavy `get_app_state`
  (screenshot + AX tree). Opened in `session_start`, closed in `session_shutdown`
  (the docs forbid starting background resources in the factory).
- **`pi.exec` one-shot** for low-frequency calls (`status`, `stop`) if a
  persistent channel is overkill.

Each request carries: the Pi session id, the capability token, the tool name,
and the latest `snapshot id` for action calls. The broker validates all of these
before doing privileged work (this is finding D's auth requirement — it applies
to Pi because the extension is a separate process).

### A.5 Token + enablement delivery

How the extension obtains its per-session capability token and the "enabled"
flag, decided in Phase 0:

- **Env at launch:** Sessio injects a token into the `pi --mode rpc` process
  environment when it starts a computer-use session. Simple, but token lifetime
  is the process, not the session — weaker for session recreation.
- **Broker handshake keyed by Pi session id:** the extension connects and asks
  the broker "is computer use on for my session, and what's my token?"; the
  broker answers from the runtime's session state. Better fit for the
  recreate-on-toggle model.

### A.6 Approvals and takeover UX

- Session/app approval prompts can use `ctx.ui.confirm()` — works in RPC mode.
- `ctx.ui.custom()` returns `undefined` in RPC mode, so the **foreground
  takeover overlay is rendered by Sessio's frontend**, not Pi's TUI. The
  extension only signals takeover state to the broker; Sessio draws and owns the
  abort path. This matches [UI changes](#ui-changes).

### A.7 Lifecycle and cleanup

- `session_start`: open the broker channel, do the enablement/token handshake,
  register or activate tools.
- `session_shutdown`: close the channel, release any lease, deactivate tools.
- Reuse Pi's `ctx.signal` inside `execute()` so Esc / turn-abort cancels in-flight
  broker calls.

### A.8 Deployment

Prefer **shipping the extension inside the Sessio bundle** and pointing Pi at it
via Pi `settings.json` `extensions: ["<bundled path>"]`, rather than writing into
`~/.pi/agent/extensions/` (which pollutes the user's global Pi config and risks
clobbering user files). Sessio already knows Pi's directory layout
(`app_paths::pi_agent_sessions_dir()`,
[pi_rpc_transport.rs:1442](../src-tauri/src/agents/runtime/pi_rpc_transport.rs))
but does not write any Pi config today — managing `settings.json` is net-new and
must be idempotent and reversible (clean removal when the feature is off).

### A.9 Phase 0 probe vs Phase 4 build

- **Phase 0 (throwaway):** a minimal extension with only `computer_status`,
  registered under `pi --mode rpc`, doing one broker round-trip (against a stub
  broker returning fixed JSON is enough). Goal: prove the model can call an
  extension tool and the callback path works. Resolves A.3–A.5 unknowns.
- **Phase 4 (real):** the full tool surface, real broker protocol, token auth,
  approvals, lifecycle, and bundled deployment.
