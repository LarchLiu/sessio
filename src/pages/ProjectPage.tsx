import {
  type CSSProperties,
  useEffect,
  useLayoutEffect,
  type MouseEvent,
  type ReactNode,
  useMemo,
  type RefObject,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { DragDropProvider, type DragEndEvent } from "@dnd-kit/react";
import { isSortable, useSortable } from "@dnd-kit/react/sortable";
import {
  ArrowDown,
  ArrowUp,
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  FileCode2,
  FilePlus2,
  GitBranch,
  GitCommitHorizontal,
  GitCompareArrows,
  GitPullRequestArrow,
  FolderOpen,
  GripVertical,
  Link2,
  LoaderCircle,
  Minus,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  Undo2,
  Upload,
  Workflow,
  X,
} from "lucide-react";
import { FileTree, useFileTree, useFileTreeSearch, useFileTreeSelection } from "@pierre/trees/react";
import type {
  Agent,
  AgentInfo,
  AssistantInfo,
  ProjectGitAction,
  ProjectGitChange,
  ProjectGitCommit,
  ProjectGitState,
  ProjectGitStatus,
  ProjectGitStatusEntry,
  ProjectInfo,
  ProjectStageInfo,
  SessionInfo,
  StageInfo,
  ThreadAgentInfo,
  ThreadInfo,
  ThreadKind,
} from "../api";
import {
  AGENT_LABEL,
  addThreadStage,
  createThread,
  deleteThread,
  deleteThreadStage,
  getProjectGitStatus,
  getProjectGitSummary,
  getProjectGitState,
  isAgent,
  listAgents,
  listAssistants,
  listProjectFiles,
  listProjectStages,
  listProjectGitCommits,
  listThreads,
  runProjectGitAction,
  updateThread,
  updateThreadStage,
} from "../api";
import { AgentGlyph } from "../components/AgentIcon";
import CreateAssistantDialog from "../components/CreateAssistantDialog";
import CreateStageDialog from "../components/CreateStageDialog";
import AssistantCard from "../components/AssistantCard";
import ConfirmTooltip from "../components/ConfirmTooltip";
import MultiPicker from "../components/MultiPicker";
import StageList from "../components/StageList";
import StageSelectChip from "../components/StageSelectChip";
import Tooltip from "../components/Tooltip";
import { HashIcon, Robot3LineIcon } from "../components/IconifyIcon";
import { localeTag, useI18n } from "../i18n";
import ScrollArea from "../components/ScrollArea";
import SegmentedTabs, { type SegmentedTabItem } from "../components/SegmentedTabs";
import { projectStageIcon, projectStageLabel, stageStatusVisual } from "../utils/stageDisplay";
import { sessionDisplayTitle } from "../appUtils";

const CANVAS_ADD_FILES_EVENT = "sessio:canvas-add-files";

export type ProjectView = "threads" | "stages" | "assistants" | "files" | "sourceControl";
type ThreadPanelView = "threads" | "thread-chats";
const THREAD_KINDS: ThreadKind[] = ["process", "teamwork", "brainstorm", "debate"];
const AGENT_PARTICIPANT_KINDS = new Set<ThreadKind>(["brainstorm", "debate"]);
const GIT_COMMIT_PAGE_SIZE = 20;
const RIGHT_SIDEBAR_COMPACT_BREAKPOINT = 360;

function resizeGitCommitMessage(el: HTMLTextAreaElement) {
  el.style.height = "auto";
  el.style.height = `${Math.max(el.scrollHeight, 44)}px`;
  el.style.overflowY = "hidden";
}

function sessionIdentityKey(s: SessionInfo): string {
  return `${s.agent}:${s.id}`;
}

function collectThreadLinkedSessions(thread: ThreadInfo): SessionInfo[] {
  const byKey = new Map<string, SessionInfo>();
  for (const session of thread.sessions) {
    byKey.set(sessionIdentityKey(session), session);
  }
  for (const stage of thread.stages) {
    for (const session of stage.sessions) {
      byKey.set(sessionIdentityKey(session), session);
    }
  }
  return Array.from(byKey.values());
}

function stageAllowsThreadAddition(stage: ProjectStageInfo): boolean {
  return stage.assistants.length > 0 || stage.allowEmptyAssistants;
}

function assistantSwatch(color: string | null | undefined) {
  return (
    <span
      className="h-2.5 w-2.5 shrink-0 rounded-full border border-ink/10"
      style={{ backgroundColor: color ?? "rgb(var(--color-brand))" }}
    />
  );
}

function threadAgentOptions(agents: AgentInfo[]) {
  return agents
    .filter((agent) => agent.enabled && isRuntimeAgentId(agent.id))
    .sort((a, b) => a.order - b.order || a.displayName.localeCompare(b.displayName))
    .map((agent) => ({
      value: agent.id,
      label: threadAgentLabel(agent),
      icon: <AgentGlyph agent={agent.id as Agent} className="h-3.5 w-3.5" />,
    }));
}

function selectedAgentParticipants(ids: string[], agents: AgentInfo[]): ThreadAgentInfo[] {
  const byId = new Map(agents.map((agent) => [agent.id, agent]));
  return ids
    .map((id, index) => {
      const agent = byId.get(id);
      if (!agent || !isRuntimeAgentId(agent.id)) return null;
      const model = agent.model ?? agent.models.find((option) => option.enabled && option.value.trim())?.value ?? "";
      const effort = agent.effort ?? agent.efforts.find((option) => option.enabled && option.value.trim())?.value ?? "";
      const permissionMode = agent.permissionMode ?? agent.permissionModes.find((option) => option.enabled && option.value.trim())?.value ?? "";
      if (!model || !effort || !permissionMode) return null;
      return {
        participantId: "",
        agent: agent.id,
        model,
        effort,
        permissionMode,
        order: index,
      };
    })
    .filter((participant): participant is ThreadAgentInfo => Boolean(participant));
}

function threadCreateBlocked(kind: ThreadKind, assistantIds: string[], participantIds: string[], agents: AgentInfo[]): boolean {
  if (kind === "teamwork") return assistantIds.length === 0;
  const participants = selectedAgentParticipants(participantIds, agents);
  if (kind === "brainstorm") return participants.length < 2;
  if (kind === "debate") return participants.length !== 2;
  return false;
}

function threadAgentLabel(agent: AgentInfo): string {
  const model = agent.model ?? agent.models.find((option) => option.enabled && option.value.trim())?.displayName ?? "";
  const base = agent.displayName || agent.name || agent.id;
  return model ? `${base} · ${model}` : base;
}

function threadParticipantLabel(participant: ThreadAgentInfo): string {
  const agentLabel = AGENT_LABEL[participant.agent] ?? participant.agent;
  return participant.model ? `${agentLabel} · ${participant.model}` : agentLabel;
}

function AssistantChoiceChip({
  assistant,
  selected,
  onToggle,
}: {
  assistant: { value: string; label: string; icon?: ReactNode };
  selected: boolean;
  onToggle: (assistantId: string) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onToggle(assistant.value)}
      className={
        "inline-flex h-7 max-w-[220px] items-center gap-1.5 rounded-md border px-1.5 text-caption transition duration-150 " +
        (selected
          ? "border-ink/[0.14] bg-ink/[0.048] text-ink/70"
          : "border-ink/[0.08] bg-surface-panel text-ink/45 hover:bg-ink/[0.04] hover:text-ink/65")
      }
    >
      {assistant.icon}
      <span className="min-w-0 truncate">{assistant.label}</span>
      <span className="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border border-ink/[0.35] bg-ink/[0.04] text-ink/75">
        {selected && <Check className="h-3 w-3" />}
      </span>
    </button>
  );
}

