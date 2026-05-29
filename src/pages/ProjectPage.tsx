import {
  useEffect,
  useMemo,
  type RefObject,
  type ReactNode,
  useCallback,
  useRef,
  useState,
} from "react";
import { Check, Copy, Kanban, Link2, LoaderCircle, Pencil, Save, Send, Trash2, Unlink, CircleDashed, CircleDot, CircleGauge, CircleUserRound, CircleCheck, CircleSlash, type LucideIcon } from "lucide-react";
import type { Agent, KanbanItem, KanbanStatus, ProjectInfo, ProjectType, RuntimeAgentMetadata, SessionInfo } from "../api";
import { AGENT_LABEL, archiveProject, createKanbanItem, deleteKanbanItem, linkKanbanItemSession, listKanbanItems, sendAgentInput, startAgentSession, unlinkKanbanItemSession, updateKanbanItem, updateKanbanItemStatus, updateProject, updateRuntimeAgentPreferences } from "../api";
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
  const [loading, setLoading] = useState(true);
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
        </ScrollArea>
      </div>
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
