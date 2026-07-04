# Spotify Playbook

Target bundle identifiers: `com.spotify.client`

Preferred strategy: AX first, with a quick second snapshot if the first Electron
tree is thin. Use screenshot coordinates for custom playback controls, cards,
and dense content grids.

## First Snapshot Checklist

- Call `computer_get_app_state` with `appId: "com.spotify.client"`.
- If the element tree is sparse immediately after launch, call
  `computer_get_app_state` once more before deciding the app is pixel-only.
- Preserve screenshot dimensions for pixel fallback; Spotify frequently renders
  controls as custom surfaces.
- Confirm the intended target app before sending global-feeling shortcuts such
  as `Space`.

## Common Actions

- Use AX refs for top-level navigation, search fields, modal buttons, and
  obvious labeled controls.
- Use `Cmd+L` or a visible search-field ref to focus search when the field is
  exposed, then type the query.
- Open playlists, albums, and artist cards with double click or coordinate
  click from the latest screenshot when labels are ambiguous.
- Use `Space` for play or pause only after the latest snapshot confirms Spotify
  is the active target for the action.
- Use screenshot-coordinate drag for scrubber and volume adjustments unless AX
  exposes a reliable slider value.

## Gotchas

- The Electron AX tree may improve after accessibility flags are applied, so a
  second snapshot can materially change available refs.
- Visible cards can share repeated labels. When in doubt, use screenshot pixels
  anchored to the card's visual position.
- Avoid acting on stale refs after search, navigation, or opening a playlist.
