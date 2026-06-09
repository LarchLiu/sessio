# Thread Kind And Process Template Rename Plan

## Summary

- Rename the thread kind currently called `workflow` to the internal/API value `process`.
- Show the process thread kind in UI as `Process` / `流程规划`.
- Rename the project-level reusable process template system from workflow terminology to process template terminology.
- Show process templates in UI as `Process template` / `流程模版`.
- Migrate API, frontend types, Rust models, store methods, and SQLite schema together so the naming is consistent across the app.
- Preserve the existing product behavior while making `process` threads execute according to the user-defined stage order.

## Naming Changes

### Thread Kind

- `workflow` -> `process`
- Rust enum: `ThreadKind::Workflow` -> `ThreadKind::Process`
- TypeScript/API union: `"workflow"` -> `"process"`
- UI label:
  - English: `Process`
  - Chinese: `流程规划`
- Compatibility:
  - Released v4 databases do not contain `workflows`, `threads`, or `threads.kind`; those structures were introduced in unreleased v5.
  - Do not keep a legacy `workflow` thread-kind read alias.
  - Revised v5 serializes and writes only `process`.

### Process Template

- `WorkflowInfo` -> `ProcessTemplateInfo`
- `WorkflowType` -> `ProcessTemplateType`
- `workflowId` -> `processTemplateId`
- `workflow_id` -> `process_template_id`
- `listWorkflows` -> `listProcessTemplates`
- `createWorkflow` -> `createProcessTemplate`
- `updateWorkflow` -> `updateProcessTemplate`
- `deleteWorkflow` -> `deleteProcessTemplate`
- `listWorkflowStages` -> `listProcessTemplateStages`
- i18n:
  - `workflow.*` -> `process_template.*`
  - `settings.workflows` -> `settings.process_templates`

## Implementation Changes

### Rust Models, Store, And Commands

- Rename project-level template models and command handlers to `ProcessTemplate*`.
- Rename store trait methods and SQLite helper functions from workflow terminology to process template terminology.
- Update all thread kind branches from `ThreadKind::Workflow` to `ThreadKind::Process`.
- Keep project stage behavior unchanged: a project still initializes and manages stages from a reusable template.

### SQLite Migration

- Schema version policy:
  - keep the database schema version at v5 for this work;
  - do not add v6/v7/v8 or any later migration version for this rename;
  - v5 has not been truly released, so existing v5 development databases do not need compatibility support;
  - update the v5 bootstrap/current schema directly and provide compatibility only for upgrading v4 databases to the revised v5 schema.
- Released v4 databases do not have the template/thread tables introduced in v5, so there is no released persisted `workflow` table, `workflow_id` column, or `threads.kind = 'workflow'` value to rewrite.
- Rename `workflows` table to `process_templates`.
- Rename all referencing columns to `process_template_id` in:
  - `projects`
  - `assistants`
  - `stages`
- Rebuild affected tables, indexes, and foreign keys where SQLite cannot cleanly rename constraints.
- Before writing the migration, scan the full SQLite schema for every persisted `workflow` identifier and migrate all matches consistently:
  - tables, columns, indexes, foreign keys, checks, seed data helpers, and raw SQL query strings;
  - there is no separate `workflow_stages` table in the current schema, but `stages.workflow_id` represents template stages and must be migrated;
  - add a regression test that fails if a current-schema database still contains a persisted workflow table/column name after migration, except intentional UI/i18n legacy aliases.
- Change `threads.kind` default/check constraint from `workflow` to `process`.
- Create revised-v5 thread rows with `process` directly; do not add compatibility logic for unreleased v5 development rows with `workflow`.

### Frontend

- Rename API types/functions and all call sites to `ProcessTemplate`.
- Keep Settings and Project UI workflows behaviorally the same, changing labels to `Process template` / `流程模版`.
- New Chat thread mode list uses `process`, displayed as `Process` / `流程规划`.
- Replay, timeline, thread detail, and manual stage-task checks use `thread.kind === "process"`.

## Process Execution

- New `process` threads create selected thread stages in the user-defined order.
- After creation, New Chat immediately starts an Astra run for the process thread. This is an explicit v1 UX decision, matching teamwork/brainstorm/debate creation behavior: pressing send creates the thread and starts execution without a second confirmation screen.
- Process preview/confirmation before execution is out of scope for this rename/execution pass; add it later as a separate UX feature if needed.
- `process` runs always use the rule-based deterministic backend, regardless of configured Astra planner provider.
- The process planner creates one `PlanRoundMode::Sequential` plan round:
  - stages sorted by thread stage order;
  - assistants sorted by stage assistant order;
  - one stage-bound task per assistant;
  - tasks execute fully serially.
