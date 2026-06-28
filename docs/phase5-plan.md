# Phase 5 — Approvals + Foreground UX

Goal (v3 plan §"Phase 5"): session-level approvals, app-level approvals,
cancel/abort path, settings visibility. Backend approval policy already exists
(`ApprovalRegistry`, `host.start` gating, `set_computer_use_app_approval`,
`computer_use_status`). This phase adds the **abort/foreground** backend bits and
the **frontend UX**: composer toggle, Settings section, in-app takeover overlay.

## A. Backend (Rust)

1. **Foreground-takeover + abort state in the host** (`computer_use/host.rs`)
   - Add `foreground_active: Arc<Mutex<HashSet<String>>>` (session ids) OR a per
     session flag tracked in the lease. Simpler: track in `ApprovalRegistry`-like
     `Mutex<HashSet>` on the host.
   - `begin_foreground(session_id)` / `end_foreground(session_id)` /
     `foreground_active(session_id) -> bool`.
   - `abort(session_id)`: end foreground + `leases.close(session_id)` (idempotent).
     This is the reliable cancel path the overlay calls.
   - Surface `foreground_active: bool` in `ComputerUseStatus` (new field, camelCase).
   - `require_control` already exists; when an action needs foreground (per
     settings.allow_foreground_takeover) the host marks `begin_foreground` before
     the provider call — minimal: mark foreground on any successful control action
     so the overlay can show "agent is controlling <app>". Keep it conservative.

2. **Tauri commands** (`lib.rs`) — register in invoke_handler:
   - `computer_use_abort(sessio_runtime_session_id)` → `runtime.computer_use_abort(id)`
     (new `RuntimeManager::computer_use_abort` → `host.abort`).
   - `get_computer_use_status` / `set_computer_use_app_approval` already registered.
   - Add `set_computer_use_session_approval(sessio_runtime_session_id, approved)`
     → approve/revoke session (needed so the UI grants session approval explicitly
     rather than implicitly on inject). Manager method + register.

3. **Tests**: host abort releases lease + clears foreground (idempotent);
   foreground status reflected in `ComputerUseStatus`; session-approval command
   path. Extend existing host tests.

## B. Frontend — API surface (`src/api.ts`)

- Add `ComputerUseStatus` interface (enabled, sessionApproved, hasLease,
  canObserve, canInspect, canControl, foregroundActive).
- Add functions: `getComputerUseStatus(sessioRuntimeSessionId)`,
  `setComputerUseAppApproval(sessioRuntimeSessionId, appId, approved)`,
  `setComputerUseSessionApproval(sessioRuntimeSessionId, approved)`,
  `computerUseAbort(sessioRuntimeSessionId)`.

## C. Frontend — Composer toggle (eligibility-gated)

- `useChatComposer.ts`:
  - Add `computerUseEnabled: boolean` state + `setComputerUseEnabled`.
  - Expose `computerUseEligible = selectedRuntimeAgent?.computerUseEligible ?? false`.
  - Reset to false when the selected agent becomes ineligible.
  - Thread `computerUse: computerUseEnabled` into the options builder
    (`runtimeSessionOptions`) at the useChatComposer call site (line ~362) AND in
    `ChatPage.tsx` `runtimeSessionOptions` (line 225) — unify by adding a
    `computerUse?: boolean` param.
- `ChatComposer.tsx`: render a toggle button (mirror the permission/model
  `RuntimeMenuSelect`/icon-button pattern) only when `composer.computerUseEligible`.
  Tooltip via i18n. Active state styling when enabled.

## D. Frontend — Settings "Desktop Control" section (`SettingsPage.tsx`)

- Mirror the Appshot `SettingsGroup`:
  - title `t("settings.desktop_control")`.
  - Load `getDesktopControlPermissionStatus()` on mount + refresh on window focus
    (reuse the appshot focus-refresh effect).
  - Render permission tiers via `desktopControlPermissionPresentation`:
    screenshots / accessibility / control rows + overall description.
  - "Manage permissions" button → reuse `onOpenAppshotPermissions` action (same
    macOS panel) OR a new `open_desktop_control_permission_settings` (reuse appshot
    settings opener; same System Settings panes). Use existing appshot opener to
    avoid new backend command.
- Add i18n: `settings.desktop_control` (section title) EN+ZH. Tier description
  keys already exist.

## E. Frontend — Foreground takeover overlay (in-app React)

- New `src/components/ComputerUseTakeoverOverlay.tsx`:
  - Props: `status: ComputerUseStatus | null`, `targetLabel`, `onAbort`.
  - Render a fixed, high-z warning banner/overlay when `status.foregroundActive`
    (or `status.hasLease && status.canControl`): "Agent is controlling <app>" +
    prominent **Stop / 中止** button calling `onAbort` (→ `computerUseAbort` +
    optionally `cancel_turn`).
  - Mount in `ChatPage` (or `AppOverlays.tsx`): poll `getComputerUseStatus` for the
    active session while a turn is running (interval ~1s) to drive visibility.
    (No backend event channel for computer-use yet; polling is the MVP per the
    plan's "settings visibility" scope. Document the poll.)
- i18n: `computer_use.takeover_title`, `computer_use.takeover_stop`,
  `computer_use.controlling_app` EN+ZH.

## F. Tests

- Rust: host abort/foreground tests (B.A.3 above) + command-method tests.
- TS: extend `desktopControlPermissionPresentation.test.ts` if presentation
  changes (likely none). Add a small unit for the composer options builder
  including `computerUse`. `pnpm typecheck` clean.

## G. Acceptance + commit

- `cargo build` + `cargo test --lib` green; `pnpm typecheck` green.
- Manual smoke not required (no live agent); polling/overlay verified via state.
- Commit `feat:` (no scope) with bullet points, per the standing goal.

## Out of scope (deferred to Phase 6)

- Persisted input-injection enablement (stays observe-only by default; UI shows
  state read-only). Real foreground-takeover OS focus management. Event-channel
  push for status (polling used now).