function isRuntimeAgentId(value: string): value is Agent {
  return isAgent(value);
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
    { label: t("meta.title"), value: sessionDisplayTitle(session), clampLines: 2 },
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

function formatShortDate(ts: number | null, lang: string): string {
  if (!ts) return "";
  return new Date(ts).toLocaleString(localeTag(lang === "zh" ? "zh" : "en"), {
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
  onSelectThreadChatSession,
  onError,
  view: viewProp,
  onViewChange,
  hideTabs = false,
  filesReloadKey = 0,
  activeCanvasSessionId = null,
  onOpenFile,
  onAddFileToCanvas,
  projectHasGit,
  onProjectGitRepoDetected,
}: {
  project: ProjectInfo;
  onSelectThreadChatSession: (session: SessionInfo) => void;
  onError: (error: string | null) => void;
  view?: ProjectView;
  onViewChange?: (view: ProjectView) => void;
  hideTabs?: boolean;
  filesReloadKey?: number;
  activeCanvasSessionId?: string | null;
  onOpenFile?: (path: string) => void;
  onAddFileToCanvas?: (paths: string[] | string) => void;
  projectHasGit?: boolean;
  onProjectGitRepoDetected?: (projectPath: string, isRepo: boolean) => void;
}) {
  const { t } = useI18n();
  const projectViewTabs = useMemo<SegmentedTabItem<ProjectView>[]>(
    () => [
      { value: "threads", label: t("thread.title"), icon: HashIcon },
      { value: "stages", label: t("project.processTemplateId"), icon: Workflow },
      { value: "assistants", label: t("assistant.title"), icon: Robot3LineIcon },
      { value: "files", label: t("project.files"), icon: FolderOpen },
      { value: "sourceControl", label: t("project.source_control"), icon: GitBranch },
    ],
    [t],
  );
  const [threads, setThreads] = useState<ThreadInfo[]>([]);
  const [projectStages, setProjectStages] = useState<ProjectStageInfo[]>([]);
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [processLoading, setProcessTemplateLoading] = useState(true);
  const [internalView, setInternalView] = useState<ProjectView>("threads");
  const [compactMode, setCompactMode] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const activeView = viewProp ?? internalView;
  const setActiveView = (next: ProjectView) => {
    if (onViewChange) onViewChange(next);
    if (viewProp === undefined) setInternalView(next);
  };

  useEffect(() => {
    let cancelled = false;
    setProcessTemplateLoading(true);
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
        if (!cancelled) setProcessTemplateLoading(false);
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

  useLayoutEffect(() => {
    if (!hideTabs) {
      setCompactMode(false);
      return;
    }
    const node = containerRef.current;
    if (!node) return;
    const update = () => {
      setCompactMode(node.getBoundingClientRect().width < RIGHT_SIDEBAR_COMPACT_BREAKPOINT);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(node);
    return () => observer.disconnect();
  }, [hideTabs]);

  return (
    <div
      ref={containerRef}
      className={
        "flex h-full min-h-0 flex-1 flex-col overflow-hidden " +
        (hideTabs ? "bg-transparent" : "bg-surface-panel")
      }
    >
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {!hideTabs && (
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
        )}
        {activeView === "files" ? (
          <ProjectFilesPanel
            project={project}
            reloadKey={filesReloadKey}
            activeCanvasSessionId={activeCanvasSessionId}
            onOpenFile={onOpenFile}
            onAddFileToCanvas={onAddFileToCanvas}
            projectHasGit={projectHasGit}
            onProjectGitRepoDetected={onProjectGitRepoDetected}
          />
        ) : activeView === "sourceControl" ? (
          <ProjectSourceControlPanel
            project={project}
            reloadKey={filesReloadKey}
            onOpenFile={onOpenFile}
            onError={onError}
            onProjectGitRepoDetected={onProjectGitRepoDetected}
          />
        ) : (
          <ScrollArea
            className="min-h-0 flex-1"
            viewportClassName={hideTabs ? "px-5 pb-5 pt-5" : "px-5 pb-5 pt-4"}
          >
            {activeView === "threads" && (
              <ThreadProcessTemplatePanel
                project={project}
                threads={threads}
                projectStages={projectStages}
                assistants={assistants}
                agents={agents}
                loading={processLoading}
                compact={compactMode}
                sidebarMode={hideTabs}
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
                onSelectThreadChatSession={onSelectThreadChatSession}
                onError={onError}
              />
            )}
            {activeView === "stages" && (
              <ProjectStagePicker
                project={project}
                stages={projectStages}
                assistants={assistants}
                compact={compactMode}
                sidebarMode={hideTabs}
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
                loading={processLoading}
                compact={compactMode}
                sidebarMode={hideTabs}
                compactMode={compactMode}
                onAssistantCreated={(assistant) => setAssistants((prev) => [...prev, assistant])}
                onAssistantUpdated={patchAssistant}
                onAssistantDeleted={(assistantId) => setAssistants((prev) => prev.filter((assistant) => assistant.id !== assistantId))}
                onError={onError}
              />
            )}
          </ScrollArea>
        )}
      </div>
    </div>
  );
}

function ProjectFilesPanel({
  project,
  reloadKey = 0,
  activeCanvasSessionId = null,
  onOpenFile,
  onAddFileToCanvas,
  projectHasGit,
  onProjectGitRepoDetected,
}: {
  project: ProjectInfo;
  reloadKey?: number;
  activeCanvasSessionId?: string | null;
  onOpenFile?: (path: string) => void;
  onAddFileToCanvas?: (paths: string[] | string) => void;
  projectHasGit?: boolean;
  onProjectGitRepoDetected?: (projectPath: string, isRepo: boolean) => void;
}) {
  const { t } = useI18n();
  const [paths, setPaths] = useState<string[] | null>(null);
  const [gitStatus, setGitStatus] = useState<ProjectGitStatusEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const pathsRef = useRef<string[] | null>(null);
  const requestIdRef = useRef(0);

  useEffect(() => {
    pathsRef.current = paths;
  }, [paths]);

  useEffect(() => {
    if (!project.path) {
      setPaths(null);
      setGitStatus([]);
      setError(t("project.files_path_missing"));
      setLoading(false);
      setRefreshing(false);
    }
  }, [project.path, t]);

  const loadProjectFiles = async ({
    background,
    detectGit,
  }: {
    background: boolean;
    detectGit?: boolean;
  }) => {
    const currentRequestId = requestIdRef.current + 1;
    requestIdRef.current = currentRequestId;
    if (!project.path) return;

    if (background) setRefreshing(true);
    else {
      setLoading(true);
      setError(null);
    }

    try {
      const [rows, gitStatusResult, gitSummary] = await Promise.all([
        listProjectFiles(project.path),
        projectHasGit ? getProjectGitStatus(project.path).catch(() => [] as ProjectGitStatusEntry[]) : Promise.resolve([] as ProjectGitStatusEntry[]),
        detectGit && !projectHasGit
          ? getProjectGitSummary(project.path).catch(() => null)
          : Promise.resolve(null),
      ]);
      if (requestIdRef.current !== currentRequestId) return;
      setPaths(rows);
      setGitStatus(gitStatusResult);
      if (gitSummary) onProjectGitRepoDetected?.(project.path, gitSummary.isRepo);
      setError(null);
    } catch (err) {
      if (requestIdRef.current !== currentRequestId) return;
      setError(String(err));
      if (!background || pathsRef.current === null) {
        setPaths([]);
        setGitStatus([]);
      }
    } finally {
      if (requestIdRef.current !== currentRequestId) return;
      setLoading(false);
      setRefreshing(false);
    }
  };

  useEffect(() => {
    if (!project.path) return;
    setPaths(null);
    pathsRef.current = null;
    setGitStatus([]);
    setError(null);
    setLoading(true);
    setRefreshing(false);
    void loadProjectFiles({ background: false, detectGit: false });
  }, [project.path]);

  useEffect(() => {
    if (!project.path || pathsRef.current === null) return;
    void loadProjectFiles({ background: true, detectGit: false });
  }, [project.path, reloadKey]);

  return (
    <div className={"flex min-h-0 flex-1 flex-col overflow-hidden py-4 "}>
      {loading && paths === null ? (
        <div className="flex flex-1 items-center justify-center text-body-sm text-ink/40">
          {t("project.files_loading")}
        </div>
      ) : error && paths === null ? (
        <div className="flex flex-1 items-center justify-center px-4 text-center text-body-sm text-ink/45">
          {error}
        </div>
      ) : (
        <ProjectFilesTree
          paths={paths ?? []}
          gitStatus={gitStatus}
          refreshing={refreshing}
          error={error}
          activeCanvasSessionId={activeCanvasSessionId}
          onRefresh={() => {
            void loadProjectFiles({ background: pathsRef.current !== null, detectGit: true });
          }}
          onOpenFile={onOpenFile}
          onAddFileToCanvas={onAddFileToCanvas}
        />
      )}
    </div>
  );
}

function ProjectFilesTree({
  paths,
  gitStatus,
  refreshing,
  error,
  activeCanvasSessionId,
  onRefresh,
  onOpenFile,
  onAddFileToCanvas,
}: {
  paths: string[];
  gitStatus: ProjectGitStatusEntry[];
  refreshing: boolean;
  error: string | null;
  activeCanvasSessionId: string | null;
  onRefresh: () => void;
  onOpenFile?: (path: string) => void;
  onAddFileToCanvas?: (paths: string[] | string) => void;
}) {
  const { t } = useI18n();
  const { model } = useFileTree({
    paths,
    initialExpansion: "closed",
    flattenEmptyDirectories: false,
    search: true,
    gitStatus,
    composition: {
      contextMenu: {
        enabled: true,
        triggerMode: "button",
        buttonVisibility: "when-needed",
      },
    },
    unsafeCSS: `
      [data-file-tree-search-container] {
        display: none !important;
      }
    `,
  });
  const search = useFileTreeSearch(model);
  const selectedPaths = useFileTreeSelection(model);
  const lastOpenedFileRef = useRef<string | null>(null);

  useEffect(() => {
    model.resetPaths(paths);
  }, [model, paths]);

  useEffect(() => {
    model.setGitStatus(gitStatus);
  }, [gitStatus, model]);

  useEffect(() => {
    const selectedPath = selectedPaths[0];
    if (!selectedPath || !onOpenFile) {
      lastOpenedFileRef.current = null;
      return;
    }
    const item = model.getItem(selectedPath);
    if (!item || item.isDirectory()) {
      lastOpenedFileRef.current = null;
      return;
    }
    if (lastOpenedFileRef.current === selectedPath) return;
    lastOpenedFileRef.current = selectedPath;
    onOpenFile(selectedPath);
  }, [model, onOpenFile, selectedPaths]);

  const treeStyle: CSSProperties & Record<string, string> = {
    height: "100%",
    width: "100%",
    backgroundColor: "transparent",
    "--trees-bg-override": "transparent",
    "--trees-bg-muted-override": "rgb(var(--color-fg) / 0.045)",
    "--trees-input-bg-override": "transparent",
    "--trees-search-bg-override": "transparent",
    "--trees-selected-bg-override": "rgb(var(--color-fg) / 0.08)",
    "--trees-border-color-override": "rgb(var(--color-fg) / 0.10)",
    "--trees-fg-override": "rgb(var(--color-fg) / 0.78)",
    "--trees-fg-muted-override": "rgb(var(--color-fg) / 0.42)",
    "--trees-search-fg-override": "rgb(var(--color-fg) / 0.78)",
    "--trees-selected-fg-override": "rgb(var(--color-fg) / 0.88)",
    "--trees-selected-focused-border-color-override": "rgb(var(--color-fg) / 0.16)",
    "--trees-focus-ring-color-override": "rgb(var(--color-fg) / 0.18)",
    "--trees-context-menu-trigger-inline-offset": "32px",
  };
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-0 px-4 pb-3">
        <label className="flex h-9 min-w-0 flex-1 items-center gap-2 rounded-md border border-ink/10 bg-surface-panel px-2.5 text-ink/70 transition-colors focus-within:border-ink/18 focus-within:text-ink/90">
          <Search className="h-4 w-4 shrink-0 text-ink/35" />
          <input
            type="search"
            value={search.value}
            onFocus={() => {
              if (!search.isOpen) search.open(search.value || undefined);
            }}
            onChange={(event) => {
              const value = event.target.value;
              if (value && !search.isOpen) search.open(value);
              search.setValue(value || null);
              if (!value) search.close();
            }}
            placeholder={t("header.search")}
            aria-label={t("header.search")}
            className="min-w-0 flex-1 bg-transparent text-body-sm text-ink outline-none placeholder:text-ink/30"
          />
        </label>
        <Tooltip content={t("project.files_refresh")} placement="bottom">
          <button
            type="button"
            aria-label={t("project.files_refresh")}
            onClick={onRefresh}
            className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-surface-panel text-ink/55 transition-colors hover:bg-ink/[0.04] hover:text-ink/85"
          >
            {refreshing ? (
              <LoaderCircle className="h-4 w-4 animate-spin" />
            ) : (
              <RefreshCw className="h-4 w-4" />
            )}
          </button>
        </Tooltip>
      </div>
      {error ? (
        <div className="mx-3 mb-3 rounded-md border border-status-error/30 bg-status-error/10 px-3 py-2 text-body-sm text-status-error">
          {error}
        </div>
      ) : null}
      <div className="relative min-h-0 flex-1">
        <FileTree
          model={model}
          className="h-full flex-1"
          style={treeStyle}
          renderContextMenu={(item, context) => {
            if (item.kind !== "file") return null;
            return (
              <ProjectFileTreeContextMenu
                path={item.path}
                context={context}
                activeCanvasSessionId={activeCanvasSessionId}
                onOpenFile={onOpenFile}
                onAddFileToCanvas={onAddFileToCanvas}
              />
            );
          }}
        />
        {paths.length === 0 ? (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center px-4 text-center text-body-sm text-ink/40">
            {t("project.files_empty")}
          </div>
        ) : null}
      </div>
    </div>
  );
}

function ProjectFileTreeContextMenu({
  path,
  context,
  activeCanvasSessionId,
  onOpenFile,
  onAddFileToCanvas,
}: {
  path: string;
  context: {
    anchorElement: HTMLElement;
    close: (options?: { restoreFocus?: boolean }) => void;
  };
  activeCanvasSessionId: string | null;
  onOpenFile?: (path: string) => void;
  onAddFileToCanvas?: (paths: string[] | string) => void;
}) {
  const menuWidth = 200;
  const menuHeight = 96;
  const viewportMargin = 12;
  const gap = 8;
  const triggerRect = context.anchorElement.getBoundingClientRect();
  const top = Math.min(
    Math.max(viewportMargin, triggerRect.top - 4),
    Math.max(viewportMargin, window.innerHeight - viewportMargin - menuHeight),
  );
  const left = Math.max(
    viewportMargin,
    Math.min(
      triggerRect.left - menuWidth - gap,
      window.innerWidth - viewportMargin - menuWidth,
    ),
  );

  return createPortal(
    <div
      data-file-tree-context-menu-root="true"
      className="fixed min-w-[200px] rounded-xl border border-ink/10 bg-surface-panel p-1.5 shadow-[0_20px_60px_rgba(0,0,0,0.22)]"
      style={{ top, left, zIndex: 1200 }}
      role="menu"
    >
      <button
        type="button"
        className="flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-body-sm text-ink/72 transition hover:bg-ink/[0.08] hover:text-ink"
        role="menuitem"
        onClick={() => {
          context.close({ restoreFocus: false });
          onOpenFile?.(path);
        }}
      >
        <FileCode2 className="h-4 w-4 shrink-0 text-ink/55" />
        <span>Add to code view</span>
      </button>
      {onAddFileToCanvas && (
        <button
          type="button"
          className="flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-body-sm text-ink/72 transition hover:bg-ink/[0.08] hover:text-ink"
          role="menuitem"
          onClick={() => {
            context.close({ restoreFocus: false });
            if (activeCanvasSessionId) {
              window.dispatchEvent(
                new CustomEvent(CANVAS_ADD_FILES_EVENT, {
                  detail: { paths: [path], sessionId: activeCanvasSessionId },
                }),
              );
            }
            onAddFileToCanvas(path);
          }}
        >
          <FilePlus2 className="h-4 w-4 shrink-0 text-ink/55" />
          <span>Add to canvas</span>
        </button>
      )}
    </div>,
    document.body,
  );
}

function ProjectSourceControlPanel({
  project,
  reloadKey = 0,
  onOpenFile,
  onError,
  onProjectGitRepoDetected,
}: {
  project: ProjectInfo;
  reloadKey?: number;
  onOpenFile?: (path: string) => void;
  onError: (error: string | null) => void;
  onProjectGitRepoDetected?: (projectPath: string, isRepo: boolean) => void;
}) {
  const { lang, t } = useI18n();
  const [state, setState] = useState<ProjectGitState | null>(null);
  const [commits, setCommits] = useState<ProjectGitCommit[]>([]);
  const [hasMoreCommits, setHasMoreCommits] = useState(false);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [activeGitAction, setActiveGitAction] = useState<ProjectGitAction | null>(null);
  const activeGitActionRef = useRef<ProjectGitAction | null>(null);
  const commitMessageRef = useRef<HTMLTextAreaElement>(null);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({
    staged: true,
    changes: true,
    untracked: true,
    history: false,
  });
  const requestIdRef = useRef(0);
  const graphOffsetRef = useRef(0);

  const loadSourceControl = async ({ background }: { background: boolean }) => {
    const currentRequestId = requestIdRef.current + 1;
    requestIdRef.current = currentRequestId;
    if (!project.path) {
      setState(null);
      setCommits([]);
      setHasMoreCommits(false);
      setError(t("project.files_path_missing"));
      setLoading(false);
      setRefreshing(false);
      return;
    }

    if (background) setRefreshing(true);
    else {
      setLoading(true);
      setError(null);
    }

    try {
      const [nextState, page] = await Promise.all([
        getProjectGitState(project.path),
        listProjectGitCommits(project.path, 0, GIT_COMMIT_PAGE_SIZE),
      ]);
      if (requestIdRef.current !== currentRequestId) return;
      setState(nextState);
      setCommits(page.commits);
      setHasMoreCommits(page.hasMore);
      graphOffsetRef.current = page.commits.length;
      onProjectGitRepoDetected?.(project.path, nextState.summary.isRepo);
      setError(null);
    } catch (err) {
      if (requestIdRef.current !== currentRequestId) return;
      setError(String(err));
      if (!background || state === null) {
        setState(null);
        setCommits([]);
        setHasMoreCommits(false);
        graphOffsetRef.current = 0;
      }
    } finally {
      if (requestIdRef.current !== currentRequestId) return;
      setLoading(false);
      setRefreshing(false);
    }
  };

  useEffect(() => {
    setState(null);
    setCommits([]);
    setHasMoreCommits(false);
    graphOffsetRef.current = 0;
    setError(null);
    setLoading(true);
    setRefreshing(false);
    void loadSourceControl({ background: false });
  }, [project.path]);

  useEffect(() => {
    if (!project.path || state === null) return;
    void loadSourceControl({ background: true });
  }, [project.path, reloadKey]);

  useLayoutEffect(() => {
    if (commitMessageRef.current) resizeGitCommitMessage(commitMessageRef.current);
  }, [message]);

  const staged = useMemo(
    () => (state?.changes ?? []).filter((change) => change.staged),
    [state?.changes],
  );
  const trackedChanges = useMemo(
    () => (state?.changes ?? []).filter((change) => !change.staged && change.status !== "untracked"),
    [state?.changes],
  );
  const untrackedChanges = useMemo(
    () => (state?.changes ?? []).filter((change) => !change.staged && change.status === "untracked"),
    [state?.changes],
  );

  const runAction = async (
    action: ProjectGitAction,
    options: { paths?: string[]; message?: string | null } = {},
  ) => {
    if (!project.path) return;
    if (activeGitActionRef.current) return;
    activeGitActionRef.current = action;
    setActiveGitAction(action);
    try {
      onError(null);
      await runProjectGitAction(project.path, action, options);
      if (action === "commit") setMessage("");
      await loadSourceControl({ background: true });
    } catch (err) {
      onError(String(err));
    } finally {
      activeGitActionRef.current = null;
      setActiveGitAction(null);
    }
  };

  const loadMoreCommits = async () => {
    if (!project.path || loadingMore || !hasMoreCommits) return;
    setLoadingMore(true);
    try {
      const page = await listProjectGitCommits(
        project.path,
        graphOffsetRef.current,
        GIT_COMMIT_PAGE_SIZE,
      );
      setCommits((current) => [...current, ...page.commits]);
      graphOffsetRef.current += page.commits.length;
      setHasMoreCommits(page.hasMore);
    } catch (err) {
      onError(String(err));
    } finally {
      setLoadingMore(false);
    }
  };

  const handleGraphScroll = (viewport: HTMLDivElement) => {
    if (
      hasMoreCommits &&
      !loadingMore &&
      viewport.scrollTop + viewport.clientHeight >= viewport.scrollHeight - 56
    ) {
      void loadMoreCommits();
    }
  };

  const toggleSection = (key: string) => {
    setExpanded((current) => ({ ...current, [key]: !current[key] }));
  };

  const summary = state?.summary ?? null;
  const canCommit = Boolean(message.trim()) && staged.length > 0;
  const gitActionBusy = activeGitAction !== null;

  if (loading && state === null) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center text-body-sm text-ink/40">
        {t("project.source_control_loading")}
      </div>
    );
  }

  if (error && state === null) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center px-4 text-center text-body-sm text-ink/45">
        {error}
      </div>
    );
  }

  if (!summary?.isRepo) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-body-sm text-ink/45">
        <GitBranch className="h-5 w-5 text-ink/35" />
        <div>{t("project.source_control_not_repo")}</div>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden py-4">
      <div className="shrink-0 px-4">
        <div className="flex min-w-0 flex-col gap-2">
          <div className="min-w-0">
            <div className="flex min-w-0 items-center justify-between gap-2">
              <div className="flex min-w-0 items-center gap-2 text-body-sm font-medium text-ink/85">
                <GitBranch className="h-4 w-4 shrink-0 text-ink/45" />
                <span className="min-w-0 truncate">{summary.branch ?? summary.head ?? "HEAD"}</span>
                {summary.ahead > 0 ? (
                  <span className="inline-flex items-center gap-0.5 rounded bg-ink/[0.06] px-1.5 py-0.5 text-caption text-ink/60">
                    <ArrowUp className="h-3 w-3" />
                    {summary.ahead}
                  </span>
                ) : null}
                {summary.behind > 0 ? (
                  <span className="inline-flex items-center gap-0.5 rounded bg-ink/[0.06] px-1.5 py-0.5 text-caption text-ink/60">
                    <ArrowDown className="h-3 w-3" />
                    {summary.behind}
                  </span>
                ) : null}
              </div>
              <GitIconButton
                label={t("project.source_control_refresh")}
                disabled={refreshing || gitActionBusy}
                onClick={() => void loadSourceControl({ background: true })}
                icon={refreshing ? LoaderCircle : RefreshCw}
                spin={refreshing}
              />
            </div>
            <div className="mt-1 min-w-0 truncate text-caption text-ink/45">
              {[
                summary.upstream,
                t("project.source_control_change_count", { count: state?.changes.length ?? 0 }),
                summary.head,
              ].filter(Boolean).join(" · ")}
            </div>
          </div>
          <div className="flex items-center gap-1 leading-none">
            <GitIconButton
              label={t("project.source_control_fetch")}
              disabled={gitActionBusy}
              onClick={() => void runAction("fetch")}
              icon={activeGitAction === "fetch" ? LoaderCircle : ArrowDown}
              spin={activeGitAction === "fetch"}
              className="h-6 w-8"
            />
            <GitIconButton
              label={t("project.source_control_sync")}
              disabled={gitActionBusy}
              onClick={() => void runAction("sync")}
              icon={activeGitAction === "sync" ? LoaderCircle : GitCompareArrows}
              spin={activeGitAction === "sync"}
              className="h-6 w-8"
            />
            <GitIconButton
              label={t("project.source_control_pull")}
              disabled={gitActionBusy}
              onClick={() => void runAction("pull")}
              icon={activeGitAction === "pull" ? LoaderCircle : GitPullRequestArrow}
              spin={activeGitAction === "pull"}
              className="h-6 w-8"
            />
            <GitIconButton
              label={t("project.source_control_push")}
              disabled={gitActionBusy}
              onClick={() => void runAction("push")}
              icon={activeGitAction === "push" ? LoaderCircle : Upload}
              spin={activeGitAction === "push"}
              className="h-6 w-8"
            />
          </div>
          <div className="mt-3 grid gap-2">
            <ScrollArea
              className="min-h-[44px] max-h-[96px] rounded-md border border-ink/10 bg-surface-panel transition-colors focus-within:border-ink/22"
              viewportClassName="max-h-[96px]"
              persistScrollbars
            >
              <textarea
                ref={commitMessageRef}
                value={message}
                onChange={(event) => {
                  resizeGitCommitMessage(event.currentTarget);
                  setMessage(event.target.value);
                }}
                onInput={(event) => resizeGitCommitMessage(event.currentTarget)}
                onKeyDown={(event) => {
                  if ((event.metaKey || event.ctrlKey) && event.key === "Enter" && canCommit && !gitActionBusy) {
                    event.preventDefault();
                    void runAction("commit", { message });
                  }
                }}
                rows={2}
                placeholder={t("project.source_control_commit_placeholder")}
                className="block min-h-[44px] w-full resize-none overflow-hidden bg-transparent px-3 py-2 pr-6 text-body-sm leading-5 text-ink outline-none placeholder:text-ink/35"
              />
            </ScrollArea>
            <Tooltip content={t("project.source_control_commit")} placement="bottom">
              <button
                type="button"
                disabled={!canCommit || gitActionBusy}
                onClick={() => void runAction("commit", { message })}
                className="inline-flex h-8 w-full items-center justify-center gap-1.5 rounded-md border border-transparent bg-ink/[0.1] px-3 text-body-sm font-medium text-ink/88 shadow-[inset_0_1px_0_rgb(var(--color-ink)/0.08)] transition hover:bg-ink/[0.14] hover:text-ink/95 disabled:bg-ink/[0.025] disabled:text-ink/45 disabled:shadow-none"
                aria-label={t("project.source_control_commit")}
              >
                <Check className="h-4 w-4" />
                <span>{t("project.source_control_commit")}</span>
              </button>
            </Tooltip>
          </div>
        </div>
        {error ? (
          <div className="mt-3 rounded-md border border-status-error/30 bg-status-error/10 px-3 py-2 text-body-sm text-status-error">
            {error}
          </div>
        ) : null}
      </div>

      <ScrollArea className="mt-3 min-h-0 flex-1" viewportClassName="px-4 pb-4">
        <GitChangeSection
          id="staged"
          title={t("project.source_control_staged")}
          count={staged.length}
          expanded={expanded.staged}
          onToggle={() => toggleSection("staged")}
          actions={
            staged.length > 0 ? (
              <>
                <GitIconButton
                  label={t("project.source_control_unstage_all")}
                  disabled={gitActionBusy}
                  onClick={() => void runAction("unstageAll")}
                  icon={Minus}
                />
              </>
            ) : null
          }
        >
          {staged.length > 0 ? (
            staged.map((change) => (
              <GitChangeRow
                key={`staged:${change.path}:${change.status}`}
                change={change}
                onOpenFile={onOpenFile}
                actions={
                  <>
                    <GitIconButton
                      label={t("project.source_control_unstage")}
                      disabled={gitActionBusy}
                      onClick={() => void runAction("unstage", { paths: [change.path] })}
                      icon={Minus}
                    />
                  </>
                }
              />
            ))
          ) : (
            <div className="px-2 py-2 text-caption text-ink/35">{t("project.source_control_empty_staged")}</div>
          )}
        </GitChangeSection>

        <GitChangeSection
          id="changes"
          title={t("project.source_control_changes")}
          count={trackedChanges.length}
          expanded={expanded.changes}
          onToggle={() => toggleSection("changes")}
          actions={
            trackedChanges.length > 0 ? (
              <>
                <GitIconButton
                  label={t("project.source_control_stage_all")}
                  disabled={gitActionBusy}
                  onClick={() => void runAction("stageAll")}
                  icon={Plus}
                />
                <ConfirmTooltip>
                  {(confirm) => (
                    <GitIconButton
                      label={t("project.source_control_discard_all")}
                      disabled={gitActionBusy}
                      onClick={(event) =>
                        confirm(event, {
                          title: t("project.source_control_discard_all_confirm"),
                          confirmLabel: t("confirm.ok"),
                          placement: "bottom",
                          onConfirm: () => runAction("discardAll"),
                        })
                      }
                      icon={Undo2}
                    />
                  )}
                </ConfirmTooltip>
              </>
            ) : null
          }
        >
          {trackedChanges.length > 0 ? (
            trackedChanges.map((change) => (
              <GitChangeRow
                key={`unstaged:${change.path}:${change.status}`}
                change={change}
                onOpenFile={onOpenFile}
                actions={
                  <>
                    <GitIconButton
                      label={t("project.source_control_stage")}
                      disabled={gitActionBusy}
                      onClick={() => void runAction("stage", { paths: [change.path] })}
                      icon={Plus}
                    />
                    <ConfirmTooltip>
                      {(confirm) => (
                        <GitIconButton
                          label={t("project.source_control_discard")}
                          disabled={gitActionBusy}
                          onClick={(event) =>
                            confirm(event, {
                              title: t("project.source_control_discard_confirm"),
                              body: change.path,
                              confirmLabel: t("confirm.ok"),
                              placement: "bottom",
                              onConfirm: () => runAction("discard", { paths: [change.path] }),
                            })
                          }
                          icon={Undo2}
                        />
                      )}
                    </ConfirmTooltip>
                  </>
                }
              />
            ))
          ) : (
            <div className="px-2 py-2 text-caption text-ink/35">{t("project.source_control_empty_changes")}</div>
          )}
        </GitChangeSection>

        <GitChangeSection
          id="untracked"
          title={t("project.source_control_untracked")}
          count={untrackedChanges.length}
          expanded={expanded.untracked}
          onToggle={() => toggleSection("untracked")}
          actions={
            untrackedChanges.length > 0 ? (
              <>
                <GitIconButton
                  label={t("project.source_control_stage_all_untracked")}
                  disabled={gitActionBusy}
                  onClick={() => void runAction("stage", { paths: untrackedChanges.map((change) => change.path) })}
                  icon={Plus}
                />
                <ConfirmTooltip>
                  {(confirm) => (
                    <GitIconButton
                      label={t("project.source_control_clean_all")}
                      disabled={gitActionBusy}
                      onClick={(event) =>
                        confirm(event, {
                          title: t("project.source_control_clean_all_confirm"),
                          confirmLabel: t("confirm.ok"),
                          placement: "bottom",
                          onConfirm: () => runAction("cleanAll"),
                        })
                      }
                      icon={Trash2}
                    />
                  )}
                </ConfirmTooltip>
              </>
            ) : null
          }
        >
          {untrackedChanges.length > 0 ? (
            untrackedChanges.map((change) => (
              <GitChangeRow
                key={`untracked:${change.path}:${change.status}`}
                change={change}
                onOpenFile={onOpenFile}
                actions={
                  <>
                    <GitIconButton
                      label={t("project.source_control_stage")}
                      disabled={gitActionBusy}
                      onClick={() => void runAction("stage", { paths: [change.path] })}
                      icon={Plus}
                    />
                    <ConfirmTooltip>
                      {(confirm) => (
                        <GitIconButton
                          label={t("project.source_control_clean")}
                          disabled={gitActionBusy}
                          onClick={(event) =>
                            confirm(event, {
                              title: t("project.source_control_clean_confirm"),
                              body: change.path,
                              confirmLabel: t("confirm.ok"),
                              placement: "bottom",
                              onConfirm: () => runAction("clean", { paths: [change.path] }),
                            })
                          }
                          icon={Trash2}
                        />
                      )}
                    </ConfirmTooltip>
                  </>
                }
              />
            ))
          ) : (
            <div className="px-2 py-2 text-caption text-ink/35">{t("project.source_control_empty_untracked")}</div>
          )}
        </GitChangeSection>

        <GitChangeSection
          id="history"
          title={t("project.source_control_history")}
          count={commits.length}
          expanded={expanded.history}
          onToggle={() => toggleSection("history")}
        >
          <ScrollArea
            className="max-h-[420px] min-h-[220px]"
            viewportClassName="py-1"
            onScroll={handleGraphScroll}
          >
            {commits.length > 0 ? (
              commits.map((commit, index) => (
                <GitCommitRow
                  key={commit.hash}
                  commit={commit}
                  lang={lang}
                  first={index === 0}
                  last={index === commits.length - 1 && !hasMoreCommits}
                />
              ))
            ) : (
              <div className="px-3 py-6 text-center text-body-sm text-ink/35">
                {t("project.source_control_history_empty")}
              </div>
            )}
            {loadingMore ? (
              <div className="flex items-center justify-center gap-2 px-3 py-3 text-caption text-ink/40">
                <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
                {t("project.source_control_loading_more")}
              </div>
            ) : hasMoreCommits ? (
              <button
                type="button"
                onClick={() => void loadMoreCommits()}
                disabled={gitActionBusy}
                className="flex w-full items-center justify-center px-3 py-3 text-caption text-ink/45 transition hover:bg-ink/[0.04] hover:text-ink/70"
              >
                {t("project.source_control_load_more")}
              </button>
            ) : null}
          </ScrollArea>
        </GitChangeSection>
      </ScrollArea>
    </div>
  );
}

