import { type ReactNode, useEffect, useMemo, useState } from "react";
import { DragDropProvider, type DragEndEvent } from "@dnd-kit/react";
import { isSortable, useSortable } from "@dnd-kit/react/sortable";
import { ArrowLeft, Bot, Check, ChevronDown, GripVertical, Languages, ListChecks, Monitor, Moon, Pencil, Plus, RefreshCw, Settings, Sun, Trash2, Workflow } from "lucide-react";
import type { AgentInfo, AssistantAgentInfo, AssistantInfo, AssistantType, ProjectStageInfo, StageAssistantInfo, WorkflowInfo } from "../api";
import {
  createAssistant,
  createProjectStage,
  createWorkflow,
  deleteAssistant,
  deleteProjectStage,
  listAgents,
  listAssistants,
  listWorkflowStages,
  listWorkflows,
  updateAssistant,
  updateProjectStage,
  updateProjectStageAssistants,
} from "../api";
import AssistantAgentSelector, { dbAgentsAsRuntimeAgents, defaultAssistantAgent } from "../components/AssistantAgentSelector";
import InlineMenuSelect from "../components/InlineMenuSelect";
import ScrollArea from "../components/ScrollArea";
import Tooltip from "../components/Tooltip";
import { type Lang, useI18n } from "../i18n";
import type { ThemeMode } from "../theme";

type SettingsSection = "general" | "assistants" | "workflows";

export default function SettingsPage({
  lang,
  onLangChange,
  themeMode,
  onThemeModeChange,
  rebuilding,
  indexing,
  onBack,
  onError,
  onRebuildFinished,
}: {
  lang: Lang;
  onLangChange: (lang: Lang) => void;
  themeMode: ThemeMode;
  onThemeModeChange: (mode: ThemeMode) => void;
  rebuilding: boolean;
  indexing: boolean;
  onBack: () => void;
  onError: (error: string | null) => void;
  onRebuildFinished: () => Promise<void> | void;
}) {
  const { t } = useI18n();
  const [section, setSection] = useState<SettingsSection>("general");

  const navItems = [
    { id: "general" as const, label: t("settings.general"), icon: Settings },
    { id: "assistants" as const, label: t("assistant.title"), icon: Bot },
    { id: "workflows" as const, label: t("settings.workflows"), icon: Workflow },
  ];
  const sectionTitle = navItems.find((item) => item.id === section)?.label ?? t("settings.general");

  return (
    <div className="flex h-screen text-body text-ink">
      <aside className="flex w-[300px] shrink-0 flex-col bg-surface-sidebar">
        <header data-tauri-drag-region className="h-12 shrink-0 select-none bg-surface-sidebar" />
        <div className="min-h-0 flex-1 px-2">
          <button
            type="button"
            onClick={onBack}
            data-tauri-drag-region="false"
            className="mb-3 flex h-8 w-full items-center gap-2 rounded-md px-2.5 text-left text-body-sm text-ink/55 transition hover:bg-ink/5 hover:text-ink"
          >
            <ArrowLeft className="h-4 w-4" />
            {t("settings.back_to_app")}
          </button>
          <nav data-tauri-drag-region="false" className="grid gap-1">
            {navItems.map(({ id, label, icon: Icon }) => {
              const active = section === id;
              return (
                <button
                  key={id}
                  type="button"
                  onClick={() => setSection(id)}
                  className={
                  "flex h-8 items-center gap-2 rounded-md px-2.5 text-left text-body-sm transition " +
                  (active ? "bg-ink/[0.065] text-ink/88" : "text-ink/72 hover:bg-ink/5 hover:text-ink")
                  }
                >
                  <Icon className="h-4 w-4 shrink-0" />
                  <span className="truncate">{label}</span>
                </button>
              );
            })}
          </nav>
        </div>
      </aside>
      <main className="relative flex min-w-0 flex-1 flex-col">
        <header data-tauri-drag-region className="grid h-12 shrink-0 select-none grid-cols-3 items-center border-b border-ink/[0.12] bg-surface px-5">
          <h1 data-tauri-drag-region className="col-start-2 justify-self-center truncate text-title font-semibold text-ink/85">{sectionTitle}</h1>
        </header>
        <ScrollArea className="min-h-0 flex-1 bg-surface-panel" viewportClassName="px-10 pb-16 pt-6">
          <div className="mx-auto max-w-[840px]">
            {section === "general" && (
              <GeneralSettings
                lang={lang}
                themeMode={themeMode}
                rebuilding={rebuilding}
                indexing={indexing}
                onLangChange={onLangChange}
                onThemeModeChange={onThemeModeChange}
                onError={onError}
                onRebuildFinished={onRebuildFinished}
              />
            )}
            {section === "assistants" && <AssistantsSettings onError={onError} />}
            {section === "workflows" && <WorkflowsSettings onError={onError} />}
          </div>
        </ScrollArea>
      </main>
    </div>
  );
}

