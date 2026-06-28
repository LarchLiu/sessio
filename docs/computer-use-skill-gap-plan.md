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
  - macOS `CGEventPostToPid` dispatch for physical mouse/keyboard/scroll
    events, key chords, best-effort focus restoration, and Electron AX flags
    (`AXEnhancedUserInterface`, `AXManualAccessibility`)
- Remaining non-parity items are intentionally scoped as future architecture
  work rather than hidden TODOs in this in-process phase:
  - replacing the current `screencapture`/CGWindow-based capture path with
    ScreenCaptureKit
  - deriving true recently-used app frequency for `list_apps --days`
  - adding helper/daemon-only CLI affordances such as `raise`, `shot`, `lens`,
    and `shutdown`

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

## Capability Gaps Against SKILL.md

### A. Tool surface gaps

The skill describes a richer action surface than Sessio currently exposes.

Missing or materially incomplete tools:

- `launch_app`
- pixel/coordinate click
- right click / secondary action
- double click
- drag
- `set_value`
- explicit onboarding tools such as `permissions` and `grant`

Current constraint:

- [provider.rs](../src-tauri/src/computer_use/provider.rs) only models four
  allowed actions: click element, type text, press key, scroll
- [protocol.rs](../src-tauri/src/computer_use/mcp_http/protocol.rs) only
  advertises the nine-tool MVP catalog above

### B. No pixel fallback path

`SKILL.md` treats AX refs plus pixel fallback as the central operating model.
Sessio currently supports only element-targeted click, not coordinate-targeted
actions.

That means Sessio cannot currently handle the important class of apps described
in the skill:

- sparse AX trees
- custom-drawn apps
- Qt apps
- Electron apps with incomplete AX
- AX-disabled apps

### C. No secondary-click, double-click, or drag semantics

The skill expects these to be first-class capabilities. Sessio currently has no
tool schema or provider contract for them.

This is not only a missing tool name problem. It also means:

- no action authorization path
- no host-level allowed-action modeling
- no MCP schema
- no macOS implementation hooks

### D. No direct value-setting API

The skill includes `set_value` for sliders, steppers, and editable fields. In
Sessio, value changes are currently limited to typed keystrokes.

That is weaker for:

- sliders
- numeric steppers
- fields that are easier to mutate through AX than through typing

### E. No app auto-launch / background launch

The skill expects:

- `get_app_state` can auto-launch a target app in the background
- `launch_app` can open an app without activating it

Sessio currently requires the target app to be running already. The current
macOS provider resolves by bundle id against running apps only.

There is also an approval-boundary issue to keep explicit here:

- `launch_app` and auto-launch via `computer_get_app_state` should go through
  the same target-app approval path
- a read-shaped tool such as `computer_get_app_state` should not silently widen
  permissions just because it can trigger launch as a side effect
- if `computer_get_app_state` launches the app, the response should say so

### F. No installed/recent app discovery

The skill's `list_apps` behavior includes running and recently used apps.
Sessio currently lists running apps only.

This makes discovery weaker and makes cold-start workflows harder.

### G. No pid-scoped or background-targeted event routing

`SKILL.md` explicitly describes event routing to the target pid without forcing
foreground activation. Sessio's current macOS provider posts events to the
global HID tap.

That means Sessio does not yet match the skill's stronger guarantees around:

- reduced focus stealing
- target-specific input routing
- better behavior when the user is active in another app

### H. No screenshot-coordinate mapping contract

The skill defines an important coordinate-space rule:

- pixel coordinates are based on the screenshot returned by the latest
  `get_app_state`
- the runtime maps screenshot pixels back to real screen points

Sessio currently returns display metadata and screenshot references, but it does
not yet define a coordinate-space contract because pixel actions do not exist.

This must be added before coordinate actions are trustworthy on Retina or any
downsampled screenshot flow.

The local [computer-use-skill.md](./computer-use-skill.md) also makes one more
expectation explicit here: MCP should default to screenshot-space coordinates,
not leave coordinate semantics implicit.

### I. No post-action screenshot return shape

The skill's preferred MCP path automatically returns a fresh screenshot after
every action. Sessio currently returns text-only MCP tool results.

This is a major usability gap because it forces the agent to reason from stale
state unless it explicitly calls `computer_get_app_state` again.

The local [computer-use-skill.md](./computer-use-skill.md) strengthens this
into an interface expectation, not just a convenience note. That makes it a
good acceptance target for future MCP response-shape work.