function GitChangeSection({
  id,
  title,
  count,
  expanded,
  actions,
  onToggle,
  children,
}: {
  id: string;
  title: string;
  count: number;
  expanded: boolean;
  actions?: ReactNode;
  onToggle: () => void;
  children: ReactNode;
}) {
  return (
    <section className="mb-3" aria-labelledby={`git-section-${id}`}>
      <div className="group flex min-h-8 items-center gap-2 border-b border-ink/[0.06] px-2 py-1 text-body-sm text-ink/72 hover:bg-ink/[0.045]">
        <button
          type="button"
          onClick={onToggle}
          className="flex min-w-0 flex-1 items-center gap-2 text-left outline-none"
          aria-expanded={expanded}
          id={`git-section-${id}`}
        >
          {expanded ? (
            <ChevronDown className="h-4 w-4 shrink-0 text-ink/40" />
          ) : (
            <ChevronRight className="h-4 w-4 shrink-0 text-ink/40" />
          )}
          <span className="min-w-0 truncate">{title}</span>
          <span className="text-caption text-ink/35">{count}</span>
        </button>
        {actions ? (
          <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition group-hover:opacity-100 group-focus-within:opacity-100">
            {actions}
          </div>
        ) : null}
      </div>
      {expanded ? <div className="py-1">{children}</div> : null}
    </section>
  );
}

