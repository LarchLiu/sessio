---
name: computer-use
description: macOS desktop automation — click, right-click, type, key combos, scroll, drag, and screenshot other apps (Notes, Music, Notion, Slack, 网易云音乐…). Works on apps with rich AX trees AND apps that expose no AX (Qt, custom-drawn, AX-disabled) by falling back to pixel-coordinate CGEvent dispatch. Use when the user asks Sessio to interact with a native Mac app. Not for web pages (use the `browser` skill for those).
allowed-tools:
  - Bash
  - Read
---

# Computer Use Skill (macOS)

Drives native macOS apps through two complementary paths:

1. **Accessibility API** (`AXUIElementPerformAction`) — when the app exposes a
   usable AX tree, we act on structured element refs. Tight token budget, no
   cursor movement, no focus steal.
2. **CGEvent physical dispatch** (mouse/keyboard at screen coordinates,
   routed via `CGEventPostToPid`) — when AX is empty/sparse, or you need a
   double-click, right-click, drag, or modifier key combo. Posted to the
   target pid so events land without requiring the window to become
   frontmost.

## How to reach the tools

There are **two equivalent interfaces** — pick based on context:

### Preferred: Sessio `computer_*` MCP tools (direct)

Sessio auto-registers an MCP server (`sessio-computer-use`) that exposes the
Codex-compatible action verbs plus `launch_app` (Sessio extension) and
two onboarding helpers. Every action tool call automatically returns a
fresh post-action screenshot as an image content block — you don't need
to re-call `get_app_state` to see the result.

Tools: `computer_launch_app`, `computer_list_apps`,
`computer_get_app_state`, `computer_click`,
`computer_perform_secondary_action`, `computer_scroll`,
`computer_drag`, `computer_type_text`,
`computer_press_key`, `computer_set_value`, plus
`computer_permissions` and `computer_grant` for onboarding.

Call them directly when available — you'll see the target app's state
update visually after every action.

### Fallback: `sessio cu` CLI via Bash

Use this when scripting or when you need deterministic stdout (CI,
automation pipelines). Same verbs, same semantics, but output is text
only — to see the screen after an action you have to call
`get_app_state` separately and `Read` the resulting JPG path.

## Opening apps — DO NOT use `open -b`

`open -b <bundle>` activates the app by default, which steals focus from
whatever the user is doing. **Never use it.** Instead:

- `get_app_state` **auto-launches** the target app in the background if it
  is not running. This is the default — just call it.