- Empty-assistant stages are manual checkpoints:
  - pause the run;
  - mark the stage `needs_review`;
  - allow a later run to resume from the first non-completed/non-skipped stage.
- When all tasks for a stage complete, mark the stage `completed`.
- If any task fails, cancels, or errors:
  - mark the current stage `blocked`;
  - stop the run with a terminal errored/cancelled status that preserves the task error;
  - do not introduce new stage statuses such as `failed` or `error` in this pass.

## Resume Rules

- Resume is manual: a user starts another process Astra run from the thread UI or New Chat flow; there is no automatic background retry.
- A resumed run starts from the first enabled thread stage whose status is not `completed` and not `skipped`, ordered by thread stage order.
- If a prior run failed and left a stage `blocked`, the next run resumes from that blocked stage unless the user manually changes it to `skipped` or `completed`.
- `skipped` is a user-controlled stage status. The process planner does not automatically mark stages skipped.
- Existing completed/skipped stages remain untouched during resume; newly generated tasks only cover the remaining stages.

## Implementation Phases

### Phase 1: Audit And Rename Map

- Goal: build the exact rename inventory before touching schema or runtime behavior.
- Search and classify every occurrence of:
  - thread kind terms: `workflow`, `ThreadKind::Workflow`, `thread.kind.workflow`, `kind === "workflow"`;
  - template terms: `WorkflowInfo`, `WorkflowType`, `workflowId`, `workflow_id`, `listWorkflows`, `listWorkflowStages`, `settings.workflows`, `workflow.description.*`;
  - persisted SQL identifiers: `workflows`, `workflow_id`, workflow-related index names, foreign key references, seed helpers, and raw SQL strings.
- Separate intentional non-product uses that should remain untouched, such as GitHub Actions release workflow text in README/changelog content.
- Deliverable: a short implementation checklist or commit note mapping old names to new names and listing any intentional leftovers.
- Acceptance: no implementation decision remains about which identifiers are renamed, aliased, or intentionally retained.

### Phase 2: Revised v5 SQLite Schema

- Goal: make the database schema use `process_templates` and `process` without increasing the schema version.
- Update the v5 bootstrap/current schema directly:
  - `workflows` -> `process_templates`;
  - `workflow_id` -> `process_template_id` in `projects`, `assistants`, and `stages`;
  - related index names and foreign key references use process-template naming;
  - `threads.kind` default/check uses `process` instead of `workflow`.
- Update only the v4 -> revised v5 migration path:
  - preserve existing v4 data;
  - create the new process-template/project/stage/thread tables using process naming directly;
  - do not migrate unreleased v5 development rows.
- Do not add any schema migration version beyond v5. Existing unreleased v5 development databases are not part of the compatibility target.
- Acceptance:
  - a fresh revised-v5 database contains no persisted workflow table/column names;
  - a synthetic v4 database upgrades to revised v5 with existing v4 data intact and new process-template tables available;
  - thread kind values written by the store are `process`.

### Phase 3: Rust Model, Store, And Command Rename

- Goal: move backend public types and commands to process terminology without carrying unreleased workflow aliases.
- Rename model types and fields:
  - `WorkflowInfo` -> `ProcessTemplateInfo`;
  - `WorkflowType` -> `ProcessTemplateType`;
  - `workflow_id` fields -> `process_template_id`.
- Rename store trait methods, SQLite helpers, seed helpers, and Tauri commands:
  - `list_workflows` -> `list_process_templates`;
  - `create_workflow` -> `create_process_template`;
  - `update_workflow` -> `update_process_template`;
  - `delete_workflow` -> `delete_process_template`;
  - `list_workflow_stages` -> `list_process_template_stages`.
- Rename `ThreadKind::Workflow` to `ThreadKind::Process`.
- Update `ThreadKind::from_db_str` to accept only released/current thread kind values; remove `workflow` because v4 never persisted it and v5 is unreleased.
- Update Rust tests that construct workflow/process threads to use the new naming.
- Acceptance:
  - Rust compiles with no backend references to old workflow template APIs except schema-history notes or unrelated GitHub Actions wording;
  - Tauri command names exposed to the frontend are process-template named;
  - no legacy `workflow` thread-kind parser test remains.

### Phase 4: Frontend API And UI Rename

- Goal: align TypeScript API, frontend state, and UI copy with process/process-template terminology.
- Rename frontend API types and fields:
  - `WorkflowInfo` -> `ProcessTemplateInfo`;
  - `WorkflowType` -> `ProcessTemplateType`;
  - `workflowId` -> `processTemplateId`;
  - thread kind union member `"workflow"` -> `"process"`.
