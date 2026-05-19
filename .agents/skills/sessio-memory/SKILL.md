---
name: sessio-memory
description: Search and resolve project-level Sessio session memory. Use this whenever the user asks about prior project discussions, previous agent sessions, historical decisions, implementation context, qmd memory, or "what did we decide before" in a Sessio-enabled workspace.
---

# Sessio Memory

Use Sessio as the source of truth for project session memory. Sessio stores compact project memory cards and source references; raw JSONL remains outside this skill and should only be accessed through Sessio commands.

## Default Workflow

1. Determine the project path. Prefer the current working directory when the user asks about the current project.
2. Search memory with the Sessio CLI:

```bash
sessio memory search --project "$PWD" "<query>" --json
```

3. Read the JSON response. Use the stable `hits` array and `backendError` field. The qmd-internal payload is **not** included by default; pass `--include-raw` only when debugging the backend.
4. If `hits` contains useful memory cards, summarize only what those hits support.
5. If the user asks for details or source provenance, resolve a card:

```bash
sessio memory resolve --card-id "<card_id>" --json
```

6. If the user asks which base card covered a specific card, use:

```bash
sessio memory covered-by --card-id "<card_id>" --json
```

7. If the user asks which cards were covered by a base card, use:

```bash
sessio memory base --card-id "<card_id>" --json
```

8. If search returns no hits, an empty result, or a non-null `backendError`, say that Sessio memory did not return a usable match. Do not invent historical context.

When a hit, resolved card, or `covered-by`/`base` result contains continuation metadata:

- Treat `continuation` / `continuationSummary` as provenance about replay trimming, not as the substantive project memory itself.
- Use it to explain where a card's kept suffix came from or which earlier session covered the trimmed prefix.
- Use `covered-by` when the user asks "what covered this card?" and `base` when they ask "what cards did this one cover?".
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

Resolve sources for a card:

```bash
sessio memory resolve --card-id "<card_id>" --json
```

Read a raw source excerpt for debugging provenance only when needed:

```bash
sessio memory resolve --card-id "<card_id>" --include-source-excerpt --json
```

Inspect cards covered by a base card:

```bash
sessio memory base --card-id "<card_id>" --json
```

Inspect which base card covered a given card:

```bash
sessio memory covered-by --card-id "<card_id>" --json
```

## Interpretation Rules

- Do not call `qmd` directly from this skill.
- Do not parse agent JSONL directly from this skill.
- Prefer concise answers that cite card titles, summaries, and source refs when available.
- Distinguish clearly between "Sessio memory says..." and your own inference.
- If `backendError` is present, report it briefly and suggest running `sessio memory status --json` only if the user wants troubleshooting.
- Do not pass `--include-raw` in normal workflows; it is for debugging the qmd backend.
- Do not use `--include-source-excerpt` in normal workflows; it is for provenance debugging and can return large payloads.
- If `continuationSummary` is present, prefer that human-readable summary over unpacking raw `continuation` fields by hand.

## No-Hit Behavior

When search has no useful memory result, use language like:

```text
Sessio memory did not return a usable match for that query in this project, so I should not rely on prior-session context here.
```

Then continue with normal project inspection if the task can be solved from the current files.
