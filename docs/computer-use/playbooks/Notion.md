# Notion Playbook

Target bundle identifiers: `notion.id`

Preferred strategy: AX first for app chrome, page list, dialogs, and command
surfaces. Use screenshot-coordinate clicks for the block editor canvas when refs
are too generic.

## First Snapshot Checklist

- Call `computer_get_app_state` with `appId: "notion.id"`.
- If the app was just launched, refresh once after the workspace finishes
  loading.
- Identify whether the target is app chrome, sidebar navigation, or the block
  editor. Choose AX for chrome and pixels for ambiguous editor regions.
- Treat refs inside the page body as short-lived after typing, pressing Enter,
  or moving blocks.

## Common Actions

- Open quick find with `Cmd+P`, type the page name, then choose the best
  visible result by ref or screenshot coordinate.
- Focus a block by clicking its visual text area from the latest screenshot,
  then type text or press `Enter` for a new block.
- Use slash commands by typing `/` after focusing an editor block, then type the
  command and press `Return`.
- Use AX refs for dialogs such as share, publish, date pickers, and confirmation
  buttons when labels are available.
- Use `computer_set_value` for exposed text fields. For editor blocks, direct
  typing after a coordinate click is usually more reliable.

## Gotchas

- Notion's editor often exposes generic or nested refs that are hard to map to
  the visible block. Pixel targeting is acceptable after a fresh snapshot.
- Pressing `Esc` can move focus from editing to block selection. Re-snapshot
  before continuing if the focus mode is unclear.
- Dragging blocks should use screenshot coordinates and a fresh snapshot because
  block handles appear only on hover or focus.
