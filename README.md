<p align="center">
  <img src="assets/logo.jpg" alt="Sessio logo" width="960" />
</p>

<h1 align="center">Sessio</h1>

<p align="center">A desktop workspace for coding agents: browse local session history, chat with agents live, and orchestrate multi-agent threads.</p>

<p align="center">
  <a href="./README-cn.md">中文</a> · <a href="./README.md">English</a>
</p>

## Features

### Session browser

- Aggregates local sessions from `Codex`, `Claude Code`, `Gemini`, and the bundled `Astra Pi` agent
- Builds a local `SQLite` index to avoid full disk scans on every launch
- Watches file changes (plus periodic polling) and refreshes the list automatically
- Groups sessions by project in the sidebar, with unread markers and live status indicators
- Renders full message timelines with markdown, KaTeX math, syntax highlighting, and file-edit diffs
- Supports Claude subagents, session rename, and session delete

### Live agent chat

- Starts and continues agent sessions directly in the app over the [Agent Client Protocol (ACP)](https://agentclientprotocol.com)
- Streams text, reasoning, and tool-call output; answers permission prompts in-chat
- Per-session model, reasoning effort, and permission mode selection; image and file attachments
- Cross-agent continuation: fork a session to a different agent and carry the context across

### Channels — chat agents from IM

- Connects external chat platforms to the same Sessio runtime used by the desktop UI
- Supported channels include Telegram bots, Discord bots, Lark / Feishu long connections, and WeChat iLink bots
- Each channel can choose its own default agent, model, reasoning effort, and workspace
- Persists chat-to-agent session bindings so IM conversations can resume after app restart
- Supports text, image, and file attachments where the platform allows it; outbound files are uploaded back to the chat
- Telegram includes slash commands and inline menus for `/new`, `/agent`, `/model`, `/effort`, `/workspace`, `/cancel`, and `/end`
- Permission requests are sent back to the IM channel with approve / deny actions when supported
- Designed for local desktop use: Telegram, Discord, and Lark connect outward, so no public HTTP server is required

### Threads — multi-agent collaboration

- Four thread kinds: `Workflow` (staged process templates), `Teamwork` (project assistants), `Brainstorm` (two or more participants), and `Debate` (exactly two participants)
- Multi-session chat timeline with per-lane status, rounds, and orchestration entries
- Stage tracker for workflow threads: per-stage status, summary / outcome, and issue tracking
- Built-in and custom process templates with a drag-and-drop stage editor

### Astra orchestrator

- Rust-native, in-process orchestrator that plans and dispatches thread work to agents
- Plan rounds and tasks with dependency-aware dispatch waves, retries, and per-task output artifacts written to `<project>/.sessio/astra`
- Configurable orchestrator agent / model / effort / permission mode
- Ships with the bundled `astra-pi` sidecar (built from [pi_agent_rust](https://github.com/Dicklesworthstone/pi_agent_rust)), including custom AI provider channels (base URL, API key, model list)

### More

- Custom assistants (backing agent + model + system prompt + permission mode), managed globally or per project
- Project memory: builds searchable memory records from sessions via the `qmd` backend, with continuation provenance (`covered-by` / `base`)
- CLI mode with `sessions`, `thread`, `stage`, `config`, and `memory` command groups
- Tray menu for quick access to recent sessions and threads
- In-app updater (Tauri updater artifacts) with a GitHub Releases fallback
- English and Chinese UI, light / dark / system themes, and HTTP(S) proxy settings

## Data Sources

Sessio reads the session files already on your machine and does not rely on any cloud service.

By default it scans:

- Codex
  - `~/.codex/sessions`
  - `~/.codex/archived_sessions`
- Claude Code
  - `~/.claude/projects`
- Gemini
  - `~/.gemini/tmp`
  - `~/.gemini/projects.json`
- Astra Pi (sessions created by Sessio itself)
  - `~/.sessio/astra-pi-agent/sessions`

App data lives under:

- release builds: `~/.sessio`
- debug / local dev builds: `~/.sessio-dev`

Examples:

- `~/.sessio/db-data/sessio-index.db` — SQLite index
- `~/.sessio/config.toml` — memory / index / proxy / debug configuration
- `~/.sessio/im-bridge.yaml` — Channels configuration for Telegram / Discord / Lark / WeChat
- `~/.sessio/bin/sessio` — CLI symlink created on launch

## Agent Runtime

Live chats spawn agents as ACP subprocesses. Default commands:

- Astra Pi: bundled `astra-pi` sidecar
- Codex: `npx -y @zed-industries/codex-acp@latest`
- Claude Code: `npx -y @zed-industries/claude-code-acp@latest`
- Gemini: `npx -y @google/gemini-cli@latest --experimental-acp`

Agents can be enabled / disabled in Settings → Agents, where you can also edit each agent's model catalog, default model, reasoning effort, and permission mode. The orchestrator agent used by Astra is configured separately in the same settings section.

Channels can be configured in Settings → Workflows → Channels. Sessio stores per-platform defaults and workspace allowlists in the active app-home `im-bridge.yaml`, while active chat bindings live in the local SQLite index. Because a channel can drive agents that run local tools, restrict allowed users / chats and workspaces before enabling it.

## Tech Stack

- Frontend: `React 19` + `TypeScript` + `Vite` + `Tailwind CSS`
- Desktop shell: `Tauri v2`
- Backend: `Rust` (edition 2021, `agent-client-protocol`)
- Storage: `SQLite` (`rusqlite` bundled)

Backend modules:

- `src-tauri/src/agents/sources` for parsing raw agent session files
- `src-tauri/src/agents/runtime` for running live agent sessions over ACP
- `src-tauri/src/astra` for the multi-agent orchestrator
- `src-tauri/src/im_bridge` for Channels integrations and IM-to-runtime routing
- `src-tauri/src/store` for local index storage
- `src-tauri/src/indexer` for full rebuilds and incremental updates
- `src-tauri/src/watch` for file watching
- `src-tauri/src/polling.rs` for polling-based refreshes
- `src-tauri/src/memory` for the project-memory pipeline (`qmd` backend)
- `src-tauri/src/turns.rs` for normalizing raw events into renderable turns
- `src-tauri/src/cli.rs` for the `sessio` CLI

## Development

Recommended versions:

- `Node.js 24.x`
- `pnpm 11.1.0`
- `Rust 1.95+`

Install dependencies:

```bash
pnpm install
```

Prepare the `astra-pi` sidecar binary (required once before running or bundling the desktop app):

```bash
node scripts/prepare-astra-pi-sidecar.mjs <target-triple|all>
# e.g. on Apple Silicon:
node scripts/prepare-astra-pi-sidecar.mjs aarch64-apple-darwin
```

Run the frontend dev server:

```bash
pnpm dev
```

Run the Tauri desktop app in development:

```bash
pnpm tauri dev
```

Type check:

```bash
pnpm typecheck
```

Run tests:

```bash
pnpm test
```

Type check plus tests:

```bash
pnpm check
```

Build the frontend:

```bash
pnpm build
```

Build the desktop app without bundling installers:

```bash
pnpm ci:build
```

Build release packages:

```bash
pnpm bundle
```

## Platform Notes

GitHub Actions currently builds release installers and updater artifacts for:

- macOS universal `.dmg` plus signed `.app.tar.gz` updater package
- Linux `x86_64` `.deb` / `.rpm` plus updater signatures
- Linux `arm64` `.deb` / `.rpm` plus updater signatures
- Windows `x86_64` NSIS installer plus updater signature

The release workflow requires the Tauri updater private key in the
`TAURI_SIGNING_PRIVATE_KEY` GitHub secret. `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
is optional when the key has no password.

Linux builds usually need Tauri/WebKitGTK dependencies such as:

- `libwebkit2gtk-4.1-dev`
- `libgtk-3-dev`
- `libsoup-3.0-dev`
- `libjavascriptcoregtk-4.1-dev`
- `libssl-dev`

## Usage

After launch, Sessio builds an index in the background and shows your session list.

You can:

- Browse sessions and threads by project in the sidebar
- Open session details to inspect messages, tool calls, and diffs
- Start a new chat with any enabled agent, or continue an existing session
- Fork a session to a different agent to continue the context there
- Create a thread (workflow / teamwork / brainstorm / debate) and let Astra orchestrate it
- Track workflow stages and issues from the thread page
- Enable Channels to chat with local agents from Telegram, Discord, Lark, or WeChat
- Jump to recent sessions and threads from the tray menu

Sessio also runs as a CLI when invoked with arguments:

```bash
sessio sessions list --json
sessio sessions messages --agent codex --session-id <id> --json
sessio thread list --json
sessio stage list --thread-id <id> --json
sessio memory search --project "$PWD" <query> --json
sessio memory resolve --record-id <id> --json
```

Command groups:

- `sessions` — `list`, `messages`
- `thread` — `list`, `show`
- `stage` — `list`, `show`, `set-status`, `update`, plus `issue add | list | set`
- `config` — `show`, `memory set`
- `memory` — `status`, `sync`, `build`, `search`, `resolve`, `covered-by`, `base`, `jobs`

If the original session files are cleaned up by the agent, Sessio keeps the index metadata when possible. When the message file is gone, the detail view will show that the content is no longer readable.

## Project Structure

```text
.
├── src/                  # React frontend
├── src-tauri/            # Tauri + Rust backend
├── docs/                 # Design and implementation docs
├── scripts/              # Release and sidecar helpers
├── test/                 # Frontend unit tests (vitest)
├── package.json
└── README.md
```

## Release

Local release helper:

```bash
pnpm release -- 0.5.0
# or a beta/prerelease tag:
pnpm release -- 0.5.0-beta.1
```

or:

```bash
./scripts/release.sh 0.5.0
```

The script will:

- update `package.json`
- update `src-tauri/Cargo.toml`
- update `src-tauri/tauri.conf.json`
- refresh `src-tauri/Cargo.lock`
- create a local release commit and tag

Pushing the tag triggers the GitHub Actions release workflow.
Tags with a prerelease suffix such as `v0.5.0-beta.1` are published as GitHub prereleases and are not marked as the latest release.

## Known Limitations

- Only session metadata is indexed; message content is still read from the original files on demand
- Raw log formats vary across agents, so compatibility depends on the current on-disk layout
- If third-party tools change their directory structure or log format, the source parsers may need to be updated
- Live agent runtimes for Codex / Claude Code / Gemini are fetched via `npx` and require the corresponding agent CLI accounts / API access

## License

No license file is currently declared in the repository. Add a `LICENSE` file before open-source distribution.
