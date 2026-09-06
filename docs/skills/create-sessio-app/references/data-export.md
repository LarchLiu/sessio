# Runtime Data and File Export

Read this reference when a Sessio App lets its user export JSON, CSV, plain
text, SVG, a game save, a report, or another generated file. For PNG or other
runtime screenshots, also read
[Runtime Screenshot Export](screenshot-export.md).

## Required permission

Add `"downloads"` to `web/config.json` for every runtime export feature:

```json
{
  "permissions": ["downloads"]
}
```

Every Sessio App that exports runtime data/files must implement both export
paths with a defined fallback order. They are complementary capabilities, not
two files that must be created for every export:

1. An ordinary browser download created with a Blob and `<a download>`.
2. A `sessio-app-write-file` request that saves inside the App's `web/`
   directory.

For one user-triggered export action, generate one snapshot and try the Sessio
`postMessage` bridge first. If the page is embedded in Sessio and the bridge
returns success, report the saved `web/` path and do not start a second browser
download. If the bridge is unavailable, times out, or returns failure, fall
back to the ordinary browser download and report that fallback. In a direct
browser, `window.parent === window`, so the bridge is unavailable and the
browser download is the expected path. Keep both implementations in the App;
they are a preferred path plus a fallback, not an either/or implementation
choice.

Do not omit the permission because a direct-browser test succeeds. Direct pages
are not subject to Sessio's iframe sandbox, which needs `allow-downloads` for a
download. Sessio also rejects file-write bridge requests when `downloads` is
absent and reads `config.json` again before each write.

Importing a file through a user-triggered `<input type="file">` and reading it
with `FileReader` does not by itself require `downloads`. An App that only
imports and never exports may omit the permission.

## Define the exported contract

Choose the format before implementing the UI and document its complete schema
in AGENTS.md. Prefer formats that preserve the data without depending on the
rendered DOM:

- Use JSON for nested records, settings, game saves, and data that will be
  imported again. Include a numeric `schemaVersion`, a stable App identifier,
  an ISO 8601 export timestamp, and a named payload object.
- Use CSV for a flat table intended for spreadsheet tools. Use a consistent
  column order, RFC 4180 quoting, and an explicit policy for null values and
  line endings. Neutralize cells beginning with `=`, `+`, `-`, or `@` when the
  data is untrusted and the CSV may be opened in a spreadsheet.
- Use plain text only when formatting loss is acceptable and the encoding and
  newline rules are documented.
- Use SVG only for self-contained vector output. Do not embed remote resources,
  scripts, or unsanitized markup from user data.
- Use Base64 through the Sessio bridge for binary formats. Send only the Base64
  payload, without a `data:` URL prefix.

Keep the runtime data JS as the App's configuration/data source. An exported
file may contain the current interactive state, user edits, filters, selections,
or a replay history derived from that source. State exactly which values are
included. If the App supports re-import, validate the file independently rather
than assigning parsed input directly to `window.SESSIO_APP_DATA`.

Example JSON envelope:

```json
{
  "schemaVersion": 1,
  "app": "example-app",
  "exportedAt": "2026-09-06T12:34:56.000Z",
  "data": {
    "records": []
  }
}
```

## Build a stable snapshot

Create one immutable export snapshot when the user starts the action. Do not
read changing controls repeatedly while serializing. Derive values through the
same normalization rules used by the view, and exclude transient implementation
state such as timers, DOM nodes, object URLs, callbacks, and caches.

For JSON, use `JSON.stringify(snapshot, null, 2)`. Detect circular references
before the user reaches the download step. For CSV and text, construct rows from
validated values and escape each field according to the documented format; do
not concatenate unescaped user strings into markup.

## Preferred path: Sessio file write

