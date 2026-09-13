---
name: create-sessio-app
description: >-
  Create a self-contained Sessio web app from a natural-language product or
  data-visualization request, using plain HTML or a static production build
  from React, Vue, Svelte, Vite, or another frontend framework. Use this skill
  whenever the user asks to generate an HTML dashboard, report, chart, table,
  data tool, or small offline app for Sessio, especially when the data should be
  easy to replace or regenerate. Keep application behavior, replaceable
  runtime data, local assets, and documentation in a clear package contract.
---

# Create Sessio App

Create small, inspectable web applications that work in a browser and preview
safely in Sessio. The application may be hand-written HTML/CSS/JavaScript or a
production build from React, Vue, Svelte, or another framework. The central
contract is data separation:

Compatibility: a static production build loaded by Sessio's sandboxed iframe.
Sessio supplies a scoped `sessio-app://` base for App-local resources, so the
installed App does not need a development server or network access. A browser
fallback is validated by serving the same output from a local static HTTP
server.

```text
<app-dir>/
  web/
    <app-slug>.html     # UI, styles, rendering, and interaction logic
    assets/              # optional framework/bundler output and local assets
    <app-slug>-data.js  # the only runtime data source
    <app-slug>-migrations.js # optional schema migration and validation logic
    logo.<ext>          # optional app logo or brand asset
    screenshot.<ext>   # optional app screenshot
    config.json         # app metadata for Sessio and other agents
  AGENTS.md             # purpose, usage, data usage, and the complete contract
```

Use an existing directory when the user names one. Otherwise create
`apps/<app-slug>/` for a new app, unless the repository's local convention says
otherwise. Use lowercase ASCII kebab-case for `<app-slug>` and do not overwrite
existing files without explicit permission.

## Framework builds

React, Vue, Svelte, and other frameworks are supported after they are compiled
to static files. Do not ship a Vite or other development server as the App
runtime. The published `web/` directory must contain the generated HTML entry
and every generated JavaScript, CSS, font, image, model, and media file it
references.

For Vite projects, use a relative base and keep the production output in the
App web directory (or copy it there during packaging):

```js
// vite.config.js
import { defineConfig } from "vite";

export default defineConfig({
  base: "./",
  build: {
    outDir: "dist/web",
    assetsDir: "assets",
  },
});
```

The same `base: "./"` rule applies to React, Vue, and Svelte Vite templates.
For Vue CLI or another bundler, configure its equivalent relative public path.
Do not leave emitted URLs such as `/assets/main.js`, `/models/book.glb`, or
`https://cdn.example/...` when the App is expected to work offline in Sessio.
If the output contains more than one HTML file, name the intended entry
`web/<app-slug>.html`; Sessio otherwise selects the only HTML file or the file
whose name matches the App slug.

Framework routing must also work without a server-side history fallback. Use
hash routing or keep navigation inside the single HTML entry unless the App
provides its own fallback files. Dynamic imports and code-split chunks are
supported when their generated URLs remain relative to the entry document.

Keep the data separation contract below for data-driven Apps. A framework may
render the data, but a replaceable `web/<app-slug>-data.js` must remain a
separate script rather than being bundled into the view when later agents need
to update it without rebuilding the UI.

### Framework data integration

Keep the replaceable data file outside the framework module graph. With Vite,
place the source file at `public/<app-slug>-data.js`; Vite copies it unchanged
to the root of the production output. Add a classic script tag to the HTML
entry before the framework module entry:

```html
<script src="./<app-slug>-data.js"></script>
<script type="module" src="./src/main.tsx"></script>
```

Vite rewrites the module entry during the build while preserving the data
script as a separate file. React, Vue, and Svelte code can then read the
already-initialized global without importing the file:

```ts
type AppData = {
  schemaVersion: number;
  records: Array<Record<string, unknown>>;
};

declare global {
  interface Window {
    SESSIO_APP_DATA?: AppData;
  }
}

const data = window.SESSIO_APP_DATA;
```

Use the app-specific global name documented in AGENTS.md when it differs from
`window.SESSIO_APP_DATA`. Do not put the data file under `src/`, import it from
TypeScript/JavaScript, or replace it with a runtime `fetch()` if later updates
must avoid rebuilding the bundle. A direct browser load executes the classic
script before the deferred module entry; Sessio serves the same local data
script through the scoped App resource protocol. Keep the data assignment
JSON-compatible and free of rendering code.

## Required workflow

1. **Clarify the app contract from the request.** Identify the app purpose,
   audience, expected inputs, visual outputs, interactions, language, and
   destination directory. Make reasonable defaults when the request is clear;
   ask only when a missing choice changes the data model or user workflow.
2. **Inspect the repository.** Look for existing Sessio preview behavior, local
   style conventions, related files, and any existing framework/build setup.
   Reuse the repository's established React, Vue, Svelte, Vite, or plain HTML
   patterns. For a small app with no existing build, plain HTML remains the
   simplest option; do not introduce a framework only for Sessio compatibility.
