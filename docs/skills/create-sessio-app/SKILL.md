---
name: create-sessio-app
description: >-
  Create a self-contained Sessio HTML app from a natural-language product or
  data-visualization request. Use this skill whenever the user asks to generate
  an HTML dashboard, report, chart, table, data tool, or small offline app for
  Sessio, especially when the data should be easy to replace or regenerate.
  Always keep application markup/behavior, runtime data, and documentation in
  separate files so later agents can update data without rewriting the view.
---

# Create Sessio App

Create small, inspectable HTML applications that open directly in a browser and
preview safely in Sessio. The central contract is data separation:

Compatibility: Sessio HTML preview with optional inline JavaScript enabled; no
server or network access required.

```text
<app-dir>/
  web/
    <app-slug>.html     # UI, styles, rendering, and interaction logic
    <app-slug>-data.js  # the only runtime data source
    logo.<ext>          # optional app logo or brand asset
    screenshot.<ext>   # optional app screenshot
    config.json         # app metadata for Sessio and other agents
  AGENTS.md             # purpose, usage, data usage, and the complete contract
```

Use an existing directory when the user names one. Otherwise create
`apps/<app-slug>/` for a new app, unless the repository's local convention says
otherwise. Use lowercase ASCII kebab-case for `<app-slug>` and do not overwrite
existing files without explicit permission.

## Required workflow

1. **Clarify the app contract from the request.** Identify the app purpose,
   audience, expected inputs, visual outputs, interactions, language, and
   destination directory. Make reasonable defaults when the request is clear;
   ask only when a missing choice changes the data model or user workflow.
2. **Inspect the repository.** Look for existing Sessio HTML preview behavior,
   local style conventions, and related files. Reuse existing project patterns
   rather than introducing a framework or build step for a small app.
3. **Plan optional visual assets.** If the request calls for a logo, generate
   or create a suitable local logo asset at `web/logo.<ext>` and reference it
   with a relative path from the HTML. If a screenshot is requested, capture a
   representative app view as `web/screenshot.<ext>` after browser validation.
   Keep both assets inside `web/`. If generation or capture is unavailable or
   fails, omit the optional asset and continue; do not block app delivery or
   invent a broken placeholder reference. If the App itself must let users
   export a screenshot at runtime, treat that as a separate feature and read
   [references/screenshot-export.md](references/screenshot-export.md).
4. **Define app metadata.** Create `web/config.json` with these required
   string fields: `nameZh` (Chinese name), `nameEn` (English name), `description`
   (app introduction), `author`, `email`, and `version`. Use a semantic version
   such as `1.0.0` for `version`. Add the optional `permissions` array only when
   the app needs a Sessio-supported browser capability. Currently the supported
   values are listed in the App permissions section below.
   Keep this metadata separate from runtime data; the HTML must not fetch or
   parse `config.json` unless the user explicitly requests that behavior.
5. **Define the data schema before writing the view.** Decide the exact top-level
   data envelope and every record field. Keep presentation metadata separate
   from records. Record units, requiredness, null behavior, allowed values, and
   an example for every field.
6. **Create the data file first.** Put all sample or supplied data in
   `web/<app-slug>-data.js`. It must assign one global value and contain no rendering
   code:

   ```js
   window.SESSIO_APP_DATA = {
     schemaVersion: 1,
     meta: { title: "Example" },
     records: []
   };
   ```

   The global name may be app-specific when needed, but the HTML and AGENTS.md
   must state it exactly. Never duplicate records, labels derived from records,
   or default sample rows inside the HTML.
7. **Create the HTML view.** Put it at `web/<app-slug>.html` and reference the
   data file with a same-directory relative script tag such as
   `<script src="./<app-slug>-data.js"></script>`. Read the global data
   object after that script and render the empty state when it is missing or
   invalid. Keep the page useful when opened as `file://`; do not require Vite,
   a local server, `fetch`, XHR, WebSocket, CDN assets, npm imports, or a
   backend. Inline CSS and JavaScript are preferred for portability.
8. **Build for Sessio preview.** The view must work after the user enables
   “Allow inline JavaScript”. Local scripts in the same directory and child
   directories are supported by Sessio's preview; resolve them relative to
   `web/`, using paths such as `./<app-slug>-data.js` or `scripts/data.js`. Do not depend on `document.currentScript`
   or the original external script URL after it is inlined. Follow the Sessio
   theme contract below so the app uses the current light or dark chat
   background and updates without reloading when the Sessio theme changes.
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
   contains all required metadata fields. Exercise the initial render and
   at least one requested interaction. Check a desktop width and a narrow width
   when the app has a visual layout. Use the browser test method below for
   visual and interaction verification. Do not start a persistent development
   server; if temporary serving is essential for verification, stop it before
   finishing.
