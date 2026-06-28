# Computer Use Background Driving Plan

## Summary

This document describes how Sessio can evolve from:

- foreground-oriented, global input injection

to:

- **background-targeted native-app driving on macOS**
- minimized focus stealing
- explicit fallback to foreground takeover only when necessary

It is a companion to
[computer-use-skill-gap-plan.md](./computer-use-skill-gap-plan.md).

Use that document for the overall parity roadmap. Use this document for the
specific technical design needed to support the `SKILL.md` expectation that
computer use can often drive a native Mac app without stealing focus from the
user's current app.

Current architecture decision:

- this phase is an **in-process implementation**
- computer-use orchestration stays inside the main Sessio desktop/native host
- a helper/sidecar is a future extraction option, not part of the current
  implementation baseline

## Current Verified State

Sessio does **not** currently support reliable background driving.

Verified constraints in the current code:

- the macOS provider posts synthetic mouse/keyboard/scroll events to the global
  HID tap, not to a target pid:
  - [src-tauri/src/computer_use/platform/macos.rs](../src-tauri/src/computer_use/platform/macos.rs)
- the host still treats control as foreground takeover control:
  - [src-tauri/src/computer_use/host.rs](../src-tauri/src/computer_use/host.rs)
- the current action contract is snapshot-validated but not post-action
  stateful:
  - [src-tauri/src/computer_use/lease.rs](../src-tauri/src/computer_use/lease.rs)
  - [src-tauri/src/computer_use/mcp_http/dispatch.rs](../src-tauri/src/computer_use/mcp_http/dispatch.rs)

In practice, this means the current system is much closer to:

- "desktop automation with foreground semantics"

than to:

- "background-targeted native-app control"

## Desired Outcome

For supported macOS apps, Sessio should be able to:

- inspect an app without activating it
- launch an app without stealing focus
- deliver some control actions directly to the target app
- keep the user's current frontmost app undisturbed when the target app can
  accept background-directed actions
- fall back to foreground takeover only when an action or target app requires it

The important design point is that this is **capability-graded**, not absolute.

The target is not "all apps, all actions, never steal focus." The target is:

- use the strongest background-safe strategy first
- know when that strategy is unsupported or unreliable
- degrade explicitly to takeover semantics

## Principles

### 1. Background driving is a host/provider capability, not a prompt trick

The model cannot infer whether an action will steal focus. The runtime must know
this and expose it explicitly.

### 2. Action routing must be strategy-aware

Different actions need different delivery paths:

- AX action
- pid-targeted physical event
- global HID event
- explicit foreground takeover

These are not interchangeable.

### 3. Focus preservation is best-effort and measurable

Some apps and system surfaces will ignore or reject synthetic events. The
runtime should detect and surface that instead of pretending background driving
is universal.

### 4. State must advance after every mutation

Background driving is only practical if post-action state becomes authoritative
without requiring the agent to guess whether the app accepted the action.

## Architecture Direction

Following the native-shell principle from
[/Users/alex/.agents/skills/native-feel-cross-platform-desktop/references/02-architecture.md](/Users/alex/.agents/skills/native-feel-cross-platform-desktop/references/02-architecture.md:49),
the native shell must own:

- app launch semantics
- event routing strategy
- focus preservation / restoration checks
- per-action capability decisions

The WebView/UI should only render:

- whether background driving is available
- whether an action used background or takeover mode
- why takeover was required

## Proposed Capability Model

Add an explicit action-delivery model.

### Delivery strategies

- `ax`
  - structured Accessibility action against an element
  - preferred when available
  - usually safest for background operation
- `pid_event`
  - synthetic keyboard/mouse event directed to a target pid
  - preferred physical fallback for apps that can accept it
- `global_event`
  - synthetic event sent to the global HID path
  - legacy fallback; likely to disturb focus
- `foreground_takeover`
  - bring the action into the explicit takeover flow
  - last resort, not default

### Action classes

At minimum, classify actions into:

- `background_preferred`
  - examples: AX press, AX set value, pid-scoped key press, pid-scoped scroll
- `background_possible`
  - examples: pixel click, secondary click, some drag flows
- `takeover_likely`
  - examples: drag across custom surfaces, menu navigation, apps known to reject
    background events
- `takeover_required`
  - examples: privileged system UI, targets verified to require focus

### Target capability facts

Maintain per-target runtime facts:

- `supports_ax_actions`
- `supports_pid_events`
- `supports_background_launch`
- `requires_foreground_for_physical_events`
- `last_background_delivery_succeeded`
- `last_background_delivery_failed`

These do not need to be persisted initially. Session-local state is enough for
the first implementation.

## Required API and Model Changes

### Provider trait

The current `ComputerUseProvider` trait is too narrow for background driving.

