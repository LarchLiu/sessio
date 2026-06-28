# Computer Use Prompt Refactor Plan

## Summary

This document captures a code-verified comparison between Sessio's current
computer-use / prompt assembly flow and the implementation in
`~/work/cloudgeek/openhanako`, then turns that comparison into a concrete
refactor checklist for Sessio.

The main conclusion is:

- Sessio already has a solid backend split for capability gating, approvals,
  lease/snapshot discipline, and MCP injection.
- The weakest layer is the prompt pipeline itself: prompt assembly is still
  scattered across UI send paths, provider/model quirks do not have a dedicated
  patch layer, and computer use does not yet expose a strong model-visible
  operating contract beyond tool schemas.
- openhanako is stronger here because it separates prompt concerns into stable,
  testable layers: platform note, provider patch, session snapshot, prompt
  layout, computer-use tool contract, and tests.

This plan focuses on importing those prompt-engineering patterns into Sessio
without weakening the current runtime / permission architecture.

## Verified References

### Sessio

- Assistant prompt injection is still assembled in UI send paths:
  - [src/hooks/useChatComposer.ts](../src/hooks/useChatComposer.ts)
  - [src/pages/ChatPage.tsx](../src/pages/ChatPage.tsx)
- Prompt block wrappers already exist:
  - [src/historyMerge.ts](../src/historyMerge.ts)
- Astra orchestration prompt contract is large and centralized:
  - [src-tauri/src/astra/prompt.rs](../src-tauri/src/astra/prompt.rs)
- Computer-use runtime injection is already separated from prompt assembly:
  - [src-tauri/src/agents/runtime/computer_use_runtime.rs](../src-tauri/src/agents/runtime/computer_use_runtime.rs)
- Computer-use host policy split is already strong:
  - [src-tauri/src/computer_use/host.rs](../src-tauri/src/computer_use/host.rs)
- Desktop-control presentation already distinguishes observe / inspect /
  control tiers:
  - [src/desktopControlPermissionPresentation.ts](../src/desktopControlPermissionPresentation.ts)

### openhanako

- Platform prompt note:
  - `~/work/cloudgeek/openhanako/core/platform-prompt.ts`
- Provider/model prompt patches:
  - `~/work/cloudgeek/openhanako/core/provider-prompt-patches.ts`
- Session prompt snapshot:
  - `~/work/cloudgeek/openhanako/core/session-prompt-snapshot.ts`
- Stable prompt layout metadata:
  - `~/work/cloudgeek/openhanako/lib/llm/prompt-layout.ts`
- Computer-use tool contract and filtered action presentation:
  - `~/work/cloudgeek/openhanako/lib/tools/computer-use-tool.ts`
- Contract tests:
  - `~/work/cloudgeek/openhanako/tests/platform-prompt.test.ts`
  - `~/work/cloudgeek/openhanako/tests/provider-prompt-patches.test.ts`
  - `~/work/cloudgeek/openhanako/tests/computer-use-tool.test.ts`
  - `~/work/cloudgeek/openhanako/tests/memory-prompt-layout.test.ts`

## Current Gaps In Sessio

### 1. Prompt assembly is still scattered

Sessio currently composes assistant prompt, visible context, and user input in
multiple send paths. The most obvious examples are:

- [src/hooks/useChatComposer.ts](../src/hooks/useChatComposer.ts)
- [src/pages/ChatPage.tsx](../src/pages/ChatPage.tsx)

This creates three problems:

- logic duplication between entry points
- prompt behavior that is harder to test
- prompt priority that is implicit rather than declared

### 2. Assistant instructions are still treated as visible text blocks

`buildSessioAssistantPromptBlock(...)` is useful as a wrapper, but it is still
assembled as part of the message payload. That keeps Sessio compatible with the
current runtime flow, but it is not yet a real prompt pipeline with clearly
separated layers such as:

- stable system instructions
- session-scoped additions
- provider/model patches
- user-visible context
- user input

### 3. No dedicated provider/model patch layer

openhanako has a narrow, explicit place for provider-specific output contracts.
Sessio currently has no equivalent shared layer for known model quirks or
provider-specific prompt requirements.