There is also a deeper contract problem underneath the missing screenshot:

- actions currently validate the latest snapshot id before acting, but they do
  not mint a fresh post-action snapshot id
- the same pre-action snapshot remains the lease's latest snapshot until another
  explicit `computer_get_app_state`
- MCP helpers currently wrap tool success as text-only content, not structured
  post-action state

For parity with the skill, the fix should be a **post-action state contract**,
not only a screenshot convenience:

- every action response should return the new authoritative state the model
  should reason from
- that state should include either a full `app_state` or enough data to replace
  it, including a fresh snapshot id
- the pre-action snapshot should no longer remain implicitly authoritative after
  a successful mutation

### J. Permission and onboarding tools are not exposed to the model

The skill exposes onboarding helpers:

- `permissions`
- `grant`

Sessio currently has settings screens and permission-panel commands, but does
not expose these as part of the computer-use MCP tool family.

That means the agent cannot guide its own onboarding path through the same
computer-use interface.

### K. Host control policy conflates input control with foreground takeover

The shared desktop-control truth source is now in better shape than earlier
drafts of this plan assumed. The remaining problem is higher in the stack:

- [src-tauri/src/computer_use/host.rs](../src-tauri/src/computer_use/host.rs)
  currently defines control availability as
  `allow_input_injection && allow_foreground_takeover`
- control actions currently mark the session as foreground-active
- [src/pages/SettingsPage.tsx](../src/pages/SettingsPage.tsx) still presents
  `Input control` and `Foreground takeover` as separate toggles

That creates a policy-model mismatch:

- the UI suggests two independent settings
- the runtime treats them as one combined gate
- future pid-scoped or non-activating control paths have nowhere to plug in
  because all control is treated as takeover control

This plan should therefore treat the issue as a host-policy design gap, not a
desktop-control truth-source bug.

The immediate direction for Sessio should be:

- treat `computer use enabled` as already implying that control is allowed
- remove separate product-policy gates such as `allow_input_injection`
- only keep `foreground takeover` as a future concept if actions are explicitly
  modeled by whether they actually require foreground takeover

### L. No CLI parity layer (historical gap)

The skill provides two interfaces:

- MCP tools
- CLI commands

At the time this plan was written, Sessio had the MCP path only. The current
implementation now exposes a `sessio cu ...` CLI surface aligned with the
in-repo skill contract.

That means Sessio still lacks:

- scripting entry points
- deterministic stdout workflows
- a stable JSON CLI for other agent skills or external automation

There is also an architecture boundary to keep explicit:

- CLI parity should not implicitly introduce a helper, daemon, or separate
  always-on automation service in this phase
- the first CLI shape should be a thin operator surface attached to the
  existing desktop-host implementation
- if no eligible desktop host/session is available, the CLI should fail
  explicitly instead of silently creating a different runtime model

### M. No app-specific playbooks or skill resources

The skill references app-specific playbooks. Sessio currently has no equivalent
computer-use playbook layer for app-specific guidance such as:

- Music
- Spotify
- Notion
- Numbers
- NetEase Music

This does not block the MVP, but it does block parity with the richer skill
experience.

The in-repo [computer-use-skill.md](./computer-use-skill.md) is especially
useful here because it already names concrete playbook candidates and how they
should be used operationally.

The boundary to keep here is:

- playbook content and inventory belong in this capability plan
- how those playbooks are injected into model context belongs in the prompt
  refactor / skill-integration work, not here

## Gap Categories

### Category 1: Must-fix correctness gaps

These create inconsistent or misleading behavior today:

- host policy still models control as one combined
  `allow_input_injection && allow_foreground_takeover` gate
- auto-launch semantics are not yet pinned to the existing app-approval model
- tool surface does not match what the product UI implies
- action responses do not provide enough post-action state or snapshot turnover

### Category 2: Must-have capability gaps for native-app usefulness

These are required for broad real-world native-app automation:

- pixel click fallback
- right click
- double click
- drag
- app launch
- value setting

### Category 3: Important robustness gaps

- cold-start workflow parity for `get_app_state`
- pid-scoped / background-targeted event delivery
- screenshot coordinate mapping
- installed/recent app discovery
- onboarding tools

### Category 4: Productization gaps

- CLI parity
- app-specific playbooks
- richer response rendering

## Recommended Implementation Order

### Phase 1: Re-baseline host control policy

Goal:

- make `computer use enabled` map directly to the runtime's control semantics

