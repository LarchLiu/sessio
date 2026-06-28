# NetEase Music Playbook

Target bundle identifiers: `com.netease.163music`

Preferred strategy: pixel first. NetEase Music commonly exposes a very sparse
or empty AX tree, so the screenshot is the primary source of truth.

## First Snapshot Checklist

- Call `computer_get_app_state` with `appId: "com.netease.163music"`.
- Expect `elements` to be empty or incomplete. This is normal for this app.
- Use screenshot-space coordinates from the returned image for clicks, right
  clicks, and drags.
- Re-snapshot after every navigation or playback state change because the layout
  can shift.

## Common Actions

- Click sidebar destinations, playlists, search, and playback controls by
  screenshot coordinate from the latest snapshot.
- Use `Space` for play or pause only after confirming the current view and
  target app state.
- Use coordinate drag for progress or volume controls.
- Use visible search coordinates to focus search, type the query, then press
  `Return`.
- Prefer keyboard shortcuts only when the requested action is unambiguous and a
  recent snapshot confirms the app state.

## Gotchas

- Do not wait for reliable AX refs. Treat the screenshot as authoritative.
- Retina and downsampled screenshots make raw screen points risky. Use the
  default screenshot coordinate space.
- Some custom controls may ignore synthetic events. If a pixel action appears to
  do nothing, re-snapshot before retrying at a slightly different visual anchor.
