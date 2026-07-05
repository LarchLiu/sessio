---
name: create-thread
description: >-
  Create Sessio teamwork, workflow/process, brainstorm, or debate threads from a
  natural-language description. Use this skill whenever the user asks to set up,
  design, create, scaffold, or coordinate a Sessio thread/workflow. The skill
  must use the Sessio CLI for thread creation, assistant creation, and
  process-stage creation, while respecting each thread kind's structure: process
  uses stages, teamwork uses thread-level assistants without stages, and
  brainstorm/debate use participant-style lanes without stages.
---

# Sessio Create Thread

Use the Sessio CLI as the source of truth for creating collaboration structures. This skill turns an ordinary user description into persisted Sessio objects:

- assistants
- threads
- process stages

The app exposes the CLI at:

```bash
~/.sessio/bin/sessio
```

Use this absolute path in examples and automation for reliable access. Prefer `--json` and parse returned ids from JSON output.

## When To Use

Use this skill when the user asks to create or scaffold any of these Sessio thread kinds:

```text
process
teamwork
brainstorm
debate
```

Choose the kind from the user's intent:

- `process`: ordered workflow, delivery pipeline, implementation plan, review flow. This is the only kind that uses stages.
- `teamwork`: multiple thread-level assistants coordinate on parallel responsibilities. Do not create stages for teamwork.
- `brainstorm`: multiple agent participants explore ideas in divergent lanes. Do not create stages.
- `debate`: exactly two agent participants argue opposing viewpoints and converge on tradeoffs. Do not create stages.

If the user says "workflow", map it to `process` unless they clearly mean multi-person collaboration.

## Common Creation Workflow

1. Identify the target project.
   Prefer the current working directory when the user does not specify a project. Use the project path with CLI `--project`; the CLI resolves project ids.
2. Convert the user's description into a short creation plan:
   - thread goal
   - thread kind
   - kind-specific members: stages for `process`, assistants for `teamwork`, participants for `brainstorm`/`debate`
3. Follow the kind-specific workflow below. Do not apply process-stage behavior to other kinds.
4. Show the creation plan in chat before executing if the requested structure is large, ambiguous, or likely to create more than three objects. For straightforward requests, proceed and include the plan in the final result.
5. Run CLI commands with `--json`. Never edit the Sessio SQLite database directly.
6. After each successful create command, record returned ids before continuing.
7. Finish with a compact summary containing ids, names, kind, and the next action.

## Kind-Specific Workflows

### Process

Use `process` for ordered work with explicit phases. A process thread is composed of project stages and optional stage assistants.

1. Create or reuse the assistants needed by the process.
2. Create the thread with `--kind process` and any thread-level `--assistant-id` values that should be visible across the whole process.
3. Read the project stage catalog.
4. Add only selected catalog stages to the thread.
5. Configure stage order and stage assistant assignments when needed.
6. Set the active stage when the first actionable stage is clear.
7. Report the exact stage order.

### Teamwork

Use `teamwork` for parallel collaboration between assistants. A teamwork thread has thread-level assistants, not stages.

1. Create or reuse assistants for the requested roles.
2. Create the thread with `--kind teamwork` and attach those assistants with repeated `--assistant-id`.
3. Do not call `stage catalog`, `stage add`, `stage configure`, or `thread set-stage`.
4. If the user describes phases for a teamwork request, encode them as responsibilities or suggested rounds in the description, or ask whether they actually want a `process` thread.
5. Report assistants and suggested coordination rounds, not stage order.

### Brainstorm

Use `brainstorm` for divergent exploration with multiple agent participant lanes. A brainstorm thread has participants and rounds, not stages.

1. Choose at least two participant lanes from the user's requested agents/models.
2. Create the thread with `--kind brainstorm`. The current CLI persists the thread kind and description; if participant lanes cannot be persisted by CLI flags, record the intended participants in the thread description and final report.
3. Do not create assistants or stages unless the user explicitly asks for reusable assistants outside the brainstorm lanes.
4. Do not call `stage catalog`, `stage add`, `stage configure`, or `thread set-stage`.
5. Report participants, exploration prompt, and suggested synthesis criteria.

### Debate

Use `debate` for two-sided critique and decision pressure-testing. A debate thread has exactly two participant lanes, not stages.

1. Choose exactly two participant lanes. If the user supplies fewer or more than two, ask or reduce only when the user's intent is unambiguous.
2. Create the thread with `--kind debate`. The current CLI persists the thread kind and description; if participant lanes cannot be persisted by CLI flags, record the intended lanes in the thread description and final report.
3. Do not create assistants or stages unless the user explicitly asks for reusable assistants outside the debate lanes.
4. Do not call `stage catalog`, `stage add`, `stage configure`, or `thread set-stage`.
5. Report the two positions, evaluation criteria, and convergence prompt.

## CLI Surface

List existing assistants when deciding whether to reuse one:

```bash
~/.sessio/bin/sessio assistant list --project "<projectPathOrId>" --json
```

Create assistants:

```bash
~/.sessio/bin/sessio assistant create --project "<projectPathOrId>" --name "Planner" --agent-id codex --system-prompt "Plan the work and keep collaborators aligned." --json
```

