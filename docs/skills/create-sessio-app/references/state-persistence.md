# App State Across Sessio Views

Read this reference when an interactive Sessio App should retain its current
state after the user switches to another App, Project Chat, Settings, Auto
Tasks, an App file preview, or another Sessio view and later returns.

## State adapter

Expose `window.SESSIO_APP_STATE` after the App's state and render functions are
available:

```js
window.SESSIO_APP_STATE = {
  schemaVersion: 1,

  capture() {
    return {
      edits: structuredClone(runtimeState.edits),
      selectionId: runtimeState.selectionId,
      zoom: runtimeState.zoom
    };
  },

  restore(saved) {
    const restored = validateSavedState(saved);
    if (!restored) return;
    runtimeState = applySavedState(createInitialState(window.SESSIO_APP_DATA), restored);
    render(runtimeState);
  }
};
```

Both functions are required. `schemaVersion` must be a positive safe integer.
`capture()` may return a value or a Promise, but normal in-memory capture should
be synchronous. It must produce JSON-serializable data no larger than 1 MiB.
Sessio rejects invalid, cyclic, unsupported, or oversized snapshots. Capture
must settle within 500 ms; on timeout or failure, navigation continues and
Sessio retains the previous valid snapshot when one exists.

`restore()` receives only a snapshot with the same `schemaVersion`. Treat the
value as untrusted: validate every field before modifying App state, preserve
the initial state when validation fails, and render after applying a valid
snapshot. Increment the adapter version when a breaking snapshot change makes
the previous format unsafe to restore.

## What to capture

Capture the smallest state that reconstructs the user's work:

- User-created, edited, reordered, or deleted records.
- Game moves, replay position, current mode, and settings that affect play.
- Meaningful selection, filters, zoom, pan, and expanded sections when users
  expect them to survive a view switch.

Do not capture DOM nodes, callbacks, timers, object URLs, caches, animation
frames, Canvas pixels, hover state, or values that can be derived cheaply. Do
not copy unchanged records from `<app-slug>-data.js`; load initial data from the
data JS and apply the saved changes during `restore()`.

If the user can replace or edit the complete dataset at runtime and those edits
must survive switching views, capture the validated changed dataset or a
versioned operation log. State exactly which approach is used in AGENTS.md.

## Sessio lifecycle

Sessio probes for the adapter after the App iframe loads. Apps without the
adapter continue normally and do not delay navigation. For a supported App,
Sessio requests one snapshot before a normal navigation unmounts the iframe,
including App-to-App switches and transitions to Project Chat, Settings, Auto
Tasks, or an App file preview. It also requests a snapshot before reloading the
App HTML after an agent turn.

Sessio caches the latest valid snapshot by App ID in frontend memory. When the
App is mounted again, Sessio sends the cached snapshot to the adapter. The cache
lasts only for the current Sessio process; it does not survive an application
restart, crash, forced termination, App deletion, or an incompatible schema
version. This bridge does not require a `config.json` permission and does not
write a file into `web/`.

The adapter does not need to send continuous updates or listen for unload
events. Do not use `beforeunload`, `unload`, or `pagehide` as the primary save
path. Sessio owns normal navigation and performs the request before unmounting.

## Direct-browser behavior

A directly opened page has no Sessio state bridge. Initialize from the data JS
and remain fully usable without receiving state messages. Browser-only durable
storage is a separate product choice and must not be required for ordinary App
operation.

## AGENTS.md requirements

Document:

- The adapter `schemaVersion` and the exact fields returned by `capture()`.
- Each field's type, allowed values, validation, and reconstruction behavior.
- Which initial values come from the data JS and how saved changes are applied.
- That Sessio keeps the snapshot only in memory for the current process, with a
  1 MiB serialized limit and no restart or crash recovery.
- Any personal or sensitive data that can be present in the snapshot.

## Validation

In Sessio, make a meaningful change, then test at least these transitions:

1. Switch to another App and return.
2. Switch to Project Chat or Settings and return.
3. Open an App file preview, return to the App, and reload the HTML when that
   workflow is available.

Verify that the state is restored once, no action is duplicated, and the App
remains usable. Also verify that malformed state and a different schema version
leave the initial state unchanged. Test the page directly in a browser to
confirm that it works without the Sessio bridge.
