# Thread Kind And Process Template Rename Audit

## Rename Map

- Thread kind value: `workflow` -> `process`.
- Rust thread enum: `ThreadKind::Workflow` -> `ThreadKind::Process`.
- Frontend thread union: `"workflow"` -> `"process"`.
- Thread kind i18n key: `thread.kind.workflow` -> `thread.kind.process`.
- Template type: `WorkflowInfo` -> `ProcessTemplateInfo`.
- Template type enum: `WorkflowType` -> `ProcessTemplateType`.
- Template id field: `workflow_id` / `workflowId` -> `process_template_id` / `processTemplateId`.
- Template commands and APIs:
  - `list_workflows` / `listWorkflows` -> `list_process_templates` / `listProcessTemplates`.
  - `create_workflow` / `createWorkflow` -> `create_process_template` / `createProcessTemplate`.
  - `update_workflow` / `updateWorkflow` -> `update_process_template` / `updateProcessTemplate`.
  - `delete_workflow` / `deleteWorkflow` -> `delete_process_template` / `deleteProcessTemplate`.
  - `list_workflow_stages` / `listWorkflowStages` -> `list_process_template_stages` / `listProcessTemplateStages`.
- Template i18n prefix: `workflow.description.*` -> `process_template.description.*`.
- Settings section key: `settings.workflows` -> `settings.process_templates`.

## Persisted SQL Inventory

All product schema references are in `src-tauri/src/store/sqlite.rs`.

- Rename table `workflows` to `process_templates`.
- Rename index `idx_workflows_type_name` to `idx_process_templates_type_name`.
- Rename `projects.workflow_id` to `projects.process_template_id`.
- Rename `assistants.workflow_id` to `assistants.process_template_id`.
- Rename `stages.workflow_id` to `stages.process_template_id`.
- Rename indexes and constraints that reference the renamed table/columns:
  - assistant workflow/project index.
  - stage workflow/project unique/index entries.
  - foreign keys from projects, assistants, and stages.
  - stage builtin/custom check that currently uses `workflow_id`.
- Change `threads.kind` default/check from `workflow` to `process`.
- Update seed SQL, row mappers, helper functions, raw queries, and tests that reference persisted workflow identifiers.
- Keep schema version at v5 and update the v4 -> revised-v5 path to create process-template structures directly.

## Backend Inventory

- `src-tauri/src/models.rs`: template structs/enums and `ThreadKind` serialization/parsing.
- `src-tauri/src/store/mod.rs`: store trait methods and data fields.
- `src-tauri/src/store/cached.rs`: cached store pass-through method names and fields.
- `src-tauri/src/store/sqlite.rs`: SQLite schema, seed helpers, query helpers, CRUD methods, tests.
- `src-tauri/src/lib.rs`: Tauri command names, event names, request payload fields, invoke handler list.
- `src-tauri/src/astra/*`: thread kind branches and tests that currently reject or special-case workflow threads.

## Frontend Inventory

- `src/api.ts`: public types, field names, invoke command names, thread kind union.
- `src/i18n.tsx`: thread kind label, template labels, description keys, Astra/status copy.
- `src/pages/NewChatPage.tsx`: thread mode list, process stage selection, send flow.
- `src/pages/ProjectPage.tsx`: thread kind selectors, stage/thread workflow panels, stage-task branches.
- `src/pages/ThreadPage.tsx` and `src/pages/ThreadMultiSessionChatPage.tsx`: manual stage-task checks.
- `src/pages/SettingsPage.tsx`: template settings section, CRUD UI, labels.
- `src/components/AppSidebar.tsx`, `src/components/AppHeader.tsx`, `src/components/CreateStageDialog.tsx`, and `src/App.tsx`: project template id display/input.
- `src/threadReplayView.ts` and tests: workflow/process grouping by stage.
- CSS class names such as `workflow-list-item` are visual-only and can remain unless touched during UI cleanup.

## Intentional Leftovers

- `src/updater.ts` release-note text about a GitHub Actions release workflow is unrelated and should remain.
- Historical/planning docs may keep quoted old terms where they explain migration history.
- The implementation plan itself intentionally mentions old terms while describing the rename.
- Test titles and fixture strings may mention old terms only when they are historical free text, not product identifiers or persisted schema names.
