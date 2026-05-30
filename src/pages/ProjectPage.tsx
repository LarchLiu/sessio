import {
  useEffect,
  useMemo,
  type RefObject,
  type ReactNode,
  useCallback,
  useRef,
  useState,
} from "react";
import { Bot, Check, Copy, GitBranch, Kanban, Link2, ListChecks, LoaderCircle, Pencil, Plus, Save, Send, Trash2, Unlink, CircleDashed, CircleDot, CircleGauge, CircleUserRound, CircleCheck, CircleSlash, type LucideIcon } from "lucide-react";
import type { Agent, AgentInfo, AssistantAgentInfo, AssistantInfo, AssistantType, KanbanItem, KanbanStatus, ProjectInfo, ProjectStageInfo, ProjectType, RuntimeAgentMetadata, RuntimeAgentOptionMetadata, SessionInfo, StageAssistantInfo, StageInfo, StageType, ThreadInfo } from "../api";
import { AGENT_LABEL, addThreadStage, archiveProject, createAssistant, createKanbanItem, createProjectStage, createThread, deleteAssistant, deleteKanbanItem, deleteProjectStage, deleteThread, deleteThreadStage, linkKanbanItemSession, linkStageSession, listAgents, listAssistants, listKanbanItems, listProjectStages, listThreads, sendAgentInput, setThreadStage, startAgentSession, unlinkKanbanItemSession, unlinkStageSession, updateAssistant, updateKanbanItem, updateKanbanItemStatus, updateProject, updateProjectStage, updateRuntimeAgentPreferences, updateThread, updateThreadStage, updateThreadStageAssistantAgent } from "../api";
import { AgentGlyph } from "../components/AgentIcon";
import {
  agentModelSelectOptions,
  agentModelSelectValue,
  initialRuntimeEffort,
  parseAgentModelSelectValue,
  runtimeEffortOptions,
} from "../components/AgentSelect";
import InlineMenuSelect, { type InlineMenuSelectOption } from "../components/InlineMenuSelect";
import { RuntimeEffortControl, RuntimeMenuSelect } from "../components/RuntimeMenuSelect";
import { localeTag, useI18n } from "../i18n";
import type { PendingNewChatSession } from "../navigation";
import { dispatchSessionStartedFallback, type LiveRuntimeAction, type LiveRuntimeState } from "../runtimeChat";
import ScrollArea from "../components/ScrollArea";
import Tooltip from "../components/Tooltip";

const KANBAN_STATUSES: KanbanStatus[] = [
  "todo",
  "in_progress",
  "agent_review",
  "human_review",
  "done",
  "canceled",
];

const KANBAN_STATUS_ICONS: Record<KanbanStatus, LucideIcon> = {
  todo: CircleDashed,
  in_progress: CircleDot,
  canceled: CircleSlash,
  agent_review: CircleGauge,
  human_review: CircleUserRound,
  done: CircleCheck,
};

const STAGE_TYPE_ICONS: Record<StageType, LucideIcon> = {
  research: CircleGauge,
  plan: ListChecks,
  build: GitBranch,
  review: CircleDot,
  human: CircleUserRound,
  done: CircleCheck,
};

const PROJECT_TYPES: ProjectType[] = [
  "code",
  "writing",
  "research",
  "general",
  "video_production",
];

function initialRuntimeModel(agent: RuntimeAgentMetadata | null): string {
  return agent?.model ?? agent?.models[0]?.value ?? "";
}

function runtimeSessionOptions(model: string, permissionMode: string, effort = ""): Record<string, unknown> {
  return {
    transport: "acp",
    ...(model ? { model } : {}),
    ...(effort ? { effort } : {}),
    ...(permissionMode ? { permissionMode } : {}),
  };
}

function sessionIdentityKey(s: SessionInfo): string {
  return `${s.agent}:${s.id}`;
}

function projectTypeLabel(type: ProjectType, t: (key: string) => string): string {
  return t(`project.type.${type}`);
}

function kanbanStatusLabel(status: KanbanStatus, t: (key: string) => string): string {
  return t(`kanban.status.${status}`);
}

function stageTypeLabel(type: StageType, t: (key: string) => string): string {
  return t(`stage.type.${type}`);
}

function projectStageLabel(stage: ProjectStageInfo, t: (key: string) => string): string {
  return stage.type === "builtin" && stage.kind
    ? stageTypeLabel(stage.kind, t)
    : stage.name || t("stage.custom");
}

function projectStageIcon(stage: ProjectStageInfo) {
  const Icon = stage.kind ? STAGE_TYPE_ICONS[stage.kind] : ListChecks;
  return <Icon className="h-3.5 w-3.5" />;
}

function projectTypeOptions(t: (key: string) => string): InlineMenuSelectOption[] {
  return PROJECT_TYPES.map((type) => ({
    value: type,
    label: projectTypeLabel(type, t),
  }));
}

interface MetaRow {
  label: string;
  value: string | null;
  copyable?: boolean;
  clampLines?: number;
}

export function SessionMetaList({
  session,
  anchorRefs,
}: {
  session: SessionInfo;
  anchorRefs?: RefObject<Map<string, HTMLDivElement>>;
}) {
  const { lang, t } = useI18n();
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const rows: MetaRow[] = [
    { label: t("meta.title"), value: session.title, clampLines: 2 },
    { label: t("meta.agent"), value: AGENT_LABEL[session.agent] },
    { label: t("meta.session_id"), value: session.id, copyable: true },
    {
      label: t("meta.project"),
      value: session.projectPath ?? session.projectName,
      copyable: Boolean(session.projectPath ?? session.projectName),
    },
    { label: t("meta.started"), value: formatDate(session.startedAt, lang) },
    { label: t("meta.updated"), value: formatDate(session.updatedAt, lang) },
    {
      label: t("meta.messages"),
      value: `${session.partial ? "~" : ""}${session.messageCount}`,
    },
    { label: t("meta.file"), value: session.filePath, copyable: true },
    { label: t("meta.file_size"), value: formatBytes(session.fileSize) },
    { label: t("meta.archived"), value: session.archived ? "Yes" : "No" },
    { label: t("meta.subagents"), value: String(session.subagents.length) },
  ];

  const copyValue = async (label: string, value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedKey(label);
      window.setTimeout(() => setCopiedKey(null), 1200);
    } catch (err) {
      console.error("copy metadata value failed", err);
    }
  };

  return (
    <div className="overflow-hidden rounded-b-xl border-x border-b border-ink/10 bg-surface-panel">
      {rows.map((row) => (
        <SessionMetaRow
          key={row.label}
          id={row.label}
          row={row}
          copied={copiedKey === row.label}
          onCopy={copyValue}
          anchorRefs={anchorRefs}
        />
      ))}
    </div>
  );
}

function SessionMetaRow({
  id,
  row,
  copied,
  onCopy,
  anchorRefs,
}: {
  id: string;
  row: MetaRow;
  copied: boolean;
  onCopy: (label: string, value: string) => void;
  anchorRefs?: RefObject<Map<string, HTMLDivElement>>;
}) {
  const copyValue = row.copyable && row.value ? row.value : null;
  return (
    <div
      ref={(el) => {
        if (!anchorRefs) return;
        if (el) anchorRefs.current.set(id, el);
        else anchorRefs.current.delete(id);
      }}
      className="grid grid-cols-[140px_minmax(0,1fr)] items-center gap-4 border-b border-ink/[0.06] px-4 py-2.5 last:border-b-0"
    >
      <div className="text-caption uppercase text-ink/35">{row.label}</div>
      <div className="flex min-w-0 items-center gap-2 text-body-sm text-ink/75">
        <span
          className="min-w-0 flex-1 break-words"
          style={
            row.clampLines
              ? {
                  display: "-webkit-box",
                  WebkitLineClamp: row.clampLines,
                  WebkitBoxOrient: "vertical",
                  overflow: "hidden",
                }
              : undefined
          }
        >
          {row.value || <span className="text-ink/30">-</span>}
        </span>
        {copyValue && (
          <button
            type="button"
            className="shrink-0 rounded p-1 text-ink/35 transition-colors hover:bg-ink/[0.06] hover:text-ink/70"
            onClick={() => onCopy(row.label, copyValue)}
            aria-label={`Copy ${row.label}`}
          >
            {copied ? (
              <Check className="h-3.5 w-3.5" />
            ) : (
              <Copy className="h-3.5 w-3.5" />
            )}
          </button>
        )}
      </div>
    </div>
  );
}