11. **Publish the tested app to Sessio's app directory.** Only after all checks
   pass, read the absolute `SESSIO_APP_HOME` environment variable supplied by
   running Sessio process. Invoke the bundled publisher for the current shell:
   use `scripts/publish_app.sh <source-dir> <app-slug>` on macOS/Linux (or Git
   Bash/WSL), and `scripts/publish_app.ps1 <source-dir> <app-slug>` on native
   Windows PowerShell. Both scripts copy the complete source app directory to
   `$SESSIO_APP_HOME/apps/<app-slug>/`. They refuse to update an existing
   destination unless the user explicitly requests `--update`/`-Update`.
   Update publishing merges recursively: source paths replace matching
   destination paths, while destination-only files such as runtime screenshots
   and saved data remain in place. If a matching path changes between a file and
   a directory, the source type wins and that conflicting destination path is
   replaced. Update publishing is not a clean reinstall and does not remove
   stale destination-only package files.
   The publisher is an execution step, not a completion message: run it after
   validation and then verify that the destination contains `web/<app-slug>.html`,
   `web/<app-slug>-data.js`, `web/config.json`, AGENTS.md, CLAUDE.md, and any
   optional assets or child directories. The publisher creates
   `CLAUDE.md` as an independent copy of `AGENTS.md` when the source contains
   AGENTS.md, so Claude can discover the same instructions without symlinks.
   Resolve `scripts/` relative to the
   directory containing the loaded `SKILL.md` when invoking the bundled file.
   If `SESSIO_APP_HOME` is missing, do not guess a profile or write to a
   hard-coded home directory; report that publishing is blocked. AGENTS.md
   should record the installed path and the source/development path.

## Data separation rules

- The data JS is the single source of truth for all runtime rows, categories,
  series, labels, units, thresholds, and user-configurable values.
- The HTML may contain structural labels such as “No data” or column headings,
  but must not contain a copy of the supplied/sample records or data-specific
  constants that should change with the data file.
- Keep derived values in the view logic when they are deterministic from the
  data. If a derived value is expensive or intentionally curated, put it in the
  data file and document it in the schema.
- Keep schema versioning explicit. A breaking field change increments
  `schemaVersion` and updates the AGENTS.md migration note.
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
- `web/<app-slug>.html`: view and interaction logic.
- `web/<app-slug>-data.js`: only runtime data, exported as `window.<GLOBAL>`.
- `web/config.json`: required app metadata with `nameZh`, `nameEn`,
  `description`, `author`, `email`, and `version`, plus optional permissions.
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
  "author": "作者姓名",
  "email": "author@example.com",
  "version": "1.0.0",
  "permissions": ["pointerLock"]
}
```

All six metadata fields are required strings. `permissions` is optional and
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

### Saving files through Sessio

For an App feature that captures a chart, board, canvas, diagram, or other
rendered view as an image, read
[references/screenshot-export.md](references/screenshot-export.md). It explains
the Sessio sandbox constraint, export rendering choices, PNG generation,
browser fallback, and an equivalent sandbox test.

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
Browser steps, Sessio preview steps, and whether inline JavaScript must be enabled.

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

Open the app in Chrome or another available browser automation surface at
`http://127.0.0.1:8765/<relative-app-path>/<app-slug>.html`. Read the
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

When the user asks for a dashboard or report but does not provide data, create a
small representative dataset in the data JS and mark it clearly in AGENTS.md
as sample data. Never infer sensitive conclusions from fabricated values.

## Handoff checklist

Before reporting completion, verify:

- [ ] Exactly one HTML view, one data JS file, one `config.json`, and one
      `AGENTS.md` exist for the app; include `web/logo.<ext>` or
      `web/screenshot.<ext>` when requested and successfully generated, and
      omit each optional asset cleanly when unavailable.
- [ ] The HTML references `web/<app-slug>-data.js` by a same-directory relative
      path.
- [ ] All runtime data is in the data JS; no duplicated rows are in the HTML.
- [ ] AGENTS.md schema tables match the fields and types consumed by the HTML,
      and its data-usage/update rules match the implementation.
- [ ] User-provided images, documents, and text are checked against the schema
      before extracting values into the data JS.
- [ ] Missing/invalid data produces a visible, actionable empty/error state.
- [ ] The page works offline and does not require a server.
- [ ] `permissions` is omitted unless the app needs a supported capability;
      each requested capability is documented in AGENTS.md and tested in Sessio.
- [ ] Any Sessio file-write workflow declares `downloads`, handles success and
      failure responses, stays below 25 MiB, and documents generated files and
      overwrite behavior in AGENTS.md.
- [ ] Any runtime screenshot export follows
      `references/screenshot-export.md` and is tested without
      `allow-same-origin`.
- [ ] The page background uses `--sessio-chat-background`; light mode uses
      `#f6f6f4`, dark mode uses `#232831`, and theme-dependent charts redraw
      after `sessio:themechange` when needed.
- [ ] A real-browser test covers initial render, the primary interaction, a
      screenshot review, and responsive viewport checks; positioned visuals
      have bounding-box alignment measured when applicable.
- [ ] After validation, the complete app directory is copied to
      `$SESSIO_APP_HOME/apps/<app-slug>/` using the bundled platform publisher;
      `--update`/`-Update` preserves destination-only runtime screenshots and saved
      data, and when AGENTS.md exists, the destination also contains an
      independent CLAUDE.md copy.
- [ ] `web/config.json` is valid JSON and contains the required string fields:
      `nameZh`, `nameEn`, `description`, `author`, `email`, and `version`; its
      optional `permissions` array contains only supported capability names.
- [ ] HTML, JS, config, assets, and AGENTS.md paths are reported using absolute
      paths.