In
[src-tauri/src/computer_use/provider.rs](../src-tauri/src/computer_use/provider.rs),
expand the trait with concepts like:

- target resolution that returns pid and window identity
- background launch
- pid-targeted event delivery
- direct value setting
- secondary click / double click / drag
- action result metadata

Recommended additions:

- `launch_app_background(app_id) -> ProviderResult<AppTarget>`
- `resolve_target_context(target) -> ProviderResult<TargetContext>`
- `perform_action(action_request) -> ProviderResult<ActionExecution>`

Where:

- `TargetContext` contains bundle id, pid, frontmost window id, window bounds,
  and strategy facts
- `ActionExecution` contains:
  - `delivery_strategy`
  - `focus_changed`
  - `background_succeeded`
  - optional provider notes / failure reason

This is preferable to continuing to add one trait method per verb because the
strategy result becomes part of the contract.

### Host policy

In
[src-tauri/src/computer_use/host.rs](../src-tauri/src/computer_use/host.rs):

- remove the assumption that all control implies foreground takeover
- split:
  - `control permitted`
  - `takeover required for this action`
- let the host decide per action whether:
  - it may run in background
  - it must enter takeover
  - it should fail with an explicit `takeover_required`

Recommended host additions:

- `ActionPolicyDecision`
  - `AllowedBackground`
  - `AllowedTakeover`
  - `DeniedRequiresTakeover`
  - `DeniedUnsupported`
- `ActionExecutionMode`
  - `Background`
  - `ForegroundTakeover`

The overlay should observe `ForegroundTakeover` execution, not generic control.

### MCP protocol

The MCP layer currently returns text-only `ok` for mutations in
[src-tauri/src/computer_use/mcp_http/dispatch.rs](../src-tauri/src/computer_use/mcp_http/dispatch.rs).

That is insufficient for background driving because the model needs to know:

- whether the action ran in background
- whether takeover was used
- whether focus changed despite the request
- what the new authoritative state is

Mutation responses should include:

- `mode`: `background` or `foreground_takeover`
- `deliveryStrategy`: `ax` / `pid_event` / `global_event`
- `focusChanged`: bool
- `targetState`: fresh post-action state
- optional `warning`: e.g. `"background delivery failed; retry with takeover"`

### Settings and UI

The product-level direction should be:

- `computer use enabled` implies control is allowed

Do **not** keep separate product toggles for:

- `allow_input_injection`
- `allow_foreground_takeover`

Instead, reflect runtime facts like:

- `background control available`
- `this action required takeover`
- `takeover currently active`

The UI should explain runtime mode, not expose misleading policy toggles.

## Helper / Sidecar Decision

Short answer:

- a separate helper is **more isolated as a process/package**
- it is **not automatically less coupled as a product subsystem**

For Sessio's current phase, background driving does **not** require a separate
`bin`, sidecar, or helper app.

The feature is inherently coupled to Sessio's native host because the host
still owns:

- session-scoped approvals
- lease lifecycle and snapshot authority
- takeover overlay and abort semantics
- runtime MCP injection
- computer-use status surfaced to the UI

Moving the execution path into a helper would remove some in-process code, but
it would not remove those product couplings. It would instead replace direct
calls with:

- helper lifecycle management
- helper discovery and version matching
- IPC protocol and auth
- crash recovery and restart semantics
- permission/status synchronization

That is usually **better isolation**, but not simpler architecture.

### Recommended direction for Sessio

Near term:

- keep computer-use orchestration in the main desktop/native host as an
  **in-process implementation**
- do not introduce a separate sidecar `bin` just for background driving
- keep the provider boundary clean so extraction remains possible later

This matches the existing desktop-owned MCP direction in
[computer-use-implementation-plan-v3.md](./computer-use-implementation-plan-v3.md),
which already correctly avoids premature sidecar complexity for the MVP.

### When a separate helper becomes justified

A dedicated helper becomes worth it when one or more of these become hard
requirements:

- a separate TCC-visible app identity such as `Computer Use.app`
- a distinct signing/notarization surface for permissions UX
- long-lived automation outside the main Sessio app lifecycle
- crash/process isolation strong enough to justify IPC overhead
- a reusable local automation service shared by multiple entry points
- a CLI/daemon contract that should survive UI restarts

In other words:

- if the goal is just "background-drive apps better", a helper is not required
- if the goal is "ship a separately branded, separately permissioned automation
  agent", a helper probably is required

### Suggested extraction path

If we later decide to split it out, do it in stages:

1. keep policy and UX in Sessio host
2. keep provider contracts strategy-aware and serialization-friendly
3. move only the low-level macOS execution backend behind IPC
4. only after that, decide whether it should remain a plain sidecar `bin` or
   become a separate signed `.app` helper

That sequencing preserves product control while keeping the escape hatch open.

## Proposed Phased Implementation

