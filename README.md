<p align="center">
  <img src="assets/logo.jpg" alt="Sessio logo" width="960" />
</p>

<h1 align="center">Sessio</h1>

<p align="center">A desktop app for managing local multiple agents session history.</p>

<p align="center">
  <a href="./README-cn.md">中文</a> · <a href="./README.md">English</a>
</p>

## Features

- Aggregates local sessions from `Codex`, `Claude Code`, and `Gemini`
- Builds a local `SQLite` index to avoid full disk scans on every launch
- Watches file changes and refreshes the list automatically
- Filters sessions by agent and project
- Shows session details and message timelines
- Supports Claude subagents
- Copies native `resume` commands for each agent
- Generates cross-agent continuation commands
- Exposes a CLI for listing sessions and working with project memory
- Builds, searches, and resolves project memory records
- Provides a tray menu for quick access to recent sessions
- Supports English and Chinese, plus light / dark / system themes
- Checks GitHub for the latest release in production builds

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

The index database is stored at:

- `~/.sessio/db-data/sessio-index.db`

## Tech Stack

- Frontend: `React 19` + `TypeScript` + `Vite` + `Tailwind CSS`
- Desktop shell: `Tauri v2`
- Backend: `Rust`
- Storage: `SQLite` (`rusqlite` bundled)

Backend modules:

- `src-tauri/src/readers` for parsing raw agent session files
- `src-tauri/src/store` for local index storage
- `src-tauri/src/indexer` for full rebuilds and incremental updates
- `src-tauri/src/watch` for file watching
- `src-tauri/src/polling.rs` for polling-based refreshes

## Development

Recommended versions:

- `Node.js 24.x`
- `pnpm 11.1.0`
- `Rust 1.95+`

Install dependencies:

```bash
pnpm install
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

GitHub Actions currently builds artifacts for:

- macOS universal binary
- Linux `x86_64`
- Linux `arm64`
- Windows `x86_64`

Linux builds usually need Tauri/WebKitGTK dependencies such as:

- `libwebkit2gtk-4.1-dev`
- `libgtk-3-dev`
- `libsoup-3.0-dev`
- `libjavascriptcoregtk-4.1-dev`
- `libssl-dev`

## Usage

After launch, Sessio builds an index in the background and shows your session list.

You can:

- Browse sessions by agent or project in the sidebar
- Open session details to inspect messages
- Copy `resume` commands for the same agent
- Copy `cross` commands to continue the context in another agent
- Jump to recent sessions from the tray menu

Sessio also runs as a CLI when invoked with arguments:

```bash
sessio sessions list --json
sessio sessions messages --agent codex --session-id <id> --json
sessio memory search --project "$PWD" <query> --json
sessio memory resolve --record-id <id> --json
```

The `memory` namespace covers project-memory operations, including `build`, `search`, `resolve`, `base`, `covered-by`, `status`, `sync`, and `jobs`.

If the original session files are cleaned up by the agent, Sessio keeps the index metadata when possible. When the message file is gone, the detail view will show that the content is no longer readable.

## Project Structure

```text
.
├── src/                  # React frontend
├── src-tauri/            # Tauri + Rust backend
├── docs/                 # Design and implementation docs
├── scripts/              # Release helpers
├── package.json
└── README.md
```

## Release

Local release helper:

```bash
pnpm release -- 0.3.3
```

or:

```bash
./scripts/release.sh 0.3.3
```

The script will:

- update `package.json`
- update `src-tauri/Cargo.toml`
- update `src-tauri/tauri.conf.json`
- refresh `src-tauri/Cargo.lock`
- create a local release commit and tag

Pushing the tag triggers the GitHub Actions release workflow.

## Known Limitations

- Only session metadata is indexed; message content is still read from the original files on demand
- Raw log formats vary across agents, so compatibility depends on the current on-disk layout
- If third-party tools change their directory structure or log format, the readers may need to be updated

## License

No license file is currently declared in the repository. Add a `LICENSE` file before open-source distribution.
