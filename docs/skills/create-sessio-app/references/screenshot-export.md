# Runtime Screenshot Export

Read this reference when a Sessio App must let its user capture a chart, board,
tree, diagram, game state, or other rendered view and save it as an image. This
is a runtime feature. It is separate from the optional `web/screenshot.<ext>`
asset captured by the developer for an App listing or handoff.

## Required contract

Add `"downloads"` to `web/config.json`. This permission authorizes both an
ordinary browser download and Sessio's file-write bridge. It does not create a
screenshot automatically; the HTML must provide a user-triggered control,
render the image, encode it, send the write request, and display the result.

Choose and document the capture scope before implementing it:

- **Full logical view:** Export the whole calculated scene independent of the
  current pan, zoom, scrolling, or viewport clipping. Preserve filters and
  visibility options unless the product requires a canonical unfiltered view.
- **Current viewport:** Export only what the user currently sees, including the
  current camera transform and clipping bounds.
- **Selected region:** Export a named component or data region with stable
  logical bounds.

Use a button such as `保存截图` or `Export PNG`. While rendering or writing,
disable it, set `aria-busy="true"`, and expose progress and the final path or
error through a visible `role="status"` region.

## Sessio sandbox constraint

Sessio runs App scripts in a sandboxed iframe with `allow-scripts` but without
`allow-same-origin`. The document therefore has an opaque origin. Do not add
`allow-same-origin` to make screenshot code work: combining it with scripts can
weaken the boundary between the App and the Sessio host.

DOM rasterizers such as `html2canvas` commonly clone the document into another
iframe. In Sessio this can fail with an error like:

```text
Sandbox access violation: Blocked a frame at "null" from accessing a
cross-origin frame. Both frames are sandboxed and lack the
"allow-same-origin" flag.
```

Do not use an iframe-based DOM clone as the primary Sessio screenshot path.
SVG `foreignObject` capture is also inconsistent across WebKit/WKWebView and
should not be the only export path.

Prefer one of these strategies:

1. For an existing `<canvas>`, export that canvas directly with `toBlob()`.
2. For a pure SVG view, serialize or redraw the SVG into a canvas without
   creating a child iframe, then export the canvas.
3. For DOM, SVG, or mixed positioned visuals, draw an export canvas from the
   same normalized data and layout coordinates used by the live view. Draw the
   background first, then links or grid lines, then nodes, markers, labels, and
   overlays that belong in the exported image.

The third strategy is usually the most predictable for trees, charts, boards,
and editors. Keep layout calculation shared so the HTML view and exported image
cannot drift. The export renderer may use Canvas primitives instead of copying
DOM styling exactly, but it must preserve the information, relationships,
ordering, colors, and selected capture scope.

## Canvas sizing and theme

Start from logical content dimensions, then select a scale subject to a pixel
budget. A 12-megapixel cap gives useful resolution without unbounded memory use:

```js
const MAX_EXPORT_PIXELS = 12_000_000;
const width = Math.ceil(layout.width);
const height = Math.ceil(layout.height);
const scale = Math.min(
  2,
  Math.sqrt(MAX_EXPORT_PIXELS / Math.max(1, width * height))
);

const canvas = document.createElement("canvas");
canvas.width = Math.max(1, Math.floor(width * scale));
canvas.height = Math.max(1, Math.floor(height * scale));

const context = canvas.getContext("2d");
if (!context) throw new Error("Canvas export is unavailable");
context.scale(scale, scale);
```

Canvas pixels do not update automatically when Sessio's theme changes. Resolve
the current CSS colors immediately before drawing. A custom property may itself
contain another `var(...)`; use a temporary element when a fully resolved color
is needed:

```js
function resolveCssColor(variableName, fallback) {
  const probe = document.createElement("span");
  probe.style.color = `var(${variableName}, ${fallback})`;
  probe.hidden = true;
  document.body.appendChild(probe);
  const color = getComputedStyle(probe).color || fallback;
  probe.remove();
  return color;
}
```

Use `--sessio-chat-background` or the App's resolved workspace background for
the canvas. Resolve text, muted text, border, accent, surface, and data-series
colors the same way. Draw at logical coordinates after scaling; do not multiply
each coordinate manually.

## Generate the PNG

Use `canvas.toBlob()` rather than a large `toDataURL()` as the first encoding
step. Check the decoded Blob size before Base64 conversion because Sessio's
file-write limit is 25 MiB:

```js
function canvasToPngBlob(canvas) {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) resolve(blob);
      else reject(new Error("The browser could not create a PNG"));
    }, "image/png");
  });
}

const blob = await canvasToPngBlob(canvas);
if (blob.size > 25 * 1024 * 1024) {
  throw new Error("The screenshot exceeds Sessio's 25 MiB limit");
}
```

Base64 expands data by about one third and creates additional in-memory copies.
Keep the pixel budget conservative and avoid retaining old canvases, data URLs,
or Base64 strings after the operation completes.

## Save in Sessio

Follow the complete request and response contract in
[Saving files through Sessio](../SKILL.md#saving-files-through-sessio). Convert
only the PNG Blob payload to Base64; do not include the `data:image/png;base64,`
prefix.

Use a visible, relative destination such as:

```text
screenshots/<app-slug>-<UTC-timestamp>.png
```

Build timestamps without colons, slashes, spaces, or other path punctuation.
A millisecond timestamp or another unique suffix allows the request to keep
`overwrite: false`, which should be the default for user-created screenshots.
Use `overwrite: true` only for an explicitly documented fixed destination such
as `screenshots/latest.png`.

Correlate the response with a unique `requestId`. Require
`event.source === window.parent`, `source === "sessio"`, the expected result
type, and the matching request ID. Remove the listener and timeout after either
success or failure. Report the returned `relativePath`, output dimensions, and
human-readable file size to the user.

## Direct-browser fallback

When the page is opened directly and `window.parent === window`, download the
same Blob without Base64 conversion:

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
```

An App embedded by a non-Sessio parent may not provide the bridge. If that use
case matters, define a bounded response timeout and an explicit fallback or
error instead of waiting indefinitely.

## AGENTS.md requirements

Document:

- The capture scope and which filters, visibility settings, selections, theme,
  pan, and zoom affect the result.
- The relative path or filename rule, image format, maximum pixel and file-size
  limits, and overwrite behavior.
- The fact that the image can contain the same personal or sensitive data shown
  by the App.
- The user action that triggers rendering and writing.
- Sessio local-save behavior and the direct-browser download fallback.
- Any intentional differences between the interactive HTML view and the export
  renderer.

## Validation

Test the ordinary browser path and the Sessio-equivalent sandbox path. A direct
browser test alone cannot detect opaque-origin failures.

Create a temporary host page whose child iframe uses at least:

```html
<iframe sandbox="allow-scripts allow-downloads" src="./app-test.html"></iframe>
```

Do not add `allow-same-origin`. Make the host receive the App's write request,
decode the Base64 payload, and send a matching success response. Verify:

- The request path is inside `screenshots/`, uses `encoding: "base64"`, and has
  the intended overwrite value.
- Decoded bytes begin with the PNG signature
  `89 50 4e 47 0d 0a 1a 0a`.
- The decoded payload is below 25 MiB and the pixel dimensions match the chosen
  logical bounds and scale.
- The image visibly includes the expected background, labels, links, markers,
  and edge content without clipping.
- The status region reports success and the button returns to its enabled
  state; a simulated failure produces a clear error.
- The browser console contains no sandbox, cross-origin, CSP, or canvas errors.

Also test a narrow viewport. The controls must remain usable and status text
must wrap without overlapping the exported view or adjacent controls.
