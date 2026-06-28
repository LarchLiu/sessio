---
name: computer-use
description: macOS desktop automation — click, right-click, type, key combos, scroll, drag, and screenshot other apps (Notes, Music, Notion, Slack, 网易云音乐…). Works on apps with rich AX trees AND apps that expose no AX (Qt, custom-drawn, AX-disabled) by falling back to pixel-coordinate CGEvent dispatch. Use when the user asks Alma to interact with a native Mac app. Not for web pages (use the `browser` skill for those).
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

### Preferred: `computer-use__*` MCP tools (direct)

Alma auto-registers an MCP server (`computer-use`) that exposes the nine
Codex-compatible action verbs plus `launch_app` (Alma extension) and
two onboarding helpers. Every action tool call automatically returns a
fresh post-action screenshot as an image content block — you don't need
to re-call `get_app_state` to see the result.

Tools: `computer-use__launch_app`, `computer-use__list_apps`,
`computer-use__get_app_state`, `computer-use__click`,
`computer-use__perform_secondary_action`, `computer-use__scroll`,
`computer-use__drag`, `computer-use__type_text`,
`computer-use__press_key`, `computer-use__set_value`, plus
`computer-use__permissions` and `computer-use__grant` for onboarding.

Call them directly when available — you'll see the target app's state
update visually after every action.

### Fallback: `alma cu` CLI via Bash

Use this when scripting or when you need deterministic stdout (CI,
automation pipelines). Same verbs, same semantics, but output is text
only — to see the screen after an action you have to call
`get_app_state` separately and `Read` the resulting JPG path.

## Opening apps — DO NOT use `open -b`

`open -b <bundle>` activates the app by default, which steals focus from
whatever the user is doing. **Never use it.** Instead:

- `get_app_state` **auto-launches** the target app in the background if it
  is not running. This is the default — just call it.