function GitChangeRow({
  change,
  actions,
  onOpenFile,
}: {
  change: ProjectGitChange;
  actions: ReactNode;
  onOpenFile?: (path: string) => void;
}) {
  return (
    <div className="group flex min-h-8 items-center gap-2 px-2 py-1 text-body-sm text-ink/72 hover:bg-ink/[0.045]">
      <span className={"w-4 shrink-0 text-center text-caption font-semibold " + gitStatusTextClass(change.status)}>
        {gitStatusLetter(change.status)}
      </span>
      <button
        type="button"
        onClick={() => onOpenFile?.(change.path)}
        className="min-w-0 flex-1 text-left"
      >
        <span className="block min-w-0 truncate">{change.path}</span>
        {change.originalPath ? (
          <span className="block min-w-0 truncate text-caption text-ink/35">{change.originalPath}</span>
        ) : null}
      </button>
      <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition group-hover:opacity-100 group-focus-within:opacity-100">
        {actions}
      </div>
    </div>
  );
}

function GitCommitRow({
  commit,
  lang,
  first,
  last,
}: {
  commit: ProjectGitCommit;
  lang: string;
  first: boolean;
  last: boolean;
}) {
  const nodeClass = commit.pushed
    ? "border-[rgb(var(--color-emerald)/0.55)] bg-[rgb(var(--color-emerald)/0.18)]"
    : "border-amber-500/65 bg-amber-500/20";
  const lineClass = commit.pushed ? "bg-[rgb(var(--color-emerald)/0.22)]" : "bg-amber-500/[0.28]";
  return (
    <Tooltip content={commit.message || commit.subject} placement="left" maxWidth={420}>
      <div className="grid grid-cols-[28px_minmax(0,1fr)] gap-2 px-3 py-2 text-body-sm hover:bg-ink/[0.035]">
        <div className="relative flex min-h-[38px] justify-center">
          <span
            className={
              "absolute left-1/2 w-px -translate-x-1/2 " +
              (first ? "top-[14px] " : "top-[-8px] ") +
              (last ? "bottom-[24px] " : "bottom-[-10px] ") +
              lineClass
            }
            aria-hidden
          />
          <span className={"relative z-10 mt-[9px] h-2.5 w-2.5 rounded-full border shadow-[0_0_0_2px_rgb(var(--color-bg-panel))] " + nodeClass} aria-hidden />
        </div>
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-2">
            <GitCommitHorizontal className="h-3.5 w-3.5 shrink-0 text-ink/35" />
            <span className="min-w-0 flex-1 truncate text-ink/78">{commit.subject}</span>
            <span className="shrink-0 rounded bg-ink/[0.06] px-1.5 py-0.5 font-mono text-[11px] text-ink/45">
              {commit.shortHash}
            </span>
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-1.5 text-caption text-ink/38">
            <span className="truncate">{commit.author}</span>
            <span>{formatShortDate(commit.timestamp, lang)}</span>
            {commit.refs.slice(0, 3).map((ref) => (
              <span key={ref} className="max-w-[140px] truncate rounded bg-ink/[0.055] px-1.5 py-0.5 text-ink/50">
                {ref}
              </span>
            ))}
          </div>
        </div>
      </div>
    </Tooltip>
  );
}

