# Computer Use Skill Gap Plan

## Summary

This document compares the macOS computer-use capability described in the
versioned in-repo spec
[computer-use-skill.md](./computer-use-skill.md) with Sessio's current
implementation, then turns that comparison into a concrete implementation plan.

For repo planning purposes, [computer-use-skill.md](./computer-use-skill.md) is
the canonical, reviewable truth source. Any external or desktop-local copies
used while drafting this plan are background inputs, not normative references.

This plan is intentionally separate from
[computer-use-prompt-refactor-plan.md](./computer-use-prompt-refactor-plan.md).
That document focuses on prompt engineering, prompt assembly, snapshotting, and
diagnostics. This document focuses on **capability parity**:

- tool surface
- macOS provider behavior
- desktop-control truth source
- runtime / onboarding flow
- CLI / skill integration
- response shape and operator ergonomics

For the detailed background-driving design behind the provider/host/runtime
changes referenced here, see
[computer-use-background-driving-plan.md](./computer-use-background-driving-plan.md).

The main conclusion is:

- Sessio already has a good MVP core: HTTP MCP injection, approvals, lease and
  snapshot discipline, foreground abort, AX tree capture, screenshot capture,
  basic CGEvent control, and a shared desktop-control truth source that now
  reflects provider support correctly.
- The `SKILL.md` implementation is materially broader. It supports a richer
  action model, explicit onboarding tools, app launch behavior, pixel fallback,
  more robust background targeting semantics, a stronger post-action
  state/snapshot contract, and a CLI/skill contract that does not yet exist in
  Sessio.

## Relationship To The Prompt Refactor Plan

Keep the two documents separate.

- Use [computer-use-prompt-refactor-plan.md](./computer-use-prompt-refactor-plan.md)
  for prompt builder, provider patch pipeline, session prompt snapshot, and
  computer-use operating contract injection.
- Use this document for tool/provider/runtime parity against `SKILL.md`.

There is only one meaningful overlap:

- once the capability gaps here are implemented, the prompt plan should expose
  those capabilities cleanly to the model

There is also one process-architecture constraint worth keeping explicit here:

- the current implementation direction is **in-process** inside the existing
  Sessio desktop host
- the current roadmap should **not** introduce a separate computer-use helper
  or sidecar binary as a prerequisite for capability parity
- background driving and richer action semantics should first land inside the
  existing desktop-owned host/provider architecture
- if a separate helper is introduced later, it should be for explicit reasons
  such as TCC identity, daemon lifecycle, or packaging isolation, not as a
  default response to feature growth

## Verified References

### Skill reference

- Canonical source of expected behavior for this plan:
  - [computer-use-skill.md](./computer-use-skill.md)

### Sessio current implementation

- Runtime MCP injection:
  - [src-tauri/src/agents/runtime/computer_use_runtime.rs](../src-tauri/src/agents/runtime/computer_use_runtime.rs)
  - [src-tauri/src/agents/runtime/manager.rs](../src-tauri/src/agents/runtime/manager.rs)
- Host policy and lifecycle:
  - [src-tauri/src/computer_use/host.rs](../src-tauri/src/computer_use/host.rs)
  - [src-tauri/src/computer_use/approvals.rs](../src-tauri/src/computer_use/approvals.rs)
  - [src-tauri/src/computer_use/lease.rs](../src-tauri/src/computer_use/lease.rs)
- MCP tool protocol and dispatch:
  - [src-tauri/src/computer_use/mcp_http/protocol.rs](../src-tauri/src/computer_use/mcp_http/protocol.rs)
  - [src-tauri/src/computer_use/mcp_http/dispatch.rs](../src-tauri/src/computer_use/mcp_http/dispatch.rs)
- Provider contract:
  - [src-tauri/src/computer_use/provider.rs](../src-tauri/src/computer_use/provider.rs)
- macOS provider:
  - [src-tauri/src/computer_use/platform/macos.rs](../src-tauri/src/computer_use/platform/macos.rs)
