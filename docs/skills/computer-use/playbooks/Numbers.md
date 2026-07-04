# Numbers Playbook

Target bundle identifiers: `com.apple.iWork.Numbers`

Preferred strategy: hybrid. Use AX refs for windows, sheets, toolbar controls,
menus, dialogs, and inspectors. Use screenshot-coordinate targeting plus
keyboard navigation for table cells and canvas-level interactions.

## First Snapshot Checklist

- Call `computer_get_app_state` with `appId: "com.apple.iWork.Numbers"`.
- Determine whether the target is document chrome, a table cell, a sheet tab, or
  an inspector panel.
- Keep screenshot dimensions and table location from the latest snapshot before
  using pixel clicks on cells.
- Re-snapshot after creating sheets, changing selection, resizing panels, or
  editing cell contents.

## Common Actions

- Select cells with screenshot-coordinate click from the latest snapshot, then
  type the value and press `Return`.
- Use keyboard navigation from a known selected cell for repeated edits. Arrow
  keys are often more stable than recomputing pixel locations.
- Use double click or `Return` to enter cell-editing mode when replacing only
  part of a cell value.
- Use AX refs for toolbar actions such as Format, Organize, Insert, Share, and
  dialog confirmation buttons.
- Save with `Cmd+S` when a document is open and the task requires persistence.

## Gotchas

- Spreadsheet cells are often canvas-rendered and may not have useful AX refs.
  Pixel targeting is expected, but only after a fresh snapshot.
- A selected cell and an editing cell are different states. If typing does not
  replace the intended value, press `Esc`, re-snapshot, and select again.
- Large sheets can scroll both horizontally and vertically. Anchor coordinates
  to visible headers or table edges from the latest screenshot.
