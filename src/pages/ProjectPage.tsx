import {
  useEffect,
  useLayoutEffect,
  useMemo,
  type RefObject,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { DragDropProvider, type DragEndEvent } from "@dnd-kit/react";
import { isSortable, useSortable } from "@dnd-kit/react/sortable";
import HashIcon from "@iconify-react/mynaui/hash";
import Robot3LineIcon from "@iconify-react/ri/robot-3-line";
import HashtagChatLinearIcon from "@iconify-react/solar/hashtag-chat-linear";
import { Check, ChevronDown, Copy, GripVertical, Link2, LoaderCircle, Pencil, Plus, Trash2, Workflow, X } from "lucide-react";
import type { AgentInfo, AssistantInfo, ProjectInfo, ProjectStageInfo, SessionInfo, StageInfo, ThreadInfo } from "../api";
import { AGENT_LABEL, addThreadStage, createThread, deleteThread, deleteThreadStage, listAgents, listAssistants, listProjectStages, listThreads, setThreadStage, updateThread, updateThreadStage } from "../api";
import { AgentGlyph } from "../components/AgentIcon";
import CreateAssistantDialog from "../components/CreateAssistantDialog";
import CreateStageDialog from "../components/CreateStageDialog";
import AssistantCard from "../components/AssistantCard";
import ConfirmTooltip from "../components/ConfirmTooltip";
import MultiPicker from "../components/MultiPicker";
import StageList from "../components/StageList";
import StageSelectChip from "../components/StageSelectChip";
import Tooltip from "../components/Tooltip";
import { localeTag, useI18n } from "../i18n";
import ScrollArea from "../components/ScrollArea";
import SegmentedTabs, { type SegmentedTabItem } from "../components/SegmentedTabs";
import { projectStageIcon, projectStageLabel } from "../utils/stageDisplay";

type ProjectView = "threads" | "stages" | "assistants";
type ThreadPanelView = "threads" | "thread-chats";

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

function formatShortRelativeTime(ts: number | null, t: (key: string, vars?: Record<string, string | number>) => string): string {
  if (!ts) return "";
  const diffMs = Math.max(0, Date.now() - ts);
  const minute = 60 * 1000;
  const hour = 60 * minute;
  const day = 24 * hour;
  const week = 7 * day;
  const month = 30 * day;
  if (diffMs < hour) return t("time.minute", { count: Math.max(1, Math.floor(diffMs / minute)) });
  if (diffMs < day) return t("time.hour", { count: Math.floor(diffMs / hour) });
  if (diffMs < week) return t("time.day", { count: Math.floor(diffMs / day) });
  if (diffMs < month) return t("time.week", { count: Math.floor(diffMs / week) });
  return t("time.month", { count: Math.floor(diffMs / month) });
}


export function ProjectWorkbenchPage({
  project,
  onNewThreadChat,
  onSelectSession,
  onError,
}: {
  project: ProjectInfo;
  onNewThreadChat: (thread: ThreadInfo) => void;
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
      prev.map((thread) => {
        if (thread.id !== stage.threadId) return thread;
        const currentStage = thread.stages.find((item) => item.id === stage.id);
        if (!currentStage) return thread;
        const stages =
          currentStage.order === stage.order
            ? thread.stages
                .map((item) => (item.id === stage.id ? stage : item))
                .sort((a, b) => a.order - b.order)
            : reorderThreadStages(thread.stages, stage);
        return {
          ...thread,
          updatedAt: Math.max(thread.updatedAt, stage.updatedAt),
          stages,
        };
      }),
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
              onNewThreadChat={onNewThreadChat}
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

function reorderThreadStages(stages: StageInfo[], movedStage: StageInfo): StageInfo[] {
  const ordered = [...stages].sort((a, b) => a.order - b.order);
  const withoutMoved = ordered.filter((stage) => stage.id !== movedStage.id);
  const targetIndex = Math.max(0, Math.min(movedStage.order, withoutMoved.length));
  withoutMoved.splice(targetIndex, 0, movedStage);
  return withoutMoved.map((stage, index) => ({ ...stage, order: index }));
}

function stageActivationBody(
  fromIndex: number,
  toIndex: number,
  stageLabel: string,
  t: (key: string, vars?: Record<string, string | number>) => string,
): string {
  if (fromIndex < 0) return t("thread.activate_stage_body", { stage: stageLabel });
  if (toIndex < fromIndex) return t("thread.activate_stage_back_body", { stage: stageLabel });
  if (toIndex > fromIndex + 1) return t("thread.activate_stage_skip_body", { stage: stageLabel });
  return t("thread.activate_stage_forward_body", { stage: stageLabel });
}

function ThreadWorkflowPanel({
  project,
  threads,
  projectStages,
  loading,
  onThreadCreated,
  onThreadUpdated,
  onThreadDeleted,
  onStageAdded,
  onStageUpdated,
  onStageDeleted,
  onSelectSession,
  onNewThreadChat,
  onError,
}: {
  project: ProjectInfo;
  threads: ThreadInfo[];
  projectStages: ProjectStageInfo[];
  loading: boolean;
  onThreadCreated: (thread: ThreadInfo) => void;
  onThreadUpdated: (thread: ThreadInfo) => void;
  onThreadDeleted: (threadId: string) => void;
  onStageAdded: (stage: StageInfo) => void;
  onStageUpdated: (stage: StageInfo) => void;
  onStageDeleted: (threadId: string, stageId: string) => void;
  onSelectSession: (session: SessionInfo) => void;
  onNewThreadChat: (thread: ThreadInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [goal, setGoal] = useState("");
  const [description, setDescription] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  const [panelView, setPanelView] = useState<ThreadPanelView>("threads");
  const [selectedThreadChatThreadId, setSelectedThreadChatThreadId] = useState<string | null>(null);
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
    }
    return keys;
  }, [threads]);
  const threadPanelTabs = useMemo<SegmentedTabItem<ThreadPanelView>[]>(
    () => [
      { value: "threads", label: t("thread.title"), icon: HashIcon, badge: threads.length },
      { value: "thread-chats", label: t("thread.chats"), icon: HashtagChatLinearIcon, badge: linkedSessionKeys.size },
    ],
    [linkedSessionKeys.size, t, threads.length],
  );
  const threadChatSessions = useMemo(() => {
    const byKey = new Map<string, SessionInfo>();
    const sourceThreads = selectedThreadChatThreadId
      ? threads.filter((thread) => thread.id === selectedThreadChatThreadId)
      : threads;
    for (const thread of sourceThreads) {
      for (const session of thread.sessions) byKey.set(sessionIdentityKey(session), session);
    }
    return Array.from(byKey.values()).sort((a, b) => (b.updatedAt ?? b.startedAt ?? 0) - (a.updatedAt ?? a.startedAt ?? 0));
  }, [selectedThreadChatThreadId, threads]);

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

  useEffect(() => {
    if (selectedThreadChatThreadId && !threads.some((thread) => thread.id === selectedThreadChatThreadId)) {
      setSelectedThreadChatThreadId(null);
    }
  }, [selectedThreadChatThreadId, threads]);

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
    <div className="min-w-0 rounded-lg border border-card-border/[0.12] bg-ink/[0.025] p-5">
        <SegmentedTabs
          items={threadPanelTabs}
          value={panelView}
          onChange={(value) => {
            setPanelView(value);
            if (value === "threads") setSelectedThreadChatThreadId(null);
          }}
          variant="underline"
          itemWidth={148}
          itemHeight={34}
          className="mb-4"
          endAdornment={
            panelView === "threads" ? (
              <button
                type="button"
                onClick={() => setCreateOpen(true)}
                className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2.5 text-body-sm font-medium text-card-fg/75 transition hover:text-card-fg/90"
              >
                <Plus className="h-3.5 w-3.5" />
                {t("thread.add")}
              </button>
            ) : null
          }
        />
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
        ) : panelView === "thread-chats" ? (
          <ThreadChatList
            sessions={threadChatSessions}
            onSelectSession={onSelectSession}
          />
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
                onThreadUpdated={onThreadUpdated}
                onThreadDeleted={onThreadDeleted}
                onStageAdded={onStageAdded}
                onStageUpdated={onStageUpdated}
                onStageDeleted={onStageDeleted}
                onShowSessions={(threadId) => {
                  setSelectedThreadChatThreadId(threadId);
                  setPanelView("thread-chats");
                }}
                onNewThreadChat={onNewThreadChat}
                onError={onError}
              />
            ))}
          </div>
        )}
    </div>
  );
}