- Desktop-control truth source:
  - [src-tauri/src/lib.rs](../src-tauri/src/lib.rs)
  - [src-tauri/src/desktop_control/mod.rs](../src-tauri/src/desktop_control/mod.rs)
- Settings and frontend API:
  - [src-tauri/src/computer_use/settings.rs](../src-tauri/src/computer_use/settings.rs)
  - [src-tauri/src/config.rs](../src-tauri/src/config.rs)
  - [src/api.ts](../src/api.ts)
  - [src/pages/SettingsPage.tsx](../src/pages/SettingsPage.tsx)
  - [src/components/ComputerUseTakeoverOverlay.tsx](../src/components/ComputerUseTakeoverOverlay.tsx)

## Current Verified Notes

- 2026-06-28 status update: the in-process Sessio implementation now aligns
  with the core [computer-use-skill.md](./computer-use-skill.md) operating
  contract while preserving Sessio naming (`sessio cu`, not `alma cu`).
  Completed parity includes:
  - AX-first unified click via `computer_click`, with `elementId` / `ref`
    preferred over coordinates
  - ref-targeted secondary actions through `computer_perform_secondary_action`
    / `computer_secondary_click`, using AXShowMenu before CGEvent right-click
  - ref-targeted scroll through AX scroll actions, with wheel fallback
  - post-action app state with inline MCP image content when the screenshot
    handle is readable
  - `bundle`, `ref`, and `coord_space` compatibility aliases alongside
    Sessio's existing `appId`, `elementId`, and `coordSpace`
  - `sessio cu` ref-first command forms plus `--bundle` and `--days`
    compatibility
  - `computer_raise_app` / `sessio cu raise` as the explicit foreground
    recovery path for hidden or minimized apps when `get_app_state` cannot find
    a visible window
  - model-visible guidance for hidden/minimized window recovery: MCP tool
    descriptions, runtime prompt notes, CLI help, and `no_visible_window` errors
    now point agents to `computer_raise_app` / `sessio cu raise` and explicitly
    warn against `open -a` / AppleScript `activate` / Window-menu fallbacks that
    can report success without restoring a Dock-minimized window
  - bundled computer-use skill resources: release builds include
    `computer-use-skill/SKILL.md` plus `playbooks/`, and computer-use turns tell
    agents the resolved local skill path so they can read the full workflow on
    demand instead of relying only on short prompt hints
  - ScreenCaptureKit-first macOS window screenshots, targeting the selected
    desktop-independent window and falling back to `screencapture -l` when SCK
    fails or is unavailable
  - true recent-use ranking for `computer_list_apps` / `sessio cu list-apps
    --days`, using macOS activity metadata when available and falling back
    cleanly when the OS withholds it
  - macOS `CGEventPostToPid` dispatch for physical mouse/keyboard/scroll
    events, key chords, best-effort focus restoration, and Electron AX flags
    (`AXEnhancedUserInterface`, `AXManualAccessibility`)
- Remaining non-parity items are intentionally scoped as future architecture
  work rather than hidden TODOs in this in-process phase:
  - adding helper/daemon-only CLI affordances such as `shot`, `lens`, and
    `shutdown`

- The shared desktop-control truth source no longer hardcodes
  `input_injection_supported: false` on macOS. In
  [src-tauri/src/lib.rs](../src-tauri/src/lib.rs), `desktop_control_inputs()`
  now derives that value from the current provider's `supports_control()`
  result.
- The remaining policy problem is therefore not the desktop-control truth
  source itself. It is the host-side control model in
  [src-tauri/src/computer_use/host.rs](../src-tauri/src/computer_use/host.rs),
  where `control_enabled()` currently requires both
  `allow_input_injection` and `allow_foreground_takeover`, even though enabling
  computer use should already imply that control is allowed.

## What Sessio Already Has

### 1. Session-scoped MCP injection