function formatDate(ts: number | null, lang: "en" | "zh"): string | null {
  if (!ts) return null;
  return new Date(ts).toLocaleString(localeTag(lang), {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let size = bytes;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}


export function ProjectWorkbenchPage({
  project,
  sessions,
  runtimeAgents,
  debugAcpConfig,
  liveState,
  dispatchLiveEvent,
  onProjectUpdated,
  onProjectArchived,
  onSelectSession,
  onPendingSession,
  onChatStarted,
  onError,
}: {
  project: ProjectInfo;
  sessions: SessionInfo[];
  runtimeAgents: RuntimeAgentMetadata[];
  debugAcpConfig: boolean;
  liveState: LiveRuntimeState;
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onProjectUpdated: (project: ProjectInfo) => void;
  onProjectArchived: (projectId: string) => void;
  onSelectSession: (session: SessionInfo) => void;
  onPendingSession: (session: PendingNewChatSession) => void;
  onChatStarted: () => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [items, setItems] = useState<KanbanItem[]>([]);
  const [threads, setThreads] = useState<ThreadInfo[]>([]);
  const [projectStages, setProjectStages] = useState<ProjectStageInfo[]>([]);
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [workflowLoading, setWorkflowLoading] = useState(true);
  const [activeView, setActiveView] = useState<"threads" | "kanban" | "assistants">("threads");
  const [newTitle, setNewTitle] = useState("");
  const [editingName, setEditingName] = useState(project.name);
  const [editingType, setEditingType] = useState<ProjectType>(project.type);
  const [projectSaving, setProjectSaving] = useState(false);

  useEffect(() => {
    setEditingName(project.name);
    setEditingType(project.type);
  }, [project.id, project.name, project.type]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    listKanbanItems(project.id)
      .then((rows) => {
        if (!cancelled) setItems(rows);
      })
      .catch((err) => {
        if (!cancelled) onError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [onError, project.id]);

  useEffect(() => {
    let cancelled = false;
    setWorkflowLoading(true);
    Promise.all([listThreads(project.id), listProjectStages(project.id), listAssistants(project.id), listAgents()])
      .then(([threadRows, projectStageRows, assistantRows, agentRows]) => {
        if (cancelled) return;
        setThreads(threadRows);
        setProjectStages(projectStageRows);
        setAssistants(assistantRows);
        setAgents(agentRows);
      })
      .catch((err) => {
        if (!cancelled) onError(String(err));
      })
      .finally(() => {
        if (!cancelled) setWorkflowLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [onError, project.id]);

  const addTodo = async () => {
    const title = newTitle.trim();
    if (!title) return;
    onError(null);
    try {
      const item = await createKanbanItem(project.id, title);
      setItems((prev) => [...prev, item]);
      setNewTitle("");
    } catch (err) {
      onError(String(err));
    }
  };

  const saveProject = async () => {
    setProjectSaving(true);
    onError(null);
    try {
      const updated = await updateProject(project.id, {
        name: editingName,
        type: editingType,
      });
      onProjectUpdated(updated);
    } catch (err) {
      onError(String(err));
    } finally {
      setProjectSaving(false);
    }
  };

  const archive = async () => {
    onError(null);
    try {
      await archiveProject(project.id);
      onProjectArchived(project.id);
    } catch (err) {
      onError(String(err));
    }
  };

  const patchItem = (item: KanbanItem) => {
    setItems((prev) => prev.map((current) => (current.id === item.id ? item : current)));
  };

  const patchThread = (thread: ThreadInfo) => {
    setThreads((prev) => prev.map((current) => (current.id === thread.id ? thread : current)));
  };

  const patchStage = (stage: StageInfo) => {
    setThreads((prev) =>
      prev.map((thread) =>
        thread.id === stage.threadId
          ? {
              ...thread,
              updatedAt: Math.max(thread.updatedAt, stage.updatedAt),
              stages: thread.stages
                .map((currentStage) => (currentStage.id === stage.id ? stage : currentStage))
                .sort((a, b) => a.order - b.order),
            }
          : thread,
      ),
    );
  };

  const patchProjectStage = (stage: ProjectStageInfo) => {
    setProjectStages((prev) =>
      prev.map((current) => (current.id === stage.id ? stage : current)).sort((a, b) => a.order - b.order),
    );
  };

  const patchAssistant = (assistant: AssistantInfo) => {
    setAssistants((prev) => prev.map((current) => (current.id === assistant.id ? assistant : current)));
  };

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface-panel">
      <div className="border-b border-ink/10 px-6 py-5">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="min-w-0 flex-1">
            <div className="mb-2 flex items-center gap-2 text-caption uppercase tracking-[0.16em] text-ink/40">
              <Kanban className="h-4 w-4" />
              {t("project.workbench")}
            </div>
            <input
              value={editingName}
              onChange={(event) => setEditingName(event.target.value)}
              className="w-full max-w-[520px] bg-transparent text-[28px] font-medium leading-tight text-ink outline-none"
            />
            <div className="mt-1 truncate text-body-sm text-ink/45">{project.path}</div>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <RuntimeMenuSelect
              ariaLabel={t("project.type")}
              value={editingType}
              options={projectTypeOptions(t)}
              onChange={(value) => setEditingType(value as ProjectType)}
            />
            <Tooltip content={t("project.save")} placement="bottom">
              <button
                type="button"
                disabled={projectSaving}
                onClick={() => void saveProject()}
                className="rounded-md p-2 text-ink/55 transition hover:bg-ink/8 hover:text-ink disabled:opacity-45"
              >
                <Save className="h-4 w-4" />
              </button>
            </Tooltip>
            <Tooltip content={t("project.archive")} placement="bottom">
              <button
                type="button"
                onClick={() => void archive()}
                className="rounded-md p-2 text-ink/45 transition hover:bg-status-error/10 hover:text-status-error"
              >
                <Trash2 className="h-4 w-4" />
              </button>
            </Tooltip>
          </div>
        </div>
      </div>
      <div className="min-h-0 flex-1">
        <ScrollArea className="h-full min-h-0 p-5">
          <div className="mb-4 inline-flex rounded-lg bg-ink/[0.06] p-1">
            {([
              ["threads", t("thread.title"), GitBranch],
              ["kanban", t("project.workbench"), Kanban],
              ["assistants", t("assistant.title"), Bot],
            ] as const).map(([view, label, Icon]) => (
              <button
                key={view}
                type="button"
                onClick={() => setActiveView(view)}
                className={
                  "inline-flex h-8 min-w-[96px] items-center justify-center gap-1.5 rounded-md px-3 text-body-sm transition " +
                  (activeView === view ? "bg-surface-panel text-ink shadow-sm" : "text-ink/45 hover:text-ink/75")
                }
              >
                <Icon className="h-4 w-4" />
                <span className="truncate">{label}</span>
              </button>
            ))}
          </div>
          {activeView === "threads" && (
            <ThreadWorkflowPanel
              project={project}
              threads={threads}
              projectStages={projectStages}
              assistants={assistants}
              agents={agents}
              sessions={sessions}
              loading={workflowLoading}
              onThreadCreated={(thread) => setThreads((prev) => [thread, ...prev])}
              onThreadUpdated={patchThread}
              onThreadDeleted={(threadId) => setThreads((prev) => prev.filter((thread) => thread.id !== threadId))}
              onProjectStageCreated={(stage) => setProjectStages((prev) => [...prev, stage].sort((a, b) => a.order - b.order))}
              onProjectStageUpdated={patchProjectStage}
              onProjectStageDeleted={(stageId) => setProjectStages((prev) => prev.filter((stage) => stage.id !== stageId))}
              onStageAdded={(stage) =>
                setThreads((prev) =>
                  prev.map((thread) =>
                    thread.id === stage.threadId
                      ? {
                          ...thread,
                          stageId: thread.stageId ?? stage.id,
                          stages: [...thread.stages, stage].sort((a, b) => a.order - b.order),
                        }
                      : thread,
                  ),
                )
              }
              onStageUpdated={patchStage}
              onStageDeleted={(threadId, stageId) =>
                setThreads((prev) =>
                  prev.map((thread) =>
                    thread.id === threadId ? removeStageFromThread(thread, stageId) : thread,
                  ),
                )
              }
              onAssistantCreated={(assistant) => setAssistants((prev) => [...prev, assistant])}
              onAssistantUpdated={patchAssistant}
              onAssistantDeleted={(assistantId) => setAssistants((prev) => prev.filter((assistant) => assistant.id !== assistantId))}
              onSelectSession={onSelectSession}
              onError={onError}
            />
          )}
          {activeView === "kanban" && (
            <>
              <div className="mb-4 flex gap-2">
                <input
                  value={newTitle}
                  onChange={(event) => setNewTitle(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") void addTodo();
                  }}
                  placeholder={t("kanban.add_placeholder")}
                  className="min-w-0 flex-1 rounded-lg border border-ink/10 bg-ink/5 px-3 py-2 text-body text-ink outline-none placeholder:text-ink/35 focus:border-ink/25"
                />
                <button
                  type="button"
                  onClick={() => void addTodo()}
                  disabled={!newTitle.trim()}
                  className="rounded-lg bg-ink px-3 py-2 text-body-sm font-medium text-[rgb(var(--color-bg-panel))] disabled:opacity-35"
                >
                  {t("kanban.add")}
                </button>
              </div>
              {loading ? (
                <div className="py-12 text-center text-body-sm text-ink/45">{t("memory_search.searching")}</div>
              ) : (
                <div className="flex flex-col gap-3">
                  {KANBAN_STATUSES.map((status) => (
                    <KanbanColumn
                      key={status}
                      status={status}
                      items={items.filter((item) => item.status === status)}
                      project={project}
                      sessions={sessions}
                      runtimeAgents={runtimeAgents}
                      debugAcpConfig={debugAcpConfig}
                      liveState={liveState}
                      dispatchLiveEvent={dispatchLiveEvent}
                      onSelectSession={onSelectSession}
                      onItemUpdated={patchItem}
                      onItemDeleted={(itemId) => setItems((prev) => prev.filter((item) => item.id !== itemId))}
                      onPendingSession={onPendingSession}
                      onChatStarted={onChatStarted}
                      onError={onError}
                    />
                  ))}
                </div>
              )}
            </>
          )}
          {activeView === "assistants" && (
            <AssistantManagementPanel
              project={project}
              assistants={assistants}
              agents={agents}
              runtimeAgents={runtimeAgents}
              loading={workflowLoading}
              onAssistantCreated={(assistant) => setAssistants((prev) => [...prev, assistant])}
              onAssistantUpdated={patchAssistant}
              onAssistantDeleted={(assistantId) => setAssistants((prev) => prev.filter((assistant) => assistant.id !== assistantId))}
              onError={onError}
            />
          )}
        </ScrollArea>
      </div>
    </div>
  );
}

function assistantNames(ids: string[], assistants: AssistantInfo[], t: (key: string) => string): string {
  const names = ids
    .map((id) => assistants.find((assistant) => assistant.id === id)?.name)
    .filter((name): name is string => Boolean(name));
  return names.length > 0 ? names.join(", ") : t("assistant.empty");
}

function normalizeAssistantIds(ids: string[], assistants: AssistantInfo[]): string[] {
  const available = new Set(assistants.map((assistant) => assistant.id));
  const seen = new Set<string>();
  return ids.filter((id) => available.has(id) && !seen.has(id) && seen.add(id));
}

function agentOptions(agents: AgentInfo[]): InlineMenuSelectOption[] {
  return agents
    .filter((agent) => agent.enabled)
    .map((agent) => ({
      value: agent.id,
      label: agent.name,
      suffix: agent.transport,
      icon: agentIcon(agent),
    }));
}

function optionLabel(options: RuntimeAgentOptionMetadata[], value: string): string {
  return options.find((option) => option.value === value)?.label ?? value;
}

function optionValue(options: RuntimeAgentOptionMetadata[], fallback: string) {
  return options[0]?.value ?? fallback;
}

function selectOptions(options: RuntimeAgentOptionMetadata[], current: string): InlineMenuSelectOption[] {
  const mapped = options.map((option) => ({ value: option.value, label: option.label }));
  if (mapped.length > 0 || !current) return mapped;
  return [{ value: current, label: current }];
}

function assistantAgentPayload(agent: AgentInfo | null | undefined, model: string, mode: string, effort: string): AssistantAgentInfo {
  return {
    id: agent?.id ?? "",
    name: agent?.name ?? "",
    model,
    mode,
    effort,
  };
}

function agentIcon(agent: AgentInfo) {
  const id = agent.icon ?? agent.id;
  if (id === "codex" || id === "claude" || id === "gemini") {
    return <AgentGlyph agent={id as Agent} className="h-3.5 w-3.5" />;
  }
  return <Bot className="h-3.5 w-3.5" />;
}

function removeStageFromThread(thread: ThreadInfo, stageId: string): ThreadInfo {
  const stages = thread.stages
    .filter((stage) => stage.id !== stageId)
    .sort((a, b) => a.order - b.order)
    .map((stage, index) => ({ ...stage, order: index }));
  return {
    ...thread,
    stageId: thread.stageId === stageId ? (stages[0]?.id ?? null) : thread.stageId,
    stages,
  };
}

function ThreadWorkflowPanel({
  project,
  threads,
  projectStages,
  assistants,
  agents,
  sessions,
  loading,
  onThreadCreated,
  onThreadUpdated,
  onThreadDeleted,
  onProjectStageCreated,
  onProjectStageUpdated,
  onProjectStageDeleted,
  onStageAdded,
  onStageUpdated,
  onStageDeleted,
  onAssistantCreated,
  onAssistantUpdated,
  onAssistantDeleted,
  onSelectSession,
  onError,
}: {
  project: ProjectInfo;
  threads: ThreadInfo[];
  projectStages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  agents: AgentInfo[];
  sessions: SessionInfo[];
  loading: boolean;
  onThreadCreated: (thread: ThreadInfo) => void;
  onThreadUpdated: (thread: ThreadInfo) => void;
  onThreadDeleted: (threadId: string) => void;
  onProjectStageCreated: (stage: ProjectStageInfo) => void;
  onProjectStageUpdated: (stage: ProjectStageInfo) => void;
  onProjectStageDeleted: (stageId: string) => void;
  onStageAdded: (stage: StageInfo) => void;
  onStageUpdated: (stage: StageInfo) => void;
  onStageDeleted: (threadId: string, stageId: string) => void;
  onAssistantCreated: (assistant: AssistantInfo) => void;
  onAssistantUpdated: (assistant: AssistantInfo) => void;
  onAssistantDeleted: (assistantId: string) => void;
  onSelectSession: (session: SessionInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [goal, setGoal] = useState("");
  const [description, setDescription] = useState("");
  const [creating, setCreating] = useState(false);

  const create = async () => {
    const nextGoal = goal.trim();
    if (!nextGoal) return;
    setCreating(true);
    onError(null);
    try {
      const thread = await createThread(project.id, nextGoal, description);
      onThreadCreated(thread);
      setGoal("");
      setDescription("");
    } catch (err) {
      onError(String(err));
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="grid min-h-0 grid-cols-[minmax(0,1fr)_minmax(260px,320px)] gap-4 max-xl:grid-cols-1">
      <div className="min-w-0">
        <ProjectStagePicker
          project={project}
          stages={projectStages}
          onCreated={onProjectStageCreated}
          onUpdated={onProjectStageUpdated}
          onDeleted={onProjectStageDeleted}
          onError={onError}
        />
        <div className="mb-3 grid grid-cols-[minmax(0,1fr)_auto] gap-2 rounded-lg border border-ink/10 bg-ink/[0.035] p-3">
          <div className="grid min-w-0 gap-2">
            <input
              value={goal}
              onChange={(event) => setGoal(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) void create();
              }}
              placeholder={t("thread.goal_placeholder")}
              className="min-w-0 rounded-md border border-ink/10 bg-surface-panel px-3 py-2 text-body text-ink outline-none placeholder:text-ink/35 focus:border-ink/25"
            />
            <input
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              placeholder={t("thread.description_placeholder")}
              className="min-w-0 rounded-md border border-ink/10 bg-surface-panel px-3 py-2 text-body-sm text-ink outline-none placeholder:text-ink/35 focus:border-ink/25"
            />
          </div>
          <button
            type="button"
            onClick={() => void create()}
            disabled={creating || !goal.trim()}
            className="inline-flex h-10 items-center gap-1.5 rounded-md bg-ink px-3 text-body-sm font-medium text-[rgb(var(--color-bg-panel))] disabled:opacity-35"
          >
            {creating ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}
            {t("thread.add")}
          </button>
        </div>
        {loading ? (
          <div className="py-12 text-center text-body-sm text-ink/45">{t("memory_search.searching")}</div>
        ) : threads.length === 0 ? (
          <div className="rounded-lg border border-dashed border-ink/15 py-12 text-center text-body-sm text-ink/40">
            {t("thread.empty")}
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            {threads.map((thread) => (
              <ThreadCard
                key={thread.id}
                thread={thread}
                projectStages={projectStages}
                assistants={assistants}
                agents={agents}
                sessions={sessions}
                onThreadUpdated={onThreadUpdated}
                onThreadDeleted={onThreadDeleted}
                onStageAdded={onStageAdded}
                onStageUpdated={onStageUpdated}
                onStageDeleted={onStageDeleted}
                onSelectSession={onSelectSession}
                onError={onError}
              />
            ))}
          </div>
        )}
      </div>
      <AssistantManagementPanel
        project={project}
        assistants={assistants}
        agents={agents}
        runtimeAgents={[]}
        loading={loading}
        compact
        onAssistantCreated={onAssistantCreated}
        onAssistantUpdated={onAssistantUpdated}
        onAssistantDeleted={onAssistantDeleted}
        onError={onError}
      />
    </div>
  );
}

function ThreadCard({
  thread,
  projectStages,
  assistants,
  agents,
  sessions,
  onThreadUpdated,
  onThreadDeleted,
  onStageAdded,
  onStageUpdated,
  onStageDeleted,
  onSelectSession,
  onError,
}: {
  thread: ThreadInfo;
  projectStages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  agents: AgentInfo[];
  sessions: SessionInfo[];
  onThreadUpdated: (thread: ThreadInfo) => void;
  onThreadDeleted: (threadId: string) => void;
  onStageAdded: (stage: StageInfo) => void;
  onStageUpdated: (stage: StageInfo) => void;
  onStageDeleted: (threadId: string, stageId: string) => void;
  onSelectSession: (session: SessionInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [editing, setEditing] = useState(false);
  const [goal, setGoal] = useState(thread.goal);
  const [description, setDescription] = useState(thread.description ?? "");
  const [newStageId, setNewStageId] = useState(projectStages[0]?.id ?? "");
  const [newAssistantIds, setNewAssistantIds] = useState<string[]>(() => (assistants[0]?.id ? [assistants[0].id] : []));

  useEffect(() => {
    setGoal(thread.goal);
    setDescription(thread.description ?? "");
  }, [thread.description, thread.goal]);

  useEffect(() => {
    setNewAssistantIds((current) => {
      const normalized = normalizeAssistantIds(current, assistants);
      if (normalized.length > 0 || !assistants[0]?.id) return normalized;
      return [assistants[0].id];
    });
  }, [assistants]);

  useEffect(() => {
    const availableStages = projectStages.filter((stage) => !thread.stages.some((threadStage) => threadStage.stageId === stage.id));
    if (availableStages.some((stage) => stage.id === newStageId)) return;
    setNewStageId(availableStages[0]?.id ?? "");
  }, [newStageId, projectStages, thread.stages]);

  const save = async () => {
    try {
      onThreadUpdated(await updateThread(thread.id, { goal, description }));
      setEditing(false);
    } catch (err) {
      onError(String(err));
    }
  };

  const remove = async () => {
    try {
      await deleteThread(thread.id);
      onThreadDeleted(thread.id);
    } catch (err) {
      onError(String(err));
    }
  };

  const add = async () => {
    if (!newStageId || newAssistantIds.length === 0) return;
    try {
      onStageAdded(await addThreadStage(thread.id, newStageId, newAssistantIds));
    } catch (err) {
      onError(String(err));
    }
  };

  const currentStageId = thread.stageId ?? "";
  const stageOptions = thread.stages.map((stage) => {
    const Icon = stage.kind ? STAGE_TYPE_ICONS[stage.kind] : ListChecks;
    return {
      value: stage.id,
      label: stage.kind ? stageTypeLabel(stage.kind, t) : stage.name || t("stage.custom"),
      suffix: assistantNames(stage.assistantIds, assistants, t),
      icon: <Icon className="h-3.5 w-3.5" />,
    };
  });
  const availableProjectStageOptions = projectStages
    .filter((stage) => !thread.stages.some((threadStage) => threadStage.stageId === stage.id))
    .map((stage) => {
      return {
        value: stage.id,
        label: projectStageLabel(stage, t),
        suffix: stage.type === "builtin" ? t("stage.builtin") : t("stage.custom"),
        icon: projectStageIcon(stage),
      };
    });

  return (
    <section className="rounded-lg border border-ink/10 bg-surface-panel p-3 shadow-sm">
      <div className="flex items-start justify-between gap-3">
        {editing ? (
          <div className="grid min-w-0 flex-1 gap-2">
            <input
              value={goal}
              onChange={(event) => setGoal(event.target.value)}
              className="rounded-md border border-ink/10 bg-ink/5 px-2 py-1.5 text-body-sm text-ink outline-none"
            />
            <textarea
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              rows={2}
              className="resize-none rounded-md border border-ink/10 bg-ink/5 px-2 py-1.5 text-body-sm text-ink outline-none"
            />
          </div>
        ) : (
          <div className="min-w-0 flex-1">
            <div className="text-body font-medium text-ink/85">{thread.goal}</div>
            {thread.description && <div className="mt-1 whitespace-pre-wrap text-body-sm text-ink/50">{thread.description}</div>}
          </div>
        )}
        <div className="flex shrink-0 items-center gap-1">
          {editing ? (
            <>
              <button type="button" onClick={() => setEditing(false)} className="rounded px-2 py-1 text-caption text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
              <button type="button" onClick={() => void save()} className="rounded bg-ink px-2 py-1 text-caption text-[rgb(var(--color-bg-panel))]">{t("project.save")}</button>
            </>
          ) : (
            <>
              <button type="button" onClick={() => setEditing(true)} className="rounded p-1.5 text-ink/35 hover:bg-ink/5 hover:text-ink/70"><Pencil className="h-3.5 w-3.5" /></button>
              <button type="button" onClick={() => void remove()} className="rounded p-1.5 text-ink/35 hover:bg-status-error/10 hover:text-status-error"><Trash2 className="h-3.5 w-3.5" /></button>
            </>
          )}
        </div>
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-2 border-t border-ink/10 pt-3">
        <InlineMenuSelect
          value={currentStageId}
          options={stageOptions}
          onChange={async (stageId) => {
            try {
              onThreadUpdated(await setThreadStage(thread.id, stageId));
            } catch (err) {
              onError(String(err));
            }
          }}
          ariaLabel={t("thread.current_stage")}
          placeholder={t("thread.current_stage")}
          emptyContent={t("stage.empty")}
          minMenuWidth={220}
          className="max-w-[180px] border-r-0 rounded-md bg-ink/[0.05] px-2"
        />
        <InlineMenuSelect
          value={newStageId}
          options={availableProjectStageOptions}
          onChange={setNewStageId}
          ariaLabel={t("stage.type")}
          placeholder={t("stage.type")}
          emptyContent={t("stage.empty")}
          minMenuWidth={160}
          className="max-w-[140px] border-r-0 rounded-md bg-ink/[0.05] px-2"
        />
        <AssistantMultiPicker
          assistantIds={newAssistantIds}
          assistants={assistants}
          onChange={setNewAssistantIds}
          className="max-w-[200px] rounded-md bg-ink/[0.05] px-2"
        />
        <button
          type="button"
          disabled={!newStageId || newAssistantIds.length === 0}
          onClick={() => void add()}
          className="inline-flex h-7 items-center gap-1 rounded-md bg-ink/80 px-2 text-caption font-medium text-[rgb(var(--color-bg-panel))] disabled:opacity-35"
        >
          <Plus className="h-3.5 w-3.5" />
          {t("stage.add")}
        </button>
      </div>
      {thread.stages.length > 0 && (
        <div className="mt-3 grid gap-2">
          {thread.stages.map((stage) => (
            <StageRow
              key={stage.id}
              thread={thread}
              stage={stage}
              assistants={assistants}
              agents={agents}
              sessions={sessions}
              active={stage.id === thread.stageId}
              onThreadUpdated={onThreadUpdated}
              onStageUpdated={onStageUpdated}
              onStageDeleted={onStageDeleted}
              onSelectSession={onSelectSession}
              onError={onError}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function ProjectStagePicker({
  project,
  stages,
  onCreated,
  onUpdated,
  onDeleted,
  onError,
}: {
  project: ProjectInfo;
  stages: ProjectStageInfo[];
  onCreated: (stage: ProjectStageInfo) => void;
  onUpdated: (stage: ProjectStageInfo) => void;
  onDeleted: (stageId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");

  const create = async () => {
    const nextName = name.trim();
    const nextDescription = description.trim();
    if (!nextName) return;
    try {
      onCreated(await createProjectStage(project.id, nextName, nextDescription || null));
      setName("");
      setDescription("");
    } catch (err) {
      onError(String(err));
    }
  };

  return (
    <div className="mb-3 rounded-lg border border-ink/10 bg-ink/[0.025] p-3">
      <div className="mb-2 flex items-center gap-2 text-body-sm font-medium text-ink/65">
        <ListChecks className="h-4 w-4 text-ink/40" />
        {t("stage.project_stages")}
      </div>
      <div className="flex flex-wrap gap-2">
        {stages.map((stage) => {
          const custom = stage.type === "custom";
          return (
            <div key={stage.id} className="inline-flex min-h-7 items-center gap-1 rounded-md bg-surface-panel px-1.5 py-0.5 text-caption text-ink/65">
              {projectStageIcon(stage)}
              {custom ? (
                <div className="flex items-center gap-1">
                  <input
                    defaultValue={stage.name ?? ""}
                    onBlur={async (event) => {
                      const nextName = event.target.value.trim();
                      if (!nextName || nextName === stage.name) return;
                      try {
                        onUpdated(await updateProjectStage(stage.id, { name: nextName }));
                      } catch (err) {
                        onError(String(err));
                      }
                    }}
                    className="h-6 w-24 rounded bg-ink/5 px-1 text-caption text-ink outline-none"
                  />
                  <input
                    defaultValue={stage.description ?? ""}
                    placeholder={t("stage.description")}
                    onBlur={async (event) => {
                      const nextDescription = event.target.value.trim();
                      if ((stage.description ?? "") === nextDescription) return;
                      try {
                        onUpdated(await updateProjectStage(stage.id, { description: nextDescription || null }));
                      } catch (err) {
                        onError(String(err));
                      }
                    }}
                    className="h-6 w-32 rounded bg-ink/5 px-1 text-caption text-ink outline-none placeholder:text-ink/30"
                  />
                </div>
              ) : (
                <span className="px-1">{projectStageLabel(stage, t)}</span>
              )}
              <span className="rounded bg-ink/8 px-1 py-0.5 text-meta text-ink/40">
                {stage.type === "builtin" ? t("stage.builtin") : t("stage.custom")}
              </span>
              <button
                type="button"
                disabled={!custom}
                onClick={async () => {
                  if (!custom) return;
                  try {
                    await deleteProjectStage(stage.id);
                    onDeleted(stage.id);
                  } catch (err) {
                    onError(String(err));
                  }
                }}
                className="rounded p-0.5 text-ink/25 hover:bg-status-error/10 hover:text-status-error disabled:opacity-25"
              >
                <Trash2 className="h-3 w-3" />
              </button>
            </div>
          );
        })}
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder={t("stage.name")}
          className="h-7 min-w-[140px] rounded-md border border-ink/10 bg-surface-panel px-2 text-caption text-ink outline-none placeholder:text-ink/35"
        />
        <input
          value={description}
          onChange={(event) => setDescription(event.target.value)}
          placeholder={t("stage.description")}
          className="h-7 min-w-[160px] rounded-md border border-ink/10 bg-surface-panel px-2 text-caption text-ink outline-none placeholder:text-ink/35"
        />
        <button
          type="button"
          disabled={!name.trim()}
          onClick={() => void create()}
          className="inline-flex h-7 items-center gap-1 rounded-md bg-ink/80 px-2 text-caption font-medium text-[rgb(var(--color-bg-panel))] disabled:opacity-35"
        >
          <Plus className="h-3.5 w-3.5" />
          {t("stage.add")}
        </button>
      </div>
    </div>
  );
}

function AssistantMultiPicker({
  assistantIds,
  assistants,
  onChange,
  className = "",
}: {
  assistantIds: string[];
  assistants: AssistantInfo[];
  onChange: (assistantIds: string[]) => void;
  className?: string;
}) {
  const { t } = useI18n();
  const selected = new Set(assistantIds);
  const label = assistantNames(assistantIds, assistants, t);

  const toggle = (assistantId: string) => {
    if (selected.has(assistantId)) {
      onChange(assistantIds.filter((id) => id !== assistantId));
      return;
    }
    onChange([...assistantIds, assistantId]);
  };

  return (
    <div className={"group relative inline-flex h-7 min-w-[150px] items-center gap-1 border-r border-ink/10 text-caption text-ink/65 " + className}>
      <Bot className="h-3.5 w-3.5 shrink-0 text-ink/40" />
      <span className="truncate">{label}</span>
      <div className="invisible absolute left-0 top-full z-30 mt-1 w-56 rounded-lg border border-ink/10 bg-surface-panel p-1.5 opacity-0 shadow-lg transition group-focus-within:visible group-focus-within:opacity-100 group-hover:visible group-hover:opacity-100">
        {assistants.length === 0 ? (
          <div className="px-2 py-1.5 text-caption text-ink/40">{t("assistant.empty")}</div>
        ) : (
          assistants.map((assistant) => (
            <button
              key={assistant.id}
              type="button"
              onClick={() => toggle(assistant.id)}
              className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-caption text-ink/65 hover:bg-ink/5"
            >
              <span className="flex h-3.5 w-3.5 items-center justify-center rounded border border-ink/15 bg-ink/5">
                {selected.has(assistant.id) && <Check className="h-3 w-3" />}
              </span>
              <Bot className="h-3.5 w-3.5 text-ink/35" />
              <span className="min-w-0 flex-1 truncate">{assistant.name}</span>
              <span className="text-meta text-ink/35">{assistant.type === "builtin" ? t("assistant.builtin") : t("assistant.custom")}</span>
            </button>
          ))
        )}
      </div>
    </div>
  );
}

function StageAssistantAgentControls({
  stage,
  binding,
  agents,
  onStageUpdated,
  onError,
}: {
  stage: StageInfo;
  binding: StageAssistantInfo;
  agents: AgentInfo[];
  onStageUpdated: (stage: StageInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const selectedAgent = agents.find((agent) => agent.id === binding.agent.id) ?? null;

  const save = async (nextAgent: AssistantAgentInfo) => {
    try {
      onStageUpdated(await updateThreadStageAssistantAgent(stage.id, binding.assistantId, nextAgent));
    } catch (err) {
      onError(String(err));
    }
  };

  const selectAgent = (agentId: string) => {
    const next = agents.find((agent) => agent.id === agentId);
    if (!next) return;
    void save(
      assistantAgentPayload(
        next,
        optionValue(next.models, binding.agent.model),
        optionValue(next.permissionModes, binding.agent.mode),
        optionValue(next.efforts, binding.agent.effort),
      ),
    );
  };

  return (
    <div className="grid gap-1.5 rounded bg-surface-panel px-1.5 py-1">
      <div className="flex min-w-0 items-center gap-1.5 text-caption text-ink/55">
        <Bot className="h-3.5 w-3.5 shrink-0 text-ink/35" />
        <span className="min-w-0 flex-1 truncate">{binding.name}</span>
      </div>
      <div className="grid grid-cols-4 gap-1">
        <RuntimeMenuSelect ariaLabel={t("agent.title")} value={binding.agent.id} options={agentOptions(agents)} onChange={selectAgent} minMenuWidth={180} maxWidthClassName="max-w-none" />
        <RuntimeMenuSelect ariaLabel={t("assistant.model")} value={binding.agent.model} options={selectOptions(selectedAgent?.models ?? [], binding.agent.model)} onChange={(model) => void save({ ...binding.agent, model })} minMenuWidth={180} maxWidthClassName="max-w-none" />
        <RuntimeMenuSelect ariaLabel={t("assistant.permission_mode")} value={binding.agent.mode} options={selectOptions(selectedAgent?.permissionModes ?? [], binding.agent.mode)} onChange={(mode) => void save({ ...binding.agent, mode })} minMenuWidth={180} maxWidthClassName="max-w-none" />
        <RuntimeMenuSelect ariaLabel={t("assistant.effort")} value={binding.agent.effort} options={selectOptions(selectedAgent?.efforts ?? [], binding.agent.effort)} onChange={(effort) => void save({ ...binding.agent, effort })} minMenuWidth={140} maxWidthClassName="max-w-none" />
      </div>
    </div>
  );
}

function StageRow({
  thread,
  stage,
  assistants,
  agents,
  sessions,
  active,
  onThreadUpdated,
  onStageUpdated,
  onStageDeleted,
  onSelectSession,
  onError,
}: {
  thread: ThreadInfo;
  stage: StageInfo;
  assistants: AssistantInfo[];
  agents: AgentInfo[];
  sessions: SessionInfo[];
  active: boolean;
  onThreadUpdated: (thread: ThreadInfo) => void;
  onStageUpdated: (stage: StageInfo) => void;
  onStageDeleted: (threadId: string, stageId: string) => void;
  onSelectSession: (session: SessionInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const linkedSessionKeys = useMemo(() => new Set(stage.sessions.map(sessionIdentityKey)), [stage.sessions]);
  const sessionByKey = useMemo(() => {
    const map = new Map<string, SessionInfo>();
    for (const session of sessions) map.set(sessionIdentityKey(session), session);
    return map;
  }, [sessions]);
  const availableSessionOptions = useMemo(
    () =>
      sessions
        .filter((session) => !linkedSessionKeys.has(sessionIdentityKey(session)))
        .map((session) => ({
          value: sessionIdentityKey(session),
          label: session.title ?? session.firstUserMessage ?? t("list.no_user_message"),
          icon: <AgentGlyph agent={session.agent} className="h-3.5 w-3.5" />,
        })),
    [linkedSessionKeys, sessions, t],
  );
  const Icon = stage.kind ? STAGE_TYPE_ICONS[stage.kind] : ListChecks;

  const linkSession = async (value: string) => {
    const session = sessionByKey.get(value);
    if (!session) return;
    try {
      onStageUpdated(await linkStageSession(stage.id, session.agent, session.id));
    } catch (err) {
      onError(String(err));
    }
  };

  const unlinkSession = async (session: SessionInfo) => {
    try {
      onStageUpdated(await unlinkStageSession(stage.id, session.agent, session.id));
    } catch (err) {
      onError(String(err));
    }
  };

  return (
    <div className={"rounded-md border p-2 " + (active ? "border-ink/25 bg-ink/[0.055]" : "border-ink/10 bg-ink/[0.025]")}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <button
          type="button"
          onClick={async () => {
            try {
              onThreadUpdated(await setThreadStage(thread.id, stage.id));
            } catch (err) {
              onError(String(err));
            }
          }}
          className="inline-flex min-w-0 items-center gap-2 text-left text-body-sm font-medium text-ink/75 hover:text-ink"
        >
          <Icon className="h-4 w-4 shrink-0 text-ink/45" />
          <span>{stage.order + 1}.</span>
          <span>{stage.kind ? stageTypeLabel(stage.kind, t) : stage.name || t("stage.custom")}</span>
          {active && <span className="rounded bg-ink/10 px-1.5 py-0.5 text-meta text-ink/50">{t("thread.active")}</span>}
        </button>
        <div className="flex items-center gap-1">
          <AssistantMultiPicker
            assistantIds={stage.assistantIds}
            assistants={assistants}
            onChange={async (assistantIds) => {
              try {
                onStageUpdated(await updateThreadStage(stage.id, { assistantIds }));
              } catch (err) {
                onError(String(err));
              }
            }}
            className="max-w-[160px] rounded px-1"
          />
          <button
            type="button"
            onClick={async () => {
              try {
                await deleteThreadStage(stage.id);
                onStageDeleted(stage.threadId, stage.id);
              } catch (err) {
                onError(String(err));
              }
            }}
            className="rounded p-1 text-ink/35 hover:bg-status-error/10 hover:text-status-error"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-1.5 text-caption text-ink/45">
        <Bot className="h-3.5 w-3.5" />
        <span>{assistantNames(stage.assistantIds, assistants, t)}</span>
        <InlineMenuSelect
          value=""
          options={availableSessionOptions}
          onChange={(value) => void linkSession(value)}
          ariaLabel={t("kanban.link_session")}
          placeholder={t("kanban.link_session")}
          minMenuWidth={220}
          className="ml-auto h-6 max-w-[110px] border-r-0 px-1 py-0 text-caption text-ink/45 hover:text-ink"
          emptyContent={t("kanban.no_unlinked_sessions")}
        />
      </div>
      {stage.assistants.length > 0 && (
        <div className="mt-2 grid gap-1.5">
          {stage.assistants.map((binding) => (
            <StageAssistantAgentControls
              key={binding.assistantId}
              stage={stage}
              binding={binding}
              agents={agents}
              onStageUpdated={onStageUpdated}
              onError={onError}
            />
          ))}
        </div>
      )}
      {stage.sessions.length > 0 && (
        <div className="mt-2 flex flex-col gap-1">
          {stage.sessions.map((session) => (
            <div key={sessionIdentityKey(session)} className="group flex min-w-0 items-center gap-1.5 rounded bg-surface-panel px-1.5 py-1">
              <button
                type="button"
                onClick={() => onSelectSession(session)}
                className="flex min-w-0 flex-1 items-center gap-1.5 text-left text-caption text-ink/60 hover:text-ink"
              >
                <AgentGlyph agent={session.agent} className="h-3.5 w-3.5 shrink-0" />
                <span className="truncate">{session.title ?? session.firstUserMessage ?? t("list.no_user_message")}</span>
              </button>
              <button
                type="button"
                aria-label={t("kanban.unlink_session")}
                onClick={() => void unlinkSession(session)}
                className="rounded p-0.5 text-ink/25 opacity-0 transition hover:bg-ink/8 hover:text-ink/65 group-hover:opacity-100"
              >
                <Unlink className="h-3 w-3" />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function AssistantManagementPanel({
  project,
  assistants,
  agents,
  runtimeAgents,
  loading,
  compact = false,
  onAssistantCreated,
  onAssistantUpdated,
  onAssistantDeleted,
  onError,
}: {
  project: ProjectInfo;
  assistants: AssistantInfo[];
  agents: AgentInfo[];
  runtimeAgents: RuntimeAgentMetadata[];
  loading: boolean;
  compact?: boolean;
  onAssistantCreated: (assistant: AssistantInfo) => void;
  onAssistantUpdated: (assistant: AssistantInfo) => void;
  onAssistantDeleted: (assistantId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const firstRuntime = runtimeAgents[0] ?? null;
  const firstAgent = agents.find((agent) => agent.enabled) ?? null;
  const [agentId, setAgentId] = useState(firstAgent?.id ?? "");
  const [name, setName] = useState("");
  const selectedAgent = agents.find((agent) => agent.id === agentId) ?? firstAgent;
  const [model, setModel] = useState(firstAgent?.model ?? firstAgent?.models[0]?.value ?? firstRuntime?.model ?? firstRuntime?.models[0]?.value ?? "gpt-5-codex");
  const [permissionMode, setPermissionMode] = useState(firstAgent?.permissionMode ?? firstAgent?.permissionModes[0]?.value ?? "workspace-write");
  const [effort, setEffort] = useState(firstAgent?.effort ?? firstAgent?.efforts[0]?.value ?? firstRuntime?.effort ?? "medium");
  const [systemPrompt, setSystemPrompt] = useState("");
  const visibleAssistants = assistants.filter((assistant) => assistant.type === "custom" && assistant.projectId === project.id);

  useEffect(() => {
    if (agentId && agents.some((agent) => agent.id === agentId)) return;
    setAgentId(firstAgent?.id ?? "");
  }, [agentId, agents, firstAgent?.id]);

  useEffect(() => {
    if (!selectedAgent) return;
    if (!selectedAgent.models.some((option) => option.value === model)) {
      setModel(selectedAgent.models[0]?.value ?? model);
    }
    if (!selectedAgent.permissionModes.some((option) => option.value === permissionMode)) {
      setPermissionMode(selectedAgent.permissionModes[0]?.value ?? permissionMode);
    }
    if (!selectedAgent.efforts.some((option) => option.value === effort)) {
      setEffort(selectedAgent.efforts[0]?.value ?? effort);
    }
  }, [effort, model, permissionMode, selectedAgent]);

  const create = async () => {
    const nextName = name.trim();
    if (!nextName) return;
    try {
      const assistant = await createAssistant({
        name: nextName,
        agent: assistantAgentPayload(selectedAgent, model, permissionMode, effort),
        systemPrompt,
        type: "custom" satisfies AssistantType,
        projectId: project.id,
      });
      onAssistantCreated(assistant);
      setName("");
      setSystemPrompt("");
    } catch (err) {
      onError(String(err));
    }
  };

  return (
    <aside className={compact ? "min-w-0 rounded-lg border border-ink/10 bg-surface-panel p-3" : "min-w-0"}>
      <div className="mb-3 flex items-center gap-2 text-body-sm font-medium text-ink/70">
        <Bot className="h-4 w-4 text-ink/45" />
        {t("assistant.title")}
      </div>
      <div className="grid gap-2 rounded-lg border border-ink/10 bg-ink/[0.035] p-3">
        <input value={name} onChange={(event) => setName(event.target.value)} placeholder={t("assistant.name")} className="rounded-md border border-ink/10 bg-surface-panel px-2 py-1.5 text-body-sm text-ink outline-none placeholder:text-ink/35" />
        <div className="grid grid-cols-4 gap-2">
          <RuntimeMenuSelect ariaLabel={t("agent.title")} value={agentId} options={agentOptions(agents)} onChange={setAgentId} minMenuWidth={180} maxWidthClassName="max-w-none" />
          <RuntimeMenuSelect ariaLabel={t("assistant.model")} value={model} options={selectOptions(selectedAgent?.models ?? [], model)} onChange={setModel} minMenuWidth={180} maxWidthClassName="max-w-none" />
          <RuntimeMenuSelect ariaLabel={t("assistant.permission_mode")} value={permissionMode} options={selectOptions(selectedAgent?.permissionModes ?? [], permissionMode)} onChange={setPermissionMode} minMenuWidth={180} maxWidthClassName="max-w-none" />
          <RuntimeMenuSelect ariaLabel={t("assistant.effort")} value={effort} options={selectOptions(selectedAgent?.efforts ?? [], effort)} onChange={setEffort} minMenuWidth={140} maxWidthClassName="max-w-none" />
        </div>
        {!compact && (
          <textarea value={systemPrompt} onChange={(event) => setSystemPrompt(event.target.value)} placeholder={t("assistant.system_prompt")} rows={3} className="resize-none rounded-md border border-ink/10 bg-surface-panel px-2 py-1.5 text-body-sm text-ink outline-none placeholder:text-ink/35" />
        )}
        <button type="button" onClick={() => void create()} disabled={!name.trim()} className="inline-flex h-8 items-center justify-center gap-1 rounded-md bg-ink px-2 text-caption font-medium text-[rgb(var(--color-bg-panel))] disabled:opacity-35">
          <Plus className="h-3.5 w-3.5" />
          {t("assistant.add")}
        </button>
      </div>
      {loading ? (
        <div className="py-8 text-center text-body-sm text-ink/40">{t("memory_search.searching")}</div>
      ) : visibleAssistants.length === 0 ? (
        <div className="py-8 text-center text-body-sm text-ink/35">{t("assistant.empty")}</div>
      ) : (
        <div className="mt-3 grid gap-2">
          {visibleAssistants.map((assistant) => (
            <AssistantRow
              key={assistant.id}
              assistant={assistant}
              agents={agents}
              compact={compact}
              onUpdated={onAssistantUpdated}
              onDeleted={onAssistantDeleted}
              onError={onError}
            />
          ))}
        </div>
      )}
    </aside>
  );
}

function AssistantRow({
  assistant,
  agents,
  compact,
  onUpdated,
  onDeleted,
  onError,
}: {
  assistant: AssistantInfo;
  agents: AgentInfo[];
  compact: boolean;
  onUpdated: (assistant: AssistantInfo) => void;
  onDeleted: (assistantId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [editing, setEditing] = useState(false);
  const [agentId, setAgentId] = useState(assistant.agent.id);
  const [name, setName] = useState(assistant.name);
  const [model, setModel] = useState(assistant.agent.model);
  const [permissionMode, setPermissionMode] = useState(assistant.agent.mode);
  const [effort, setEffort] = useState(assistant.agent.effort);
  const [systemPrompt, setSystemPrompt] = useState(assistant.systemPrompt ?? "");
  const selectedAgent = agents.find((agent) => agent.id === agentId) ?? null;

  useEffect(() => {
    setAgentId(assistant.agent.id);
    setName(assistant.name);
    setModel(assistant.agent.model);
    setPermissionMode(assistant.agent.mode);
    setEffort(assistant.agent.effort);
    setSystemPrompt(assistant.systemPrompt ?? "");
  }, [assistant]);

  useEffect(() => {
    if (!selectedAgent) return;
    if (!selectedAgent.models.some((option) => option.value === model)) {
      setModel(optionValue(selectedAgent.models, model));
    }
    if (!selectedAgent.permissionModes.some((option) => option.value === permissionMode)) {
      setPermissionMode(optionValue(selectedAgent.permissionModes, permissionMode));
    }
    if (!selectedAgent.efforts.some((option) => option.value === effort)) {
      setEffort(optionValue(selectedAgent.efforts, effort));
    }
  }, [effort, model, permissionMode, selectedAgent]);

  const save = async () => {
    try {
      onUpdated(await updateAssistant(assistant.id, { name, agent: assistantAgentPayload(selectedAgent, model, permissionMode, effort), systemPrompt }));
      setEditing(false);
    } catch (err) {
      onError(String(err));
    }
  };

  const remove = async () => {
    try {
      await deleteAssistant(assistant.id);
      onDeleted(assistant.id);
    } catch (err) {
      onError(String(err));
    }
  };

  return (
    <div className="rounded-md border border-ink/10 bg-surface-panel p-2">
      {editing ? (
        <div className="grid gap-2">
          <input value={name} onChange={(event) => setName(event.target.value)} className="rounded border border-ink/10 bg-ink/5 px-2 py-1 text-body-sm text-ink outline-none" />
          <div className="grid grid-cols-4 gap-1.5">
            <RuntimeMenuSelect ariaLabel={t("agent.title")} value={agentId} options={agentOptions(agents)} onChange={setAgentId} minMenuWidth={180} maxWidthClassName="max-w-none" />
            <RuntimeMenuSelect ariaLabel={t("assistant.model")} value={model} options={selectOptions(selectedAgent?.models ?? [], model)} onChange={setModel} minMenuWidth={180} maxWidthClassName="max-w-none" />
            <RuntimeMenuSelect ariaLabel={t("assistant.permission_mode")} value={permissionMode} options={selectOptions(selectedAgent?.permissionModes ?? [], permissionMode)} onChange={setPermissionMode} minMenuWidth={180} maxWidthClassName="max-w-none" />
            <RuntimeMenuSelect ariaLabel={t("assistant.effort")} value={effort} options={selectOptions(selectedAgent?.efforts ?? [], effort)} onChange={setEffort} minMenuWidth={140} maxWidthClassName="max-w-none" />
          </div>
          {!compact && <textarea value={systemPrompt} onChange={(event) => setSystemPrompt(event.target.value)} rows={3} className="resize-none rounded border border-ink/10 bg-ink/5 px-2 py-1 text-body-sm text-ink outline-none" />}
          <div className="flex justify-end gap-1">
            <button type="button" onClick={() => setEditing(false)} className="rounded px-2 py-1 text-caption text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
            <button type="button" onClick={() => void save()} className="rounded bg-ink px-2 py-1 text-caption text-[rgb(var(--color-bg-panel))]">{t("project.save")}</button>
          </div>
        </div>
      ) : (
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0">
            <div className="truncate text-body-sm font-medium text-ink/75">{assistant.name}</div>
            <div className="mt-1 truncate text-caption text-ink/45">
              {assistant.agent.name} · {optionLabel(selectedAgent?.models ?? [], assistant.agent.model)} · {optionLabel(selectedAgent?.permissionModes ?? [], assistant.agent.mode)} · {optionLabel(selectedAgent?.efforts ?? [], assistant.agent.effort)}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            <button type="button" onClick={() => setEditing(true)} className="rounded p-1 text-ink/35 hover:bg-ink/5 hover:text-ink/70"><Pencil className="h-3.5 w-3.5" /></button>
            <button type="button" onClick={() => void remove()} className="rounded p-1 text-ink/35 hover:bg-status-error/10 hover:text-status-error"><Trash2 className="h-3.5 w-3.5" /></button>
          </div>
        </div>
      )}
    </div>
  );
}

function KanbanColumn({
  status,
  items,
  project,
  sessions,
  runtimeAgents,
  debugAcpConfig,
  liveState,
  dispatchLiveEvent,
  onSelectSession,
  onItemUpdated,
  onItemDeleted,
  onPendingSession,
  onChatStarted,
  onError,
}: {
  status: KanbanStatus;
  items: KanbanItem[];
  project: ProjectInfo;
  sessions: SessionInfo[];
  runtimeAgents: RuntimeAgentMetadata[];
  debugAcpConfig: boolean;
  liveState: LiveRuntimeState;
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onSelectSession: (session: SessionInfo) => void;
  onItemUpdated: (item: KanbanItem) => void;
  onItemDeleted: (itemId: string) => void;
  onPendingSession: (session: PendingNewChatSession) => void;
  onChatStarted: () => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const StatusIcon = KANBAN_STATUS_ICONS[status];
  return (
    <section className="rounded-lg border border-ink/10 bg-ink/[0.035] p-2.5">
      <div className="mb-2 flex items-center justify-between gap-2 px-1.5">
        <h3 className="inline-flex min-w-0 items-center gap-1.5 text-body-sm font-medium text-ink/75">
          <StatusIcon className="h-4 w-4 shrink-0 text-ink/45" />
          <span className="truncate">{kanbanStatusLabel(status, t)}</span>
        </h3>
        <span className="rounded-full bg-ink/8 px-1.5 py-0.5 text-meta text-ink/40">{items.length}</span>
      </div>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-2">
        {items.map((item) => (
          <KanbanCard
            key={item.id}
            item={item}
            project={project}
            sessions={sessions}
            runtimeAgents={runtimeAgents}
            debugAcpConfig={debugAcpConfig}
            liveState={liveState}
            dispatchLiveEvent={dispatchLiveEvent}
            onSelectSession={onSelectSession}
            onUpdated={onItemUpdated}
            onDeleted={onItemDeleted}
            onPendingSession={onPendingSession}
            onChatStarted={onChatStarted}
            onError={onError}
          />
        ))}
      </div>
    </section>
  );
}

function KanbanCard({
  item,
  project,
  sessions,
  runtimeAgents,
  debugAcpConfig: _debugAcpConfig,
  liveState,
  dispatchLiveEvent,
  onSelectSession,
  onUpdated,
  onDeleted,
  onPendingSession,
  onChatStarted,
  onError,
}: {
  item: KanbanItem;
  project: ProjectInfo;
  sessions: SessionInfo[];
  runtimeAgents: RuntimeAgentMetadata[];
  debugAcpConfig: boolean;
  liveState: LiveRuntimeState;
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onSelectSession: (session: SessionInfo) => void;
  onUpdated: (item: KanbanItem) => void;
  onDeleted: (itemId: string) => void;
  onPendingSession: (session: PendingNewChatSession) => void;
  onChatStarted: () => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(item.title);
  const [description, setDescription] = useState(item.description ?? "");
  const [agent, setAgent] = useState<Agent>(() => runtimeAgents[0]?.agent ?? "codex");
  const [sending, setSending] = useState(false);
  const fallbackRuntimeSequenceRef = useRef(0);

  useEffect(() => {
    setTitle(item.title);
    setDescription(item.description ?? "");
  }, [item.description, item.title]);

  const linkedSessionKeys = useMemo(
    () => new Set(item.sessions.map(sessionIdentityKey)),
    [item.sessions],
  );
  const selectedKanbanRuntimeAgent =
    runtimeAgents.find((runtimeAgent) => runtimeAgent.agent === agent) ?? null;
  const [model, setModel] = useState(() => initialRuntimeModel(runtimeAgents[0] ?? null));
  const [effort, setEffort] = useState(() => initialRuntimeEffort(runtimeAgents[0] ?? null));
  const handleEffortChange = useCallback(async (targetAgent: Agent, nextValue: string) => {
    if (targetAgent === agent) setEffort(nextValue);
    try {
      await updateRuntimeAgentPreferences({ agent: targetAgent, effort: nextValue });
    } catch (err) {
      onError(String(err));
    }
  }, [agent, onError]);
  const agentModelOptions = useMemo(
    () =>
      agentModelSelectOptions(
        runtimeAgents,
        Object.fromEntries(
          runtimeAgents.map((runtimeAgent) => [
            runtimeAgent.agent,
            <RuntimeEffortControl
              value={runtimeAgent.agent === agent ? effort : initialRuntimeEffort(runtimeAgent)}
              options={runtimeEffortOptions(runtimeAgent)}
              onChange={(value) => void handleEffortChange(runtimeAgent.agent, value)}
              disabled={sending}
            />,
          ]),
        ) as Partial<Record<Agent, ReactNode>>,
        { [agent]: effort },
      ),
    [agent, effort, handleEffortChange, runtimeAgents, sending],
  );
  const selectedAgentModelValue = agentModelSelectValue(agent, model);
  const availableSessionOptions = useMemo(
    () =>
      sessions
        .filter((session) => !linkedSessionKeys.has(sessionIdentityKey(session)))
        .map((session) => ({
          value: sessionIdentityKey(session),
          label: session.title ?? session.firstUserMessage ?? t("list.no_user_message"),
          icon: <AgentGlyph agent={session.agent} className="h-3.5 w-3.5" />,
        })),
    [linkedSessionKeys, sessions, t],
  );
  const sessionByKey = useMemo(() => {
    const map = new Map<string, SessionInfo>();
    for (const session of sessions) {
      map.set(sessionIdentityKey(session), session);
    }
    return map;
  }, [sessions]);

  useEffect(() => {
    if (agentModelOptions.some((option) => option.value === selectedAgentModelValue)) return;
    const current = runtimeAgents.find((item) => item.agent === agent) ?? null;
    const next = current ?? runtimeAgents[0] ?? null;
    if (!next) return;
    setAgent(next.agent);
    setModel(initialRuntimeModel(next));
    setEffort(initialRuntimeEffort(next));
  }, [agent, agentModelOptions, runtimeAgents, selectedAgentModelValue]);

  useEffect(() => {
    if (!selectedKanbanRuntimeAgent) return;
    if (agentModelOptions.some((option) => option.value === selectedAgentModelValue)) return;
    setModel(initialRuntimeModel(selectedKanbanRuntimeAgent));
    setEffort(initialRuntimeEffort(selectedKanbanRuntimeAgent));
  }, [
    agentModelOptions,
    selectedAgentModelValue,
    selectedKanbanRuntimeAgent?.effort,
    selectedKanbanRuntimeAgent?.efforts,
    selectedKanbanRuntimeAgent?.agent,
    selectedKanbanRuntimeAgent?.model,
  ]);

  const prompt = useMemo(() => {
    const trimmedDescription = item.description?.trim();
    return trimmedDescription ? `${item.title}\n\n${trimmedDescription}` : item.title;
  }, [item.description, item.title]);

  const move = async (status: KanbanStatus) => {
    try {
      onUpdated(await updateKanbanItemStatus(item.id, status));
    } catch (err) {
      onError(String(err));
    }
  };

  const save = async () => {
    try {
      const updated = await updateKanbanItem(item.id, {
        title,
        description,
      });
      onUpdated(updated);
      setEditing(false);
    } catch (err) {
      onError(String(err));
    }
  };

  const remove = async () => {
    try {
      await deleteKanbanItem(item.id);
      onDeleted(item.id);
    } catch (err) {
      onError(String(err));
    }
  };

  const linkSession = async (value: string) => {
    const session = sessionByKey.get(value);
    if (!session) return;
    try {
      onUpdated(await linkKanbanItemSession(item.id, session.agent, session.id));
    } catch (err) {
      onError(String(err));
    }
  };

  const unlinkSession = async (session: SessionInfo) => {
    try {
      onUpdated(await unlinkKanbanItemSession(item.id, session.agent, session.id));
    } catch (err) {
      onError(String(err));
    }
  };

  const sendItem = async () => {
    if (sending || !prompt.trim()) return;
    if (!agentModelOptions.some((option) => option.value === selectedAgentModelValue)) {
      onError("No configured runtime agent available");
      return;
    }
    setSending(true);
    onError(null);
    try {
      const handle = await startAgentSession({
        agent,
        workspacePath: project.path,
        options: runtimeSessionOptions(model, "", effort),
      });
      const timestamp = Date.now();
      dispatchSessionStartedFallback({
        dispatch: dispatchLiveEvent,
        handle,
        liveState,
        sequenceRef: fallbackRuntimeSequenceRef,
        timestamp,
      });
      onPendingSession({
        sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
        agent: handle.agent,
        projectPath: project.path,
        projectName: project.name,
        prompt,
        timestamp,
        kanbanItemId: item.id,
        kanbanItemStatus: item.status,
      });
      await sendAgentInput(handle.sessioRuntimeSessionId, { text: prompt });
      onChatStarted();
    } catch (err) {
      onError(String(err));
    } finally {
      setSending(false);
    }
  };

  return (
    <article className="rounded-lg border border-ink/10 bg-surface-panel p-2.5 shadow-sm">
      {editing ? (
        <div className="flex flex-col gap-2">
          <input value={title} onChange={(event) => setTitle(event.target.value)} className="rounded border border-ink/10 bg-ink/5 px-2 py-1 text-body-sm text-ink outline-none" />
          <textarea value={description} onChange={(event) => setDescription(event.target.value)} rows={3} className="resize-none rounded border border-ink/10 bg-ink/5 px-2 py-1 text-body-sm text-ink outline-none" />
          <div className="flex justify-end gap-1">
            <button type="button" onClick={() => setEditing(false)} className="rounded px-2 py-1 text-caption text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
            <button type="button" onClick={() => void save()} className="rounded bg-ink px-2 py-1 text-caption text-[rgb(var(--color-bg-panel))]">{t("project.save")}</button>
          </div>
        </div>
      ) : (
        <>
          <div className="text-body-sm font-medium leading-snug text-ink/85">{item.title}</div>
          {item.description && <div className="mt-1 whitespace-pre-wrap text-caption leading-relaxed text-ink/50">{item.description}</div>}
          <div className="mt-2 border-t border-ink/10 pt-2">
            <div className="flex min-w-0 items-center justify-between gap-2">
              <RuntimeMenuSelect
                ariaLabel={t("new_chat.agent")}
                value={selectedAgentModelValue}
                onChange={(value) => {
                  const parsed = parseAgentModelSelectValue(value);
                  if (!parsed) return;
                  setAgent(parsed.agent);
                  setModel(parsed.model);
                  const targetRuntimeAgent =
                    runtimeAgents.find((runtimeAgent) => runtimeAgent.agent === parsed.agent) ?? null;
                  setEffort(initialRuntimeEffort(targetRuntimeAgent));
                }}
                disabled={agentModelOptions.length === 0 || sending}
                options={agentModelOptions}
              />
              <Tooltip content={sending ? t("new_chat.sending") : t("new_chat.send")} placement="top">
                <button
                  type="button"
                  disabled={sending || agentModelOptions.length === 0 || !prompt.trim()}
                  onClick={() => void sendItem()}
                  className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-ink/70 text-[rgb(var(--color-bg-panel))] transition hover:bg-ink disabled:cursor-not-allowed disabled:bg-ink/25 disabled:text-[rgb(var(--color-bg-panel)/0.7)]"
                  aria-label={sending ? t("new_chat.sending") : t("new_chat.send")}
                >
                  {sending ? (
                    <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Send className="h-3.5 w-3.5" />
                  )}
                </button>
              </Tooltip>
            </div>
          </div>
          <div className="mt-2 border-t border-ink/10 pt-2">
            <div className="mb-1.5 flex items-center justify-between gap-2">
              <div className="inline-flex min-w-0 items-center gap-1 text-meta text-ink/40">
                <Link2 className="h-3 w-3" />
                <span>{t("kanban.sessions_count", { count: item.sessions.length })}</span>
              </div>
              <InlineMenuSelect
                value=""
                options={availableSessionOptions}
                onChange={(value) => void linkSession(value)}
                ariaLabel={t("kanban.link_session")}
                placeholder={t("kanban.link_session")}
                minMenuWidth={220}
                className="h-6 max-w-[96px] border-r-0 px-1 py-0 text-caption text-ink/45 hover:text-ink"
                emptyContent={t("kanban.no_unlinked_sessions")}
              />
            </div>
            {item.sessions.length > 0 && (
              <div className="flex flex-col gap-1">
                {item.sessions.slice(0, 4).map((session) => (
                  <div key={sessionIdentityKey(session)} className="group flex min-w-0 items-center gap-1.5 rounded bg-ink/[0.035] px-1.5 py-1">
                    <button
                      type="button"
                      onClick={() => onSelectSession(session)}
                      className="flex min-w-0 flex-1 items-center gap-1.5 text-left text-caption text-ink/60 hover:text-ink"
                    >
                      <AgentGlyph agent={session.agent} className="h-3.5 w-3.5 shrink-0" />
                      <span className="truncate">{session.title ?? session.firstUserMessage ?? t("list.no_user_message")}</span>
                    </button>
                    <button
                      type="button"
                      aria-label={t("kanban.unlink_session")}
                      onClick={() => void unlinkSession(session)}
                      className="rounded p-0.5 text-ink/25 opacity-0 transition hover:bg-ink/8 hover:text-ink/65 group-hover:opacity-100"
                    >
                      <Unlink className="h-3 w-3" />
                    </button>
                  </div>
                ))}
                {item.sessions.length > 4 && (
                  <div className="px-1.5 text-meta text-ink/35">
                    {t("kanban.more_sessions", { count: item.sessions.length - 4 })}
                  </div>
                )}
              </div>
            )}
          </div>
          <div className="mt-3 flex items-center justify-between gap-2">
            <InlineMenuSelect
              value={item.status}
              options={KANBAN_STATUSES.map((status) => ({
                value: status,
                label: kanbanStatusLabel(status, t),
                icon: (() => {
                  const StatusIcon = KANBAN_STATUS_ICONS[status];
                  return <StatusIcon className="h-3.5 w-3.5" />;
                })(),
              }))}
              onChange={(value) => void move(value as KanbanStatus)}
              minMenuWidth={150}
              className="h-6 max-w-[120px] px-1 py-0 text-caption"
            />
            <div className="flex items-center gap-1">
              <button type="button" onClick={() => setEditing(true)} className="rounded p-1 text-ink/35 hover:bg-ink/5 hover:text-ink/70">
                <Pencil className="h-3.5 w-3.5" />
              </button>
              <button type="button" onClick={() => void remove()} className="rounded p-1 text-ink/35 hover:bg-status-error/10 hover:text-status-error">
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>
        </>
      )}
    </article>
  );
}