### Phase A: Normalize host semantics

Goal:

- stop treating all control actions as takeover actions

Tasks:

- remove combined policy gating
- make `foreground_active` mean actual takeover, not any control action
- update settings/config/frontend terminology accordingly

Acceptance:

- a control action can be represented as background-capable without activating
  the takeover overlay

### Phase B: Background launch and target context

Goal:

- obtain enough target metadata to make action-routing decisions

Tasks:

- add background launch via native macOS app launch APIs
- resolve bundle id to pid and active window context
- enrich captured state with pid/window metadata

Acceptance:

- `get_app_state` can launch if needed without focus-stealing launch primitives
- actions can reference a resolved pid-capable target

### Phase C: Introduce strategy-aware execution

Goal:

- route actions via AX or pid-targeted delivery before considering takeover

Tasks:

- add action request/result types at the provider layer
- implement AX-first delivery for element-backed actions
- implement pid-targeted physical delivery path
- retain global HID delivery only as explicit fallback

Acceptance:

- provider reports which strategy was actually used
- host can distinguish background success from foreground fallback

### Phase D: Post-action state contract

Goal:

- make every successful mutation yield authoritative next state

Tasks:

- capture fresh post-action state
- mint fresh snapshot ids after successful mutation
- include action execution metadata in MCP results

Acceptance:

- agents do not need to call `computer_get_app_state` immediately after every
  action just to confirm background success

### Phase E: Takeover fallback

Goal:

- make takeover an explicit, explainable fallback mode

Tasks:

- add host-level `takeover_required` decision path
- update overlay copy to explain why takeover was needed
- preserve abort semantics

Acceptance:

- takeover is no longer the silent default
- when takeover happens, the reason is model-visible and user-visible

### Phase F: App-specific reliability matrix

Goal:

- validate real-world behavior by app family

Test matrix should include:

- AppKit native apps
- Catalyst apps
- Electron apps
- Chromium apps with partial AX
- Qt apps
- custom-drawn / no-AX apps

Per app, validate:

- background launch
- background snapshot
- AX click
- pid-targeted key press
- pid-targeted scroll
- pixel click fallback
- whether focus changed
- whether takeover was required

## macOS-Specific Notes

### AX should remain the preferred path

For structured element actions, AX is the least disruptive route and most
aligned with background operation. Physical event fallback should only be used
when AX cannot express the action.

### Global HID posting is the wrong default

The current `CGEventTapLocation::HID` posting path is useful as a fallback but
should not remain the primary delivery path for background-driving semantics.

### Focus preservation should be observable

Even after pid-targeted delivery is implemented, the runtime should check
whether the frontmost app changed across the action and record that in the
action result.

That allows:

- telemetry
- per-app heuristics
- explicit fallback decisions

## Suggested File Areas

- provider abstraction:
  - [src-tauri/src/computer_use/provider.rs](../src-tauri/src/computer_use/provider.rs)
- macOS routing implementation:
  - [src-tauri/src/computer_use/platform/macos.rs](../src-tauri/src/computer_use/platform/macos.rs)
- host policy and execution mode:
  - [src-tauri/src/computer_use/host.rs](../src-tauri/src/computer_use/host.rs)
  - [src-tauri/src/computer_use/lease.rs](../src-tauri/src/computer_use/lease.rs)
- MCP schema and responses:
  - [src-tauri/src/computer_use/mcp_http/protocol.rs](../src-tauri/src/computer_use/mcp_http/protocol.rs)
  - [src-tauri/src/computer_use/mcp_http/dispatch.rs](../src-tauri/src/computer_use/mcp_http/dispatch.rs)
- runtime injection:
  - [src-tauri/src/agents/runtime/computer_use_runtime.rs](../src-tauri/src/agents/runtime/computer_use_runtime.rs)
  - [src-tauri/src/agents/runtime/manager.rs](../src-tauri/src/agents/runtime/manager.rs)
- settings and frontend:
  - [src-tauri/src/computer_use/settings.rs](../src-tauri/src/computer_use/settings.rs)
  - [src-tauri/src/config.rs](../src-tauri/src/config.rs)
  - [src/pages/SettingsPage.tsx](../src/pages/SettingsPage.tsx)
  - [src/components/ComputerUseTakeoverOverlay.tsx](../src/components/ComputerUseTakeoverOverlay.tsx)

## Relationship To The Main Gap Plan

This document elaborates the design behind these gaps in
[computer-use-skill-gap-plan.md](./computer-use-skill-gap-plan.md):

- `G. No pid-scoped or background-targeted event routing`
- `I. No post-action screenshot return shape`
- `K. Host control policy conflates input control with foreground takeover`

Implementation should keep the two documents aligned:

- update the gap plan when the high-level roadmap changes
- update this document when the routing model, provider contract, or validation
  matrix changes