Optional assistant creation flags:

```text
--model <model>
--mode <permissionMode>
--permission-mode <permissionMode>
--effort <effort>
--color <cssColor>
--skill-id <skillId>
--mcp-id <mcpId>
--process-template-id <id>
```

The CLI can fill model, permission mode, and effort from the configured agent defaults when only `--agent-id` is provided.

Create a thread. Use the `--kind` that matches the selected workflow:

```bash
~/.sessio/bin/sessio thread create --project "<projectPathOrId>" --goal "short goal" --description "optional details" --kind process --assistant-id "<assistantId>" --json
```

Read available project stages only for `process` threads:

```bash
~/.sessio/bin/sessio stage catalog --project "<projectPathOrId>" --json
```

Add stages from catalog ids only for `process` threads:

```bash
~/.sessio/bin/sessio stage add --thread-id "<threadId>" --stage-id "<projectStageId>" --assistant-id "<assistantId>" --json
```

Configure order or assignments only for `process` threads:

```bash
~/.sessio/bin/sessio stage configure --id "<threadStageId>" --order 1 --enabled true --json
~/.sessio/bin/sessio stage configure --id "<threadStageId>" --assistant-id "<assistantId>" --json
```

Set the active stage only for `process` threads:

```bash
~/.sessio/bin/sessio thread set-stage --thread-id "<threadId>" --stage-id "<threadStageId>" --json
```

Verify the final structure:

```bash
~/.sessio/bin/sessio thread show --id "<threadId>" --json
```

For `process`, also verify stages:

```bash
~/.sessio/bin/sessio stage list --thread-id "<threadId>" --json
```

## Canvas-Friendly Reporting

When the task is being done from a chat page with canvas available, make the creation process easy for the canvas to display:

- Keep the plan/result in stable sections: `Thread`, one kind-specific member section, and `Next Action`.
- Include ids exactly as returned by the CLI.
- For `process`, present stages as an ordered list with `order`, `threadStageId`, `projectStageId`, `name`, `assistantIds`, and `status`.
- For `teamwork`, present thread assistants; do not include a stages section.
- For `brainstorm` and `debate`, present participant lanes; do not include a stages section.
- If the app has a thread canvas view, mention the created `threadId` so the user can open that thread's canvas/workflow view.
- If the current message includes canvas context, answer in a concise thread-map shape so the canvas can be updated from the chat response.

Use this shape for `process`:

```text
Thread
- id: <threadId>
- kind: process
- goal: <goal>

Assistants
- <assistantId>: <name> (<agentId>, <model>)

Stages
1. <threadStageId> / <projectStageId> - <name> - <assistantIds> - <status>
2. <threadStageId> / <projectStageId> - <name> - <assistantIds> - <status>

Next Action
- Open thread <threadId> and switch to canvas/workflow view, or start work on stage <threadStageId>.
```

Use this shape for `teamwork`:

```text
Thread
- id: <threadId>
- kind: teamwork
- goal: <goal>

Assistants
- <assistantId>: <name> (<agentId>, <model>) - <responsibility>

Coordination
- mode: parallel responsibilities
- suggested rounds: <short description>

Next Action
- Open thread <threadId> and start the teamwork run.
```

Use this shape for `brainstorm` or `debate`:

```text
Thread
- id: <threadId>
- kind: <brainstorm|debate>
- goal: <goal>

Participants
- lane 1: <agentId> (<model>) - <role/position>
- lane 2: <agentId> (<model>) - <role/position>

Next Action
- Open thread <threadId> and start the <brainstorm|debate> run.
```

## Process Stage Selection

Only add stages that exist in `stage catalog`. Do not invent `projectStageId` values. If the catalog does not contain a requested stage:

1. Add the closest existing stage.
2. Explain the substitution.
3. If no reasonable stage exists, create the thread and assistants, then stop with a clear note that a project stage must be added to the project template before that part can be represented.

This section applies only to `process`. For `teamwork`, `brainstorm`, and `debate`, do not substitute stages for responsibilities, participants, lanes, rounds, or positions.

## Assistant Creation Guidance

Prefer reusing existing project assistants when the requested role already exists. Create a new assistant when the user requests a new role or the current assistants do not match.

For generated assistant system prompts:

- Keep them short and role-specific.
- Include the expected output responsibility.
- Avoid promising capabilities the selected agent does not have.
- Add `--skill-id builtin:create-thread` only to an assistant that will create or update Sessio process/teamwork/brainstorm/debate threads.
- Add other skill/MCP ids only when the user explicitly asks for those capabilities or they are clearly required.

## Safety Boundary

- Do not use destructive delete commands.
- Do not edit the Sessio SQLite database directly.
- Do not claim objects were created until the CLI returns success.
- If a CLI command fails, report the command purpose, the error, and the last successfully created ids.
- Ask before creating a very large process, such as more than five assistants or more than eight stages, unless the user explicitly requested that scale.
- Ask before creating a very large teamwork thread, such as more than five assistants, unless the user explicitly requested that scale.
- For debate, keep exactly two participant lanes.
