---
name: create-thread
description: Create Sessio teamwork, workflow/process, brainstorm, or debate threads from a natural-language description. Use this skill whenever the user asks to set up, design, create, scaffold, or coordinate a Sessio thread/workflow, including creating assistants and stages. The skill must use the Sessio CLI for thread, assistant, and stage creation, and should surface the creation plan/results in a chat-canvas-friendly structure.
---

# Sessio Create Thread

Use the Sessio CLI as the source of truth for creating collaboration structures. This skill turns an ordinary user description into persisted Sessio objects:

- assistants
- threads
- stages

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

- `process`: ordered workflow, delivery pipeline, implementation plan, review flow.
- `teamwork`: multiple assistants or people coordinate on parallel responsibilities.
- `brainstorm`: divergent ideation, option generation, exploration.
- `debate`: opposing viewpoints, critique, decision pressure-testing.

If the user says "workflow", map it to `process` unless they clearly mean multi-person collaboration.

## Creation Workflow

1. Identify the target project.
   Prefer the current working directory when the user does not specify a project. Use the project path with CLI `--project`; the CLI resolves project ids.
2. Convert the user's description into a short creation plan:
   - thread goal
   - thread kind
   - assistants to create or reuse
   - stages to add from the project stage catalog
   - stage order and assistant assignment
3. Show the creation plan in chat before executing if the requested structure is large, ambiguous, or likely to create more than three objects. For straightforward requests, proceed and include the plan in the final result.
4. Run CLI commands with `--json`. Never edit the Sessio SQLite database directly.
5. After each successful create command, record returned ids before continuing.
6. Finish with a compact summary containing ids, names, and the exact stage order.

## CLI Surface

List existing assistants when deciding whether to reuse one:

```bash
~/.sessio/bin/sessio assistant list --project "<projectPathOrId>" --json
```

Create assistants:

```bash
~/.sessio/bin/sessio assistant create --project "<projectPathOrId>" --name "Planner" --agent-id codex --system-prompt "Plan the workflow and keep stages unblocked." --json
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

Create a thread:

```bash
~/.sessio/bin/sessio thread create --project "<projectPathOrId>" --goal "short goal" --description "optional details" --kind process --assistant-id "<assistantId>" --json
```

Read available project stages before adding thread stages:

```bash
~/.sessio/bin/sessio stage catalog --project "<projectPathOrId>" --json
```

Add stages from catalog ids:

```bash
~/.sessio/bin/sessio stage add --thread-id "<threadId>" --stage-id "<projectStageId>" --assistant-id "<assistantId>" --json
```

Configure order or assignments:

```bash
~/.sessio/bin/sessio stage configure --id "<threadStageId>" --order 1 --enabled true --json
~/.sessio/bin/sessio stage configure --id "<threadStageId>" --assistant-id "<assistantId>" --json
```

Set the active stage when useful:

```bash
~/.sessio/bin/sessio thread set-stage --thread-id "<threadId>" --stage-id "<threadStageId>" --json
```

Verify the final structure:

```bash
~/.sessio/bin/sessio thread show --id "<threadId>" --json
~/.sessio/bin/sessio stage list --thread-id "<threadId>" --json
```

## Canvas-Friendly Reporting

When the task is being done from a chat page with canvas available, make the creation process easy for the canvas to display:

- Keep the plan/result in stable sections: `Thread`, `Assistants`, `Stages`, `Next Action`.
- Include ids exactly as returned by the CLI.
- Present stages as an ordered list with `order`, `threadStageId`, `projectStageId`, `name`, `assistantIds`, and `status`.
- If the app has a thread canvas view, mention the created `threadId` so the user can open that thread's canvas/workflow view.
- If the current message includes canvas context, answer in a concise workflow-map shape so the canvas can be updated from the chat response.

Use this shape when the user asks to see the creation process on canvas:

```text
Thread
- id: <threadId>
- kind: <process|teamwork|brainstorm|debate>
- goal: <goal>

Assistants
- <assistantId>: <name> (<agentId>, <model>)

Stages
1. <threadStageId> / <projectStageId> - <name> - <assistantIds> - <status>
2. <threadStageId> / <projectStageId> - <name> - <assistantIds> - <status>

Next Action
- Open thread <threadId> and switch to canvas/workflow view, or start work on stage <threadStageId>.
```

## Stage Selection

Only add stages that exist in `stage catalog`. Do not invent `projectStageId` values. If the catalog does not contain a requested stage:

1. Add the closest existing stage.
2. Explain the substitution.
3. If no reasonable stage exists, create the thread and assistants, then stop with a clear note that a project stage must be added to the project template before that part can be represented.

## Assistant Creation Guidance

Prefer reusing existing project assistants when the requested role already exists. Create a new assistant when the user requests a new role or the current assistants do not match.

For generated assistant system prompts:

- Keep them short and role-specific.
- Include the expected output responsibility.
- Avoid promising capabilities the selected agent does not have.
- Add `--skill-id builtin:create-thread` only to an assistant that will create or update Sessio workflows.
- Add other skill/MCP ids only when the user explicitly asks for those capabilities or they are clearly required.

## Safety Boundary

- Do not use destructive delete commands.
- Do not edit the Sessio SQLite database directly.
- Do not claim objects were created until the CLI returns success.
- If a CLI command fails, report the command purpose, the error, and the last successfully created ids.
- Ask before creating a very large workflow, such as more than five assistants or more than eight stages, unless the user explicitly requested that scale.