function GeneralSettings({
  lang,
  themeMode,
  rebuilding,
  indexing,
  onLangChange,
  onThemeModeChange,
  onError,
  onRebuildFinished,
}: {
  lang: Lang;
  themeMode: ThemeMode;
  rebuilding: boolean;
  indexing: boolean;
  onLangChange: (lang: Lang) => void;
  onThemeModeChange: (mode: ThemeMode) => void;
  onError: (error: string | null) => void;
  onRebuildFinished: () => Promise<void> | void;
}) {
  const { t } = useI18n();
  return (
    <section className="min-w-0 max-w-full">
      <SettingsGroup title={t("settings.appearance")} flush>
        <SettingsRow icon={<Languages className="h-4 w-4" />} label={t("sidebar.language")} description={t("settings.language_description")}>
          <SegmentedControl
            value={lang}
            options={[
              { value: "en", label: "EN" },
              { value: "zh", label: "中" },
            ]}
            onChange={(value) => onLangChange(value as Lang)}
          />
        </SettingsRow>
        <SettingsRow icon={<Monitor className="h-4 w-4" />} label={t("sidebar.theme")} description={t("settings.theme_description")}>
          <ThemeSelector mode={themeMode} onChange={onThemeModeChange} />
        </SettingsRow>
      </SettingsGroup>
      <SettingsGroup title={t("settings.index")} flush>
        <SettingsRow icon={<RefreshCw className="h-4 w-4" />} label={t("sidebar.rebuild_index")} description={t("settings.rebuild_description")}>
          <button
            type="button"
            disabled={indexing}
            onClick={() => {
              import("../api")
                .then(({ rebuildSessionIndex }) => rebuildSessionIndex())
                .catch((err) => onError(String(err)))
                .finally(() => {
                  void onRebuildFinished();
                });
            }}
            className="inline-flex h-8 items-center gap-2 rounded-md border border-ink/10 bg-ink/[0.035] px-3 text-body-sm text-ink/70 transition hover:bg-ink/[0.06] hover:text-ink disabled:opacity-45"
          >
            <RefreshCw className={"h-3.5 w-3.5 " + (rebuilding ? "animate-spin" : "")} />
            {indexing ? t("sidebar.status_indexing") : t("sidebar.rebuild_index")}
          </button>
        </SettingsRow>
      </SettingsGroup>
    </section>
  );
}