### 4. No session prompt snapshot

openhanako freezes prompt inputs at session start. Sessio currently freezes many
runtime decisions elsewhere, but not the full prompt stack. That makes it
harder to answer:

- which assistant prompt was actually used
- which context blocks were injected
- whether computer-use guidance was present
- whether a provider/model patch changed the final prompt

### 5. Computer use lacks a model-visible operating contract

Sessio already exposes a disciplined backend:

- status probe
- start / lease
- snapshot freshness
- control gating
- approvals

But the prompt side still relies mostly on tool names and backend errors. The
model would behave more reliably if Sessio also injected a concise operating
contract such as:

- always call `computer_status` first
- prefer element-based actions over coordinate-based control
- never act on stale `snapshotId`
- refresh app state after a control action
- stop and ask for help when approval or control capability is missing

### 6. Astra prompt contract is too centralized

[src-tauri/src/astra/prompt.rs](../src-tauri/src/astra/prompt.rs) already has
the right direction on contract strictness, but too much of the prompt logic is
still concentrated in one place. The file mixes:

- stable response contract text
- dynamic payload assembly
- language behavior
- task planning semantics
- formatting wrappers

This makes prompt evolution harder to isolate and test.

### 7. Prompt-layer tests are too thin

Sessio already has strong runtime and protocol work, but the prompt layer does
not yet have the same level of dedicated contract coverage that openhanako has
for:

- platform note shape
- provider patch activation
- stable prompt layout semantics
- model-visible computer-use behavior contract

## What To Borrow From openhanako

### A. Prompt layers should be explicit

openhanako separates prompt responsibilities into small modules. Sessio should
do the same.

Target layers:

- platform/environment note
- provider/model patch note
- assistant / thread prompt blocks
- computer-use operating contract
- user-visible context
- final user input

### B. Stable rules should be separated from dynamic input

openhanako's prompt layout makes stable rules easier to cache, audit, and test.
Sessio does not need to copy the exact caching design, but it should adopt the
same separation principle.

### C. Session prompt inputs should be snapshotted

This is the most important structural improvement. Session start should freeze
the prompt stack used for that session, not just the raw message text.

### D. Model-visible computer-use behavior should be intentional

openhanako does not only expose tools; it filters and shapes what the model is
expected to see and do. Sessio should introduce its own explicit behavior
contract rather than relying on backend errors to teach the model how to use the
tool family.

### E. Prompt contracts need dedicated tests

If prompt behavior matters, it needs direct tests. This is one of the clearest
places where openhanako is ahead.

## Refactor Goals

1. Move Sessio prompt composition out of page-level send paths and into a shared
   builder.
2. Make assistant, thread, provider, and computer-use instructions explicit
   prompt layers.
3. Freeze the effective prompt stack at session start with a snapshot.
4. Add a model-visible computer-use operating contract.
5. Split Astra prompt construction into smaller, testable parts.
6. Add prompt-focused tests and diagnostics.

## Refactor Checklist

### Workstream 1: Shared Prompt Builder

Create a single prompt-builder path for session startup and message send.

Suggested outputs:

- one shared builder module for chat/session prompt assembly
- one canonical ordering of prompt layers
- removal of duplicated prompt concatenation from:
  - [src/hooks/useChatComposer.ts](../src/hooks/useChatComposer.ts)
  - [src/pages/ChatPage.tsx](../src/pages/ChatPage.tsx)

Suggested shape:

- `buildAssistantPromptLayer(...)`
- `buildThreadPromptLayer(...)`
- `buildComputerUsePromptLayer(...)`
- `buildProviderPromptPatchLayer(...)`
- `buildVisibleContextLayer(...)`
- `buildFinalInputText(...)`

Acceptance criteria:

- all entry points use the same builder order
- assistant prompt wrapping is no longer duplicated across pages
- the final assembled prompt is testable without UI rendering

### Workstream 2: Session Prompt Snapshot

Introduce a session-scoped snapshot of the effective prompt inputs used when a
runtime session starts.

Suggested contents:

- assistant prompt
- thread prompt blocks
- extra visible context
- computer-use enabled flag
- selected model / effort / permission mode
- provider/model patch ids
- final assembled prompt metadata

Suggested use:

- attach to pending/live session metadata
- reuse in debugging and history views
- surface in diagnostics when a session behaves unexpectedly

Acceptance criteria:

- a session can report which prompt layers were active
- prompt diagnostics do not require reconstructing the send path from UI state

### Workstream 3: Computer Use Prompt Contract

Add a dedicated prompt layer that teaches the model how to use Sessio's
computer-use tools correctly.

Minimum contract:

- call `computer_status` before acting
- use `computer_list_apps` and `computer_start` before inspection/control
- call `computer_get_app_state` before element-targeted actions
- only use the latest `snapshotId`
- refresh app state after input actions
- prefer element-based interaction
- if control is unavailable, stop and explain what is missing

Important note:

This layer is about model behavior, not OS authorization. It should describe the
contract already enforced by:

- [src-tauri/src/computer_use/host.rs](../src-tauri/src/computer_use/host.rs)
- [src-tauri/src/agents/runtime/computer_use_runtime.rs](../src-tauri/src/agents/runtime/computer_use_runtime.rs)

Acceptance criteria:

- computer-use guidance is injected by one shared builder layer
- the guidance matches actual backend rules
- the layer has direct unit coverage

### Workstream 4: Provider / Model Prompt Patches

Add a narrow patch pipeline for model-specific or provider-specific prompt
contracts.

Principles:

- keep patches explicit and small
- activate only when a specific provider/model needs it
- test patch activation and non-activation
- avoid mixing provider quirks into assistant prompt text

Acceptance criteria:

- Sessio has one shared place for provider/model prompt patches
- known quirks do not require ad hoc prompt edits in UI code

### Workstream 5: Astra Prompt Decomposition

Refactor [src-tauri/src/astra/prompt.rs](../src-tauri/src/astra/prompt.rs) into
smaller pieces.

Suggested decomposition:

- stable response contract module
- planner payload builder
- language selection helper
- prompt wrapper / marker helper
- diagnostics formatter

This should keep the strict contract behavior, but make future changes safer and
more testable.

Acceptance criteria:

- stable contract text is separated from dynamic payload assembly
- prompt-building helpers have their own tests
- Astra prompt behavior remains byte-for-byte stable where intended

### Workstream 6: Prompt Diagnostics

Add lightweight diagnostics for prompt composition.

Suggested diagnostics:

- active prompt layers
- provider/model patches applied
- computer-use contract enabled or disabled
- snapshot version
- assistant/thread prompt source identifiers

Acceptance criteria:

- prompt-related bugs can be debugged without reading UI send code
- diagnostics can explain prompt drift across sessions

### Workstream 7: Prompt-Layer Tests

Add tests that mirror openhanako's style of contract coverage.

Priority test targets:

- assistant/thread/computer-use prompt layer ordering
- provider patch activation logic
- session prompt snapshot normalization
- Astra prompt contract assembly
- environment/platform prompt note shape if added

Acceptance criteria:

- prompt assembly has direct unit coverage
- regressions in layer order or patch activation fail tests quickly

## Recommended Execution Order

1. Shared prompt builder
2. Session prompt snapshot
3. Computer-use prompt contract
4. Provider/model patch pipeline
5. Astra prompt decomposition
6. Prompt diagnostics
7. Prompt-layer tests

This order keeps the early work focused on shared infrastructure first, then
adds behavior shaping, then hardens the result with diagnostics and tests.

## Non-Goals

This plan does not require:

- changing the current MCP transport choice
- changing approval semantics
- changing desktop-control permission semantics
- adding new OS input providers
- replacing the current computer-use host policy split

Those areas are already in relatively good shape. The goal here is specifically
to improve prompt reliability, traceability, and testability.

## Expected Outcome

After this refactor, Sessio should keep its current backend policy strengths
while gaining a prompt pipeline that is:

- centralized rather than scattered
- layered rather than implicit
- snapshotted rather than reconstructed
- model-guiding rather than backend-error-driven
- directly testable rather than behaviorally inferred