3. **Plan optional visual assets.** If the request calls for a logo, generate
   or create a suitable local logo asset at `web/logo.<ext>` and reference it
   with a relative path from the HTML. If a screenshot is requested, capture a
   representative app view as `web/screenshot.<ext>` after browser validation.
   Keep both assets inside `web/`. If generation or capture is unavailable or
   fails, omit the optional asset and continue; do not block app delivery or
   invent a broken placeholder reference. If the App itself must let users
   export a screenshot at runtime, treat that as a separate feature and read
   [references/screenshot-export.md](references/screenshot-export.md). If it
   exports JSON, CSV, text, SVG, a save file, or any other data/file at runtime,
   read [references/data-export.md](references/data-export.md). Every runtime
   screenshot or file export must declare `"downloads"` in `web/config.json`,
   including exports that only use an ordinary browser download.
4. **Define app metadata.** Create `web/config.json` with these required
   fields: `nameZh` (Chinese name), `nameEn` (English name), `description`
   (app introduction), `category` (one fixed category), `topics` (zero or more
   custom topic labels), `author`, `email`, and `version`. `category` must be
   exactly one of `Life`, `Games`, `Finance`, `Medical`, `Office`, `Tools`,
   `Sports`, `Entertainment`, `News`, or `Other`. `topics` must be an array of
   unique non-empty strings chosen for the app and may be empty when the app has
   no secondary topics. Use a semantic version
   such as `1.0.0` for `version`. Add the optional `permissions` array only when
   the app needs a Sessio-supported browser capability. Currently the supported
   values are listed in the App permissions section below.
   Keep this metadata separate from runtime data; the HTML must not fetch or
   parse `config.json` unless the user explicitly requests that behavior.
5. **Define the data schema before writing the view.** Decide the exact top-level
   data envelope and every record field. Keep presentation metadata separate
   from records. Record units, requiredness, null behavior, allowed values, and
   an example for every field.
6. **Create the data file first.** Put only the intended initial runtime data in
   `web/<app-slug>-data.js`. Do not add fabricated examples, demo records, or
   personal-looking records merely to make the first screen look populated;
   initialize record collections as empty unless the user explicitly supplied
   and requested a real dataset to ship with the App. Non-personal defaults
   such as settings, labels, and an empty schema envelope are allowed. Put
   illustrative records in AGENTS.md, a fixture file, or test data instead.
   The file must assign one global value containing a JSON-compatible object
   and contain no rendering code, imports, network access, or executable
   migration logic:

   ```js
   window.SESSIO_APP_DATA = {
     schemaVersion: 1,
     meta: { title: "Example" },
     records: []
   };
   ```

   `schemaVersion` is part of the data contract. If the data structure changes
   in any way, including adding, removing, renaming, nesting, or retyping a
   field, changing units or enum values, or changing requiredness, increment it
   and document the migration in AGENTS.md. The global name may be app-specific
   when needed, but the HTML and AGENTS.md
   must state it exactly. Never duplicate records, labels derived from records,
   or default sample rows inside the HTML. Keep this as `.js` rather than
   `.json`: the same-directory script works in the browser fallback and can be
   loaded as a classic local script by Sessio's App resource protocol without a
   runtime fetch. A separate `.json` file
   would require an asynchronous fetch and would not work when the output is
   opened from a browser `file://` URL. The assigned value should remain
   JSON-compatible so agents can parse and migrate it as data.
7. **Create or build the view.** For a plain App, put the entry at
   `web/<app-slug>.html` and reference the data file with a same-directory
   relative script tag such as `<script src="./<app-slug>-data.js"></script>`.
   For React, Vue, Svelte, or another framework, run its production build and
   copy the generated entry HTML and complete asset tree into `web/`. Read the
   global data object after its data script and render the empty state when it
   is missing or invalid. Do not make the runtime depend on Vite, a development
   server, CDN assets, npm imports, or a backend.
8. **Build for Sessio preview.** The view must work with App scripts enabled.
   Plain HTML previews may inline local scripts from a normal local file. App
   previews keep compiled scripts and chunks as relative URLs and load them from
   the scoped App resource protocol. Sessio injects the App resource base
   described below before loading the document. Do not
   depend on `document.currentScript` or on an original absolute script URL
   after packaging. Follow the Sessio theme contract below so the app uses the
   current light or dark chat background and updates without reloading when the
   Sessio theme changes.
   When the App has meaningful mutable state that should survive switching to
   another App, Project Chat, Settings, or another Sessio view, read
   [references/state-persistence.md](references/state-persistence.md) and expose
   the state adapter described there.
9. **Write AGENTS.md from the implemented contract.** Explain what the app is
   for, how to open it in a browser and Sessio, the `web/` layout, config metadata, the exact
   data global, the complete schema, a valid data example, how to replace or
   regenerate data, how the app uses and transforms the data, whether data is
   sample, user-supplied, or derived, whether it leaves the local app, and
   theme behavior, and known preview/security limitations. AGENTS.md must be
   updated whenever the schema, data usage, or theme behavior changes.
10. **Validate before handing off.** Check that the HTML references the data JS,
   the data JS parses, the HTML contains no record literals, and AGENTS.md
   documents every top-level and record field, and that `web/config.json`
   contains all required metadata fields. For a framework App, run the
   production build, verify that the entry and complete asset tree are under
   `web/`, and scan emitted URLs for root-absolute paths or undeclared remote
   dependencies. Exercise the initial render and at least one requested
   interaction. Check a desktop width and a narrow width when the app has a
   visual layout. Use the browser test method below for visual and interaction
   verification. Do not start a persistent development server; if temporary
   serving is essential for verification, stop it before finishing.