- For explicit control, use `launch_app` / `alma cu launch_app <bundle>` —
  it wraps `NSWorkspace.openApplication` with `activates: false` (matches
  Codex's behaviour).

The goal: the user's frontmost app and current Space are never disturbed,
even when Alma is launching an app from cold.

## Core workflow

Always start each turn with **`get_app_state`**. It returns the AX tree
AND a window screenshot in one round trip — refs for AX-reachable
elements, plus a visual anchor for pixel coordinates when AX is
incomplete. If the target isn't running, it will be launched in the
background automatically.

Via MCP (image returns inline):

```
computer-use__list_apps                     # pick an app
computer-use__get_app_state { bundle: "com.apple.Music" }
computer-use__click { ref: "e7" }           # returns a post-action screenshot
computer-use__click { x: 820, y: 420, pid: 4521 }   # pixel fallback
computer-use__press_key { key: "cmd+shift+n", pid: 4521 }
```

Via Bash (two-step for images):

```bash
alma cu list_apps
alma cu get_app_state com.apple.Music
alma cu click e7
alma cu get_app_state com.apple.Music      # re-fetch to see the change
```

## Permission gate (required once)

The macOS Accessibility dialog **only appears when you actively call
`grant`** — `alma cu grant` or `computer-use__grant`. Other AX calls
fail fast with `ax_not_granted` if permission is missing; they don't
prompt. The dialog asks for **Alma Computer Use** (a separate signed
helper, not "Alma").

```bash
alma cu doctor          # Check Accessibility + Screen Recording
alma cu grant           # Trigger AX permission dialog if not granted
```

Screenshots additionally need **Screen & System Audio Recording**
permission. Errors surface as structured codes
(`ax_not_granted`, `sc_not_granted`).

## Verbs (both interfaces)

| Verb                          | MCP name                                | Bash form                                                     | Notes                                                          |
|-------------------------------|-----------------------------------------|---------------------------------------------------------------|----------------------------------------------------------------|
| Launch app (background)       | `computer-use__launch_app`              | `alma cu launch_app <bundle>`                                  | No foreground activation; no-op if already running             |
| List apps w/ recency          | `computer-use__list_apps`               | `alma cu list_apps [--days=14]`                                | Running + recently-used with usage frequency                   |
| Snapshot (tree + screenshot)  | `computer-use__get_app_state`           | `alma cu get_app_state <bundle\|pid>`                          | Call once per turn before acting                               |
| Click by ref or pixel         | `computer-use__click`                   | `alma cu click <ref>` / `alma cu click --pixel <x> <y>`         | MCP auto-returns post-action screenshot                        |
| Right-click / context menu    | `computer-use__perform_secondary_action`| `alma cu perform_secondary_action <ref>`                       | AXShowMenu first, CGEvent right-click as fallback              |
| Drag                          | `computer-use__drag`                    | `alma cu drag <x1> <y1> <x2> <y2>`                              | Pixel-based (AX has no drag)                                   |
| Type text                     | `computer-use__type_text`               | `alma cu type_text <text> [--pid=N]`                            | Keystroke input, focus target first                            |
| Press key combo               | `computer-use__press_key`               | `alma cu press_key <combo> [--pid=N]`                           | xdotool syntax: cmd+s, ctrl+shift+t, Return, F1…               |
| Set AX value directly         | `computer-use__set_value`               | `alma cu set_value <ref> <value>`                               | For sliders, steppers, typed fields                            |
| Scroll (AX)                   | `computer-use__scroll`                  | `alma cu scroll <ref> <up\|down\|left\|right>`                   | Falls back to no-op if not scrollable                          |
| Check permissions             | `computer-use__permissions`             | `alma cu doctor`                                               | MCP returns AX + screen recording status; CLI also pings the daemon (version/uptime) and prints the helper path |
| Trigger AX dialog             | `computer-use__grant`                   | `alma cu grant`                                                | Caller should poll permissions after                           |

Additional Bash-only:
- `alma cu raise <bundle|pid>` — raise window without activating app
- `alma cu shot <bundle|pid> [--out=PATH]` — one-off window capture
- `alma cu lens <on|off|toggle>` — virtual-cursor overlay
- `alma cu shutdown` — stop the helper daemon

## Common patterns

### 1. Play "Listen Now" in Music.app

```
computer-use__get_app_state { bundle: "com.apple.Music" }
# Scan for "Listen Now" row in the sidebar:
computer-use__click { ref: "e4" }           # screenshot returned automatically
# Find a featured playlist card in the new screenshot:
computer-use__click { ref: "e22", click_count: 2 }   # double-click to play
```

See `playbooks/AppleMusic.md` for the full cheat sheet.

### 2. Drive an app with zero AX tree (网易云音乐)

```
computer-use__list_apps                     # grab netease pid / bundle
computer-use__get_app_state { bundle: "com.netease.163music" }
# elements: [] but the returned screenshot shows every control.
# Pick coordinates from the screenshot and click:
computer-use__click { x: 180, y: 340, pid: 41663 }
computer-use__press_key { key: "space", pid: 41663 }     # play/pause
```

### 3. Save a document with Cmd-S

```
computer-use__press_key { key: "cmd+s", pid: 4521 }
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
  output is text only — use `alma cu get_app_state` for a visual after an
  action and `Read` the JPG path.
- **Two dispatch paths.** AX for refs, CGEvent for pixels. Both routed
  through `CGEventPostToPid` when a pid is available — events go straight
  into the target app's queue, bypassing global focus steal.
- **Pixel coords are screenshot pixels, not raw screen points.** When
  you pass `{ x, y }` to `click` / `perform_secondary_action` / `drag`,
  the daemon translates them back to screen points using the mapping
  recorded by the most recent `get_app_state` for that pid. Always snap
  before clicking by pixel — without a fresh snapshot the daemon falls
  back to treating coords as raw screen points, which won't line up on
  Retina or whenever `screenshot_max_width` (default 1280) downsampled
  the image. MCP defaults `coord_space` to `"screenshot"`; the CLI
  omits it and lets the daemon decide.
- **Focus-steal suppression.** If the frontmost app changes as a side
  effect of a CGEvent, the helper re-activates the original app within
  ~10ms.
- **Refs are scoped to the last snapshot.** Re-run `get_app_state` on
  `ref_stale` / `element_not_found`.
- **Electron / Chromium apps.** The helper sets both
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
  - Windows and Linux — the helper returns `unsupported_platform`. Use
    the `browser` skill there.

## Troubleshooting

| Symptom                                    | Cause / Fix                                                           |
|--------------------------------------------|------------------------------------------------------------------------|
| `helper_missing`                           | Run `pnpm build:computer-use` (dev) or reinstall Alma.                 |
| `ax_not_granted`                           | `alma cu grant`, then enable "Alma Computer Use" in Accessibility.     |
| `sc_not_granted` (on `shot`/`get_app_state`) | Enable "Alma Computer Use" in Screen & System Audio Recording.       |
| `app_not_found`                            | App isn't running. `get_app_state` auto-launches in the background; for explicit control use `alma cu launch_app <bundle>` / `computer-use__launch_app`. **Never** use `open -b` — it activates the app and steals focus. |
| `ref_stale` / `element_not_found`          | Re-run `get_app_state`.                                                |
| `action_unsupported`                       | Element has no AXPress. `strategy: "physical"` routes via CGEvent.     |
| Element snap looks empty on Slack / VS Code / Electron | Re-run `get_app_state` once; `AXEnhancedUserInterface` takes a snap to take effect. |
| `computer-use__*` tools not listed         | Restart Alma to pick up the MCP config. Check `~/.config/alma/mcp.json`. |