Sessio already injects a desktop-owned HTTP MCP server into eligible ACP
sessions. This is the correct architectural base for the feature.

### 2. Host-side policy ownership

The host already owns:

- global enable/disable
- session approval
- app approval
- lease ownership
- snapshot freshness
- foreground takeover state
- abort path

This is a strong ownership boundary and should be preserved, even though the
current control/takeover semantics still need to be re-baselined.

### 3. Basic tool catalog

Sessio already exposes:

- `computer_status`
- `computer_list_apps`
- `computer_start`
- `computer_get_app_state`
- `computer_click_element`
- `computer_type_text`
- `computer_press_key`
- `computer_scroll`
- `computer_stop`

### 4. macOS MVP provider

The macOS provider already supports:

- running app enumeration
- frontmost window screenshot capture
- AX tree extraction
- AX element click via bounds center
- basic `CGEvent` typing
- basic `CGEvent` key press
- basic `CGEvent` scroll

### 5. Frontend toggles and takeover overlay

Sessio already has:

- computer-use master enable
- input-control toggle
- foreground-takeover toggle
- takeover warning overlay with abort

These are useful product affordances, even though the current host policy still
does not give `input control` and `foreground takeover` independent runtime
meaning.

## What The In-Repo Skill Spec Adds

The in-repo [computer-use-skill.md](./computer-use-skill.md) is important
because it is the versioned, reviewable capability target for future
implementation and testing work.

It makes several expectations especially explicit:

- the preferred MCP interface should return post-action screenshots inline
- the system should support both AX and pixel-coordinate control paths
- the coordinate space should be defined relative to the latest screenshot
- the product should avoid `open -b` style focus-stealing launches
- onboarding should be exposed as explicit computer-use operations
- app-specific playbooks should exist as first-class skill resources

For Sessio, this means `computer-use-skill.md` should be treated as a local
capability target, not just as copied documentation.

## Completion Audit Against SKILL.md

The original gaps in this plan have now been implemented in the in-process
Sessio desktop host. This section is the current audit record; earlier gap
language should be treated as historical background only.

