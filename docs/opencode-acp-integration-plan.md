# OpenCode ACP Integration Plan

## Summary

This plan adds OpenCode to Sessio in a desktop-first way and keeps two responsibilities separate:

- `runtime`: treat OpenCode as a standard ACP agent and continue same-agent sessions with `resumeSession`
- `history/index`: parse OpenCode local storage for session list and transcript display

The first implementation should be usable from the desktop UI without requiring broader product work for IM bridge flows, scheduled tasks, or thread orchestration. Those surfaces can pick up OpenCode automatically from the shared runtime-agent list, but they are not part of the v1 productization scope.

Success criteria:

- Settings, New Chat, and Chat can use OpenCode as a built-in agent
- same-agent continuation uses ACP `resume`, not `load`
- Sessio can discover and display local OpenCode sessions
- missing install or login state produces actionable desktop errors
- OpenCode history rendering does not depend on ACP replay to reconstruct the visible transcript

## Design Principles

### Runtime and history stay separate

Sessio should preserve the existing architecture boundary:

- `src-tauri/src/agents/runtime` owns active process control and ACP communication
- `src-tauri/src/agents/sources` owns historical session discovery and transcript parsing

OpenCode should follow the same split.

The local history path should not be replaced by ACP `loadSession`, because Sessio already treats disk-backed session files and databases as the source of truth for transcript browsing. ACP is the live runtime path, not the historical indexing path.

### Resume is the primary continuation path

For OpenCode, same-agent continuation should continue to use Sessio's runtime resume flow:

- when Sessio already has an OpenCode runtime session id for the active session, continue with ACP `resumeSession`
- keep ACP `loadSession` for attach-style or cross-agent compatibility scenarios, not normal same-agent continuation

This matches the intended semantics:

- `resume` means "continue the agent-owned conversation"
- local source parsing means "show the full stored history"

The UI should not depend on OpenCode ACP replay depth to show prior messages.

### OpenCode mode is not Sessio permission mode

OpenCode exposes ACP config options such as model, effort/variant, and mode. Its `mode` corresponds to OpenCode agent mode or persona selection, not Sessio's current permission-mode concept.

The first implementation should avoid mapping Sessio `permissionMode` to OpenCode ACP `mode`. Runtime controls should be agent-aware so OpenCode only exposes configuration that is semantically correct.

## Implementation Plan

### Phase 1: Register OpenCode as a built-in ACP agent

Add OpenCode as a first-class built-in agent across Rust and TypeScript types.

Key changes:

- add `opencode` to the shared `Agent` enum in Rust and the generated/frontend `Agent` type
- add OpenCode display name and icon wiring in the desktop UI
- seed a built-in OpenCode agent row in SQLite with:
  - transport `acp`
  - session command `opencode acp`
  - version command `opencode --version`
- default OpenCode to `enabled = false` so users without a local install do not get a broken default runtime option

Expected outcome:

- OpenCode appears in Settings and runtime-agent lists
- no session source or runtime behavior change is required yet beyond agent registration

### Phase 2: Adapt the OpenCode runtime behavior

Integrate OpenCode into the existing ACP runtime manager without introducing a new transport type.

Key changes:

- keep the current Sessio same-agent continuation flow so OpenCode uses `AcpSessionStart::Resume`
- do not override OpenCode to prefer `Load` during normal continuation
- use the existing ACP command resolution flow so OpenCode launches through `opencode acp`
- gate runtime controls by agent semantics:
  - enable `model`
  - enable `effort`
  - hide or disable `permission mode` for OpenCode in composer and chat runtime controls

Initial config behavior for OpenCode:

- `model` can continue to flow through ACP session config updates
- `effort` should map to the OpenCode effort/variant config path already exposed through ACP config options
- `permissionMode` should not be sent as ACP `mode`

Expected outcome:

- OpenCode can be selected as a desktop runtime agent
- continuing the same session uses ACP resume semantics
- the runtime UI does not present misleading permission-mode controls

### Phase 3: Add a local OpenCode source parser

Add an OpenCode historical session source under `src-tauri/src/agents/sources`.

The implementation should follow the current OpenCode storage shape:

- primary path: `$XDG_DATA_HOME/opencode/opencode.db`
- compatibility path: `$XDG_DATA_HOME/opencode/storage/...`

Key behaviors:

- `discover` should read both SQLite-backed and legacy JSON-backed sessions
- deduplicate by OpenCode session id and prefer the SQLite-backed record when both exist
- `parse_source` should produce stable `SessionRecord` values with:
  - session id
  - title
  - project directory
  - created and updated timestamps
  - first user summary
  - metadata indicating whether the source is sqlite or legacy
- `read_messages` should reconstruct visible transcript content from OpenCode `message` and `part` records
- `roots` and `classify_path_event` should cover both the SQLite file and the legacy storage directories so indexing can rebuild correctly

Message parsing expectations for v1:

- text parts should render as text
- tool parts should at least render as recognizable tool events or tool placeholders
- ordering should be stable by message and part creation time
- non-text parts that Sessio cannot yet render richly should degrade to safe textual or block-level output

Expected outcome:

- OpenCode historical sessions appear in the session list
- opening an OpenCode session shows a transcript derived from local storage, not ACP replay

### Phase 4: Error handling and desktop UX

Add minimal desktop-first failure handling so OpenCode is usable without extra protocol work.

Key changes:

- detect command launch failures for:
  - missing `opencode`
  - non-executable command
  - ACP initialize failure
- recognize OpenCode auth-required failures and surface a clear instruction to run:

```bash
opencode auth login
```

- add a lightweight Settings or chat error message path instead of implementing a full ACP authentication UX
- document OpenCode in README/Data Sources/Agent Runtime sections if the feature is considered ready for end users

Expected outcome:

- users understand whether they need to install OpenCode or log in before using it
- desktop errors are actionable without introducing a new onboarding subsystem

## Interfaces and Type Changes

Public and cross-layer changes should stay minimal.

Required changes:

- add `opencode` to the shared `Agent` enum and corresponding TypeScript type
- add a built-in agent seed entry for OpenCode in the agent preferences store
- include OpenCode in built-in runtime-agent metadata generation
- include OpenCode in built-in source registration

Runtime-control behavior change:

- runtime UI and preference update paths should be able to decide per agent whether `permissionMode` is shown or persisted

No new transport kind is needed.

No new top-level Tauri command is required specifically for OpenCode if the current runtime/session commands remain agent-parameterized.

## Test Plan

### Rust tests

- `Agent::as_str` and `Agent::from_db_str` support `opencode`
- built-in agent seeding writes the expected OpenCode defaults
- runtime metadata generation includes OpenCode when enabled
- OpenCode source parser can:
  - read a temporary SQLite database with `session`, `message`, and `part`
  - read legacy storage files
  - deduplicate overlapping SQLite and legacy sessions
  - preserve title, project directory, timestamps, and summary correctly
- OpenCode source path classification produces appropriate reindex tasks for:
  - database changes
  - legacy storage changes

### Runtime tests

- OpenCode runtime command resolution returns `opencode acp`
- same-agent continuation still chooses `Resume`, not `Load`
- OpenCode initial session config does not send Sessio `permissionMode` as ACP `mode`

### Frontend tests

- OpenCode renders correctly in agent icon and agent selection UI
- runtime-agent lists include OpenCode when enabled
- composer/chat runtime controls hide or disable permission-mode selection for OpenCode

### Manual validation

- with OpenCode installed and logged in:
  - start a new OpenCode chat from Sessio
  - send follow-up prompts
  - continue the same session and confirm runtime uses resume semantics
- with existing local OpenCode history:
  - confirm Sessio indexes and opens historical sessions
- with missing login:
  - confirm the UI shows an actionable `opencode auth login` instruction

## Assumptions and Defaults

- v1 is desktop-first and does not add dedicated IM bridge or scheduled-task product work
- same-agent OpenCode continuation should use ACP `resumeSession`
- transcript browsing should use local source parsing, not ACP replay
- OpenCode local storage is SQLite-first with legacy JSON compatibility
- OpenCode `mode` is not treated as Sessio `permissionMode`
- OpenCode should be seeded disabled by default until users explicitly enable it


opencode 项目 [anomalyco/opencode](https://github.com/anomalyco/opencode)，
关于acp的使用 https://opencode.ai/docs/acp/
关于 session 的解析 opencode.rs [opencode.rs](https://github.com/farion1231/cc-switch/blob/main/src-tauri/src/session_manager/providers/opencode.rs)