- For explicit control, use `computer_launch_app` / `sessio cu launch-app --bundle <bundle>` —
  it wraps `NSWorkspace.openApplication` with `activates: false` (matches
  Codex's behaviour).

The goal: the user's frontmost app and current Space are never disturbed,
even when Sessio is launching an app from cold.

## Core workflow

Always start each turn with **`get_app_state`**. It returns the AX tree
AND a window screenshot in one round trip — refs for AX-reachable
elements, plus a visual anchor for pixel coordinates when AX is
incomplete. If the target isn't running, it will be launched in the
background automatically.

Via MCP (image returns inline):

```
computer_list_apps                          # pick an app
computer_get_app_state { bundle: "com.apple.Music" }
computer_click { ref: "e7" }                # returns a post-action screenshot
computer_click { x: 820, y: 420, pid: 4521 }   # pixel fallback
computer_press_key { key: "cmd+shift+n", pid: 4521 }
```

Via Bash (two-step for images):

```bash
sessio cu list-apps
sessio cu get-app-state --bundle com.apple.Music
sessio cu click --snapshot-id <id> e7
sessio cu get-app-state --bundle com.apple.Music      # re-fetch to see the change
```

## Permission gate (required once)

The macOS Accessibility dialog **only appears when you actively call
`grant`** — `sessio cu grant` or `computer_grant`. Other AX calls
fail fast with `ax_not_granted` if permission is missing; they don't
prompt. The dialog asks for the signed Sessio desktop app / helper identity.

```bash
sessio cu permissions   # Check Accessibility + Screen Recording
sessio cu grant --permission accessibility
```

Screenshots additionally need **Screen & System Audio Recording**
permission. Errors surface as structured codes
(`ax_not_granted`, `sc_not_granted`).

## Verbs (both interfaces)

| Verb                          | MCP name                                | Bash form                                                     | Notes                                                          |
|-------------------------------|-----------------------------------------|---------------------------------------------------------------|----------------------------------------------------------------|
| Launch app (background)       | `computer_launch_app`                   | `sessio cu launch-app --bundle <bundle>`                        | No foreground activation; no-op if already running             |
| List apps w/ recency hint     | `computer_list_apps`                    | `sessio cu list-apps [--days 14]`                               | Running + installed apps; `days` is accepted as a compatibility hint |
| Snapshot (tree + screenshot)  | `computer_get_app_state`                | `sessio cu get-app-state --bundle <bundle>`                     | Call once per turn before acting                               |
| Click by ref or pixel         | `computer_click`                        | `sessio cu click --snapshot-id <id> <ref>` / `sessio cu click --snapshot-id <id> --x <x> --y <y>` | MCP auto-returns post-action screenshot                        |
| Right-click / context menu    | `computer_perform_secondary_action`     | `sessio cu perform-secondary-action --snapshot-id <id> <ref>`   | AXShowMenu first, CGEvent right-click as fallback              |
| Drag                          | `computer_drag`                         | `sessio cu drag --snapshot-id <id> --from-x <x1> --from-y <y1> --to-x <x2> --to-y <y2>` | Pixel-based (AX has no drag)                                   |
| Type text                     | `computer_type_text`                    | `sessio cu type-text --snapshot-id <id> --text <text>`          | Keystroke input, routed to target pid                          |
| Press key combo               | `computer_press_key`                    | `sessio cu press-key --snapshot-id <id> --key <combo>`          | xdotool syntax: cmd+s, ctrl+shift+t, Return, F1…               |
| Set AX value directly         | `computer_set_value`                    | `sessio cu set-value --snapshot-id <id> <ref> --value <value>`  | For sliders, steppers, typed fields                            |
| Scroll (AX)                   | `computer_scroll`                       | `sessio cu scroll --snapshot-id <id> <ref> --direction <up\|down\|left\|right>` | Falls back to wheel event if AX scroll is unavailable          |
| Check permissions             | `computer_permissions`                  | `sessio cu permissions`                                        | MCP returns AX + screen recording status                       |
| Trigger AX dialog/settings    | `computer_grant`                        | `sessio cu grant --permission <screenshots\|accessibility>`     | Caller should poll permissions after                           |

Additional Bash-only commands such as `raise`, `shot`, `lens`, and
`shutdown` are intentionally out of scope for the current in-process Sessio
implementation; there is no separate always-on computer-use daemon in this
phase.

## Common patterns

### 1. Play "Listen Now" in Music.app

```
computer_get_app_state { bundle: "com.apple.Music" }
# Scan for "Listen Now" row in the sidebar:
computer_click { ref: "e4" }                # screenshot returned automatically
# Find a featured playlist card in the new screenshot:
computer_double_click { x: 620, y: 420 }    # use pixel fallback for double-click
```

See `playbooks/AppleMusic.md` for the full cheat sheet.

### 2. Drive an app with zero AX tree (网易云音乐)

```
computer_list_apps                          # grab netease pid / bundle
computer_get_app_state { bundle: "com.netease.163music" }
# elements: [] but the returned screenshot shows every control.
# Pick coordinates from the screenshot and click:
computer_click { x: 180, y: 340, pid: 41663 }
computer_press_key { key: "space", pid: 41663 }     # play/pause
```

### 3. Save a document with Cmd-S

```
computer_press_key { key: "cmd+s", pid: 4521 }
```

## App-specific playbooks

The `playbooks/` directory has per-app hints distilled from Codex's plugin
and extended for Chinese apps:

- `AppleMusic.md` — Music.app (sidebar/search/playback idioms)
- `Spotify.md` — Spotify desktop (Electron; needs AXEnhancedUserInterface)
- `Notion.md` — Notion (Electron; block editor quirks)
- `Clock.md` — Clock (alarms / timers via AX only)
- `Numbers.md` — Numbers.app spreadsheet edits
- `NetEaseMusic.md` — 网易云音乐 (Qt, zero AX tree — pixel-only playbook)

## Important behaviors and limits

- **MCP returns images inline; Bash returns paths.** MCP action responses
  include a `{ type: 'image' }` block with the post-action screenshot. Bash
  output is text only — use `sessio cu get-app-state` for a visual after an
  action and `Read` the JPG path.
- **Two dispatch paths.** AX for refs, CGEvent for pixels. Both routed
  through `CGEventPostToPid` when a pid is available — events go straight
  into the target app's queue, bypassing global focus steal.
- **Pixel coords are screenshot pixels, not raw screen points.** When
  you pass `{ x, y }` to `click` / `perform_secondary_action` / `drag`,
  the Sessio host translates them back to screen points using the mapping
  recorded by the most recent `get_app_state` for that pid. Always snap
  before clicking by pixel — without a fresh snapshot the host falls
  back to treating coords as raw screen points, which won't line up on
  Retina or whenever `screenshot_max_width` (default 1280) downsampled
  the image. MCP accepts both `coordSpace` and `coord_space`; the CLI
  omits it and lets the host default to screenshot space.
- **Focus-steal suppression.** If the frontmost app changes as a side
  effect of a CGEvent, Sessio re-activates the original app within
  ~10ms.
- **Refs are scoped to the last snapshot.** Re-run `get_app_state` on
  `ref_stale` / `element_not_found`.
- **Electron / Chromium apps.** Sessio sets both
  `AXEnhancedUserInterface` and `AXManualAccessibility` on first
  snapshot — some apps respond only to one. First snap may look thin;
  a second snap usually fills the tree.
- **Apps with no AX tree** (网易云音乐, many Qt builds, some games):
  `get_app_state` returns `elements: []` but still includes the
  screenshot and a CGWindowList-derived window frame. Use pixel clicks
  and `press_key`.
- **Not supported:**
  - Pixel clicks on apps that filter synthetic events via
    `kCGEventSourceUserData` (rare).
  - Privileged system UI (Touch ID, some System Settings panes) — macOS
    blocks synthetic input there by design.
  - Windows and Linux — the provider returns `unsupported_platform`. Use
    the `browser` skill there.

## Troubleshooting

| Symptom                                    | Cause / Fix                                                           |
|--------------------------------------------|------------------------------------------------------------------------|
| `helper_missing`                           | Not used by the current in-process Sessio implementation.              |
| `ax_not_granted`                           | `sessio cu grant --permission accessibility`, then enable Sessio in Accessibility. |
| `sc_not_granted` (on `get_app_state`)      | Enable Sessio in Screen & System Audio Recording.                      |
| `app_not_found`                            | App isn't running. `get_app_state` auto-launches in the background; for explicit control use `sessio cu launch-app --bundle <bundle>` / `computer_launch_app`. **Never** use `open -b` — it activates the app and steals focus. |
| `ref_stale` / `element_not_found`          | Re-run `get_app_state`.                                                |
| `action_unsupported`                       | Element has no AXPress. `strategy: "physical"` routes via CGEvent.     |
| Element snap looks empty on Slack / VS Code / Electron | Re-run `get_app_state` once; `AXEnhancedUserInterface` takes a snap to take effect. |
| `computer_*` tools not listed              | Restart Sessio to pick up the MCP runtime injection.                   |