| Area | Current status | Primary evidence |
| --- | --- | --- |
| Host policy | Complete. `ComputerUseSettings` has a single product switch (`enabled`); control is allowed when enabled, OS control is available, approvals pass, and the provider supports control. Legacy config keys are parsed only for compatibility and are not serialized back. | [settings.rs](../src-tauri/src/computer_use/settings.rs), [host.rs](../src-tauri/src/computer_use/host.rs), [config.rs](../src-tauri/src/config.rs), [SettingsPage.tsx](../src/pages/SettingsPage.tsx) |
| App launch and discovery | Complete. `computer_launch_app` launches without activation, `computer_raise_app` is the explicit foreground recovery path, and `computer_get_app_state` can launch approved stopped targets before snapshotting. `computer_list_apps` returns installed plus running apps with recent-use ranking metadata when macOS exposes it. | [provider.rs](../src-tauri/src/computer_use/provider.rs), [macos.rs](../src-tauri/src/computer_use/platform/macos.rs), [dispatch.rs](../src-tauri/src/computer_use/mcp_http/dispatch.rs) |
| Post-action state | Complete. Successful mutating actions capture a fresh authoritative `AppState`, mint a new snapshot id, and return structured content plus inline screenshot image content when the handle is readable. Stale pre-action snapshot ids are rejected. | [host.rs](../src-tauri/src/computer_use/host.rs), [protocol.rs](../src-tauri/src/computer_use/mcp_http/protocol.rs), [dispatch.rs](../src-tauri/src/computer_use/mcp_http/dispatch.rs), [lease.rs](../src-tauri/src/computer_use/lease.rs) |
| Screenshot coordinate contract | Complete. Coordinate actions default to screenshot-space pixels, map through the stored screenshot bounds into screen-space points, and are tied to the latest snapshot. `coordSpace` / `coord_space` allow explicit screen-space input when needed. | [provider.rs](../src-tauri/src/computer_use/provider.rs), [host.rs](../src-tauri/src/computer_use/host.rs), [dispatch.rs](../src-tauri/src/computer_use/mcp_http/dispatch.rs) |
| Action surface | Complete. The provider, host, MCP protocol, and CLI cover AX click, coordinate click, secondary action, double click, drag, set value, type text, press key, and scroll. AX refs are preferred where available, with screenshot-coordinate fallback. | [provider.rs](../src-tauri/src/computer_use/provider.rs), [host.rs](../src-tauri/src/computer_use/host.rs), [protocol.rs](../src-tauri/src/computer_use/mcp_http/protocol.rs), [cli.rs](../src-tauri/src/cli.rs) |
| macOS implementation | Complete for the current in-process scope. Screenshots prefer ScreenCaptureKit and fall back to `screencapture -l`; physical events use pid-scoped `CGEventPostToPid`; AX actions include `AXPress`, `AXShowMenu`, scroll actions, `AXValue`, minimized-window restoration, and Electron AX flags. | [macos.rs](../src-tauri/src/computer_use/platform/macos.rs), [build.rs](../src-tauri/build.rs), [Cargo.toml](../src-tauri/Cargo.toml) |
| Onboarding tools | Complete. `computer_permissions` and `computer_grant` expose permission status and supported OS settings flows through the computer-use MCP surface and `sessio cu`. | [onboarding.rs](../src-tauri/src/computer_use/onboarding.rs), [dispatch.rs](../src-tauri/src/computer_use/mcp_http/dispatch.rs), [cli.rs](../src-tauri/src/cli.rs) |
| CLI parity | Complete for the in-process architecture. `sessio cu` attaches to an already-running desktop computer-use MCP host, supports `--json`, mirrors the MCP verbs, and fails explicitly instead of starting a separate helper/runtime. | [cli.rs](../src-tauri/src/cli.rs) |
| Skill resources and playbooks | Complete. The canonical skill is bundled as `computer-use-skill/SKILL.md`, app playbooks are bundled next to it, and computer-use turns inject the resolved local skill path instead of expanding the full skill into every prompt. | [computer-use-skill.md](./computer-use-skill.md), [computer-use/playbooks](./computer-use/playbooks), [skill_resource.rs](../src-tauri/src/computer_use/skill_resource.rs), [tauri.conf.json](../src-tauri/tauri.conf.json) |

## Completed Implementation Phases

### Phase 1: Re-baseline host control policy

Status: complete.

Evidence:

- Settings expose one computer-use enable switch instead of independent
  `input control` / `foreground takeover` toggles.
- The host status reports control from settings + OS capability + provider
  support, not from product-policy sub-switches.
- Foreground activity remains a runtime overlay/abort state, not a prerequisite
  for every control action.

### Phase 2: Add app launch and discovery workflow parity

Status: complete.

Evidence:

- `computer_launch_app` performs background launch without activation.
- `computer_raise_app` handles hidden/minimized foreground recovery and is
  surfaced in tool descriptions, runtime prompt hints, CLI help, and
  `no_visible_window` errors.
- `computer_get_app_state` launches stopped targets only after session and app
  approval, then returns whether the state was launched.
- `computer_list_apps` includes installed apps, running apps, `--days`, and
  recent-use metadata from macOS Knowledge/Spotlight when available.

### Phase 3: Add post-action state and snapshot contract

Status: complete.

Evidence:

- Mutating actions return a new `AppState` rather than text-only success.
- The returned state includes a fresh snapshot id and screenshot metadata.
- MCP responses include `structuredContent` and inline screenshot image content
  when the screenshot file can be read.
- The pre-action snapshot becomes stale after a successful mutation.

### Phase 4: Add screenshot coordinate-space semantics

Status: complete.

Evidence:

- `ScreenshotRef` records image pixel dimensions plus the represented screen
  bounds.
- Screenshot-space points map linearly into screen-space points before provider
  dispatch.