- Rename frontend API functions and all call sites:
  - `listWorkflows` -> `listProcessTemplates`;
  - `createWorkflow` -> `createProcessTemplate`;
  - `updateWorkflow` -> `updateProcessTemplate`;
  - `deleteWorkflow` -> `deleteProcessTemplate`;
  - `listWorkflowStages` -> `listProcessTemplateStages`.
- Update i18n labels:
  - thread kind: `Process` / `流程规划`;
  - template UI: `Process template` / `流程模版`;
  - process template descriptions use `process_template.description.*`.
- Update thread-kind branches in pages, replay grouping, timeline views, stage-task mode, and validation from `"workflow"` to `"process"`.
- Acceptance:
  - frontend typecheck catches no old API usage;
  - Settings and Project pages still list, create, edit, and delete reusable templates;
  - New Chat can select the `process` thread kind and shows the correct localized labels.

### Phase 5: Process Planner And Runtime Semantics

- Goal: implement deterministic process execution on top of existing Astra sequential plan rounds.
- Force `ThreadKind::Process` to use the rule-based deterministic backend, not configured runtime-agent or Astra PI planner backends.
- Update deterministic planning for process threads:
  - find remaining enabled stages using resume rules;
  - generate tasks in stage order, then stage assistant order;
  - create a single `PlanRoundMode::Sequential` round;
  - bind each generated task to the target thread stage and assistant snapshot.
- Implement manual checkpoint behavior:
  - if the next remaining stage has no assistants, mark it `needs_review`;
  - complete/pause the run with a human-checkpoint reason;
  - do not generate tasks for later stages until the user resumes.
- Implement stage completion/failure closure:
  - when all tasks for a stage complete, mark the stage `completed`;
  - on failed/cancelled/errored task, mark the stage `blocked` and stop the run terminally.
- Acceptance:
  - a process run dispatches exactly one task at a time;
  - generated task order is stable and matches user stage/assistant order;
  - process runs resume from the first enabled stage not `completed` or `skipped`;
  - blocked stages are retried on the next manual run unless the user changes their status.

### Phase 6: New Chat And Thread UX

- Goal: wire process creation and execution into the user-facing flow.
- New Chat creates a `process` thread, adds selected stages in user-defined order, then immediately calls `createAstraRun(thread.id, null)`.
- This send action is the confirmation point; no separate preview/confirm screen is added in v1.
- Thread pages allow manual restart/resume by starting another process Astra run when no active run exists.
- Existing manual stage-task UI continues to work for process threads and uses `thread.kind === "process"`.
- Acceptance:
  - creating a process thread from New Chat navigates to the thread and starts execution;
  - users can resume a paused/blocked process by starting another run;
  - active-run handling prevents duplicate simultaneous process runs.

### Phase 7: Compatibility Cleanup And Verification

- Goal: remove accidental leftovers and prove the rename/execution path works end to end.
- Run targeted searches for old product identifiers after implementation:
  - `WorkflowInfo`, `WorkflowType`, `workflowId`, `workflow_id`, `listWorkflows`, `listWorkflowStages`, `ThreadKind::Workflow`, `kind === "workflow"`;
  - only schema-history notes for v4/v5 or unrelated GitHub Actions wording may remain.
- Add/update tests listed below before final verification.
- Run:
  - `pnpm run check`;
  - `pnpm run build`;
  - relevant Rust tests for store migration, thread kind parsing, process planning, and process run progression.
- Acceptance:
  - all verification commands pass;
  - no unintended old workflow terminology remains in persisted schema, API, or user-facing process/template UI;
  - test coverage proves fresh revised-v5 schema and v4 -> revised-v5 migration.

## Test Plan

### Rust

- v4 -> revised v5 migration preserves existing v4 data and creates the process-template schema directly.
- Existing unreleased v5 development databases are not part of the compatibility matrix.
- New thread kind writes serialize as `process`.
- Process planner emits sequential tasks in exact stage/assistant order.
- Empty-assistant stage pauses as a manual checkpoint.
- Sequential completion advances stage status and run status correctly.

### TypeScript

- API type/function rename compiles across all call sites.
- New Chat creates `process` threads, adds ordered stages, and starts Astra.
- Replay groups `process` sessions by stage.
- UI labels show:
  - `Process` / `流程规划`
  - `Process template` / `流程模版`

### Verification

- Run `pnpm run check`.
- Run `pnpm run build`.
- Run relevant Rust tests for `src-tauri`.
