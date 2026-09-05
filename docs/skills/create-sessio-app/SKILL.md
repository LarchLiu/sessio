---
name: create-sessio-app
description: >-
  Create a self-contained Sessio HTML app from a natural-language product or
  data-visualization request. Use this skill whenever the user asks to generate
  an HTML dashboard, report, chart, table, data tool, or small offline app for
  Sessio, especially when the data should be easy to replace or regenerate.
  Always keep application markup/behavior, runtime data, and documentation in
  separate files so later agents can update data without rewriting the view.
compatibility: Sessio HTML preview with optional inline JavaScript enabled; no server or network access required.
---

# Create Sessio App

Create small, inspectable HTML applications that open directly in a browser and
preview safely in Sessio. The central contract is data separation:

```text
<app-dir>/
  <app-slug>.html       # UI, styles, rendering, and interaction logic
  <app-slug>-data.js    # the only runtime data source
  README.md             # purpose, usage, and the complete data contract
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
3. **Define the data schema before writing the view.** Decide the exact top-level
   data envelope and every record field. Keep presentation metadata separate
   from records. Record units, requiredness, null behavior, allowed values, and
   an example for every field.
4. **Create the data file first.** Put all sample or supplied data in
   `<app-slug>-data.js`. It must assign one global value and contain no rendering
   code:

   ```js
   window.SESSIO_APP_DATA = {
     schemaVersion: 1,
     meta: { title: "Example" },
     records: []
   };
   ```

   The global name may be app-specific when needed, but the HTML and README
   must state it exactly. Never duplicate records, labels derived from records,
   or default sample rows inside the HTML.
5. **Create the HTML view.** Reference the data file with a relative script tag
   such as `<script src="./<app-slug>-data.js"></script>`. Read the global data
   object after that script and render the empty state when it is missing or
   invalid. Keep the page useful when opened as `file://`; do not require Vite,
   a local server, `fetch`, XHR, WebSocket, CDN assets, npm imports, or a
   backend. Inline CSS and JavaScript are preferred for portability.
6. **Build for Sessio preview.** The view must work after the user enables
   “Allow inline JavaScript”. Local scripts in the same directory and child
   directories are supported by Sessio's preview; use paths such as
   `./data.js` or `scripts/data.js`. Do not depend on `document.currentScript`
   or the original external script URL after it is inlined.
7. **Write README.md from the implemented contract.** Explain what the app is
   for, how to open it in a browser and Sessio, the three-file layout, the exact
   data global, the complete schema, a valid data example, how to replace or
   regenerate data, and known preview/security limitations. The README must be
   updated whenever the schema changes.
8. **Validate before handing off.** Check that the HTML references the data JS,
   the data JS parses, the HTML contains no record literals, and the README
   documents every top-level and record field. Exercise the initial render and
   at least one requested interaction. Check a desktop width and a narrow width
   when the app has a visual layout. Do not start a persistent development
   server; if temporary serving is essential for verification, stop it before
   finishing.
9. **Publish the tested app to Sessio's app directory.** Only after all checks
   pass, read the absolute `SESSIO_APP_HOME` environment variable supplied by
   running Sessio process. Invoke the bundled publisher for the current shell:
   use `scripts/publish_app.sh <source-dir> <app-slug>` on macOS/Linux (or Git
   Bash/WSL), and `scripts/publish_app.ps1 <source-dir> <app-slug>` on native
   Windows PowerShell. Both scripts copy the complete app directory to
   `$SESSIO_APP_HOME/apps/<app-slug>/`, preserving the HTML, data JS, README,
   and any child directories exactly. They refuse to replace an existing
   destination unless the user explicitly authorizes `--force`/`-Force`.
   The publisher is an execution step, not a completion message: run it after
   validation and then verify that the destination contains the expected HTML,
   data JS, README, and child directories. Resolve `scripts/` relative to the
   directory containing the loaded `SKILL.md` when invoking the bundled file.
   If `SESSIO_APP_HOME` is missing, do not guess a profile or write to a
   hard-coded home directory; report that publishing is blocked. The README
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
  `schemaVersion` and updates the README migration note.
- Treat data as untrusted input: validate types, escape text through DOM APIs,
  and avoid evaluating strings as code. Do not put secrets or personal data in
  sample files.
- If the user supplies CSV or JSON, convert it once into the data JS contract;
  do not make the HTML fetch or parse a second copy at runtime unless the user
  explicitly requests an importer.

## README data contract

Use this structure unless the user requests another documentation language:

```markdown
# App title

## Purpose
What the app shows and who uses it.

## Files
- `<app-slug>.html`: view and interaction logic.
- `<app-slug>-data.js`: only runtime data, exported as `window.<GLOBAL>`.
- `README.md`: this contract and maintenance notes.

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
Edit or regenerate only `<app-slug>-data.js`; preserve the documented schema.

## Installed location
The validated copy is installed at `$SESSIO_APP_HOME/apps/<app-slug>/`, where
`SESSIO_APP_HOME` is the absolute profile directory supplied by Sessio. Keep the
source copy for development, but update the installed data file too when
distributing a new data snapshot outside the source repository. `SESSIO_APP_VARIANT`
may be `dev` or `prod` for diagnostics, but it is not a substitute for the
resolved absolute app-home path.

## Limitations and validation
Offline/CSP behavior, empty-state behavior, and the checks run before delivery.
```

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

When the user asks for a dashboard or report but does not provide data, create a
small representative dataset in the data JS and mark it clearly in the README
as sample data. Never infer sensitive conclusions from fabricated values.

## Handoff checklist

Before reporting completion, verify:

- [ ] Exactly one HTML view, one data JS file, and one README exist for the app.
- [ ] The HTML references the data JS by a same-directory or child-directory
      relative path.
- [ ] All runtime data is in the data JS; no duplicated rows are in the HTML.
- [ ] README schema tables match the fields and types consumed by the HTML.
- [ ] Missing/invalid data produces a visible, actionable empty/error state.
- [ ] The page works offline and does not require a server.
- [ ] After validation, the complete app directory is copied to
      `$SESSIO_APP_HOME/apps/<app-slug>/` using the bundled platform publisher,
      with child directories preserved and overwrite protection enabled.
- [ ] HTML, JS, and README paths are reported using absolute paths.
