# Clock Playbook

Target bundle identifiers: `com.apple.clock`

Preferred strategy: AX first. Clock is mostly standard macOS UI, so refs and
direct values should be more reliable than pixel fallback.

## First Snapshot Checklist

- Call `computer_get_app_state` with `appId: "com.apple.clock"`.
- Check for tab or sidebar refs for World Clock, Alarm, Stopwatch, and Timer.
- Prefer labeled buttons and fields over coordinates.
- Re-snapshot after switching tabs because the visible control set changes
  substantially.

## Common Actions

- Switch modes by clicking AX refs for `World Clock`, `Alarm`, `Stopwatch`, or
  `Timer`.
- Create or edit alarms through labeled buttons and fields when exposed.
- Start, stop, reset, or cancel timers with button refs when labels are present.
- Use `computer_set_value` for exposed time fields or name fields. If the field
  does not accept direct value setting, click the field and type.
- Use screenshot coordinates only for icon-only controls that have no stable
  label in the current snapshot.

## Gotchas

- Some controls change labels based on state, such as `Start` versus `Pause`.
  Re-snapshot after each state-changing action.
- Timer and alarm inputs may have segmented fields. Prefer direct refs for each
  segment when available instead of typing blindly.
- System permission prompts or protected system UI should be treated as blocked
  if macOS refuses synthetic interaction.
