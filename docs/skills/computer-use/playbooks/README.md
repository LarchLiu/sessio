# Computer Use Playbooks

This directory contains versioned, app-specific operating notes for Sessio's
computer-use capability.

The files here are reviewable resources only. They do not define prompt
injection, runtime selection, or model-context assembly. That ownership stays in
[computer-use-prompt-refactor-plan.md](../../../computer-use-prompt-refactor-plan.md).

## Resource Shape

`index.json` is the canonical inventory. Each entry points at one Markdown
playbook and records the expected target apps, strategy, and review metadata.

Each playbook follows the same shape:

- target bundle identifiers
- preferred automation strategy
- first snapshot checklist
- common action patterns
- fallback rules and app-specific gotchas

## Shared Operating Conventions

- Always call `computer_get_app_state` before taking action in an app.
- Treat element refs as scoped to the latest authoritative snapshot.
- Treat screenshot pixels as the default coordinate space for pixel actions.
- Prefer AX refs when the tree is rich, then use screenshot-coordinate fallback
  for sparse AX trees or custom-drawn controls.
- Never use `open -b` for app launch. Use `computer_launch_app` or
  auto-launch through `computer_get_app_state`.
- Use `computer_permissions` and `computer_grant` for onboarding instead of
  asking the user to hunt through settings unaided.
- After a mutation, reason from the returned post-action state rather than from
  the pre-action snapshot.

## Initial Inventory

The first inventory is seeded from
[SKILL.md](../SKILL.md):

- Apple Music
- Spotify
- Notion
- Clock
- Numbers
- NetEase Music
