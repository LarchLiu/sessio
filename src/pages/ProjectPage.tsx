import {
  useEffect,
  useLayoutEffect,
  useMemo,
  type RefObject,
  useCallback,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { DragDropProvider, type DragEndEvent } from "@dnd-kit/react";
import { isSortable, useSortable } from "@dnd-kit/react/sortable";
import HashIcon from "@iconify-react/mynaui/hash";
import Robot3LineIcon from "@iconify-react/ri/robot-3-line";
import { Check, Copy, GripVertical, Link2, LoaderCircle, Pencil, Plus, Trash2, Unlink, Workflow, X } from "lucide-react";
import type { AgentInfo, AssistantInfo, ProjectInfo, ProjectStageInfo, SessionInfo, StageInfo, ThreadInfo } from "../api";
import { AGENT_LABEL, addThreadStage, createThread, deleteThread, deleteThreadStage, linkStageSession, linkThreadSession, listAgents, listAssistants, listProjectStages, listThreads, setThreadStage, unlinkStageSession, unlinkThreadSession, updateThread } from "../api";
import { AgentGlyph } from "../components/AgentIcon";
import InlineMenuSelect from "../components/InlineMenuSelect";
import CreateAssistantDialog from "../components/CreateAssistantDialog";
import CreateStageDialog from "../components/CreateStageDialog";
import AssistantBotIcon from "../components/AssistantBotIcon";
import AssistantCard from "../components/AssistantCard";
import StageList from "../components/StageList";
import StageSelectChip from "../components/StageSelectChip";
import { localeTag, useI18n } from "../i18n";
import ScrollArea from "../components/ScrollArea";
import SegmentedTabs, { type SegmentedTabItem } from "../components/SegmentedTabs";
import { projectStageIcon, projectStageLabel } from "../utils/stageDisplay";

const ASSISTANT_MENU_GAP = 6;
const ASSISTANT_MENU_MARGIN = 8;
const ASSISTANT_MENU_MAX_HEIGHT = 260;

type ProjectView = "threads" | "stages" | "assistants";

function sessionIdentityKey(s: SessionInfo): string {
  return `${s.agent}:${s.id}`;
}

function stageAllowsThreadAddition(stage: ProjectStageInfo): boolean {
  return stage.assistants.length > 0 || stage.allowEmptyAssistants;
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
  onSelectSession,
  onError,
}: {
  project: ProjectInfo;
  sessions: SessionInfo[];
  onSelectSession: (session: SessionInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const projectViewTabs = useMemo<SegmentedTabItem<ProjectView>[]>(
    () => [
      { value: "threads", label: t("thread.title"), icon: HashIcon },
      { value: "stages", label: t("project.workflowId"), icon: Workflow },
      { value: "assistants", label: t("assistant.title"), icon: Robot3LineIcon },
    ],
    [t],
  );
  const [threads, setThreads] = useState<ThreadInfo[]>([]);
  const [projectStages, setProjectStages] = useState<ProjectStageInfo[]>([]);
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [workflowLoading, setWorkflowLoading] = useState(true);
  const [activeView, setActiveView] = useState<ProjectView>("threads");

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
    <div className="flex h-full min-h-0 flex-1 flex-col overflow-hidden bg-surface-panel">
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        <div className="flex shrink-0 items-center gap-4 px-5 pt-5">
          <SegmentedTabs
            items={projectViewTabs}
            value={activeView}
            onChange={setActiveView}
            itemWidth={116}
            itemHeight={32}
            padding={4}
          />
        </div>
        <ScrollArea
          className="min-h-0 flex-1"
          viewportClassName="px-5 pb-5 pt-4"
        >
          {activeView === "threads" && (
            <ThreadWorkflowPanel
              project={project}
              threads={threads}
              projectStages={projectStages}
              assistants={assistants}
              sessions={sessions}
              loading={workflowLoading}
              onThreadCreated={(thread) => setThreads((prev) => [thread, ...prev])}
              onThreadUpdated={patchThread}
              onThreadDeleted={(threadId) => setThreads((prev) => prev.filter((thread) => thread.id !== threadId))}
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
              onSelectSession={onSelectSession}
              onError={onError}
            />
          )}
          {activeView === "stages" && (
            <ProjectStagePicker
              project={project}
              stages={projectStages}
              assistants={assistants}
              onCreated={(stage) => setProjectStages((prev) => [...prev, stage].sort((a, b) => a.order - b.order))}
              onUpdated={patchProjectStage}
              onDeleted={(stageId) => setProjectStages((prev) => prev.filter((stage) => stage.id !== stageId))}
              onReload={async () => {
                setProjectStages((await listProjectStages(project.id)).sort((a, b) => a.order - b.order));
              }}
              onError={onError}
            />
          )}
          {activeView === "assistants" && (
            <AssistantManagementPanel
              project={project}
              assistants={assistants}
              agents={agents}
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

function selectedAssistants(ids: string[], assistants: AssistantInfo[]): AssistantInfo[] {
  return ids
    .map((id) => assistants.find((assistant) => assistant.id === id))
    .filter((assistant): assistant is AssistantInfo => Boolean(assistant));
}

function normalizeAssistantIds(ids: string[], assistants: AssistantInfo[]): string[] {
  const available = new Set(assistants.map((assistant) => assistant.id));
  const seen = new Set<string>();
  return ids.filter((id) => available.has(id) && !seen.has(id) && seen.add(id));
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
  sessions,
  loading,
  onThreadCreated,
  onThreadUpdated,
  onThreadDeleted,
  onStageAdded,
  onStageUpdated,
  onStageDeleted,
  onSelectSession,
  onError,
}: {
  project: ProjectInfo;
  threads: ThreadInfo[];
  projectStages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  sessions: SessionInfo[];
  loading: boolean;
  onThreadCreated: (thread: ThreadInfo) => void;
  onThreadUpdated: (thread: ThreadInfo) => void;
  onThreadDeleted: (threadId: string) => void;
  onStageAdded: (stage: StageInfo) => void;
  onStageUpdated: (stage: StageInfo) => void;
  onStageDeleted: (threadId: string, stageId: string) => void;
  onSelectSession: (session: SessionInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [goal, setGoal] = useState("");
  const [description, setDescription] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  const enabledProjectStages = useMemo(
    () => projectStages.filter((stage) => stage.enabled),
    [projectStages],
  );
  const selectableProjectStages = useMemo(
    () => enabledProjectStages.filter(stageAllowsThreadAddition),
    [enabledProjectStages],
  );
  const [selectedStageIds, setSelectedStageIds] = useState<string[]>([]);
  const [createStageOrder, setCreateStageOrder] = useState<string[]>([]);
  const linkedSessionKeys = useMemo(() => {
    const keys = new Set<string>();
    for (const thread of threads) {
      for (const session of thread.sessions) keys.add(sessionIdentityKey(session));
      for (const stage of thread.stages) {
        for (const session of stage.sessions) keys.add(sessionIdentityKey(session));
      }
    }
    return keys;
  }, [threads]);

  useEffect(() => {
    setCreateStageOrder((current) => {
      const enabledIds = enabledProjectStages.map((stage) => stage.id);
      return [
        ...current.filter((id) => enabledIds.includes(id)),
        ...enabledIds.filter((id) => !current.includes(id)),
      ];
    });
    setSelectedStageIds((current) => {
      const selectableIds = selectableProjectStages.map((stage) => stage.id);
      const existing = current.filter((id) => selectableIds.includes(id));
      if (existing.length > 0 || current.length > 0) return existing;
      return selectableIds;
    });
  }, [enabledProjectStages, selectableProjectStages]);

  const toggleCreateStage = (stageId: string) => {
    if (!selectableProjectStages.some((stage) => stage.id === stageId)) return;
    setSelectedStageIds((current) =>
      current.includes(stageId)
        ? current.filter((id) => id !== stageId)
        : [...current, stageId],
    );
  };

  const orderedCreateStages = useMemo(() => {
    const byId = new Map(enabledProjectStages.map((stage) => [stage.id, stage]));
    return createStageOrder
      .map((id) => byId.get(id))
      .filter((stage): stage is ProjectStageInfo => Boolean(stage));
  }, [createStageOrder, enabledProjectStages]);

  const handleCreateStageDragEnd = (event: DragEndEvent) => {
    if (event.canceled) return;
    const { source } = event.operation;
    if (!isSortable(source)) return;
    const from = source.initialIndex;
    const to = source.index;
    if (from === to) return;
    setCreateStageOrder((current) => {
      const next = [...current];
      const [id] = next.splice(from, 1);
      if (!id) return current;
      next.splice(to, 0, id);
      return next;
    });
  };

  const create = async () => {
    const nextGoal = goal.trim();
    if (!nextGoal) return;
    setCreating(true);
    onError(null);
    try {
      const thread = await createThread(project.id, nextGoal, description);
      let nextThread = thread;
      for (const stageId of createStageOrder.filter((id) => selectedStageIds.includes(id))) {
        const stage = await addThreadStage(thread.id, stageId, []);
        nextThread = {
          ...nextThread,
          stageId: nextThread.stageId ?? stage.id,
          stages: [...nextThread.stages, stage].sort((a, b) => a.order - b.order),
          updatedAt: Math.max(nextThread.updatedAt, stage.updatedAt),
        };
      }
      onThreadCreated(nextThread);
      setGoal("");
      setDescription("");
      setCreateOpen(false);
    } catch (err) {
      onError(String(err));
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="min-w-0">
        <div className="mb-3 flex justify-end">
          <button
            type="button"
            onClick={() => setCreateOpen(true)}
            className="inline-flex h-9 items-center gap-1.5 rounded-md bg-ink px-3 text-body-sm font-medium text-[rgb(var(--color-bg-panel))] hover:opacity-90"
          >
            <Plus className="h-4 w-4" />
            {t("thread.add")}
          </button>
        </div>
        {createOpen &&
          createPortal(
            <div className="fixed inset-0 z-[90] flex items-center justify-center bg-black/35 px-4">
              <div className="w-full max-w-[720px] rounded-xl border border-ink/10 bg-surface-panel p-4 shadow-2xl">
                <div className="mb-4 flex items-center justify-between gap-3">
                  <div className="text-body font-medium text-ink">{t("thread.add")}</div>
                  <button
                    type="button"
                    onClick={() => setCreateOpen(false)}
                    className="rounded-md p-1 text-ink/45 hover:bg-ink/5 hover:text-ink"
                  >
                    <X className="h-4 w-4" />
                  </button>
                </div>
                <div className="grid gap-3">
                  <input
                    value={goal}
                    onChange={(event) => setGoal(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" && !event.shiftKey) void create();
                      if (event.key === "Escape") setCreateOpen(false);
                    }}
                    autoFocus
                    placeholder={t("thread.goal_placeholder")}
                    className="min-w-0 rounded-md border border-ink/10 bg-ink/5 px-3 py-2 text-body text-ink outline-none placeholder:text-ink/35 focus:border-ink/25"
                  />
                  <textarea
                    value={description}
                    onChange={(event) => setDescription(event.target.value)}
                    placeholder={t("thread.description_placeholder")}
                    rows={3}
                    className="min-w-0 resize-none rounded-md border border-ink/10 bg-ink/5 px-3 py-2 text-body-sm text-ink outline-none placeholder:text-ink/35 focus:border-ink/25"
                  />
                  {orderedCreateStages.length > 0 && (
                    <DragDropProvider onDragEnd={handleCreateStageDragEnd}>
                      <div className="flex flex-wrap gap-1.5 border-t border-ink/10 pt-3">
                        {orderedCreateStages.map((stage, index) => (
                          <CreateThreadStageChip
                            key={stage.id}
                            stage={stage}
                            index={index}
                            selected={selectedStageIds.includes(stage.id)}
                            selectable={stageAllowsThreadAddition(stage)}
                            onToggle={toggleCreateStage}
                          />
                        ))}
                      </div>
                    </DragDropProvider>
                  )}
                </div>
                <div className="mt-4 flex justify-end gap-2">
                  <button
                    type="button"
                    onClick={() => setCreateOpen(false)}
                    className="rounded-md px-3 py-1.5 text-body-sm text-ink/60 hover:bg-ink/5 hover:text-ink"
                  >
                    {t("delete.cancel")}
                  </button>
                  <button
                    type="button"
                    onClick={() => void create()}
                    disabled={creating || !goal.trim()}
                    className="inline-flex items-center gap-1.5 rounded-md bg-ink px-3 py-1.5 text-body-sm text-[rgb(var(--color-bg-panel))] disabled:opacity-35"
                  >
                    {creating ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}
                    {t("thread.add")}
                  </button>
                </div>
              </div>
            </div>,
            document.body,
          )}
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
                sessions={sessions}
                linkedSessionKeys={linkedSessionKeys}
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
  );
}

function CreateThreadStageChip({
  stage,
  index,
  selected,
  selectable,
  onToggle,
}: {
  stage: ProjectStageInfo;
  index: number;
  selected: boolean;
  selectable: boolean;
  onToggle: (stageId: string) => void;
}) {
  const { handleRef, isDragSource, isDropTarget, ref } = useSortable({
    id: stage.id,
    index,
    group: "create-thread-stages",
    transition: {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
      idle: true,
    },
  });

  return (
    <StageSelectChip
      ref={ref}
      stage={stage}
      selected={selected}
      selectable={selectable}
      onToggle={onToggle}
      state={
        isDragSource
          ? "dragging"
          : isDropTarget
            ? "drop-target"
            : "idle"
      }
      dragHandle={
        <button
          ref={handleRef}
          type="button"
          className="cursor-grab touch-none rounded p-0.5 text-current/50 hover:bg-ink/5 active:cursor-grabbing"
        >
          <GripVertical className="h-3.5 w-3.5" />
        </button>
      }
    />
  );
}

function ThreadCard({
  thread,
  projectStages,
  assistants,
  sessions,
  linkedSessionKeys,
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
  sessions: SessionInfo[];
  linkedSessionKeys: Set<string>;
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
  const enabledAssistants = useMemo(
    () => assistants.filter((assistant) => assistant.projectId === thread.projectId && assistant.enabled),
    [assistants, thread.projectId],
  );
  const [newAssistantIds, setNewAssistantIds] = useState<string[]>(() => (enabledAssistants[0]?.id ? [enabledAssistants[0].id] : []));
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

  useEffect(() => {
    setGoal(thread.goal);
    setDescription(thread.description ?? "");
  }, [thread.description, thread.goal]);

  useEffect(() => {
    setNewAssistantIds((current) => {
      const normalized = normalizeAssistantIds(current, enabledAssistants);
      if (normalized.length > 0 || !enabledAssistants[0]?.id) return normalized;
      return [enabledAssistants[0].id];
    });
  }, [enabledAssistants]);

  useEffect(() => {
    const availableStages = projectStages.filter(
      (stage) =>
        stage.enabled &&
        stageAllowsThreadAddition(stage) &&
        !thread.stages.some((threadStage) => threadStage.stageId === stage.id),
    );
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
    const stage = projectStages.find((item) => item.id === newStageId);
    if (!stage || (!stage.allowEmptyAssistants && newAssistantIds.length === 0)) return;
    try {
      onStageAdded(await addThreadStage(thread.id, newStageId, newAssistantIds));
    } catch (err) {
      onError(String(err));
    }
  };

  const linkSession = async (value: string) => {
    const session = sessionByKey.get(value);
    if (!session) return;
    try {
      onThreadUpdated(await linkThreadSession(thread.id, session.agent, session.id));
    } catch (err) {
      onError(String(err));
    }
  };

  const unlinkSession = async (session: SessionInfo) => {
    try {
      onThreadUpdated(await unlinkThreadSession(thread.id, session.agent, session.id));
    } catch (err) {
      onError(String(err));
    }
  };

  const currentStageId = thread.stageId ?? "";
  const stageOptions = thread.stages.map((stage) => {
    return {
      value: stage.id,
      label: projectStageLabel(stage, t),
      description: stage.description ?? undefined,
      icon: projectStageIcon(stage),
    };
  });
  const availableProjectStageOptions = projectStages
    .filter(
      (stage) =>
        stage.enabled &&
        stageAllowsThreadAddition(stage) &&
        !thread.stages.some((threadStage) => threadStage.stageId === stage.id),
    )
    .map((stage) => {
      return {
        value: stage.id,
        label: projectStageLabel(stage, t),
        description: stage.description ?? undefined,
        suffix: stage.type === "builtin" ? t("stage.builtin") : t("stage.custom"),
        icon: projectStageIcon(stage),
      };
    });
  const selectedNewStage = projectStages.find((stage) => stage.id === newStageId);

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
      <div className="mt-3 border-t border-ink/10 pt-3">
        <div className="flex flex-wrap items-center gap-1.5 text-caption text-ink/45">
          <Link2 className="h-3.5 w-3.5 text-ink/30" />
          <span>{t("kanban.sessions_count", { count: thread.sessions.length })}</span>
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
        {thread.sessions.length > 0 && (
          <div className="mt-2 flex flex-col gap-1">
            {thread.sessions.map((session) => (
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
          </div>
        )}
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
          assistants={enabledAssistants}
          onChange={setNewAssistantIds}
          className="max-w-[200px] rounded-md bg-ink/[0.05] px-2"
        />
        <button
          type="button"
          disabled={!newStageId || (!selectedNewStage?.allowEmptyAssistants && newAssistantIds.length === 0)}
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
              sessions={sessions}
              linkedSessionKeys={linkedSessionKeys}
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
  assistants,
  onCreated,
  onUpdated,
  onDeleted,
  onReload,
  onError,
}: {
  project: ProjectInfo;
  stages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  onCreated: (stage: ProjectStageInfo) => void;
  onUpdated: (stage: ProjectStageInfo) => void;
  onDeleted: (stageId: string) => void;
  onReload: () => Promise<void>;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [showCreate, setShowCreate] = useState(false);
  const projectAssistants = useMemo(
    () => assistants.filter((assistant) => assistant.projectId === project.id),
    [assistants, project.id],
  );
  const enabledAssistants = useMemo(
    () => projectAssistants.filter((assistant) => assistant.enabled),
    [projectAssistants],
  );

  return (
    <div className="mb-3 rounded-lg border border-ink/10 bg-ink/[0.025] p-5">
      <div className="grid gap-3">
        <div className="flex items-center justify-between gap-3">
          <div className="text-body-sm font-semibold text-card-fg/85">{t("stage.project_stages")}</div>
          <button type="button" onClick={() => setShowCreate((value) => !value)} className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2.5 text-body-sm font-medium text-card-fg/75 transition hover:text-card-fg/90">
            <Plus className="h-3.5 w-3.5" />
            {t("stage.add")}
          </button>
        </div>
        <StageList
          stages={stages}
          assistants={enabledAssistants}
          loading={false}
          dragGroup="project-stages"
          onUpdated={onUpdated}
          onDeleted={onDeleted}
          onReload={onReload}
          onError={onError}
        />
      </div>
      {showCreate && (
        <CreateStageDialog
          projectId={project.id}
          onCreated={onCreated}
          onClose={() => setShowCreate(false)}
          onError={onError}
        />
      )}
    </div>
  );
}

function AssistantMultiPicker({
  assistantIds,
  assistants,
  labelAssistants,
  onChange,
  className = "",
}: {
  assistantIds: string[];
  assistants: AssistantInfo[];
  labelAssistants?: AssistantInfo[];
  onChange: (assistantIds: string[]) => void;
  className?: string;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number; width: number } | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const selected = new Set(assistantIds);
  const labelSource = labelAssistants ?? assistants;
  const selectedItems = selectedAssistants(assistantIds, labelSource);

  const updatePosition = useCallback(() => {
    if (!open) return;
    const button = buttonRef.current;
    if (!button) return;
    const rect = button.getBoundingClientRect();
    const width = Math.max(224, rect.width);
    const maxLeft = Math.max(ASSISTANT_MENU_MARGIN, window.innerWidth - width - ASSISTANT_MENU_MARGIN);
    const estimatedHeight = Math.min(
      ASSISTANT_MENU_MAX_HEIGHT,
      Math.max(40, assistants.length * 32 + 12),
    );
    const roomBelow = window.innerHeight - rect.bottom - ASSISTANT_MENU_MARGIN;
    const top =
      roomBelow >= estimatedHeight
        ? rect.bottom + ASSISTANT_MENU_GAP
        : rect.top - estimatedHeight - ASSISTANT_MENU_GAP;
    const maxTop = Math.max(ASSISTANT_MENU_MARGIN, window.innerHeight - estimatedHeight - ASSISTANT_MENU_MARGIN);
    setPos({
      top: Math.round(Math.max(ASSISTANT_MENU_MARGIN, Math.min(top, maxTop))),
      left: Math.round(Math.max(ASSISTANT_MENU_MARGIN, Math.min(rect.left, maxLeft))),
      width,
    });
  }, [assistants.length, open]);

  useLayoutEffect(() => {
    updatePosition();
  }, [updatePosition]);

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (buttonRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onMouseDown);
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open, updatePosition]);

  const toggle = (assistantId: string) => {
    if (selected.has(assistantId)) {
      onChange(assistantIds.filter((id) => id !== assistantId));
      return;
    }
    onChange([...assistantIds, assistantId]);
  };

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        onClick={() => setOpen((value) => !value)}
        className={"inline-flex h-7 min-w-[150px] items-center gap-1 overflow-hidden border-r border-ink/10 text-caption text-ink/65 outline-none hover:text-ink " + className}
      >
        {selectedItems.length > 0 ? (
          <span className="flex min-w-0 flex-1 items-center gap-1 overflow-hidden">
            {selectedItems.map((assistant) => (
              <span key={assistant.id} className="inline-flex min-w-0 shrink items-center gap-1">
                <AssistantBotIcon color={assistant.color} className="h-3.5 w-3.5 shrink-0 text-ink/40" />
                <span className="truncate">{assistant.name}</span>
              </span>
            ))}
          </span>
        ) : (
          <span className="min-w-0 flex-1 truncate text-left">{t("assistant.empty")}</span>
        )}
      </button>
      {open &&
        pos &&
        createPortal(
          <div
            ref={menuRef}
            onWheel={(event) => event.stopPropagation()}
            className="fixed overflow-hidden rounded-lg border border-ink/10 bg-surface-panel p-1.5 shadow-lg"
            style={{
              top: pos.top,
              left: pos.left,
              width: pos.width,
              maxHeight: ASSISTANT_MENU_MAX_HEIGHT,
              zIndex: 90,
            }}
          >
            <ScrollArea className="max-h-[248px] overscroll-contain">
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
                    <span className="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border border-ink/15 bg-ink/5">
                      {selected.has(assistant.id) && <Check className="h-3 w-3" />}
                    </span>
                    <AssistantBotIcon color={assistant.color} className="h-3.5 w-3.5 shrink-0 text-ink/40" />
                    <span className="min-w-0 flex-1 truncate">{assistant.name}</span>
                  </button>
                ))
              )}
            </ScrollArea>
          </div>,
          document.body,
        )}
    </>
  );
}

function StageRow({
  thread,
  stage,
  sessions,
  linkedSessionKeys,
  active,
  onThreadUpdated,
  onStageUpdated,
  onStageDeleted,
  onSelectSession,
  onError,
}: {
  thread: ThreadInfo;
  stage: StageInfo;
  sessions: SessionInfo[];
  linkedSessionKeys: Set<string>;
  active: boolean;
  onThreadUpdated: (thread: ThreadInfo) => void;
  onStageUpdated: (stage: StageInfo) => void;
  onStageDeleted: (threadId: string, stageId: string) => void;
  onSelectSession: (session: SessionInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
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
          {projectStageIcon(stage, "h-4 w-4 shrink-0 text-ink/45")}
          <span>{stage.order + 1}.</span>
          <span>{projectStageLabel(stage, t)}</span>
          {active && <span className="rounded bg-ink/10 px-1.5 py-0.5 text-meta text-ink/50">{t("thread.active")}</span>}
        </button>
        <div className="flex items-center gap-1">
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
  loading: boolean;
  compact?: boolean;
  onAssistantCreated: (assistant: AssistantInfo) => void;
  onAssistantUpdated: (assistant: AssistantInfo) => void;
  onAssistantDeleted: (assistantId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [showCreate, setShowCreate] = useState(false);
  const [tab, setTab] = useState<"builtin" | "custom">("builtin");
  const projectAssistants = assistants.filter((assistant) => assistant.projectId === project.id);
  const builtin = projectAssistants.filter((assistant) => assistant.type === "builtin");
  const custom = projectAssistants.filter((assistant) => assistant.type === "custom");
  const visible = tab === "builtin" ? builtin : custom;

  return (
    <aside className={compact ? "min-w-0 rounded-lg border border-ink/10 bg-surface-panel p-3" : "min-w-0"}>
      {loading ? (
        <div className="py-8 text-center text-body-sm text-ink/40">{t("memory_search.searching")}</div>
      ) : (
        <div className="rounded-lg border border-card-border/[0.12] bg-ink/[0.025] p-5">
          <SegmentedTabs
            items={[
              { value: "builtin", label: t("assistant.builtin"), badge: builtin.length },
              { value: "custom", label: t("assistant.custom"), badge: custom.length },
            ]}
            value={tab}
            onChange={setTab}
            variant="underline"
            itemWidth={132}
            itemHeight={34}
            className="mb-4"
            endAdornment={
              <button type="button" onClick={() => setShowCreate(true)} className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2.5 text-body-sm font-medium text-card-fg/75 transition hover:text-card-fg/90">
                <Plus className="h-3.5 w-3.5" />
                {t("assistant.add")}
              </button>
            }
          />
          <div className="grid gap-3">
            {visible.map((assistant) => (
              <AssistantCard
                key={assistant.id}
                assistant={assistant}
                agents={agents}
                onUpdated={onAssistantUpdated}
                onDeleted={onAssistantDeleted}
                onError={onError}
              />
            ))}
            {visible.length === 0 && <div className="rounded-md border border-dashed border-ink/10 py-8 text-center text-body-sm text-ink/35">{t("assistant.empty")}</div>}
          </div>
        </div>
      )}
      {showCreate && (
        <CreateAssistantDialog
          agents={agents}
          projectId={project.id}
          onCreated={onAssistantCreated}
          onClose={() => setShowCreate(false)}
          onError={onError}
        />
      )}
    </aside>
  );
}