11. **Publish the tested app to Sessio's app directory.** Only after all checks
   pass, read the absolute `SESSIO_APP_HOME` environment variable supplied by
   running Sessio process. Invoke the bundled publisher for the current shell:
   use `scripts/publish_app.sh <source-dir> <app-slug>` on macOS/Linux (or Git
   Bash/WSL), and `scripts/publish_app.ps1 <source-dir> <app-slug>` on native
   Windows PowerShell. Both scripts copy the complete source app directory to
   `$SESSIO_APP_HOME/apps/<app-slug>/`. They refuse to update an existing
   destination unless the user explicitly requests `--update`/`-Update`.
   Update publishing merges recursively while protecting the installed
   `web/<app-slug>-data.js` when it already exists: this file may contain data
   written by Sessio chat or by the App at runtime, so an ordinary update must
   never replace it with the source copy. When the source contains
   `web/<app-slug>-migrations.js`, an ordinary `--update` stages the source data
   as `web/<app-slug>-data.pending.js` beside the protected data file so the App
   can validate and migrate it at startup. `--update-data` (or `-UpdateData` on
   PowerShell) bypasses staging and replaces the installed data only after you
   have deliberately migrated and reviewed it into the new source schema; the
   publisher does not guess how arbitrary application data should be merged.
   Other source paths replace matching destination paths. The publisher records
   source-managed paths in a hidden `.sessio-publish-manifest`; on later
   `--update` runs it removes only previously managed files that disappeared
   from the source, while destination-only runtime files such as screenshots,
   exports, and saved files remain in place. Existing installations without a
   manifest keep unknown destination-only files and begin tracking paths after
   the next update. If a matching path changes between a file and a directory,
   the source type wins and that conflicting destination path is replaced.
   App Store upgrades use the same reconciliation rule.
   The publisher is an execution step, not a completion message: run it after
   validation and then verify that the destination contains `web/<app-slug>.html`,
   `web/<app-slug>-data.js`, `web/config.json`, AGENTS.md, CLAUDE.md, and, when
   schema migration is required, `web/<app-slug>-migrations.js`, plus any
   optional assets or child directories. The publisher creates
   `CLAUDE.md` as an independent copy of `AGENTS.md` when the source contains
   AGENTS.md, so Claude can discover the same instructions without symlinks.
   Resolve `scripts/` relative to the
   directory containing the loaded `SKILL.md` when invoking the bundled file.
   If `SESSIO_APP_HOME` is missing, do not guess a profile or write to a
   hard-coded home directory; report that publishing is blocked. AGENTS.md
   should record the installed path and the source/development path.

## App-local resources

Sessio grants each App iframe a tokenized resource base similar to
`sessio-app://localhost/<grant-token>/`. The host injects this base into the
preview document and restricts every request to that App's installed `web/`
directory. Relative URLs in HTML, CSS, JavaScript, dynamic imports, and
framework-generated chunks therefore resolve against the App package:

```js
const modelUrl = new URL("assets/models/book.glb", document.baseURI).href;
const imageUrl = new URL("assets/images/cover.webp", document.baseURI).href;
```

Prefer relative URLs such as `./assets/main.js`, `assets/cover.webp`, and
`assets/audio/intro.mp3`. Do not use root-absolute paths such as `/assets/...`
or filesystem paths. Vite projects should use `base: "./"`; other bundlers
must use their equivalent relative public path. Do not manually inline local
images or audio as Base64/data URLs just to make Sessio work. The old preview
workaround that scanned JavaScript for mp3/wav/ogg references and converted
them to data URLs is no longer part of the contract.

The local protocol currently serves these resource classes: HTML, CSS,
JavaScript/modules, JSON, PNG/JPEG/WebP/GIF/SVG images, GLB/GLTF/BIN/WASM
assets, WOFF/WOFF2/TTF fonts, and MP3/WAV/OGG/M4A/AAC/WEBM audio. Individual
resources are limited to 64 MiB. Unknown extensions, missing files, path
traversal, symlinks outside `web/`, and requests outside the current App are
rejected.

The App entry HTML is also limited to 64 MiB. Compiled App scripts are kept as
resource URLs and are not copied through Sessio's small local-text preview
buffer, so Vite bundles and code-split chunks use the same 64 MiB per-resource
limit.

If an image is loaded with `Image` and then drawn into a Canvas or uploaded as
a Three.js texture, set `image.crossOrigin = "anonymous"` before assigning
`image.src`. The protocol supplies the corresponding non-credentialed CORS
response. This does not grant access to another App or to the filesystem.

Sessio App CSP permits App-local `sessio-app:` resources for images, media,
styles, fonts, scripts, and connections. It does not permit arbitrary remote
`http://` or `https://` images, audio, scripts, modules, or API requests; the
only built-in remote exceptions are the existing Google Fonts sources. Do not
make an App depend on CDN code, remote audio, remote images, or a remote API.
If a browser-only enhancement uses a remote URL, provide a local/offline
fallback and document that it is unavailable in Sessio.

The browser fallback uses the same relative URLs when the output is served by
a static HTTP server. Direct `file://` opening is not a reliable validation
path for framework ES modules, GLB files, or runtime fetches; use a loopback
server for browser verification.