function AssistantsSettings({ onError }: { onError: (error: string | null) => void }) {
  const { t } = useI18n();
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [workflows, setWorkflows] = useState<WorkflowInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [name, setName] = useState("");
  const [workflowId, setWorkflowId] = useState("code");
  const [systemPrompt, setSystemPrompt] = useState("");
  const runtimeAgents = useMemo(() => dbAgentsAsRuntimeAgents(agents), [agents]);
  const [agentDraft, setAgentDraft] = useState<AssistantAgentInfo>(() => ({
    id: "",
    name: "",
    model: "",
    mode: "",
    effort: "",
  }));

  const reload = async () => {
    setLoading(true);
    try {
      const [assistantRows, agentRows, workflowRows] = await Promise.all([
        listAssistants(null),
        listAgents(),
        listWorkflows(),
      ]);
      setAssistants(assistantRows);
      setAgents(agentRows);
      setWorkflows(workflowRows);
      setWorkflowId((current) => workflowRows.some((workflow) => workflow.id === current) ? current : workflowRows[0]?.id ?? "code");
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  useEffect(() => {
    if (runtimeAgents.some((agent) => agent.agent === agentDraft.id)) return;
    if (runtimeAgents[0]) setAgentDraft(defaultAssistantAgent(runtimeAgents[0]));
  }, [agentDraft.id, runtimeAgents]);

  const create = async () => {
    const nextName = name.trim();
    if (!nextName) return;
    try {
      const assistant = await createAssistant({
        name: nextName,
        agent: agentDraft,
        systemPrompt,
        type: "custom" satisfies AssistantType,
        workflowId,
      });
      setAssistants((prev) => [assistant, ...prev]);
      setName("");
      setSystemPrompt("");
    } catch (err) {
      onError(String(err));
    }
  };

  const builtin = assistants.filter((assistant) => assistant.type === "builtin");
  const custom = assistants.filter((assistant) => assistant.type === "custom");

  return (
    <section>
      <SettingsGroup title={t("assistant.builtin")}>
        <div className="grid gap-2">
          {builtin.map((assistant) => (
            <AssistantCard key={assistant.id} assistant={assistant} agents={agents} readonly onUpdated={() => {}} onDeleted={() => {}} onError={onError} />
          ))}
          {!loading && builtin.length === 0 && <EmptyState label={t("assistant.empty")} />}
        </div>
      </SettingsGroup>
      <SettingsGroup title={t("assistant.add")}>
        <div className="grid gap-2">
          <div className="grid grid-cols-[minmax(0,1fr)_180px] gap-2">
            <input value={name} onChange={(event) => setName(event.target.value)} placeholder={t("assistant.name")} className={inputClassName} />
            <InlineMenuSelect value={workflowId} options={workflows.map((workflow) => ({ value: workflow.id, label: workflow.name }))} onChange={setWorkflowId} ariaLabel={t("project.workflowId")} className="h-9 max-w-none rounded-md border border-input-border/[0.16] bg-input px-2" minMenuWidth={220} />
          </div>
          <AssistantAgentSelector agent={agentDraft} agents={agents} onChange={setAgentDraft} />
          <textarea value={systemPrompt} onChange={(event) => setSystemPrompt(event.target.value)} placeholder={t("assistant.system_prompt")} rows={4} className={textareaClassName} />
          <button type="button" onClick={() => void create()} disabled={!name.trim() || !agentDraft.id} className="inline-flex h-8 w-fit items-center gap-1.5 rounded-md bg-ink px-3 text-body-sm font-medium text-[rgb(var(--color-bg-panel))] disabled:opacity-35">
            <Plus className="h-4 w-4" />
            {t("assistant.add")}
          </button>
        </div>
      </SettingsGroup>
      <SettingsGroup title={t("assistant.custom")}>
        <div className="grid gap-2">
          {custom.map((assistant) => (
            <AssistantCard
              key={assistant.id}
              assistant={assistant}
              agents={agents}
              readonly={false}
              onUpdated={(next) => setAssistants((prev) => prev.map((item) => item.id === next.id ? next : item))}
              onDeleted={(id) => setAssistants((prev) => prev.filter((item) => item.id !== id))}
              onError={onError}
            />
          ))}
          {!loading && custom.length === 0 && <EmptyState label={t("assistant.empty")} />}
        </div>
      </SettingsGroup>
    </section>
  );
}

function WorkflowsSettings({ onError }: { onError: (error: string | null) => void }) {
  const { t } = useI18n();
  const [workflows, setWorkflows] = useState<WorkflowInfo[]>([]);
  const [selectedWorkflowId, setSelectedWorkflowId] = useState("code");
  const [stages, setStages] = useState<ProjectStageInfo[]>([]);
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);
  const [newWorkflowName, setNewWorkflowName] = useState("");
  const [newWorkflowDescription, setNewWorkflowDescription] = useState("");
  const [newStageName, setNewStageName] = useState("");
  const [newStageDescription, setNewStageDescription] = useState("");
  const [loading, setLoading] = useState(true);

  const selectedWorkflow = workflows.find((workflow) => workflow.id === selectedWorkflowId) ?? workflows[0] ?? null;
  const availableAssistants = assistants.filter((assistant) => assistant.projectId === null && (assistant.workflowId === null || assistant.workflowId === selectedWorkflowId));

  const reloadAll = async () => {
    setLoading(true);
    try {
      const [workflowRows, assistantRows] = await Promise.all([listWorkflows(), listAssistants(null)]);
      setWorkflows(workflowRows);
      setAssistants(assistantRows);
      setSelectedWorkflowId((current) => workflowRows.some((workflow) => workflow.id === current) ? current : workflowRows[0]?.id ?? "code");
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  };

  const reloadStages = async (workflowId: string) => {
    try {
      setStages(await listWorkflowStages(workflowId));
    } catch (err) {
      onError(String(err));
    }
  };

  useEffect(() => {
    void reloadAll();
  }, []);

  useEffect(() => {
    if (selectedWorkflowId) void reloadStages(selectedWorkflowId);
  }, [selectedWorkflowId]);

  const createNewWorkflow = async () => {
    const name = newWorkflowName.trim();
    if (!name) return;
    try {
      const workflow = await createWorkflow(name, newWorkflowDescription);
      setWorkflows((prev) => [...prev, workflow]);
      setSelectedWorkflowId(workflow.id);
      setNewWorkflowName("");
      setNewWorkflowDescription("");
    } catch (err) {
      onError(String(err));
    }
  };

  const refreshStages = async () => {
    if (selectedWorkflowId) await reloadStages(selectedWorkflowId);
  };

  const createStage = async () => {
    const name = newStageName.trim();
    if (!name || !selectedWorkflowId) return;
    try {
      const stage = await createProjectStage("", name, newStageDescription, selectedWorkflowId);
      setStages((prev) => [...prev, stage].sort((a, b) => a.order - b.order));
      setNewStageName("");
      setNewStageDescription("");
    } catch (err) {
      onError(String(err));
    }
  };
  const workflowDescription = (workflow: WorkflowInfo) => {
    if (!workflow.description) return t("settings.workflow_no_description");
    return workflow.type === "builtin" ? t(workflow.description) : workflow.description;
  };

  return (
    <section>
      <div className="grid grid-cols-[240px_minmax(0,1fr)] gap-5">
        <div className="min-w-0">
          <SettingsGroup title={t("settings.workflows")} flush>
            <div className="divide-y divide-card-border/10">
              {workflows.map((workflow) => (
                <Tooltip key={workflow.id} content={workflowDescription(workflow)} placement="right">
                  <button
                    type="button"
                    onClick={() => setSelectedWorkflowId(workflow.id)}
                    className={"workflow-list-item flex h-10 w-full min-w-0 items-center justify-between px-3 text-left text-body-sm transition " + (workflow.id === selectedWorkflowId ? "workflow-list-item-active" : "")}
                  >
                    <span className="truncate">{workflow.name}</span>
                    <span className="rounded bg-card-chip/8 px-1.5 py-0.5 text-meta text-card-chip-fg/55">{workflow.type}</span>
                  </button>
                </Tooltip>
              ))}
            </div>
          </SettingsGroup>
          <SettingsGroup title={t("settings.add_workflow")}>
            <div className="grid gap-2">
              <input value={newWorkflowName} onChange={(event) => setNewWorkflowName(event.target.value)} placeholder={t("settings.workflow_name")} className={inputClassName} />
              <textarea value={newWorkflowDescription} onChange={(event) => setNewWorkflowDescription(event.target.value)} placeholder={t("settings.workflow_description")} rows={3} className={textareaClassName} />
              <button type="button" onClick={() => void createNewWorkflow()} disabled={!newWorkflowName.trim()} className="inline-flex h-8 items-center justify-center gap-1.5 rounded-md bg-ink px-3 text-body-sm font-medium text-[rgb(var(--color-bg-panel))] disabled:opacity-35">
                <Plus className="h-4 w-4" />
                {t("settings.add_workflow")}
              </button>
            </div>
          </SettingsGroup>
        </div>
        <div className="min-w-0">
          {selectedWorkflow && (
            <WorkflowEditor
              stages={stages}
              assistants={availableAssistants}
              loading={loading}
              newStageName={newStageName}
              newStageDescription={newStageDescription}
              onNewStageNameChange={setNewStageName}
              onNewStageDescriptionChange={setNewStageDescription}
              onCreateStage={createStage}
              onStageUpdated={(stage) => setStages((prev) => prev.map((item) => item.id === stage.id ? stage : item).sort((a, b) => a.order - b.order))}
              onStagesReload={refreshStages}
              onStageDeleted={(id) => setStages((prev) => prev.filter((stage) => stage.id !== id))}
              onError={onError}
            />
          )}
        </div>
      </div>
    </section>
  );
}

function WorkflowEditor({
  stages,
  assistants,
  loading,
  newStageName,
  newStageDescription,
  onNewStageNameChange,
  onNewStageDescriptionChange,
  onCreateStage,
  onStageUpdated,
  onStagesReload,
  onStageDeleted,
  onError,
}: {
  stages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  loading: boolean;
  newStageName: string;
  newStageDescription: string;
  onNewStageNameChange: (value: string) => void;
  onNewStageDescriptionChange: (value: string) => void;
  onCreateStage: () => Promise<void>;
  onStageUpdated: (stage: ProjectStageInfo) => void;
  onStagesReload: () => Promise<void>;
  onStageDeleted: (stageId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();

  const moveStage = async (stage: ProjectStageInfo, direction: -1 | 1) => {
    const ordered = [...stages].sort((a, b) => a.order - b.order);
    const index = ordered.findIndex((item) => item.id === stage.id);
    const next = ordered[index + direction];
    if (!next) return;
    try {
      onStageUpdated(await updateProjectStage(stage.id, { order: next.order }));
      await onStagesReload();
    } catch (err) {
      onError(String(err));
    }
  };

  const reorderStage = async (stage: ProjectStageInfo, target: ProjectStageInfo) => {
    if (stage.id === target.id) return;
    try {
      onStageUpdated(await updateProjectStage(stage.id, { order: target.order }));
      await onStagesReload();
    } catch (err) {
      onError(String(err));
    }
  };

  const handleDragEnd = (event: DragEndEvent) => {
    if (event.canceled) return;
    const { source } = event.operation;
    if (!isSortable(source)) return;
    const from = source.initialIndex;
    const to = source.index;
    if (from === to) return;
    const ordered = [...stages].sort((a, b) => a.order - b.order);
    const stage = ordered[from];
    const target = ordered[to];
    if (stage && target) void reorderStage(stage, target);
  };

  return (
    <>
      <SettingsGroup title={t("stage.project_stages")}>
        <DragDropProvider onDragEnd={handleDragEnd}>
          <div className="grid gap-2">
            {stages.map((stage, index) => (
              <StageTemplateRow
                key={stage.id}
                stage={stage}
                index={index}
                assistants={assistants}
                onMove={moveStage}
                onUpdated={onStageUpdated}
                onDeleted={onStageDeleted}
                onError={onError}
              />
            ))}
            {!loading && stages.length === 0 && <EmptyState label={t("stage.empty")} />}
          </div>
        </DragDropProvider>
      </SettingsGroup>
      <SettingsGroup title={t("stage.add")}>
        <div className="grid gap-2">
          <input value={newStageName} onChange={(event) => onNewStageNameChange(event.target.value)} placeholder={t("stage.name")} className={inputClassName} />
          <textarea value={newStageDescription} onChange={(event) => onNewStageDescriptionChange(event.target.value)} placeholder={t("stage.description")} rows={3} className={textareaClassName} />
          <button type="button" onClick={() => void onCreateStage()} disabled={!newStageName.trim()} className="inline-flex h-8 w-fit items-center gap-1.5 rounded-md bg-ink px-3 text-body-sm font-medium text-[rgb(var(--color-bg-panel))] disabled:opacity-35">
            <Plus className="h-4 w-4" />
            {t("stage.add")}
          </button>
        </div>
      </SettingsGroup>
    </>
  );
}

function StageTemplateRow({
  stage,
  index,
  assistants,
  onMove,
  onUpdated,
  onDeleted,
  onError,
}: {
  stage: ProjectStageInfo;
  index: number;
  assistants: AssistantInfo[];
  onMove: (stage: ProjectStageInfo, direction: -1 | 1) => Promise<void>;
  onUpdated: (stage: ProjectStageInfo) => void;
  onDeleted: (stageId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const { handleRef, isDragSource, isDropTarget, ref } = useSortable({
    id: stage.id,
    index,
    group: "workflow-stages",
    transition: {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
      idle: true,
    },
  });
  const custom = stage.type === "custom";
  const [name, setName] = useState(stage.name ?? "");
  const [description, setDescription] = useState(stage.description ?? "");
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    setName(stage.name ?? "");
    setDescription(stage.description ?? "");
  }, [stage]);

  const label = stage.type === "builtin" && stage.kind ? t(`stage.type.${stage.kind}`) : stage.name || t("stage.custom");
  const selectedAssistantIds = stage.assistants.map((assistant) => assistant.assistantId);
  const assistantOptions = assistants.map((assistant) => ({
    value: assistant.id,
    label: assistant.name,
    description: `${assistant.agent.name} · ${assistant.agent.model}`,
  }));

  const save = async () => {
    if (!custom) return;
    try {
      onUpdated(await updateProjectStage(stage.id, { name, description: description || null }));
    } catch (err) {
      onError(String(err));
    }
  };

  const toggleAssistant = async (assistantId: string) => {
    const selected = new Set(selectedAssistantIds);
    if (selected.has(assistantId)) selected.delete(assistantId);
    else selected.add(assistantId);
    try {
      onUpdated(await updateProjectStageAssistants(stage.id, Array.from(selected)));
    } catch (err) {
      onError(String(err));
    }
  };

  const remove = async () => {
    if (!custom) return;
    try {
      await deleteProjectStage(stage.id);
      onDeleted(stage.id);
    } catch (err) {
      onError(String(err));
    }
  };
  return (
    <div
      ref={ref}
      data-stage-template-id={stage.id}
      className={
        "relative rounded-lg border p-2 transition duration-150 " +
        (isDragSource
          ? "z-20 cursor-grabbing border-card-border/25 bg-card shadow-[0_16px_36px_rgba(0,0,0,0.24)]"
          : isDropTarget
            ? "border-card-border/45 bg-card-active shadow-[inset_3px_0_0_rgb(var(--color-card-fg)/0.38),0_8px_24px_rgba(0,0,0,0.18)]"
            : "border-card-border/[0.12] bg-card")
      }
    >
      <div className="flex items-start gap-2">
        <button ref={handleRef} type="button" className="mt-1.5 cursor-grab touch-none rounded p-0.5 text-card-subtle/35 hover:bg-card-action-hover/5 hover:text-card-fg/60 active:cursor-grabbing">
          <GripVertical className="h-4 w-4" />
        </button>
        <button type="button" onClick={() => setExpanded((value) => !value)} className="min-w-0 flex-1 text-left">
          <div className="flex min-w-0 items-center gap-2">
            <ListChecks className="h-4 w-4 shrink-0 text-card-icon/55" />
            <span className="truncate text-body-sm font-medium text-card-fg/85">{label}</span>
            <span className="rounded bg-card-chip/8 px-1.5 py-0.5 text-meta text-card-chip-fg/55">{stage.type}</span>
          </div>
          {stage.description && <div className="mt-1 line-clamp-2 text-caption leading-relaxed text-card-muted/60">{stage.description}</div>}
          <AssistantSummary assistants={stage.assistants} />
        </button>
        <div className="flex shrink-0 items-center gap-1">
          <>
            <button type="button" onClick={() => void onMove(stage, -1)} className="rounded p-1 text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75"><ChevronDown className="h-4 w-4 rotate-180" /></button>
            <button type="button" onClick={() => void onMove(stage, 1)} className="rounded p-1 text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75"><ChevronDown className="h-4 w-4" /></button>
            {custom && (
              <button type="button" onClick={() => void remove()} className="rounded p-1 text-card-subtle/45 hover:bg-status-error/10 hover:text-status-error"><Trash2 className="h-4 w-4" /></button>
            )}
          </>
          <button type="button" onClick={() => setExpanded((value) => !value)} className="rounded p-1 text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75"><Pencil className="h-4 w-4" /></button>
        </div>
      </div>
      {expanded && (
        <div className="mt-3 grid gap-2 border-t border-card-border/10 pt-3">
          {custom && (
            <>
              <input value={name} onChange={(event) => setName(event.target.value)} onBlur={() => void save()} className={inputClassName} />
              <textarea value={description} onChange={(event) => setDescription(event.target.value)} onBlur={() => void save()} rows={3} className={textareaClassName} />
            </>
          )}
          <div>
            <div className="mb-1.5 text-caption text-card-muted/60">{t("assistant.title")}</div>
            <div className="flex flex-wrap gap-1.5">
              {assistantOptions.map((option) => {
                const active = selectedAssistantIds.includes(option.value);
                return (
                  <button key={option.value} type="button" onClick={() => void toggleAssistant(option.value)} className={"inline-flex h-7 items-center gap-1.5 rounded-md border px-2 text-caption transition " + (active ? "border-card-border/[0.18] bg-card-chip/[0.06] text-card-fg/88" : "border-card-border/[0.12] bg-card-panel text-card-muted/60 hover:text-card-fg")}>
                    {active && <Check className="h-3 w-3" />}
                    {option.label}
                  </button>
                );
              })}
              {assistantOptions.length === 0 && <span className="text-caption text-card-subtle/55">{t("assistant.empty")}</span>}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function AssistantCard({
  assistant,
  agents,
  readonly,
  onUpdated,
  onDeleted,
  onError,
}: {
  assistant: AssistantInfo;
  agents: AgentInfo[];
  readonly: boolean;
  onUpdated: (assistant: AssistantInfo) => void;
  onDeleted: (assistantId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(assistant.name);
  const [agentDraft, setAgentDraft] = useState<AssistantAgentInfo>(assistant.agent);
  const [systemPrompt, setSystemPrompt] = useState(assistant.systemPrompt ?? "");

  useEffect(() => {
    setName(assistant.name);
    setAgentDraft(assistant.agent);
    setSystemPrompt(assistant.systemPrompt ?? "");
  }, [assistant]);

  const save = async () => {
    if (readonly) return;
    try {
      onUpdated(await updateAssistant(assistant.id, { name, agent: agentDraft, systemPrompt }));
      setEditing(false);
    } catch (err) {
      onError(String(err));
    }
  };

  const remove = async () => {
    if (readonly) return;
    try {
      await deleteAssistant(assistant.id);
      onDeleted(assistant.id);
    } catch (err) {
      onError(String(err));
    }
  };

  return (
    <div className="rounded-lg border border-card-border/[0.12] bg-card p-3">
      {editing ? (
        <div className="grid gap-2">
          <input value={name} onChange={(event) => setName(event.target.value)} className={inputClassName} />
          <AssistantAgentSelector agent={agentDraft} agents={agents} onChange={setAgentDraft} />
          <textarea value={systemPrompt} onChange={(event) => setSystemPrompt(event.target.value)} rows={4} className={textareaClassName} />
          <div className="flex justify-end gap-2">
            <button type="button" onClick={() => setEditing(false)} className="rounded-md px-3 py-1.5 text-body-sm text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
            <button type="button" onClick={() => void save()} className="rounded-md bg-ink px-3 py-1.5 text-body-sm text-[rgb(var(--color-bg-panel))]">{t("project.save")}</button>
          </div>
        </div>
      ) : (
        <div className="flex items-start gap-3">
          <Bot className="mt-0.5 h-4 w-4 shrink-0 text-card-icon/55" />
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center gap-2">
              <span className="truncate text-body-sm font-medium text-card-fg/75">{assistant.name}</span>
              <span className="rounded bg-card-chip/8 px-1.5 py-0.5 text-meta text-card-chip-fg/55">{assistant.type}</span>
            </div>
            <div className="mt-1 truncate text-caption text-card-muted/60">{assistant.agent.name} · {assistant.agent.model} · {assistant.agent.mode} · {assistant.agent.effort}</div>
            {assistant.systemPrompt && <div className="mt-2 line-clamp-3 whitespace-pre-wrap text-caption leading-relaxed text-card-muted/60">{assistant.systemPrompt}</div>}
          </div>
          {!readonly && (
            <div className="flex shrink-0 items-center gap-1">
              <button type="button" onClick={() => setEditing(true)} className="rounded p-1 text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75"><Pencil className="h-4 w-4" /></button>
              <button type="button" onClick={() => void remove()} className="rounded p-1 text-card-subtle/45 hover:bg-status-error/10 hover:text-status-error"><Trash2 className="h-4 w-4" /></button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function AssistantSummary({ assistants }: { assistants: StageAssistantInfo[] }) {
  const { t } = useI18n();
  if (assistants.length === 0) {
    return <div className="mt-1 text-caption text-card-subtle/55">{t("assistant.empty")}</div>;
  }
  return (
    <div className="mt-2 flex flex-wrap gap-1.5">
      {assistants.map((assistant) => (
        <span key={assistant.assistantId} className="rounded bg-card-panel px-1.5 py-0.5 text-meta text-card-chip-fg/60">
          {assistant.name}
        </span>
      ))}
    </div>
  );
}

function SettingsGroup({ title, children, flush = false }: { title: string; children: ReactNode; flush?: boolean }) {
  return (
    <div className="mb-8">
      <h2 className="mb-3 text-body-sm font-semibold text-ink/[0.88]">{title}</h2>
      <div className={"overflow-hidden rounded-lg border border-card-border/[0.12] bg-card " + (flush ? "" : "p-3")}>{children}</div>
    </div>
  );
}

function SettingsRow({
  icon,
  label,
  description,
  children,
}: {
  icon: ReactNode;
  label: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <div className="grid min-h-[72px] grid-cols-[minmax(0,1fr)_auto] items-center gap-4 border-b border-ink/[0.12] px-3 py-3 last:border-b-0">
      <div className="flex min-w-0 gap-3">
        <span className="mt-0.5 text-ink/55">{icon}</span>
        <span className="min-w-0">
          <span className="block text-body-sm font-medium text-ink/75">{label}</span>
          <span className="mt-1 block text-caption leading-relaxed text-ink/60">{description}</span>
        </span>
      </div>
      {children}
    </div>
  );
}

function SegmentedControl({
  value,
  options,
  onChange,
}: {
  value: string;
  options: Array<{ value: string; label: string }>;
  onChange: (value: string) => void;
}) {
  return (
    <div className="inline-flex rounded-md bg-ink/[0.045] p-0.5">
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          onClick={() => onChange(option.value)}
          className={"h-7 min-w-9 rounded px-2 text-body-sm transition " + (value === option.value ? "bg-ink/[0.06] text-ink/85" : "text-ink/45 hover:text-ink/75")}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function ThemeSelector({ mode, onChange }: { mode: ThemeMode; onChange: (mode: ThemeMode) => void }) {
  const { t } = useI18n();
  const items = [
    { value: "light" as const, icon: Sun, label: t("theme.light") },
    { value: "dark" as const, icon: Moon, label: t("theme.dark") },
    { value: "system" as const, icon: Monitor, label: t("theme.system") },
  ];
  return (
    <div className="inline-flex rounded-md bg-ink/[0.045] p-0.5">
      {items.map(({ value, icon: Icon, label }) => (
        <Tooltip key={value} content={label} placement="top">
          <button
            type="button"
            onClick={() => onChange(value)}
            className={"flex h-7 w-8 items-center justify-center rounded transition " + (mode === value ? "bg-ink/[0.06] text-ink/85" : "text-ink/45 hover:text-ink/75")}
          >
            <Icon className="h-4 w-4" />
          </button>
        </Tooltip>
      ))}
    </div>
  );
}

function EmptyState({ label }: { label: string }) {
  return <div className="rounded-md border border-dashed border-ink/10 py-8 text-center text-body-sm text-ink/35">{label}</div>;
}

const inputClassName = "h-9 min-w-0 rounded-md border border-input-border/[0.16] bg-input px-3 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";
const textareaClassName = "min-w-0 resize-none rounded-md border border-input-border/[0.16] bg-input px-3 py-2 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";