Tasks:

- keep the shared desktop-control truth source as an OS/provider fact source,
  not a product-policy gate
- remove separate product-policy control gates such as
  `allow_input_injection`
- remove `allow_foreground_takeover` as a prerequisite for all control actions
- decide whether `foreground takeover` remains:
  - a purely runtime/overlay state
  - or a future per-action policy once non-takeover control paths exist
- ensure Settings, overlay, status, and allowed-actions all read the same
  policy model

Acceptance:

- enabling computer use means control actions are product-allowed, subject only
  to OS capability, approvals, and session state
- the product no longer invents fake independence between separate control
  toggles
- Settings labels map 1:1 to host behavior

### Phase 2: Add app launch and discovery workflow parity

Goal:

- support the skill's cold-start, snap-first operating model

Tasks:

- add background app launch
- add optional explicit `launch_app`
- allow `get_app_state` to satisfy the skill's "launch if needed, then snap"
  workflow without manual pre-launch
- make `launch_app` and launch-via-`get_app_state` share the same target-app
  approval rule
- do not allow `get_app_state` to silently launch an unapproved target
- expose whether the returned state came from an already-running app or a new
  background launch
- add recent/installed app discovery strategy

Acceptance:

- auto-launch never bypasses the app-approval boundary just because the entry
  point was `computer_get_app_state`
- the agent can discover and open a target app without manual pre-launch
- cold-start workflows do not depend on `open -b` or prior user setup

### Phase 3: Add post-action state and snapshot contract

Goal:

- eliminate stale-state reasoning after mutations

Tasks:

- extend MCP tool responses to include authoritative post-action state
- decide whether the returned unit is:
  - full `app_state`
  - or image + minimal metadata + fresh snapshot id
- ensure successful control actions advance snapshot authority instead of
  leaving the pre-action snapshot implicitly current
- update MCP content handling so action responses are not limited to text-only
  wrappers

Acceptance:

- the agent can observe action results without an immediate extra snapshot call
- every successful mutation yields the new state the next action must target
- the pre-action snapshot is no longer silently reusable as authoritative state

### Phase 4: Add screenshot coordinate-space semantics

Goal:

- make pixel actions reliable against the snapshot the model actually saw

Tasks:

- define screenshot-space vs screen-space contract
- record enough snapshot metadata to map image coordinates back to real screen
  points
- tie coordinate actions to the latest authoritative snapshot
- reject or clearly define stale coordinate usage

Acceptance:

- coordinate actions line up with the screenshot the model used to reason

### Phase 5: Expand the provider and tool contract

Goal:

- close the most important action-surface gaps on top of the corrected state and
  coordinate contracts

Tasks:

- add provider contract support for:
  - coordinate click
  - secondary click
  - double click
  - drag
  - set value
- extend host allowed-action modeling
- extend MCP schemas and dispatch
- expose coordinate actions only with the screenshot-space semantics from Phase
  4, not before

Acceptance:

- the tool protocol can represent both AX-targeted and pixel-targeted actions
- the host can gate the new actions consistently

### Phase 6: Expose onboarding tools

Goal:

- make permission setup agent-addressable

Tasks:

- add `permissions` status tool
- add `grant` helper tool or explicit permission-open flow
- align returned error codes with onboarding guidance

Acceptance:

- an agent can explain and advance the permission setup process using the
  computer-use surface itself

### Phase 7: Add CLI parity

Goal:

- expose the same core capability to scripts and skill-based integrations

Tasks:

- design `sessio cu ...` commands
- define the initial CLI as attached to the running desktop-host
  implementation, not as a standalone daemon/service
- define the local auth/session-routing model the CLI uses to reach that host
- add `--json` machine-readable output
- keep verb names aligned with MCP where practical

Acceptance:

- Sessio has a stable non-MCP automation surface for computer use without
  changing the current in-process architecture
- headless or daemonized external automation remains explicitly out of scope for
  this phase

### Phase 8: Add app playbooks

Goal:

- improve reliability for known apps

Tasks:

- create app-specific usage notes
- define the playbook file/resource shape and versioned inventory
- keep model-exposure and prompt-injection mechanics in the prompt refactor
  plan, not in this document
- use [computer-use-skill.md](./computer-use-skill.md) as the initial inventory
  for playbook targets and operating conventions

Acceptance:

- app-specific guidance content is versioned and reviewable in-repo
- prompt/runtime injection ownership remains intentionally separate

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