## Data separation rules

- Do not put example or personal records in `web/<app-slug>-data.js`. Initial
  installation must not appear to contain a user's real data. This includes
  people and family records such as those formerly shown in
  `family-tree-data.js`, and medical or case records such as those formerly
  shown in `case-report-trends-data.js`. Put examples in `AGENTS.md`, a separate
  fixture file, or test data instead. Empty arrays, schema envelopes, and
  non-personal default settings are allowed; for example, the board settings,
  difficulty defaults, and player labels in `gomoku-bot-data.js` are valid
  initial values.
- If an App needs user records, initialize the record collection as empty and
  create records only from explicit user input or a documented local import.
  Keep any sample-data toggle separate from the production data file and make
  it impossible for sample records to be mistaken for user-owned records.
- The data JS is the single source of truth for all runtime rows, categories,
  series, labels, units, thresholds, and user-configurable values.
- The HTML may contain structural labels such as “No data” or column headings,
  but must not contain a copy of the supplied/sample records or data-specific
  constants that should change with the data file.
- Keep derived values in the view logic when they are deterministic from the
  data. If a derived value is expensive or intentionally curated, put it in the
  data file and document it in the schema.
- Keep schema versioning explicit. Any data-structure change increments
  `schemaVersion` and updates the AGENTS.md migration note. This includes
  field additions and removals, renames, nesting changes, type or unit changes,
  enum changes, and required/optional changes. A data-only value correction
  that keeps the documented structure may keep the same version.
- Keep restorable interaction state separate from the data JS. The data JS
  remains the initial data source; a Sessio state snapshot contains only user
  changes and the minimal UI state needed to reconstruct the current view.
- Treat data as untrusted input: validate types, escape text through DOM APIs,
  and avoid evaluating strings as code. Do not put secrets or personal data in
  sample files.
- If the user supplies CSV or JSON, convert it once into the data JS contract;
  do not make the HTML fetch or parse a second copy at runtime unless the user
  explicitly requests an importer.

## AGENTS.md data contract

Use this structure unless the user requests another documentation language:

