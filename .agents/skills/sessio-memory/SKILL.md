---
name: sessio-memory
description: Search and resolve project-level Sessio session memory. Use this whenever the user asks about prior project discussions, previous agent sessions, historical decisions, implementation context, qmd memory, or "what did we decide before" in a Sessio-enabled workspace.
---

# Sessio Memory

Use Sessio as the source of truth for project session memory. Sessio stores compact project memory records and source references; raw JSONL remains outside this skill and should only be accessed through Sessio commands.

Sessio configuration lives at `~/.sessio/config.toml`. The app/CLI creates this file with defaults the first time config is loaded if it does not already exist. Treat the config file as the persistent source of truth for memory backend settings; avoid using transient environment overrides except for one-off debugging.

## Default Workflow

1. Determine the project path. Prefer the current working directory when the user asks about the current project.
2. Search memory with the Sessio CLI:

```bash
sessio memory search --project "$PWD" "<query>" --json
```

3. Read the JSON response. Use the stable `hits` array and `backendError` field. Each hit carries a `recordId` (the stable Sessio memory record identifier). The qmd-internal payload is **not** included by default; pass `--include-raw` only when debugging the backend.
4. If `hits` contains useful records, summarize only what those hits support.
5. If the user asks for details or source provenance, resolve a record:

```bash
sessio memory resolve --record-id "<record_id>" --json
```

6. If the user asks which base record covered a specific record, use:

```bash
sessio memory covered-by --record-id "<record_id>" --json
```

7. If the user asks which records were covered by a base record, use:

```bash
sessio memory base --record-id "<record_id>" --json
```

8. If search returns no hits, an empty result, or a non-null `backendError`, say that Sessio memory did not return a usable match. Do not invent historical context.
9. When troubleshooting backend availability, inspect the configured backend and install command through Sessio config/status instead of hard-coding backend names or commands.

When a hit, resolved record, or `covered-by`/`base` result contains continuation metadata:

- Treat `continuation` / `continuationSummary` as provenance about replay trimming, not as the substantive project memory itself.
- Use it to explain where a record's kept suffix came from or which earlier session covered the trimmed prefix.
- Use `covered-by` when the user asks "what covered this record?" and `base` when they ask "what records did this one cover?".
- Do not quote raw turn/byte ranges unless the user is asking about dedupe/provenance mechanics.

## Commands

Use `--project "$PWD"` by default:

```bash
sessio memory search --project "$PWD" "qmd storage design" --json
```

Use `--project-key` only when a project key is already known:

```bash
sessio memory search --project-key "-Users-alex-Work-cloudgeek-sessio" "provider abstraction" --json
```

Inspect background memory/qmd job state when search fails unexpectedly:

```bash
sessio memory jobs --project-key "<project_key>" --json
```

Inspect memory backend availability:

```bash
sessio memory status --json
```

Inspect the persisted Sessio config:

```bash
sessio config show --json
```

Persist memory backend settings to `~/.sessio/config.toml`:

```bash
sessio config memory set --binary "/path/to/backend" --index "sessio" --artifacts-root "~/.sessio/memory" --auto-embed false --install-command "npm install -g @tobilu/qmd"
```

Only include flags the user wants to change. `config memory set` writes the resulting memory backend config back to `~/.sessio/config.toml`.

Resolve sources for a record:

```bash
sessio memory resolve --record-id "<record_id>" --json
```

Read a raw source excerpt for debugging provenance only when needed:

```bash
sessio memory resolve --record-id "<record_id>" --include-source-excerpt --json
```

Inspect records covered by a base record:

```bash
sessio memory base --record-id "<record_id>" --json
```

Inspect which base record covered a given record:

```bash
sessio memory covered-by --record-id "<record_id>" --json
```

## Interpretation Rules

- Do not call `qmd` directly from this skill.
- Do not parse agent JSONL directly from this skill.
- Do not hard-code the memory backend install command in answers. Read it from `sessio config show --json` or `sessio memory status --json` details when needed.
- Say "memory backend" when discussing backend availability; use the concrete backend name only when it comes from Sessio status/config.
- Prefer concise answers that cite record titles, summaries, and source refs when available.
- Distinguish clearly between "Sessio memory says..." and your own inference.
- If `backendError` is present, report it briefly. For troubleshooting, run `sessio memory status --json` and `sessio config show --json`; if the backend is missing, mention the configured install command and that clicking the app's memory backend missing button copies the same command.
- Do not pass `--include-raw` in normal workflows; it is for debugging the qmd backend.
- Do not use `--include-source-excerpt` in normal workflows; it is for provenance debugging and can return large payloads.
- If `continuationSummary` is present, prefer that human-readable summary over unpacking raw `continuation` fields by hand.

## No-Hit Behavior

When search has no useful memory result, use language like:

```text
Sessio memory did not return a usable match for that query in this project, so I should not rely on prior-session context here.
```

Then continue with normal project inspection if the task can be solved from the current files.