- Coordinate actions default to screenshot-space and reject stale snapshots.

### Phase 5: Expand the provider and tool contract

Status: complete.

Evidence:

- The provider contract, host, MCP catalog, dispatch layer, and `sessio cu`
  surface include coordinate click, secondary click, double click, drag, and
  direct AX value setting.
- The macOS provider implements these actions through AX where appropriate and
  pid-scoped CGEvent fallback for physical interactions.

### Phase 6: Expose onboarding tools

Status: complete.

Evidence:

- `computer_permissions` reports screen recording, Accessibility, and control
  readiness with actionable guidance.
- `computer_grant` opens the relevant macOS settings flow when supported.
- Permission-related tool errors point agents back to those onboarding tools.

### Phase 7: Add CLI parity

Status: complete.

Evidence:

- `sessio cu` exposes the core MCP verbs, supports stable `--json`, and accepts
  skill-compatible aliases such as `--bundle`, positional refs, and
  `coord_space`.
- The CLI attaches to a running desktop MCP host through URL/token routing and
  does not create a helper, daemon, or separate automation runtime.

### Phase 8: Add app playbooks

Status: complete.

Evidence:

- App playbooks are versioned under
  [docs/computer-use/playbooks](./computer-use/playbooks).
- [index.json](./computer-use/playbooks/index.json) records the inventory,
  target bundles, primary strategy, fallback strategy, and review status.
- Release builds bundle the skill and playbook directory as app resources.

## Explicit Future Architecture Scope

The following items are intentionally **not** part of the current in-process
parity goal:

- helper/daemon-only commands such as `shot`, `lens`, and `shutdown`
- a separate signed helper app or always-on sidecar daemon
- CLI operation that survives desktop UI shutdowns

Those belong to a later helper/daemon architecture decision if TCC identity,
lifecycle isolation, or external automation requirements justify it.

## Proposed File Areas

Likely implementation areas:

- provider and host:
  - [src-tauri/src/computer_use/provider.rs](../src-tauri/src/computer_use/provider.rs)
  - [src-tauri/src/computer_use/host.rs](../src-tauri/src/computer_use/host.rs)
  - [src-tauri/src/computer_use/platform/macos.rs](../src-tauri/src/computer_use/platform/macos.rs)
- settings, config, and frontend policy:
  - [src-tauri/src/computer_use/settings.rs](../src-tauri/src/computer_use/settings.rs)
  - [src-tauri/src/config.rs](../src-tauri/src/config.rs)
  - [src/pages/SettingsPage.tsx](../src/pages/SettingsPage.tsx)
- MCP protocol and dispatch:
  - [src-tauri/src/computer_use/mcp_http/protocol.rs](../src-tauri/src/computer_use/mcp_http/protocol.rs)
  - [src-tauri/src/computer_use/mcp_http/dispatch.rs](../src-tauri/src/computer_use/mcp_http/dispatch.rs)
- desktop-control truth source:
  - [src-tauri/src/lib.rs](../src-tauri/src/lib.rs)
  - [src-tauri/src/desktop_control/mod.rs](../src-tauri/src/desktop_control/mod.rs)
- runtime and API:
  - [src-tauri/src/agents/runtime/manager.rs](../src-tauri/src/agents/runtime/manager.rs)
  - [src/api.ts](../src/api.ts)
- future CLI work:
  - [src-tauri/src/cli.rs](../src-tauri/src/cli.rs)

## Non-Goals

This document does not try to solve:

- prompt builder and prompt snapshot design
- provider/model prompt patching
- Astra prompt decomposition

Those belong to
[computer-use-prompt-refactor-plan.md](./computer-use-prompt-refactor-plan.md).

## Expected Outcome

After the work in this document, Sessio should evolve from a structured
computer-use MVP into a substantially more complete native-app automation
surface:

- better tool parity
- better macOS action coverage
- fewer UI/status contradictions
- better feedback after actions
- a future path for CLI- and skill-based integrations