```markdown
# App title

## Purpose
What the app shows and who uses it.

## Files
- `web/<app-slug>.html`: plain or framework-generated view entry.
- `web/assets/` (optional): framework/bundler output and other local runtime
  resources referenced by the entry.
- `web/<app-slug>-data.js`: only runtime data, exported as `window.<GLOBAL>`.
- `web/<app-slug>-migrations.js`: optional non-mutating schema migration and
  validation functions used when `schemaVersion` changes.
- `web/config.json`: required app metadata with `nameZh`, `nameEn`,
  `description`, `category`, `topics`, `author`, `email`, and `version`, plus
  optional permissions.
- `web/screenshot.<ext>`: optional screenshot for the app listing or documentation.
- `AGENTS.md`: this contract, data-usage explanation, and maintenance notes.
- `web/logo.<ext>`: optional local logo asset named `logo` with a supported image
  extension such as `.png`, `.jpg`, `.webp`, or `.svg`; omit it when no logo was
  requested or generation was unavailable.

## App metadata
`web/config.json` must be valid JSON with this shape:

```json
{
  "nameZh": "示例应用",
  "nameEn": "Example App",
  "description": "应用介绍。",
  "category": "Tools",
  "topics": ["planning", "notes"],
  "author": "作者姓名",
  "email": "author@example.com",
  "version": "1.0.0",
  "permissions": ["pointerLock"]
}
```

The eight scalar/list metadata fields are required: `nameZh`, `nameEn`,
`description`, `category`, `topics`, `author`, `email`, and `version`.
`nameZh`, `nameEn`, `description`, `author`, `email`, and `version` must be
non-empty strings. `category` accepts exactly one value from this fixed English
list: `Life`, `Games`, `Finance`, `Medical`, `Office`, `Tools`, `Sports`,
`Entertainment`, `News`, `Other`. Do not encode multiple categories as a
comma-separated string or array. `topics` is a JSON array of unique non-empty
custom strings; it may use Chinese, English, or other language labels, and
should be `[]` when there are no topics.
`permissions` is optional and
must contain only capability names supported by Sessio. Omit it or use an empty
array when the app needs no extra browser capability. Keep `description`
concise and factual.
This file describes the app for Sessio and agents; it is metadata, not runtime
data, and should not be duplicated in `<app-slug>-data.js`.

## App permissions

Treat `permissions` as a least-privilege capability request. Never place raw
iframe `allow` values, sandbox tokens, HTML, or browser policy strings in
`config.json`. Sessio maps each recognized name to its reviewed iframe policy
and ignores unknown names.

Runtime screenshots and exported data/files always require the `downloads`
permission and must implement both export capabilities: `<a download>`/Blob
download and the Sessio `postMessage` file-write bridge. They use a defined
fallback order, so one user action does not create two copies. From an App
embedded in Sessio, generate one immutable snapshot, try the bridge first, and
start the browser download only when the bridge is unavailable, times out, or
returns failure. If the bridge succeeds, do not trigger a second download. A
direct-browser page has no Sessio bridge recipient, so it uses the browser
download path. Importing a local file with a user-triggered `<input
type="file">` and `FileReader` does not by itself require `downloads`.

| Config value | Use when | iframe `allow` | Sandbox token |
|---|---|---|---|
| `autoplay` | Audio or video must start without a fresh user gesture | `autoplay` | None |
| `clipboardWrite` | A user action copies generated content | `clipboard-write` | None |
| `downloads` | A user action downloads content or saves generated data into the App's `web/` directory through the Sessio file-write bridge | None | `allow-downloads` |
| `fullscreen` | A user action opens the app or canvas fullscreen | `fullscreen` | None |
| `gamepad` | The app reads a connected game controller | `gamepad` | None |
| `modals` | The app uses `alert()`, `confirm()`, `prompt()`, or `beforeunload` | None | `allow-modals` |
| `pointerLock` | The primary interaction captures pointer movement, such as a first-person or 3D canvas | Not used | `allow-pointer-lock` |
| `popups` | A user action opens a separate browser page | None | `allow-popups` |

These capabilities only remove the corresponding iframe restriction. Browser
support, user-activation requirements, operating-system policy, and Tauri/Wry
backend limitations still apply. `popups` keeps new pages sandboxed; do not use
`allow-popups-to-escape-sandbox`.

For pointer lock, request the capability only when the app actually calls
`Element.requestPointerLock()`. Provide an explicit user action to enter pointer
lock, a visible way to exit, and a useful fallback when the browser denies the
request. Document requested capabilities, why they are needed, and fallback
behavior in AGENTS.md. Test the capability inside Sessio because direct-browser
behavior does not verify Sessio's iframe sandbox or embedding engine.

Pointer Lock is governed by transient user activation and the iframe sandboxed
pointer-lock flag; it is not a Permissions Policy-controlled feature. Do not add
`allow="pointer-lock"` or rely on a parent `Permissions-Policy` header for it.
The `allow-pointer-lock` sandbox token removes the iframe restriction but cannot
add support to an embedding engine. In particular, Pointer Lock remains broken
in Tauri/Wry's macOS WKWebView backend. Apps that request `pointerLock` must also
support a usable unlocked interaction such as click-and-drag camera movement.

Do not request camera, microphone, geolocation, display capture, clipboard read,
same-origin access, or arbitrary network access through this array. Those
capabilities expose sensitive data, weaken the isolation boundary, or require
CSP and native host changes. They need a dedicated Sessio bridge before they
can be added to the allowlist.

### Exporting and saving files

For an App feature that captures a chart, board, canvas, diagram, or other
rendered view as an image, read
[references/screenshot-export.md](references/screenshot-export.md). It explains
the Sessio sandbox constraint, export rendering choices, PNG generation,
browser fallback, and an equivalent sandbox test.

For JSON, CSV, text, SVG, game saves, or other generated files, read
[references/data-export.md](references/data-export.md). It covers format and
schema choices, Blob downloads, the Sessio file-write bridge, importing files,
AGENTS.md requirements, and validation. Both references require `"downloads"`
in `web/config.json` and require both export capabilities with the bridge-first
fallback order for every runtime export.

The `downloads` permission also authorizes the App to ask Sessio to write a file
inside that App's `web/` directory. Add `"downloads"` explicitly to
`web/config.json` before using this bridge. Sessio ignores file-write messages
when the permission is absent, and its native backend reads `config.json` again
before every write. The page cannot choose another App or write outside its own
`web/` directory.

Send this request from the App iframe:

```js
window.parent.postMessage({
  source: "sessio-app",
  type: "sessio-app-write-file",
  requestId: "save-1",
  path: "exports/state.json",
  data: JSON.stringify(window.SESSIO_APP_DATA, null, 2),
  encoding: "utf8",
  overwrite: false
}, "*");
```

`requestId` must be a non-empty string of at most 128 characters and should be
unique among outstanding requests. `path` is relative to `web/`. Use only
ordinary, visible path segments; absolute paths, `.`/`..`, hidden segments,
backslashes, colons, empty segments, symbolic-link traversal, `config.json`, and
platform-reserved file names are rejected. Child directories are created as
needed. Existing files are preserved unless `overwrite` is exactly `true`.

`encoding` may be `"utf8"` (the default) or `"base64"`. For binary content,
send only the Base64 payload without a `data:` URL prefix. The decoded file may
not exceed 25 MiB. Sessio responds to the same iframe with one of these message
shapes:

```js
// Success
{
  source: "sessio",
  type: "sessio-app-write-file-result",
  requestId: "save-1",
  ok: true,
  relativePath: "exports/state.json",
  bytesWritten: 123
}

// Failure
{
  source: "sessio",
  type: "sessio-app-write-file-result",
  requestId: "save-1",
  ok: false,
  error: "App file already exists; set overwrite to true to replace it"
}
```

Listen for `message` events, require `event.source === window.parent`, and match
both the result `type` and `requestId` before updating the UI. Show a visible
success or failure state. A page opened directly in a browser has no Sessio
parent bridge, so keep ordinary browser downloads as a fallback when the app's
workflow requires export outside Sessio.

Document every file the App can generate in AGENTS.md, including its relative
path or naming rule, format, data source, maximum expected size, whether a later
save may set `overwrite: true`, and the user action that starts the write.

## Run and preview

For a plain App, open the generated HTML through a loopback static server. For
a framework App, build first and serve the build directory, not the framework's
development server:

```bash
npm run build
python3 -m http.server 8765 --bind 127.0.0.1 --directory dist/web
```

Open `http://127.0.0.1:8765/<app-slug>.html` and verify that the generated JS,
CSS, fonts, images, models, and audio all load. Then publish the same `web/`
output and open the App in Sessio. Sessio enables App scripts, injects the
scoped `sessio-app://` base, and serves local assets through the App resource
protocol; it does not use the framework development server. Stop temporary
servers after verification. Direct `file://` opening is useful only for simple
non-module documents and is not a complete validation of a Vite build.