function ThreadChatList({
  sessions,
  onSelectSession,
}: {
  sessions: SessionInfo[];
  onSelectSession: (session: SessionInfo) => void;
}) {
  const { t } = useI18n();
  if (sessions.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-ink/15 py-12 text-center text-body-sm text-ink/40">
        {t("thread.no_chats")}
      </div>
    );
  }
  return (
    <div className="grid gap-2">
      {sessions.map((session) => (
        <button
          key={sessionIdentityKey(session)}
          type="button"
          onClick={() => onSelectSession(session)}
          className="flex min-w-0 items-center gap-3 rounded-lg border border-ink/10 bg-surface-panel px-3 py-2 text-left transition hover:bg-ink/[0.035]"
        >
          <AgentGlyph agent={session.agent} className="h-4 w-4 shrink-0" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-body-sm font-medium text-ink/75">
              {session.title ?? session.firstUserMessage ?? t("list.no_user_message")}
            </span>
            <span className="mt-0.5 block text-caption text-ink/40">
              {AGENT_LABEL[session.agent]} · {t("list.msgs", { count: session.messageCount })}
            </span>
          </span>
          <span className="shrink-0 text-meta tabular-nums text-ink/35">
            {formatShortRelativeTime(session.updatedAt ?? session.startedAt, t)}
          </span>
        </button>
      ))}
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

function ThreadStageChip({
  stage,
  index,
  active,
  locked,
  confirmationBody,
  confirmLabel,
  removeBody,
  onSelect,
  onRemove,
}: {
  stage: StageInfo;
  index: number;
  active: boolean;
  locked: boolean;
  confirmationBody: string;
  confirmLabel: string;
  removeBody: string;
  onSelect: (stageId: string) => void | Promise<void>;
  onRemove: () => void;
}) {
  const { t } = useI18n();
  const { handleRef, isDragSource, isDropTarget, ref } = useSortable({
    id: stage.id,
    index,
    group: `thread-stages-${stage.threadId}`,
    transition: {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
      idle: true,
    },
  });
  const label = projectStageLabel(stage, t);
  const showActions = !locked && !active;

  return (
    <ConfirmTooltip>
      {(confirm) => (
        <div
          ref={ref}
          className={
            "inline-flex h-7 items-center gap-1.5 rounded-md border px-1.5 text-caption transition duration-150 " +
            (isDragSource
              ? "z-20 cursor-grabbing border-ink/30 bg-surface-panel shadow-lg "
              : isDropTarget
                ? "border-ink/35 bg-ink/12 shadow-[inset_2px_0_0_rgb(var(--color-fg)/0.28)] "
                : active
                  ? "border-brand/45 bg-brand/15 text-ink shadow-[inset_0_0_0_1px_rgb(var(--color-brand)/0.12)] "
                  : "border-ink/10 bg-surface-panel text-ink/50 hover:bg-ink/5 hover:text-ink/70 ") +
            (locked ? "opacity-90 " : "")
          }
        >
          {showActions && (
            <Tooltip content={t("stage.reorder")} placement="top">
              <button
                ref={handleRef}
                type="button"
                className="cursor-grab touch-none rounded p-0.5 text-current/50 hover:bg-ink/5 active:cursor-grabbing"
              >
                <GripVertical className="h-3.5 w-3.5" />
              </button>
            </Tooltip>
          )}
          <Tooltip content={active ? t("thread.active") : confirmLabel} placement="top">
            <button
              type="button"
              onClick={(event) => {
                if (active) return;
                confirm(event, {
                  title: confirmLabel,
                  body: confirmationBody,
                  confirmLabel,
                  placement: "top",
                  onConfirm: () => onSelect(stage.id),
                });
              }}
              className="inline-flex min-w-0 items-center gap-1.5 text-left"
            >
              {projectStageIcon(stage)}
              <span className="max-w-[140px] truncate">{label}</span>
            </button>
          </Tooltip>
          {showActions && (
            <Tooltip content={t("thread.remove_stage")} placement="top">
              <button
                type="button"
                aria-label={t("thread.remove_stage")}
                onClick={(event) =>
                  confirm(event, {
                    title: t("delete.title"),
                    body: removeBody,
                    confirmLabel: t("delete.confirm"),
                    placement: "top",
                    onConfirm: onRemove,
                  })
                }
                className="rounded p-0.5 text-current/35 hover:bg-status-error/10 hover:text-status-error"
              >
                <X className="h-3 w-3" />
              </button>
            </Tooltip>
          )}
        </div>
      )}
    </ConfirmTooltip>
  );
}

function ThreadCard({
  thread,
  projectStages,
  onThreadUpdated,
  onThreadDeleted,
  onStageAdded,
  onStageUpdated,
  onStageDeleted,
  onShowSessions,
  onNewThreadChat,
  onError,
}: {
  thread: ThreadInfo;
  projectStages: ProjectStageInfo[];
  onThreadUpdated: (thread: ThreadInfo) => void;
  onThreadDeleted: (threadId: string) => void;
  onStageAdded: (stage: StageInfo) => void;
  onStageUpdated: (stage: StageInfo) => void;
  onStageDeleted: (threadId: string, stageId: string) => void;
  onShowSessions: (threadId: string) => void;
  onNewThreadChat: (thread: ThreadInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [editing, setEditing] = useState(false);
  const [expandedText, setExpandedText] = useState(false);
  const [textOverflowing, setTextOverflowing] = useState(false);
  const [goal, setGoal] = useState(thread.goal);
  const [description, setDescription] = useState(thread.description ?? "");
  const [selectedStageIds, setSelectedStageIds] = useState<string[]>([]);
  const goalRef = useRef<HTMLDivElement>(null);
  const descriptionRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setGoal(thread.goal);
    setDescription(thread.description ?? "");
    setExpandedText(false);
    setTextOverflowing(false);
  }, [thread.description, thread.goal]);

  useLayoutEffect(() => {
    const measure = () => {
      if (expandedText && textOverflowing) return;
      const goalNode = goalRef.current;
      const descriptionNode = descriptionRef.current;
      const goalOverflow = goalNode ? goalNode.scrollWidth > goalNode.clientWidth + 1 : false;
      const descriptionOverflow = descriptionNode
        ? descriptionNode.scrollWidth > descriptionNode.clientWidth + 1 || Boolean(thread.description?.includes("\n"))
        : false;
      setTextOverflowing(goalOverflow || descriptionOverflow);
    };
    measure();
    const observer = new ResizeObserver(measure);
    if (goalRef.current) observer.observe(goalRef.current);
    if (descriptionRef.current) observer.observe(descriptionRef.current);
    return () => observer.disconnect();
  }, [expandedText, textOverflowing, thread.description, thread.goal]);

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
    if (selectedStageIds.length === 0) return;
    try {
      for (const stageId of selectedStageIds) {
        onStageAdded(await addThreadStage(thread.id, stageId, []));
      }
      setSelectedStageIds([]);
    } catch (err) {
      onError(String(err));
    }
  };

  const currentStageId = thread.stageId ?? "";
  const orderedThreadStages = useMemo(
    () => [...thread.stages].sort((a, b) => a.order - b.order),
    [thread.stages],
  );
  const activeStageIndex = orderedThreadStages.findIndex((stage) => stage.id === currentStageId);
  const isStageLocked = (index: number) => activeStageIndex >= 0 && index <= activeStageIndex;
  const handleThreadStageDragEnd = (event: DragEndEvent) => {
    if (event.canceled) return;
    const { source } = event.operation;
    if (!isSortable(source)) return;
    const from = source.initialIndex;
    const to = source.index;
    if (from === to || isStageLocked(from) || isStageLocked(to)) return;
    const stage = orderedThreadStages[from];
    const target = orderedThreadStages[to];
    if (!stage || !target) return;
    void (async () => {
      try {
        onStageUpdated(await updateThreadStage(stage.id, { order: target.order }));
      } catch (err) {
        onError(String(err));
      }
    })();
  };
  const removeThreadStage = async (stage: StageInfo, locked: boolean) => {
    if (locked) return;
    try {
      await deleteThreadStage(stage.id);
      onStageDeleted(stage.threadId, stage.id);
    } catch (err) {
      onError(String(err));
    }
  };
  const availableProjectStages = useMemo(
    () =>
      projectStages.filter(
        (stage) =>
          stage.enabled &&
          stageAllowsThreadAddition(stage) &&
          !thread.stages.some((threadStage) => threadStage.stageId === stage.id),
      ),
    [projectStages, thread.stages],
  );
  const availableProjectStageOptions = useMemo(
    () =>
      availableProjectStages.map((stage) => ({
        value: stage.id,
        label: projectStageLabel(stage, t),
        icon: projectStageIcon(stage),
      })),
    [availableProjectStages, t],
  );
  const linkedSessionCount = thread.sessions.length;

  useEffect(() => {
    const availableIds = new Set(availableProjectStages.map((stage) => stage.id));
    setSelectedStageIds((current) => current.filter((id) => availableIds.has(id)));
  }, [availableProjectStages]);

  return (
    <section className="rounded-lg border border-ink/10 bg-surface-panel p-3 shadow-sm">
      {editing ? (
        <div className="grid gap-2">
          <div className="flex items-center gap-3">
            <input
              value={goal}
              onChange={(event) => setGoal(event.target.value)}
              className="min-w-0 flex-1 rounded-md border border-ink/10 bg-ink/5 px-2 py-1.5 text-body-sm text-ink outline-none"
            />
            <div className="flex shrink-0 items-center gap-1">
              <button type="button" onClick={() => setEditing(false)} className="rounded px-2 py-1 text-caption text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
              <button type="button" onClick={() => void save()} className="rounded bg-ink px-2 py-1 text-caption text-[rgb(var(--color-bg-panel))]">{t("project.save")}</button>
            </div>
          </div>
          <textarea
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            rows={2}
            className="min-w-0 resize-none rounded-md border border-ink/10 bg-ink/5 px-2 py-1.5 text-body-sm text-ink outline-none"
          />
        </div>
      ) : (
        <div className="min-w-0">
          <div className="flex items-start gap-3">
            <div
              ref={goalRef}
              className={
                "min-w-0 flex-1 text-body font-medium text-ink/85 " +
                (expandedText ? "break-words" : "truncate")
              }
            >
              <span>{thread.goal}</span>
              <Tooltip content={t("thread.chats")} placement="top">
                <button
                  type="button"
                  onClick={() => onShowSessions(thread.id)}
                  className="ml-2 inline-flex items-center gap-1 rounded px-1 align-middle text-caption font-normal text-ink/40 transition hover:bg-ink/5 hover:text-ink/65"
                >
                  <Link2 className="h-3.5 w-3.5 shrink-0" />
                  {t("thread.chats_count", { count: linkedSessionCount })}
                </button>
              </Tooltip>
            </div>
            <ConfirmTooltip>
              {(confirm) => (
                <div className="flex shrink-0 items-center gap-1">
                  <Tooltip content={t("thread.new_chat")} placement="top">
                    <button type="button" onClick={() => onNewThreadChat(thread)} className="rounded p-1.5 text-ink/35 hover:bg-ink/5 hover:text-ink/70">
                      <HashtagChatLinearIcon className="h-3.5 w-3.5" />
                    </button>
                  </Tooltip>
                  <Tooltip content={t("thread.edit")} placement="top">
                    <button type="button" onClick={() => setEditing(true)} className="rounded p-1.5 text-ink/35 hover:bg-ink/5 hover:text-ink/70"><Pencil className="h-3.5 w-3.5" /></button>
                  </Tooltip>
                  <Tooltip content={t("thread.remove")} placement="top">
                    <button
                      type="button"
                      onClick={(event) =>
                        confirm(event, {
                          title: t("thread.remove"),
                          body: t("thread.delete_body", { goal: thread.goal }),
                          confirmLabel: t("thread.remove"),
                          placement: "top",
                          onConfirm: remove,
                        })
                      }
                      className="rounded p-1.5 text-ink/35 hover:bg-status-error/10 hover:text-status-error"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </Tooltip>
                </div>
              )}
            </ConfirmTooltip>
          </div>
          {thread.description && (
            <div
              ref={descriptionRef}
              className={
                "mt-1 text-body-sm text-ink/50 " +
                (expandedText ? "whitespace-pre-wrap break-words" : "truncate")
              }
            >
              {thread.description}
            </div>
          )}
          {textOverflowing && (
            <Tooltip content={t(expandedText ? "detail.collapse" : "detail.expand")} placement="top">
              <button
                type="button"
                onClick={() => setExpandedText((value) => !value)}
                aria-expanded={expandedText}
                className="mt-1 inline-flex h-5 items-center gap-1 rounded px-1.5 text-caption text-ink/40 transition hover:bg-ink/5 hover:text-ink/65"
              >
                <span>{t(expandedText ? "detail.collapse" : "detail.expand")}</span>
                <ChevronDown className={"h-3 w-3 transition-transform " + (expandedText ? "rotate-180" : "")} />
              </button>
            </Tooltip>
          )}
          {(orderedThreadStages.length > 0 || availableProjectStages.length > 0) && (
            <DragDropProvider onDragEnd={handleThreadStageDragEnd}>
              <div className="mt-2 flex flex-wrap gap-1.5">
                {orderedThreadStages.map((stage, index) => {
                  const locked = isStageLocked(index);
                  const label = projectStageLabel(stage, t);
                  return (
                    <ThreadStageChip
                      key={stage.id}
                      stage={stage}
                      index={index}
                      active={stage.id === currentStageId}
                      locked={locked}
                      confirmationBody={stageActivationBody(activeStageIndex, index, label, t)}
                      confirmLabel={t("thread.activate_stage")}
                      removeBody={t("thread.delete_stage_body", { stage: label })}
                      onSelect={async (stageId) => {
                        if (stageId === currentStageId) return;
                        try {
                          onThreadUpdated(await setThreadStage(thread.id, stageId));
                        } catch (err) {
                          onError(String(err));
                        }
                      }}
                      onRemove={() => void removeThreadStage(stage, locked)}
                    />
                  );
                })}
                {availableProjectStages.length > 0 && (
                  <div className="inline-flex h-7 min-w-0 items-center overflow-hidden rounded-md border border-ink/10 bg-ink/[0.035]">
                    <MultiPicker
                      selectedValues={selectedStageIds}
                      options={availableProjectStageOptions}
                      onChange={setSelectedStageIds}
                      placeholder={t("stage.add")}
                    />
                    <Tooltip content={t("stage.add_selected")} placement="top">
                      <button
                        type="button"
                        disabled={selectedStageIds.length === 0}
                        onClick={() => void add()}
                        className="inline-flex h-7 w-7 shrink-0 items-center justify-center border-l border-ink/10 text-ink/45 transition hover:bg-ink/5 hover:text-ink/75 disabled:cursor-default disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-ink/45"
                      >
                        <Plus className="h-3.5 w-3.5" />
                      </button>
                    </Tooltip>
                  </div>
                )}
              </div>
            </DragDropProvider>
          )}
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
