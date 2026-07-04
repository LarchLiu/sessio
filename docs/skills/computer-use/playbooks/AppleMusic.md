# Apple Music Playbook

Target bundle identifiers: `com.apple.Music`

Preferred strategy: AX first for sidebar, toolbar, search fields, and visible
buttons. Use screenshot-coordinate actions for album art, grid cards, and custom
media surfaces whose AX labels are missing or ambiguous.

## First Snapshot Checklist

- Call `computer_get_app_state` with `appId: "com.apple.Music"`.
- If the app was launched by the snapshot call, use the returned state and
  refresh once if the library view is still loading.
- Check for labeled sidebar refs before falling back to pixels.
- Keep the latest `snapshot_id`; sidebar and card refs can churn after
  navigation.

## Common Actions

- Navigate through the sidebar by clicking AX refs with labels such as
  `Listen Now`, `Browse`, `Radio`, `Library`, `Songs`, or `Albums`.
- Search by focusing the search field through AX when present, then type the
  query and press `Return`.
- Start playback from a grid or album view with double click when a card or row
  is the obvious target.
- Toggle playback with `Space` only when Music is the intended target and a
  fresh snapshot confirms the player area.
- Adjust sliders with `computer_set_value` when AX exposes a value. Otherwise
  use screenshot-coordinate drag from the latest snapshot.

## Gotchas

- Music content refreshes asynchronously. Re-snapshot after navigation or after
  opening search results.
- Card grids may expose weak or duplicate AX labels. Prefer pixel coordinates
  anchored to the latest screenshot for visually selected media cards.
- Do not use `open -b`; launch through `computer_launch_app` or
  `computer_get_app_state`.