## Data structure
### Root object: `window.<GLOBAL>`
| Field | Type | Required | Description | Example |
...

### Record object: `records[]`
| Field | Type | Required | Description | Example |
...

## Example data
```js
...
```

## Updating data
Edit or regenerate only `web/<app-slug>-data.js`; preserve the documented schema.
The installed copy under `$SESSIO_APP_HOME/apps/<app-slug>/web/` is runtime
state and is protected by the publisher during ordinary `--update` publishing.
Do not assume that regenerating the source copy updates installed user data.
Use `--update-data`/`-UpdateData` only for an intentional data reset or a
reviewed source-data migration, after confirming that replacing user changes is
acceptable. The flag publishes the already-merged source file; it is not an
automatic deep merge.

When a schema-compatible view update is needed, use plain `--update` and keep
the installed data file. When the schema changes, merge the installed runtime
data before publishing:

1. Read and parse both the installed
   `$SESSIO_APP_HOME/apps/<app-slug>/web/<app-slug>-data.js` and the new source
   data file. Do not merge JavaScript as text.
2. Apply the migration documented in AGENTS.md. Preserve user values by the
   documented stable record identifier, map renamed fields explicitly, convert
   units and types explicitly, add documented defaults for new fields, and drop
   removed fields. Never merge records by array position or silently keep
   unknown fields. If the schema change is ambiguous or lossy, stop and report
   it for review.
3. Validate the merged envelope and set the new `schemaVersion`. Write that
   validated merged object into the source `web/<app-slug>-data.js`, then update
   AGENTS.md with the migration and any intentional data loss.
4. Run `publish_app.sh <source-dir> <app-slug> --update --update-data` or
   `publish_app.ps1 <source-dir> <app-slug> -Update -UpdateData`. This replaces
   the installed file with the reviewed merged result. Keep a copy of the
   previous installed data until the new App has been validated.

The publisher cannot safely infer field identity, deletion, conflict precedence,
or unit conversion from arbitrary JavaScript. A plain `--update-data` therefore
must never be presented as a data-preserving merge; the migration step is part
of the App's AGENTS.md contract and the agent's update task.

### Store upgrade migrations

The Sessio App Store uses a two-phase data upgrade for installed Apps. An App
that changes its data schema must ship both the new `web/<app-slug>-data.js`
and an App-specific `web/<app-slug>-migrations.js` module. During an upgrade,
Sessio keeps the installed data file and stages the new file as
`web/<app-slug>-data.pending.js`. The pending file is a candidate, not runtime
state; it is created only when the migration module is present.

The App's startup code must capture the installed data object, load the pending
candidate and migration module, then:

1. Compare the old and pending `schemaVersion` values. If they are equal, keep
   the installed object and do not migrate.
2. Apply an explicit migration chain such as `1 -> 2 -> 3`. Match records by
   documented stable identifiers, map renamed fields, convert units and types,
   add documented defaults, and deliberately remove fields that no longer
   exist. Do not merge by array position or copy unknown fields.
3. Validate the complete result against the target schema before writing it.
   If the version is unsupported, a required field is ambiguous, validation
   fails, or the migration would lose data without a documented policy, leave
   the installed data untouched and show the migration error.
4. Write the validated envelope to `web/<app-slug>-data.js` through the Sessio
   file-write bridge with `overwrite: true`, then use the merged object for the
   current session. Keep the pending file until a later package update replaces
   it; the startup check must be idempotent once both versions match.

`<app-slug>-migrations.js` must export the migration and validation functions;
it must not mutate the old object in place. Document every source and target
version, stable key, field mapping, default, conversion, rejected case, and
intentional data loss in `AGENTS.md`. The host only stages files and cannot run
arbitrary migration JavaScript, so an App Store upgrade without this module
must preserve the old data file and must not claim that data migration occurred.

### Schema version and migration rules

`schemaVersion` is a positive integer stored in the root data envelope. Every
change to the documented data structure must increment it. Keep the same
version only for changing values while field names, nesting, types, units,
allowed values, and requiredness remain unchanged. AGENTS.md must describe the
source version, target version, field mapping, defaults, conversions, rejected
cases, and any intentional data loss for each migration. The App must reject an
unsupported or invalid version instead of guessing.
When the user supplies an image, document, or plain text, compare its content
with the schema in this `AGENTS.md` before changing data. Decide whether it
contains values that map to required or optional fields, and extract only the
matching values needed to update the data JS. Do not copy unrelated prose,
layout text, captions, or decorative content into runtime data. For images and
documents, use appropriate local extraction or OCR when available; if the
content cannot be reliably mapped, leave the data JS unchanged and explain what
could not be extracted. Validate types, units, dates, enum values, required
fields, and null handling after every update, then update the data-usage notes
or schema example when the source or interpretation changes.