When the App has a Sessio parent, send the generated file through the file-write
bridge before using the browser fallback. Follow the complete request, response,
path, and security contract in
[Exporting and saving files](../SKILL.md#exporting-and-saving-files).

For text formats, send UTF-8 directly:

```js
window.parent.postMessage({
  source: "sessio-app",
  type: "sessio-app-write-file",
  requestId: `export-${Date.now()}`,
  path: "exports/example-app-state.json",
  data: JSON.stringify(exportSnapshot, null, 2),
  encoding: "utf8",
  overwrite: false
}, "*");
```

The bridge path is relative to `web/`. Prefer a visible directory such as
`exports/` and a unique timestamped filename with `overwrite: false`. Use
`overwrite: true` only for an explicitly documented fixed file such as
`exports/latest.json`. The decoded payload must not exceed 25 MiB.

Listen for the matching result message and require `event.source ===
window.parent`, `source === "sessio"`, the expected result type, and the same
`requestId`. Remove listeners and timeouts after completion. Show the returned
relative path and file size on success and the bridge error on failure.

## Fallback path: ordinary browser download

Create a Blob with an accurate MIME type, download it from an explicit user
action, and revoke the object URL after the click:

```js
function downloadBlob(blob, fileName) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = fileName;
  link.hidden = true;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

const json = JSON.stringify(exportSnapshot, null, 2);
const blob = new Blob([json], { type: "application/json;charset=utf-8" });
downloadBlob(blob, "example-app-20260906T123456Z.json");
```

Use a single button such as `导出 JSON` or `Export CSV`. Try the Sessio bridge
first; invoke this browser download only when the bridge cannot save the file.
Browser download behavior must remain a direct consequence of that click;
delayed work can lose transient user activation in some engines. If generation
is asynchronous, prepare the content first when practical and clearly report
when the download starts.

## Import and replay

When exported files can be imported again, use a hidden or visible
`<input type="file">` with a clear button or label. Restrict `accept` to the
documented extensions and MIME types, cap the file size before reading, parse
inside `try`/`catch`, and validate every field before changing App state.

For a replay or timeline, keep the imported immutable record separate from the
rendered step. Start at step zero unless the product says otherwise, rebuild the
view deterministically for each step, disable editing controls while replaying,
and provide previous, next, play/pause, progress, and exit controls when they
fit the workflow. Import must not silently overwrite the data JS.

## File names and status

Use a stable App slug, a filesystem-safe UTC timestamp, and the correct
extension. Avoid spaces, slashes, colons, hidden path segments, and
platform-reserved names. Generated names should be unique when overwrite is
false.

While generating or writing, disable the export control, set
`aria-busy="true"`, and expose progress plus the final filename/path or error in
a visible `role="status"` region. Restore the control after success or failure.
Report the first successful path as the export result. If the bridge fails and
the browser fallback succeeds, say explicitly that the file was downloaded
locally and Sessio `web/` saving failed; do not claim that the bridge saved it.
If both paths fail, show both errors and keep the export action usable.

## Privacy and data handling

Export only the fields promised by the UI and AGENTS.md. Do not include secrets,
hidden identifiers, debug state, or unrelated imported content. Tell the user
when the file can contain personal or sensitive data already shown in the App.
Keep generation local unless the App has a separately reviewed network bridge;
export does not justify adding arbitrary network access.

## AGENTS.md requirements

Document every generated file with:

- The single user action, the bridge-first/fallback order, and the direct-browser
  behavior when the bridge is unavailable.
- The filename or relative-path rule, format, MIME type, encoding, schema
  version, and a valid example.
- Every exported field, its type, requiredness, allowed values, units, null
  behavior, and source or derivation.
- Maximum expected size, the 25 MiB bridge limit when applicable, and overwrite
  behavior.
- Import validation, migration, replay, and error behavior when re-import is
  supported.
- Whether the output can contain personal or sensitive data and whether any
  content leaves the local machine.
- The required `downloads` permission and the direct-browser/Sessio behavior.

## Validation

Test the direct page and a Sessio-equivalent sandbox. The sandbox host must use
at least:

```html
<iframe sandbox="allow-scripts allow-downloads" src="./app-test.html"></iframe>
```

Do not add `allow-same-origin`. Verify:

- `web/config.json` contains `"downloads"` before testing any screenshot or
  data/file export.
- The export button has a useful accessible name and the operation follows a
  user gesture.
- Browser download produces the expected filename, MIME type, encoding, and
  parseable contents.
- The same click first sends a bridge request with an allowed relative path, the
  intended encoding and overwrite value, and a payload below 25 MiB; simulate
  bridge failure and verify that the browser fallback then runs.
- The selected path restores controls and produces useful status text. A bridge
  success must not trigger a duplicate browser download; a fallback result must
  identify that Sessio saving did not succeed.
- Imported valid files reconstruct the expected state; malformed, oversized,
  unsupported-version, out-of-range, duplicate, or otherwise invalid content
  leaves the current state unchanged and shows an error.
- Re-exporting imported data preserves the documented values and schema.
- The console contains no sandbox, cross-origin, CSP, object-URL, or uncaught
  serialization errors.

Also test a narrow viewport so file controls and status text wrap without
overlap or clipping.