function GitIconButton({
  label,
  icon: Icon,
  onClick,
  disabled,
  spin,
  className = "h-7 w-7",
}: {
  label: string;
  icon: typeof RefreshCw;
  onClick: (event: MouseEvent<HTMLButtonElement>) => void;
  disabled?: boolean;
  spin?: boolean;
  className?: string;
}) {
  return (
    <Tooltip content={label} placement="bottom">
      <button
        type="button"
        aria-label={label}
        disabled={disabled}
        onClick={onClick}
        className={"inline-flex items-center justify-center rounded-md text-ink/45 transition hover:bg-ink/[0.055] hover:text-ink/78 disabled:opacity-35 " + className}
      >
        <Icon className={"h-3.5 w-3.5 " + (spin ? "animate-spin" : "")} />
      </button>
    </Tooltip>
  );
}

function gitStatusLetter(status: ProjectGitStatus): string {
  switch (status) {
    case "added":
      return "A";
    case "deleted":
      return "D";
    case "ignored":
      return "I";
    case "modified":
      return "M";
    case "renamed":
      return "R";
    case "untracked":
      return "U";
  }
}

function gitStatusTextClass(status: ProjectGitStatus): string {
  switch (status) {
    case "added":
      return "text-[rgb(var(--color-emerald))]";
    case "deleted":
      return "text-status-error";
    case "renamed":
      return "text-sky-500";
    case "untracked":
      return "text-amber-500";
    case "ignored":
      return "text-ink/30";
    case "modified":
      return "text-[rgb(var(--color-brand))]";
  }
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

function canChangeThreadStageStructure(stage: StageInfo): boolean {
  return stage.status !== "completed" && stage.status !== "skipped";
}

function threadStageChipStatusClass(stage: StageInfo): string {
  switch (stage.status) {
    case "completed":
      return "border-[rgb(var(--color-emerald)/0.62)] bg-[rgb(var(--color-emerald)/0.14)] text-ink shadow-[inset_0_0_0_1px_rgb(var(--color-emerald)/0.16)] ";
    case "in_progress":
      return "border-[rgb(var(--color-emerald)/0.45)] bg-[rgb(var(--color-emerald)/0.10)] text-ink shadow-[inset_0_0_0_1px_rgb(var(--color-emerald)/0.10)] hover:bg-[rgb(var(--color-emerald)/0.14)] ";
    case "needs_review":
      return "border-sky-500/45 bg-sky-500/10 text-ink shadow-[inset_0_0_0_1px_rgb(14_165_233/0.10)] hover:bg-sky-500/[0.14] ";
    case "blocked":
      return "border-amber-500/55 bg-amber-500/10 text-ink shadow-[inset_0_0_0_1px_rgb(245_158_11/0.12)] hover:bg-amber-500/[0.14] ";
    case "skipped":
      return "border-ink/18 bg-ink/[0.045] text-ink/70 shadow-[inset_0_0_0_1px_rgb(var(--color-fg)/0.04)] ";
    case "not_started":
    default:
      return "border-ink/10 bg-surface-panel text-ink hover:border-ink/18 hover:bg-ink/5 ";
  }
}

function ThreadProcessTemplatePanel({
  project,
  threads,
  projectStages,
  assistants,
  agents,
  loading,
  compact = false,
  sidebarMode = false,
  onThreadCreated,
  onThreadUpdated,
  onThreadDeleted,
  onStageAdded,
  onStageUpdated,
  onStageDeleted,
  onSelectThreadChatSession,
  onError,
}: {
  project: ProjectInfo;
  threads: ThreadInfo[];
  projectStages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  agents: AgentInfo[];
  loading: boolean;
  compact?: boolean;
  sidebarMode?: boolean;
  onThreadCreated: (thread: ThreadInfo) => void;
  onThreadUpdated: (thread: ThreadInfo) => void;
  onThreadDeleted: (threadId: string) => void;
  onStageAdded: (stage: StageInfo) => void;
  onStageUpdated: (stage: StageInfo) => void;
  onStageDeleted: (threadId: string, stageId: string) => void;
  onSelectThreadChatSession: (session: SessionInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [goal, setGoal] = useState("");
  const [description, setDescription] = useState("");
  const [createKind, setCreateKind] = useState<ThreadKind>("process");
  const [createAssistantIds, setCreateAssistantIds] = useState<string[]>([]);
  const [createAgentParticipantIds, setCreateAgentParticipantIds] = useState<string[]>([]);
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
  const projectAssistants = useMemo(
    () => assistants.filter((assistant) => assistant.projectId === project.id && assistant.enabled),
    [assistants, project.id],
  );
  const assistantOptions = useMemo(
    () =>
      projectAssistants.map((assistant) => ({
        value: assistant.id,
        label: assistant.name,
        icon: assistantSwatch(assistant.color),
      })),
    [projectAssistants],
  );
  const agentParticipantOptions = useMemo(
    () => threadAgentOptions(agents),
    [agents],
  );
  const threadKindTabs = useMemo<SegmentedTabItem<ThreadKind>[]>(
    () => THREAD_KINDS.map((kind) => ({ value: kind, label: t(`thread.kind.${kind}`) })),
    [t],
  );
  const linkedSessionKeys = useMemo(() => {
    const keys = new Set<string>();
    for (const thread of threads) {
      for (const session of collectThreadLinkedSessions(thread)) {
        keys.add(sessionIdentityKey(session));
      }
    }
    return keys;
  }, [threads]);
  const threadPanelTabs = useMemo<SegmentedTabItem<ThreadPanelView>[]>(
    () => [
      { value: "threads", label: "Threads", badge: threads.length },
      { value: "thread-chats", label: "Chats", badge: linkedSessionKeys.size },
    ],
    [linkedSessionKeys.size, threads.length],
  );
  const threadChatSessions = useMemo(() => {
    const byKey = new Map<string, SessionInfo>();
    const sourceThreads = selectedThreadChatThreadId
      ? threads.filter((thread) => thread.id === selectedThreadChatThreadId)
      : threads;
    for (const thread of sourceThreads) {
      for (const session of collectThreadLinkedSessions(thread)) {
        byKey.set(sessionIdentityKey(session), session);
      }
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

  useEffect(() => {
    if (createKind === "teamwork") {
      setCreateAssistantIds((current) => {
        const valid = new Set(assistantOptions.map((option) => option.value));
        const existing = current.filter((id) => valid.has(id));
        return existing.length > 0 ? existing : assistantOptions.map((option) => option.value);
      });
    } else {
      setCreateAssistantIds([]);
    }
    if (AGENT_PARTICIPANT_KINDS.has(createKind)) {
      setCreateAgentParticipantIds((current) => {
        const valid = new Set(agentParticipantOptions.map((option) => option.value));
        const existing = current.filter((id) => valid.has(id));
        return createKind === "debate" ? existing.slice(0, 2) : existing;
      });
    } else {
      setCreateAgentParticipantIds([]);
    }
  }, [agentParticipantOptions, assistantOptions, createKind]);

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
      const thread = await createThread(
        project.id,
        nextGoal,
        description,
        createKind,
        createKind === "teamwork" ? createAssistantIds : [],
        AGENT_PARTICIPANT_KINDS.has(createKind)
          ? selectedAgentParticipants(createAgentParticipantIds, agents)
          : [],
      );
      let nextThread = thread;
      const stageIds = createKind === "process"
        ? createStageOrder.filter((id) => selectedStageIds.includes(id))
        : [];
      for (const stageId of stageIds) {
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
      setCreateKind("process");
      setCreateAssistantIds([]);
      setCreateAgentParticipantIds([]);
      setCreateOpen(false);
    } catch (err) {
      onError(String(err));
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className={sidebarMode || compact ? "min-w-0" : "min-w-0 rounded-lg border border-card-border/[0.12] bg-ink/[0.025] p-5"}>
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
              <Tooltip content={t("thread.add")} placement="top">
                <button
                  type="button"
                  aria-label={t("thread.add")}
                  onClick={() => setCreateOpen(true)}
                  className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-card-fg/75 transition hover:bg-ink/5 hover:text-card-fg/90"
                >
                  <Plus className="h-3.5 w-3.5" />
                </button>
              </Tooltip>
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
                  <SegmentedTabs
                    items={threadKindTabs}
                    value={createKind}
                    onChange={setCreateKind}
                    itemWidth={118}
                    itemHeight={30}
                    className="w-max"
                  />
                  {createKind === "teamwork" && assistantOptions.length > 0 && (
                    <div className="flex flex-wrap items-center gap-1.5">
                      {assistantOptions.map((assistant) => (
                        <AssistantChoiceChip
                          key={assistant.value}
                          assistant={assistant}
                          selected={createAssistantIds.includes(assistant.value)}
                          onToggle={(assistantId) => {
                            setCreateAssistantIds((current) =>
                              current.includes(assistantId)
                                ? current.filter((id) => id !== assistantId)
                                : [...current, assistantId],
                            );
                          }}
                        />
                      ))}
                    </div>
                  )}
                  {AGENT_PARTICIPANT_KINDS.has(createKind) && agentParticipantOptions.length > 0 && (
                    <div className="inline-flex h-8 w-max min-w-0 items-center overflow-hidden rounded-md border border-ink/10 bg-ink/[0.035]">
                      <MultiPicker
                        selectedValues={createAgentParticipantIds}
                        options={agentParticipantOptions}
                        onChange={(values) => setCreateAgentParticipantIds(createKind === "debate" ? values.slice(0, 2) : values)}
                        placeholder={t("new_chat.add_participant")}
                        className="h-8 max-w-[340px]"
                      />
                    </div>
                  )}
                  {createKind === "process" && orderedCreateStages.length > 0 && (
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
                    disabled={creating || !goal.trim() || threadCreateBlocked(createKind, createAssistantIds, createAgentParticipantIds, agents)}
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
            compact={compact}
            onSelectThreadChatSession={onSelectThreadChatSession}
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
                assistants={assistants}
                agents={agents}
                compact={compact}
                onThreadUpdated={onThreadUpdated}
                onThreadDeleted={onThreadDeleted}
                onStageAdded={onStageAdded}
                onStageUpdated={onStageUpdated}
                onStageDeleted={onStageDeleted}
                onShowSessions={(threadId) => {
                  setSelectedThreadChatThreadId(threadId);
                  setPanelView("thread-chats");
                }}
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
  compact = false,
  onSelectThreadChatSession,
}: {
  sessions: SessionInfo[];
  compact?: boolean;
  onSelectThreadChatSession: (session: SessionInfo) => void;
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
          onClick={() => onSelectThreadChatSession(session)}
          className={
            "flex min-w-0 items-center gap-3 rounded-lg border border-ink/10 px-3 py-2 text-left transition hover:bg-ink/[0.035] " +
            (compact ? "bg-ink/[0.025]" : "bg-surface-panel")
          }
        >
          <AgentGlyph agent={session.agent} className="h-4 w-4 shrink-0" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-body-sm font-medium text-ink/75">
              {sessionDisplayTitle(session) ?? t("list.no_user_message")}
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
  locked,
  removeBody,
  onRemove,
}: {
  stage: StageInfo;
  index: number;
  locked: boolean;
  removeBody: string;
  onRemove: () => void;
}) {
  const { t } = useI18n();
  const statusVisual = stageStatusVisual(stage.status);
  const StatusIcon = statusVisual.icon;
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
  const chipStatusClass = threadStageChipStatusClass(stage);
  const showActions = !locked;

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
                : chipStatusClass) +
            (!locked ? "cursor-pointer " : "") +
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
          <Tooltip content={t(`stage.status.${stage.status}`)} placement="top">
            <div className="inline-flex min-w-0 items-center gap-1.5 text-left">
              {projectStageIcon(stage)}
              <span className="max-w-[140px] truncate">{label}</span>
              <span
                className={
                  "inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-full border " +
                  statusVisual.markerClass
                }
              >
                <StatusIcon className="h-2.5 w-2.5" />
              </span>
            </div>
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
  assistants,
  agents,
  compact = false,
  onThreadUpdated,
  onThreadDeleted,
  onStageAdded,
  onStageUpdated,
  onStageDeleted,
  onShowSessions,
  onError,
}: {
  thread: ThreadInfo;
  projectStages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  agents: AgentInfo[];
  compact?: boolean;
  onThreadUpdated: (thread: ThreadInfo) => void;
  onThreadDeleted: (threadId: string) => void;
  onStageAdded: (stage: StageInfo) => void;
  onStageUpdated: (stage: StageInfo) => void;
  onStageDeleted: (threadId: string, stageId: string) => void;
  onShowSessions: (threadId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [editing, setEditing] = useState(false);
  const [expandedText, setExpandedText] = useState(false);
  const [textOverflowing, setTextOverflowing] = useState(false);
  const [goal, setGoal] = useState(thread.goal);
  const [description, setDescription] = useState(thread.description ?? "");
  const [editKind, setEditKind] = useState<ThreadKind>(thread.kind);
  const [editAssistantIds, setEditAssistantIds] = useState<string[]>(
    () => thread.assistants.map((assistant) => assistant.assistantId),
  );
  const [editAgentParticipantIds, setEditAgentParticipantIds] = useState<string[]>(
    () => thread.agentParticipants.map((participant) => participant.agent),
  );
  const [selectedStageIds, setSelectedStageIds] = useState<string[]>([]);
  const goalRef = useRef<HTMLDivElement>(null);
  const descriptionRef = useRef<HTMLDivElement>(null);
  const assistantOptions = useMemo(
    () =>
      assistants
        .filter((assistant) => assistant.projectId === thread.projectId && assistant.enabled)
        .map((assistant) => ({
          value: assistant.id,
          label: assistant.name,
          icon: assistantSwatch(assistant.color),
        })),
    [assistants, thread.projectId],
  );
  const agentParticipantOptions = useMemo(
    () => threadAgentOptions(agents),
    [agents],
  );
  const visibleAssistants = useMemo(
    () => [...thread.assistants].sort((a, b) => a.order - b.order),
    [thread.assistants],
  );
  const visibleAgentParticipants = useMemo(
    () => [...thread.agentParticipants].sort((a, b) => a.order - b.order),
    [thread.agentParticipants],
  );

  useEffect(() => {
    setGoal(thread.goal);
    setDescription(thread.description ?? "");
    setEditKind(thread.kind);
    setEditAssistantIds(thread.assistants.map((assistant) => assistant.assistantId));
    setEditAgentParticipantIds(thread.agentParticipants.map((participant) => participant.agent));
    setExpandedText(false);
    setTextOverflowing(false);
  }, [thread.agentParticipants, thread.assistants, thread.description, thread.goal, thread.kind]);

  useEffect(() => {
    if (editKind === "teamwork") {
      setEditAssistantIds((current) => {
        const valid = new Set(assistantOptions.map((option) => option.value));
        const existing = current.filter((id) => valid.has(id));
        return existing.length > 0 ? existing : assistantOptions.map((option) => option.value);
      });
    } else {
      setEditAssistantIds([]);
    }
    if (AGENT_PARTICIPANT_KINDS.has(editKind)) {
      setEditAgentParticipantIds((current) => {
        const valid = new Set(agentParticipantOptions.map((option) => option.value));
        const existing = current.filter((id) => valid.has(id));
        return editKind === "debate" ? existing.slice(0, 2) : existing;
      });
    } else {
      setEditAgentParticipantIds([]);
    }
  }, [agentParticipantOptions, assistantOptions, editKind]);

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
      onThreadUpdated(await updateThread(thread.id, {
        goal,
        description,
        kind: editKind,
        assistantIds: editKind === "teamwork" ? editAssistantIds : [],
        agentParticipants: AGENT_PARTICIPANT_KINDS.has(editKind)
          ? selectedAgentParticipants(editAgentParticipantIds, agents)
          : [],
      }));
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

  const orderedThreadStages = useMemo(
    () => [...thread.stages].sort((a, b) => a.order - b.order),
    [thread.stages],
  );
  const isStageLocked = (stage: StageInfo) => !canChangeThreadStageStructure(stage);
  const handleThreadStageDragEnd = (event: DragEndEvent) => {
    if (event.canceled) return;
    const { source } = event.operation;
    if (!isSortable(source)) return;
    const from = source.initialIndex;
    const to = source.index;
    if (from === to) return;
    const stage = orderedThreadStages[from];
    const target = orderedThreadStages[to];
    if (!stage || !target) return;
    if (isStageLocked(stage) || isStageLocked(target)) return;
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
  const linkedSessionCount = collectThreadLinkedSessions(thread).length;

  useEffect(() => {
    const availableIds = new Set(availableProjectStages.map((stage) => stage.id));
    setSelectedStageIds((current) => current.filter((id) => availableIds.has(id)));
  }, [availableProjectStages]);

  return (
    <section
      className={
        "rounded-lg border border-ink/10 p-3 shadow-sm " +
        (compact ? "bg-ink/[0.025]" : "bg-surface-panel")
      }
    >
      {editing ? (
        <div className="grid gap-2">
          <div className="flex">
            <input
              value={goal}
              onChange={(event) => setGoal(event.target.value)}
              className="min-w-0 flex-1 rounded-md border border-ink/10 bg-ink/5 px-3 py-2 text-body-sm text-ink outline-none"
            />
          </div>
          <textarea
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            rows={2}
            className="min-w-0 resize-none rounded-md border border-ink/10 bg-ink/5 px-3 py-2 text-body-sm text-ink outline-none"
          />
          <div className="flex justify-end gap-2">
            <button type="button" onClick={() => setEditing(false)} className="inline-flex h-8 items-center justify-center rounded-md px-3 text-body-sm text-ink/50 transition hover:bg-ink/5 hover:text-ink">{t("delete.cancel")}</button>
            <button
              type="button"
              disabled={threadCreateBlocked(editKind, editAssistantIds, editAgentParticipantIds, agents)}
              onClick={() => void save()}
              className="inline-flex h-8 items-center justify-center rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] hover:text-card-fg/90 disabled:cursor-not-allowed disabled:opacity-35 disabled:hover:border-card-border/[0.12] disabled:hover:bg-card-chip/[0.08] disabled:hover:text-card-fg/75"
            >
              {t("project.save")}
            </button>
          </div>
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
          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            <span className="inline-flex h-6 items-center rounded-md border border-ink/10 bg-ink/[0.035] px-2 text-caption font-medium text-ink/55">
              {t(`thread.kind.${thread.kind}`)}
            </span>
            {thread.kind === "teamwork" && visibleAssistants.map((assistant) => (
                <span
                  key={assistant.assistantId}
                  className="inline-flex h-6 max-w-[180px] items-center gap-1.5 rounded-md border border-ink/10 bg-ink/[0.025] px-2 text-caption text-ink/55"
                >
                  {assistantSwatch(assistant.color)}
                  <span className="truncate">{assistant.name}</span>
                </span>
              ))}
            {AGENT_PARTICIPANT_KINDS.has(thread.kind) && visibleAgentParticipants.map((participant) => (
                <span
                  key={participant.participantId}
                  className="inline-flex h-6 max-w-[180px] items-center gap-1.5 rounded-md border border-ink/10 bg-ink/[0.025] px-2 text-caption text-ink/55"
                  title={[threadParticipantLabel(participant), participant.effort, participant.permissionMode].filter(Boolean).join(" / ")}
                >
                  <AgentGlyph agent={participant.agent} className="h-3.5 w-3.5 shrink-0" />
                  <span className="truncate">{threadParticipantLabel(participant)}</span>
                </span>
              ))}
            {thread.kind === "teamwork" && visibleAssistants.length === 0 && (
              <span className="inline-flex h-6 items-center rounded-md border border-dashed border-ink/15 px-2 text-caption text-ink/35">
                {t("thread.no_assistants")}
              </span>
            )}
            {AGENT_PARTICIPANT_KINDS.has(thread.kind) && visibleAgentParticipants.length === 0 && (
              <span className="inline-flex h-6 items-center rounded-md border border-dashed border-ink/15 px-2 text-caption text-ink/35">
                {t("new_chat.no_participants")}
              </span>
            )}
          </div>
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
          {thread.kind === "process" && (orderedThreadStages.length > 0 || availableProjectStages.length > 0) && (
            <DragDropProvider onDragEnd={handleThreadStageDragEnd}>
              <div className="mt-2 flex flex-wrap gap-1.5">
                {orderedThreadStages.map((stage, index) => {
                  const locked = isStageLocked(stage);
                  return (
                    <ThreadStageChip
                      key={stage.id}
                      stage={stage}
                      index={index}
                      locked={locked}
                      removeBody={t("thread.delete_stage_body", { stage: projectStageLabel(stage, t) })}
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
  compact = false,
  sidebarMode = false,
  onCreated,
  onUpdated,
  onDeleted,
  onReload,
  onError,
}: {
  project: ProjectInfo;
  stages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  compact?: boolean;
  sidebarMode?: boolean;
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
    <div className={sidebarMode || compact ? "" : "mb-3 rounded-lg border border-ink/10 bg-ink/[0.025] p-5"}>
      <div className="grid gap-3">
        <div className="flex items-center justify-between gap-3">
          <div className="text-body-sm font-semibold text-card-fg/85">{t("stage.project_stages")}</div>
          <Tooltip content={t("stage.add")} placement="top">
            <button
              type="button"
              aria-label={t("stage.add")}
              onClick={() => setShowCreate((value) => !value)}
              className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-card-fg/75 transition hover:bg-ink/5 hover:text-card-fg/90"
            >
              <Plus className="h-3.5 w-3.5" />
            </button>
          </Tooltip>
        </div>
        <StageList
          stages={stages}
          assistants={enabledAssistants}
          loading={false}
          dragGroup="project-stages"
          sidebarMode={sidebarMode}
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
  sidebarMode = false,
  compactMode = false,
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
  sidebarMode?: boolean;
  compactMode?: boolean;
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
    <>
      {loading ? (
        <div className="py-8 text-center text-body-sm text-ink/40">{t("memory_search.searching")}</div>
      ) : (
        <div className={sidebarMode || compact ? "min-w-0" : "min-w-0"}>
          <div className={sidebarMode ? "" : "rounded-lg border border-card-border/[0.12] bg-ink/[0.025] p-5"}>
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
                <Tooltip content={t("assistant.add")} placement="top">
                  <button
                    type="button"
                    aria-label={t("assistant.add")}
                    onClick={() => setShowCreate(true)}
                    className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-card-fg/75 transition hover:bg-ink/5 hover:text-card-fg/90"
                  >
                    <Plus className="h-3.5 w-3.5" />
                  </button>
                </Tooltip>
              }
            />
            <div className="grid gap-3">
              {visible.map((assistant) => (
                <AssistantCard
                  key={assistant.id}
                  assistant={assistant}
                  agents={agents}
                  sidebarMode={sidebarMode}
                  compactMode={compactMode}
                  onUpdated={onAssistantUpdated}
                  onDeleted={onAssistantDeleted}
                  onError={onError}
                />
              ))}
              {visible.length === 0 && <div className="rounded-md border border-dashed border-ink/10 py-8 text-center text-body-sm text-ink/35">{t("assistant.empty")}</div>}
            </div>
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
    </>
  );
}