## Data usage
Describe the data source and whether it is sample, user-supplied, imported, or
derived. Explain which fields the view reads, how values are transformed or
aggregated, where the data is displayed, and whether any data is persisted or
sent outside the local app. State that offline use performs no network transfer.
Identify sensitive or personal data and keep secrets and real personal data out
of sample files. Document how user-provided images, documents, and text are
evaluated against the schema, which extraction or OCR method is used when
needed, and what happens when content does not map reliably to a field. If the
App uses the Sessio file-write bridge, describe the generated files and make it
clear that they remain within the local App's `web/` directory.

## Installed location
The validated copy is installed at `$SESSIO_APP_HOME/apps/<app-slug>/`, where
`SESSIO_APP_HOME` is the absolute profile directory supplied by Sessio. Keep the
source copy for development, but update the installed data file too when
distributing a new data snapshot outside the source repository. `SESSIO_APP_VARIANT`
may be `dev` or `prod` for diagnostics, but it is not a substitute for the
resolved absolute app-home path.

## Limitations and validation
Offline/CSP behavior, empty-state behavior, and the checks run before delivery.

The tables must cover every field the HTML reads, including optional metadata,
enum values, units, date formats, reference ranges, and nullable fields. Do not
document fields that the implementation silently ignores.

## UI and interaction defaults

Start with the requested visual instead of a marketing landing page. Use
semantic HTML and native controls. For charts, use responsive SVG for simple
plots or an already-installed library when the data genuinely requires it;
avoid network-loaded dependencies. Include accessible names, labeled axes,
legends only when needed, keyboard-accessible controls, and an informative
empty state. Keep tables readable at narrow widths and allow horizontal scroll
only when columns cannot fit. Avoid invented KPI cards, decorative filler, and
server-only features.

## Sessio theme contract

Sessio sets `data-sessio-theme="light"` or `data-sessio-theme="dark"` on the
HTML root and injects `--sessio-chat-background` with the matching chat canvas
color:

| Theme | Chat background |
|---|---|
| Light | `#f6f6f4` (`rgb(246 246 244)`) |
| Dark | `#232831` (`rgb(35 40 49)`) |

Use `--sessio-chat-background` as the page canvas color and derive surfaces,
borders, text, muted text, and data colors with sufficient contrast for that
base. Do not place an unrelated fixed page background over it. Define a direct
browser fallback because the injected variable and attribute exist only in
Sessio:

```css
:root {
  color-scheme: light;
  --app-bg: var(--sessio-chat-background, #f6f6f4);
  --app-fg: #1f232b;
  --app-muted: #64748b;
  --app-surface: #fcfcfa;
  --app-border: rgba(31, 35, 43, 0.12);
}

:root[data-sessio-theme="dark"] {
  color-scheme: dark;
  --app-fg: #dae0ea;
  --app-muted: #94a3b8;
  --app-surface: #2b313b;
  --app-border: rgba(218, 224, 234, 0.12);
}

@media (prefers-color-scheme: dark) {
  :root:not([data-sessio-theme]) {
    color-scheme: dark;
    --app-bg: #232831;
    --app-fg: #dae0ea;
    --app-muted: #94a3b8;
    --app-surface: #2b313b;
    --app-border: rgba(218, 224, 234, 0.12);
  }
}

html,
body {
  background: var(--app-bg);
  color: var(--app-fg);
}
```

Sessio updates the root attribute, `color-scheme`, and background variable when
its theme changes. CSS-based views update automatically. A Canvas or a chart
library that stores colors in JavaScript must redraw on this event:

```js
window.addEventListener("sessio:themechange", (event) => {
  const { theme, chatBackground } = event.detail;
  renderChart({ theme, background: chatBackground });
});
```

Do not use `prefers-color-scheme` as the primary signal inside Sessio because
the user may choose a Sessio theme that differs from the operating system.
Document the theme variables and any JavaScript redraw behavior in AGENTS.md.
During browser validation, test both root attribute values and the standalone
browser fallback.

## Browser effect testing

When visual layout or pointer interaction matters, test the app in a real
browser after static checks. Browser automation may reject `file://` URLs, so
serve the repository temporarily from loopback and stop the server after the
test:

```bash
python3 -m http.server 8765 --bind 127.0.0.1
```

For a framework build, serve its output directory instead, for example:

```bash
python3 -m http.server 8765 --bind 127.0.0.1 --directory dist/web
```

Open the app in Chrome or another available browser automation surface at the
corresponding URL, such as
`http://127.0.0.1:8765/<relative-app-path>/<app-slug>.html` for a source app or
`http://127.0.0.1:8765/<app-slug>.html` for `dist/web`. Read the
accessibility tree to confirm the page loaded, expected controls and content
exist, and controls have useful accessible names. Exercise the primary workflow
by clicking controls through their accessibility ids or semantic roles, then
read the tree again to verify state changes, rendered records, empty states,
and status text. Capture a screenshot after the initial render and after the
key interaction to inspect clipping, overlap, asset loading, and responsive
layout.

For positioned visuals such as grids, charts, or markers, use Playwright in the
browser context to compare actual element bounding boxes with their intended
coordinates. Measure the center of each rendered marker against the calculated
plot or grid point and report the maximum pixel deviation. Also perform at
least one click using calculated screen coordinates rather than an element id,
then verify the resulting record or state in the accessibility tree. Treat
small subpixel differences as rounding; investigate visible misalignment or
larger systematic offsets. Record the browser, URL, viewport sizes, actions,
observed results, and any limitations in the handoff notes.

When the user asks for a dashboard or report but does not provide data, keep the
production data JS record collections empty and render a clear empty state.
Document any illustrative records in AGENTS.md or a separate fixture instead;
never make fabricated values look like user-owned data or infer sensitive
conclusions from them.

## Handoff checklist

Before reporting completion, verify:

- [ ] Exactly one selected HTML entry, one data JS file for data-driven Apps,
      one `config.json`, and one `AGENTS.md` exist for the app. Framework
      builds may include any number of generated files under `web/assets/` and
      other documented local asset directories. Include
      `web/<app-slug>-migrations.js` when a schema migration is required, and
      include `web/logo.<ext>` or
      `web/screenshot.<ext>` when requested and successfully generated, and
      omit each optional asset cleanly when unavailable.
- [ ] The HTML references `web/<app-slug>-data.js` by a same-directory relative
      path when the App uses the data separation contract; framework code reads
      the classic script's global and does not import or bundle that file.
- [ ] A framework build uses a relative public base, includes its complete
      production asset tree under `web/`, and contains no required root-absolute
      or remote runtime URLs.
- [ ] All runtime data is in the data JS; no duplicated rows are in the HTML.
- [ ] The initial data JS contains no fabricated, demo, or personal-looking
      records; record collections are empty unless the user explicitly supplied
      and requested the dataset, and non-personal default settings are clearly
      distinguished from user records.
- [ ] If `schemaVersion` changes, `web/<app-slug>-migrations.js` exports a
      non-mutating migration chain and validation functions, and AGENTS.md
      documents stable identifiers, field mappings, defaults, conversions,
      rejected cases, and intentional data loss. The pending candidate file is
      staged only for this migration flow and is never treated as runtime data
      before validation succeeds.
- [ ] AGENTS.md schema tables match the fields and types consumed by the HTML,
      and its data-usage/update rules match the implementation.
- [ ] User-provided images, documents, and text are checked against the schema
      before extracting values into the data JS.
- [ ] Missing/invalid data produces a visible, actionable empty/error state.
- [ ] Sessio can run the App offline without a server; browser fallback is
      verified by serving the production output from a loopback static HTTP
      server. Direct `file://` opening is not treated as sufficient for a
      framework/module build.
- [ ] An App with meaningful mutable state implements
      `references/state-persistence.md`; switching between Apps, Project Chat,
      Settings, and the App restores a validated snapshot without copying DOM,
      Canvas pixels, or unchanged data JS records into the cache.
- [ ] `permissions` is omitted unless the app needs a supported capability;
      each requested capability is documented in AGENTS.md and tested in Sessio.
- [ ] Every runtime screenshot or data/file export declares `downloads` and
      implements both ordinary Blob/`<a download>` download and Sessio
      file-write bridge capabilities from the same user action and snapshot;
      Sessio tries the bridge first and falls back to the browser download only
      on bridge failure, timeout, or absence. Bridge payloads stay below 25 MiB,
      and generated files and overwrite behavior are documented in AGENTS.md.
- [ ] Any runtime screenshot export follows
      `references/screenshot-export.md` and is tested without
      `allow-same-origin`.
- [ ] Any runtime JSON, CSV, text, SVG, save-file, or other data export follows
      `references/data-export.md`; exported content and imported files are
      validated against a documented, versioned format.
- [ ] The page background uses `--sessio-chat-background`; light mode uses
      `#f6f6f4`, dark mode uses `#232831`, and theme-dependent charts redraw
      after `sessio:themechange` when needed.
- [ ] A real-browser test covers initial render, the primary interaction, a
      screenshot review, and responsive viewport checks; positioned visuals
      have bounding-box alignment measured when applicable.
- [ ] After validation, the complete app directory is copied to
      `$SESSIO_APP_HOME/apps/<app-slug>/` using the bundled platform publisher;
      `--update`/`-Update` preserves the existing installed
      `web/<app-slug>-data.js` as well as destination-only runtime screenshots,
      exports, and saved files, and removes only stale source-managed paths
      recorded by `.sessio-publish-manifest`. Use `--update-data`/`-UpdateData` only with a
      reviewed, pre-merged data file after a documented schema migration; the
      publisher itself does not merge arbitrary JS data. When AGENTS.md exists,
      the destination also contains an independent CLAUDE.md copy.
- [ ] `web/config.json` is valid JSON and contains non-empty `nameZh`, `nameEn`,
      `description`, `author`, `email`, and `version` strings, plus a `topics`
      array of unique non-empty strings; `category` contains exactly one value
      from `Life`, `Games`, `Finance`, `Medical`, `Office`, `Tools`, `Sports`,
      `Entertainment`, `News`, `Other`, and `topics` may be empty. Its optional
      `permissions` array contains only supported capability names.
- [ ] HTML, JS, config, assets, and AGENTS.md paths are reported using absolute
      paths.
