import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";
import { Search, PanelLeftClose, PanelLeftOpen, Folder, FolderOpen, Sun, Moon, Monitor, ChevronDown, RefreshCw, Settings, X, Download, Skull, ListChevronsDownUp, ListChevronsUpDown, KeyRound, CircleAlert, MailPlus, Plus, ArrowUp, Mic, GitBranch, Cpu, Hand, FileText, Image as ImageIcon, type LucideIcon } from "lucide-react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Menu } from "@tauri-apps/api/menu/menu";
import { MenuItem } from "@tauri-apps/api/menu/menuItem";
import { open } from "@tauri-apps/plugin-dialog";
import { cursorPosition, getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalPosition } from "@tauri-apps/api/dpi";
import {
  Agent,
  type AgentAttachment,
  createPendingSession,
  getIndexStatus,
  IndexPhase,
  getMemoryBackendStatus,
  MemoryBackendStatus,
  ProjectMemorySearchResult,
  RuntimeAgentMetadata,
  SessionInfo,
  rebuildSessionIndex,
  listSessions,
  removeSessionsByScope,
  removeSessionFiles,
  searchProjectMemory,
  sendAgentInput,
  startAgentSession,
  type SessionScope,
} from "./api";
import { syncTrayMenu } from "./tray";
import SessionDetail, { type ActiveMessageMeta } from "./components/SessionDetail";
import SessionMemory, { SessionMetaList } from "./components/SessionMemory";
import { AgentGlyph } from "./components/AgentIcon";
import ScrollArea from "./components/ScrollArea";
import ConfirmPopover from "./components/ConfirmPopover";
import InlineMenuSelect, { type InlineMenuSelectOption } from "./components/InlineMenuSelect";
import Tooltip from "./components/Tooltip";
import WindowControls from "./components/WindowControls";
import { ThemeMode, useTheme } from "./theme";
import { Lang, useI18n } from "./i18n";
import { useUpdateCheck, openReleasePage } from "./updater";
import {
  applyRuntimeAction,
  emptyLiveRuntimeState,
  liveSessionActivity,
  liveSessionUpdatedAt,
  normalizeAgentRuntimeEvent,
  type LiveRuntimeAction,
  type LiveRuntimeState,
} from "./runtimeChat";
import { useRuntimeAgents } from "./runtimeAgents";

type Filter =
  | SessionScope
  | { kind: "project"; key: string; label: string };

function scopeForFilter(filter: Filter): SessionScope {
  if (filter.kind === "project") return { kind: "project", key: filter.key };
  return filter;
}

export type ViewMode = "native" | "cross";
type DetailMode = "chat" | "memory";

const VIEW_MODE_STORAGE_KEY = "sessio.viewMode";

function readViewMode(): ViewMode {
  if (typeof localStorage === "undefined") return "native";
  const v = localStorage.getItem(VIEW_MODE_STORAGE_KEY);
  return v === "cross" ? "cross" : "native";
}

const SIDEBAR_SESSION_PREVIEW_LIMIT = 5;

const IS_MAC =
  typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);

function refreshMemoryBackendStatus(
  setMemoryBackendStatus: (status: MemoryBackendStatus | null) => void,
): Promise<void> {
  return getMemoryBackendStatus()
    .then(setMemoryBackendStatus)
    .catch((err) => {
      console.error("memory backend status check failed", err);
      setMemoryBackendStatus(null);
    });
}

function projectKey(s: SessionInfo): string {
  return s.projectPath ?? `__unknown__:${s.agent}`;
}

function matchesScope(scope: SessionScope, session: SessionInfo): boolean {
  if (scope.kind === "all") return true;
  if (scope.kind === "agent") return session.agent === scope.agent;
  return projectKey(session) === scope.key;
}

function sessionKey(s: SessionInfo): string {
  return `${s.agent}:${s.filePath}:${s.id}`;
}

function sessionIdentityKey(s: SessionInfo): string {
  return `${s.agent}:${s.id}`;
}

function messageCountKey(agent: Agent, filePath: string, sessionId: string): string {
  return `${agent}:${sessionId}:${filePath}`;
}

function resizeTextareaToContent(el: HTMLTextAreaElement) {
  el.style.height = "auto";
  const lineHeight = parseFloat(getComputedStyle(el).lineHeight) || 20;
  const minHeight = lineHeight * 2;
  const maxHeight = lineHeight * 6;
  const nextHeight = Math.min(Math.max(el.scrollHeight, minHeight), maxHeight);
  el.style.height = `${nextHeight}px`;
  el.style.overflowY = el.scrollHeight > maxHeight ? "auto" : "hidden";
}

type DeleteTarget =
  | { kind: "session"; session: SessionInfo; pos: { x: number; y: number } }
  | { kind: "scope"; scope: SessionScope; pos: { x: number; y: number } };

type PendingNewChatSession = {
  sessioRuntimeSessionId: string;
  agent: Agent;
  projectPath: string;
  projectName: string;
  prompt: string;
  timestamp: number;
};

type ComposerAttachment = AgentAttachment & {
  name: string;
};

const TEXT_ATTACHMENT_EXTENSIONS = [
  "txt",
  "md",
  "markdown",
  "rst",
  "json",
  "jsonl",
  "yaml",
  "yml",
  "toml",
  "xml",
  "csv",
  "ts",
  "tsx",
  "js",
  "jsx",
  "mjs",
  "cjs",
  "py",
  "rs",
  "go",
  "java",
  "kt",
  "swift",
  "rb",
  "php",
  "css",
  "scss",
  "sass",
  "less",
  "html",
  "htm",
  "sh",
  "zsh",
  "bash",
  "sql",
  "c",
  "h",
  "cpp",
  "hpp",
  "cs",
  "lua",
  "pl",
  "r",
  "ex",
  "exs",
  "erl",
  "clj",
  "scala",
  "dart",
  "vue",
  "svelte",
  "dockerfile",
  "gitignore",
  "env",
];

function basename(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

function dedupeComposerAttachments(
  attachments: ComposerAttachment[],
): ComposerAttachment[] {
  const seen = new Set<string>();
  const deduped: ComposerAttachment[] = [];
  for (const attachment of attachments) {
    if (seen.has(attachment.path)) continue;
    seen.add(attachment.path);
    deduped.push(attachment);
  }
  return deduped;
}

// Orphan main session that only exists to carry subagents (Claude cleaned
// the main jsonl, no index entry either). Don't count it as a "real" session
// but still show it in the list so subagents stay reachable.
function isSubagentOnly(s: SessionInfo): boolean {
  return s.archived && s.messageCount === 0 && s.subagents.length > 0;
}

export default function App() {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [indexPhase, setIndexPhase] = useState<IndexPhase>("indexing");
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>({ kind: "all" });
  const [selected, setSelected] = useState<SessionInfo | null>(null);
  const [expandProject, setExpandProject] = useState(true);
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(
    () => new Set(),
  );
  const [expandedProjectSessions, setExpandedProjectSessions] = useState<Set<string>>(
    () => new Set(),
  );
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [memoryBackendStatus, setMemoryBackendStatus] =
    useState<MemoryBackendStatus | null>(null);
  const [memorySearchOpen, setMemorySearchOpen] = useState(false);
  const [memorySearchMounted, setMemorySearchMounted] = useState(false);
  const [viewMode] = useState<ViewMode>(() => readViewMode());
  const [detailMode, setDetailMode] = useState<DetailMode>("chat");
  const [metaPopoverOpen, setMetaPopoverOpen] = useState(false);
  const [metaPopoverMounted, setMetaPopoverMounted] = useState(false);
  const [activeMessageMeta, setActiveMessageMeta] =
    useState<ActiveMessageMeta | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<DeleteTarget | null>(null);
  const [unreadSessionIds, setUnreadSessionIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [liveRuntimeState, dispatchLiveRuntimeEvent] = useReducer(
    applyRuntimeAction,
    emptyLiveRuntimeState,
  );
  const [pendingSelectSession, setPendingSelectSession] = useState<{
    agent: Agent;
    sessionId: string;
  } | null>(null);
  const [pendingNewChats, setPendingNewChats] = useState<Record<string, PendingNewChatSession>>({});
  const [runtimeSessionAliases, setRuntimeSessionAliases] = useState<Record<string, string>>({});
  const { mode, setMode } = useTheme();
  const [systemAppearance, setSystemAppearance] = useState<"light" | "dark">("dark");
  const { lang, setLang, t } = useI18n();
  const { agents: runtimeAgents } = useRuntimeAgents();
  const update = useUpdateCheck(__APP_VERSION__);
  const messageCountBySourceRef = useRef<Map<string, number>>(new Map());
  const selectedSessionIdRef = useRef<string | null>(null);
  const sessionsLoadedRef = useRef(false);
  const pendingNewChatWritesRef = useRef<Set<string>>(new Set());
  const indexing = indexPhase !== "idle";
  const rebuilding = indexPhase === "rebuilding";

  const availableSessions = useMemo(
    () => sessions.filter((s) => s.available),
    [sessions]
  );

  useEffect(() => {
    const previous = messageCountBySourceRef.current;
    const next = new Map<string, number>();
    const changedSessionIds = new Set<string>();
    for (const session of sessions) {
      const mainKey = messageCountKey(session.agent, session.filePath, session.id);
      next.set(mainKey, session.messageCount);
      const previousMainCount = previous.get(mainKey);
      if (
        sessionsLoadedRef.current &&
        previousMainCount !== undefined &&
        session.messageCount > previousMainCount
      ) {
        changedSessionIds.add(session.id);
      }
      for (const subagent of session.subagents) {
        const subKey = messageCountKey(session.agent, subagent.filePath, session.id);
        next.set(subKey, subagent.messageCount);
        const previousSubCount = previous.get(subKey);
        if (
          sessionsLoadedRef.current &&
          previousSubCount !== undefined &&
          subagent.messageCount > previousSubCount
        ) {
          changedSessionIds.add(session.id);
        }
      }
    }
    messageCountBySourceRef.current = next;
    sessionsLoadedRef.current = true;
    if (changedSessionIds.size > 0) {
      setUnreadSessionIds((prev) => {
        let changed = false;
        const unread = new Set(prev);
        for (const id of changedSessionIds) {
          if (id === selectedSessionIdRef.current) continue;
          if (!unread.has(id)) {
            unread.add(id);
            changed = true;
          }
        }
        return changed ? unread : prev;
      });
    }
  }, [sessions]);

  useEffect(() => {
    selectedSessionIdRef.current = selected?.id ?? null;
    if (!selected) return;
    setUnreadSessionIds((prev) => {
      if (!prev.has(selected.id)) return prev;
      const next = new Set(prev);
      next.delete(selected.id);
      return next;
    });
  }, [selected?.id]);

  // Don't let webview restore focus on the last interactive control when
  // the window is shown again — leaves a stale focus ring (and tooltip).
  useEffect(() => {
    const drop = () => {
      const el = document.activeElement as HTMLElement | null;
      if (!el || el === document.body) return;
      const tag = el.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable) return;
      el.blur();
    };
    window.addEventListener("blur", drop);
    return () => window.removeEventListener("blur", drop);
  }, []);

  useEffect(() => {
    localStorage.setItem(VIEW_MODE_STORAGE_KEY, viewMode);
  }, [viewMode]);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    listSessions()
      .then((rows) => {
        if (cancelled) return;
        setSessions(rows.filter((s) => s.available));
      })
      .catch((err) => {
        if (cancelled) return;
        setError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    getIndexStatus()
      .then((status) => {
        setIndexPhase(status.phase);
        if (status.lastError) setError(status.lastError);
      })
      .catch(() => {});
    refreshMemoryBackendStatus(setMemoryBackendStatus);

    const unlisten = listen("sessions_index_updated", () => {
      setIndexPhase("idle");
      listSessions()
        .then((rows) => setSessions(rows.filter((s) => s.available)))
        .catch(() => {});
      refreshMemoryBackendStatus(setMemoryBackendStatus);
    });
    const statusUnlisten = listen("sessions_index_status", (event) => {
      const payload = event.payload as {
        phase?: IndexPhase;
        lastError?: string | null;
      };
      if (payload.phase) setIndexPhase(payload.phase);
      if (payload.lastError !== undefined) setError(payload.lastError);
    });
    return () => {
      unlisten.then((f) => f()).catch(() => {});
      statusUnlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<unknown>("agent-runtime-event", (event) => {
      if (cancelled) return;
      const payload = normalizeAgentRuntimeEvent(event.payload);
      console.info("[sessio-runtime:frontend:event]", payload);
      if (
        payload.sessioRuntimeSessionId !== selectedSessionIdRef.current &&
        payload.kind !== "sessionEnded"
      ) {
        setUnreadSessionIds((prev) => {
          if (prev.has(payload.sessioRuntimeSessionId)) return prev;
          const next = new Set(prev);
          next.add(payload.sessioRuntimeSessionId);
          return next;
        });
      }
      dispatchLiveRuntimeEvent({ type: "runtime-event", event: payload });
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((err) => setError(String(err)));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!selected) return;
    const next =
      availableSessions.find((s) => sessionKey(s) === sessionKey(selected)) ??
      availableSessions.find((s) => sessionIdentityKey(s) === sessionIdentityKey(selected));
    if (!next) {
      setSelected(null);
      return;
    }
    if (next !== selected) {
      setSelected(next);
    }
  }, [availableSessions, selected]);

  useEffect(() => {
    if (!pendingSelectSession) return;
    const next = availableSessions.find(
      (session) =>
        session.agent === pendingSelectSession.agent &&
        session.id === pendingSelectSession.sessionId,
    );
    if (!next) return;
    setSelected(next);
    setDetailMode("chat");
    setFilter({ kind: "project", key: projectKey(next), label: next.projectName ?? next.projectPath ?? t("list.unknown_project") });
    setExpandedProjects((prev) => {
      const expanded = new Set(prev);
      expanded.add(projectKey(next));
      return expanded;
    });
    setPendingSelectSession(null);
  }, [availableSessions, pendingSelectSession, t]);

  useEffect(() => {
    for (const pending of Object.values(pendingNewChats)) {
      const liveSession = liveRuntimeState.sessions[pending.sessioRuntimeSessionId];
      if (!liveSession) continue;
      const agentSessionId = liveSession.agentRuntimeSessionId;
      if (!agentSessionId || agentSessionId.startsWith("fake-agent-session")) continue;
      if (pendingNewChatWritesRef.current.has(pending.sessioRuntimeSessionId)) continue;

      pendingNewChatWritesRef.current.add(pending.sessioRuntimeSessionId);
      createPendingSession({
        id: agentSessionId,
        agent: pending.agent,
        projectPath: pending.projectPath,
        projectName: pending.projectName,
        startedAt: pending.timestamp,
        updatedAt: pending.timestamp,
        messageCount: 0,
        title: pending.prompt,
        firstUserMessage: pending.prompt,
        filePath: "",
        fileSize: 0,
        partial: true,
        available: true,
        archived: false,
        subagents: [],
      })
        .then(() => {
          setRuntimeSessionAliases((prev) => ({
            ...prev,
            [`${pending.agent}:${agentSessionId}`]: pending.sessioRuntimeSessionId,
          }));
          setPendingSelectSession({ agent: pending.agent, sessionId: agentSessionId });
          setPendingNewChats((prev) => {
            const next = { ...prev };
            delete next[pending.sessioRuntimeSessionId];
            return next;
          });
        })
        .catch((err) => {
          pendingNewChatWritesRef.current.delete(pending.sessioRuntimeSessionId);
          setError(String(err));
        });
    }
  }, [liveRuntimeState.sessions, pendingNewChats]);

  const handleMessageCount = useCallback((
    agent: Agent,
    filePath: string,
    sessionId: string,
    count: number,
  ) => {
    const countKey = messageCountKey(agent, filePath, sessionId);
    if (messageCountBySourceRef.current.get(countKey) === count) return false;
    messageCountBySourceRef.current.set(countKey, count);

    const patchSession = (session: SessionInfo): SessionInfo => {
      if (
        session.agent === agent &&
        session.id === sessionId &&
        session.filePath === filePath
      ) {
        return { ...session, messageCount: count };
      }
      let changed = false;
      const subagents = session.subagents.map((sub) => {
        if (
          session.agent !== agent ||
          session.id !== sessionId ||
          sub.filePath !== filePath
        ) {
          return sub;
        }
        changed = true;
        return { ...sub, messageCount: count };
      });
      return changed ? { ...session, subagents } : session;
    };

    setSessions((prev) => prev.map(patchSession));
    setSelected((prev) => (prev ? patchSession(prev) : prev));
    setActiveMessageMeta((prev) =>
      prev && prev.filePath === filePath ? { ...prev, count } : prev,
    );
    return true;
  }, []);

  const handleActiveMessageMeta = useCallback((meta: ActiveMessageMeta) => {
    setActiveMessageMeta(meta);
  }, []);

  useEffect(() => {
    let cancelled = false;
    invoke<string>("get_system_appearance")
      .then((value) => {
        if (!cancelled) {
          setSystemAppearance(value === "dark" ? "dark" : "light");
        }
      })
      .catch(() => {});
    const unlisten = listen<string>("system_appearance_changed", (event) => {
      setSystemAppearance(event.payload === "dark" ? "dark" : "light");
    });
    return () => {
      cancelled = true;
      unlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    if (selected) return;
    setActiveMessageMeta(null);
  }, [selected]);

  useEffect(() => {
    setMetaPopoverOpen(false);
  }, [selected?.id]);

  useEffect(() => {
    if (metaPopoverOpen && selected) {
      setMetaPopoverMounted(true);
    }
    if (!selected) {
      setMetaPopoverMounted(false);
    }
  }, [metaPopoverOpen, selected]);

  const projectGroups = useMemo(() => {
    const m = new Map<
      string,
      {
        label: string;
        count: number;
        path: string | null;
        latest: number;
        sessions: SessionInfo[];
      }
    >();
    const unknown = t("list.unknown_project");
    for (const s of availableSessions) {
      if (isSubagentOnly(s)) continue;
      const key = projectKey(s);
      const ts = s.updatedAt ?? s.startedAt ?? 0;
      const e = m.get(key);
      if (e) {
        e.count += 1;
        if (ts > e.latest) e.latest = ts;
        e.sessions.push(s);
      } else {
        m.set(key, {
          label: s.projectName ?? s.projectPath ?? unknown,
          count: 1,
          path: s.projectPath,
          latest: ts,
          sessions: [s],
        });
      }
    }
    return [...m.entries()]
      .map(([key, v]) => ({
        key,
        ...v,
        sessions: v.sessions.sort((a, b) => {
          const aLive = liveSessionUpdatedAt(liveRuntimeState.sessions[a.id]) ?? 0;
          const bLive = liveSessionUpdatedAt(liveRuntimeState.sessions[b.id]) ?? 0;
          return (
            Math.max(b.updatedAt ?? b.startedAt ?? 0, bLive) -
            Math.max(a.updatedAt ?? a.startedAt ?? 0, aLive)
          );
        }),
      }))
      .sort((a, b) => b.latest - a.latest || a.label.localeCompare(b.label));
  }, [availableSessions, liveRuntimeState.sessions, t]);

  useEffect(() => {
    setExpandedProjects((prev) => {
      const keys = new Set(projectGroups.map((p) => p.key));
      let changed = false;
      const next = new Set<string>();
      for (const key of prev) {
        if (keys.has(key)) next.add(key);
        else changed = true;
      }
      if (next.size === 0 && projectGroups[0]) {
        next.add(projectGroups[0].key);
        changed = true;
      }
      if (
        selected &&
        keys.has(projectKey(selected)) &&
        !next.has(projectKey(selected))
      ) {
        next.add(projectKey(selected));
        changed = true;
      }
      return changed ? next : prev;
    });
  }, [projectGroups, selected]);

  useEffect(() => {
    setExpandedProjectSessions((prev) => {
      const keys = new Set(projectGroups.map((p) => p.key));
      let changed = false;
      const next = new Set<string>();
      for (const key of prev) {
        if (keys.has(key)) next.add(key);
        else changed = true;
      }
      return changed ? next : prev;
    });
  }, [projectGroups]);

  const recentForMenu = useMemo(
    () => availableSessions.filter((s) => !isSubagentOnly(s)).slice(0, 5),
    [availableSessions]
  );

  const selectedKey = selected ? sessionKey(selected) : null;
  const selectedIdentityKey = selected ? sessionIdentityKey(selected) : null;

  useEffect(() => {
    syncTrayMenu(recentForMenu, {
      show: t("menubar.show"),
      quit: t("menubar.quit"),
      noSessions: t("menubar.no_sessions"),
      noMessage: t("list.no_user_message"),
      resumeCommand: t("menubar.resume_command"),
      crossCommand: t("menubar.cross_command"),
      crossPromptPlaceholder: t("list.cross_prompt_placeholder"),
    }, systemAppearance);
  }, [recentForMenu, t, systemAppearance]);

  const removeSessionsInScope = async (scope: SessionScope) => {
    const targets = availableSessions.filter(
      (s) => !isSubagentOnly(s) && matchesScope(scope, s),
    );
    if (targets.length === 0) return;
    await removeSessionsByScope(scope);
    if (selected && targets.some((s) => sessionKey(s) === sessionKey(selected))) {
      setSelected(null);
    }
  };

  const getMenuClickPos = async (): Promise<{ x: number; y: number }> => {
    const window = getCurrentWindow();
    const [cursor, inner, scale] = await Promise.all([
      cursorPosition(),
      window.innerPosition(),
      window.scaleFactor(),
    ]);
    return {
      x: (cursor.x - inner.x) / scale,
      y: (cursor.y - inner.y) / scale,
    };
  };

  const confirmDelete = async () => {
    const target = deleteTarget;
    if (!target) return;
    try {
      if (target.kind === "session") {
        await removeSessionFiles(target.session);
        if (selected && sessionKey(selected) === sessionKey(target.session)) {
          setSelected(null);
        }
      } else {
        await removeSessionsInScope(target.scope);
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setDeleteTarget(null);
    }
  };

  const openDeleteMenu = async (
    pos: { x: number; y: number },
    onDelete: (pos: { x: number; y: number }) => void,
  ) => {
    try {
      const removeItem = await MenuItem.new({
        id: "delete",
        text: t("sidebar.remove"),
        action: async () => onDelete(await getMenuClickPos()),
      });
      const menu = await Menu.new({
        items: [removeItem],
      });
      await menu.popup(
        new LogicalPosition(pos.x + 1, pos.y),
        getCurrentWindow(),
      );
    } catch (err) {
      setError(String(err));
    }
  };

  const openScopeMenu = async (scope: SessionScope, pos: { x: number; y: number }) => {
    await openDeleteMenu(pos, (clickPos) => setDeleteTarget({ kind: "scope", scope, pos: clickPos }));
  };

  const openSessionMenu = async (session: SessionInfo, pos: { x: number; y: number }) => {
    await openDeleteMenu(pos, (clickPos) => setDeleteTarget({ kind: "session", session, pos: clickPos }));
  };

  const detailTitle =
    selected?.title ??
    selected?.firstUserMessage ??
    t("list.no_user_message");

  const memoryBackendMissing =
    memoryBackendStatus !== null && memoryBackendStatus.available === false;
  const projectSearchInitialKey = filter.kind === "project" ? filter.key : projectGroups[0]?.key;

  useEffect(() => {
    if (memorySearchOpen && projectSearchInitialKey) {
      setMemorySearchMounted(true);
    }
    if (!projectSearchInitialKey) {
      setMemorySearchMounted(false);
    }
  }, [memorySearchOpen, projectSearchInitialKey]);

  return (
    <div className="flex h-screen text-body">
      <aside
        className={
          "shrink-0 border-r border-ink/5 bg-surface-sidebar flex flex-col overflow-hidden transition-[width] duration-600 ease-in-out " +
          (sidebarOpen ? "w-64" : "w-0")
        }
      >
        <div
          data-tauri-drag-region
          className="relative h-12 shrink-0 w-64"
        >
          {!IS_MAC && (
            <span className="absolute left-3 top-1/2 -translate-y-1/2 text-title font-semibold text-ink/85 pointer-events-none select-none">
              Sessio
            </span>
          )}
          <Tooltip content={t("sidebar.close")} placement="bottom">
            <button
              type="button"
              aria-label={t("sidebar.close")}
              data-tauri-drag-region="false"
              onClick={() => setSidebarOpen(false)}
              className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-ink/55 hover:text-ink transition rounded-md"
            >
              <PanelLeftClose className="w-4 h-4" />
            </button>
          </Tooltip>
        </div>

        <nav className="flex-1 min-h-0 w-64 p-2 pb-0 flex flex-col gap-0.5">
          <button
            type="button"
            onClick={() => {
              setSelected(null);
              setDetailMode("chat");
            }}
            className={
              "mb-2 flex h-8 w-full items-center gap-2 rounded-md px-2.5 text-left text-body-sm font-medium transition " +
              (!selected
                ? "bg-ink/10 text-ink"
                : "text-ink/72 hover:bg-ink/5 hover:text-ink")
            }
          >
            <Plus className="h-4 w-4 shrink-0" />
            <span className="truncate">{t("sidebar.new_chat")}</span>
          </button>
          <div className="shrink-0 flex flex-col gap-0.5">
            <SectionHeader
              label={t("sidebar.by_project")}
              collapsed={!expandProject}
              onToggle={() => setExpandProject((v) => !v)}
            />
          </div>

          {expandProject && (
            <ScrollArea
              className="flex-1 min-h-0 -mr-2"
              viewportClassName="pr-3 flex flex-col gap-1"
            >
              {projectGroups.map((p) => {
                const expanded = expandedProjects.has(p.key);
                return (
                  <ProjectSidebarGroup
                    key={p.key}
                    project={p}
                    expanded={expanded}
                    sessionsExpanded={expandedProjectSessions.has(p.key)}
                    selectedKey={selectedKey}
                    selectedIdentityKey={selectedIdentityKey}
                    liveState={liveRuntimeState}
                    runtimeSessionAliases={runtimeSessionAliases}
                    unreadSessionIds={unreadSessionIds}
                    onSelectProject={() => {
                      setFilter({ kind: "project", key: p.key, label: p.label });
                      setExpandedProjects((prev) => {
                        const next = new Set(prev);
                        if (next.has(p.key)) next.delete(p.key);
                        else next.add(p.key);
                        return next;
                      });
                    }}
                    onSelectSession={(session) => {
                      setFilter({ kind: "project", key: p.key, label: p.label });
                      setSelected(session);
                    }}
                    onToggleSessionLimit={() => {
                      setExpandedProjectSessions((prev) => {
                        const next = new Set(prev);
                        if (next.has(p.key)) next.delete(p.key);
                        else next.add(p.key);
                        return next;
                      });
                    }}
                    onProjectContextMenu={(e) => {
                      e.preventDefault();
                      void openScopeMenu(
                        scopeForFilter({ kind: "project", key: p.key, label: p.label }),
                        { x: e.clientX, y: e.clientY },
                      );
                    }}
                    onSessionContextMenu={(session, pos) =>
                      void openSessionMenu(session, pos)
                    }
                  />
                );
              })}
            </ScrollArea>
          )}
        </nav>

        <SidebarFooter
          sidebarOpen={sidebarOpen}
          lang={lang}
          onLangChange={setLang}
          themeMode={mode}
          onThemeModeChange={setMode}
          update={update}
          rebuilding={rebuilding}
          indexing={indexing}
          onError={setError}
          onRebuildFinished={() => refreshMemoryBackendStatus(setMemoryBackendStatus)}
        />
      </aside>

      <main className="relative flex-1 flex flex-col min-w-0">
        <div
          data-tauri-drag-region
          className={
            "relative h-12 shrink-0 grid grid-cols-3 items-center px-5 bg-surface border-b border-ink/10 select-none " +
            (IS_MAC ? "" : "pr-[138px]")
          }
        >
          <Tooltip content={t("sidebar.open")} placement="bottom">
            <button
              type="button"
              aria-label={t("sidebar.open")}
              data-tauri-drag-region="false"
              onClick={() => setSidebarOpen(true)}
              className={
                "absolute top-1/2 -translate-y-1/2 p-1 text-ink/55 hover:text-ink rounded-md transition-opacity duration-300 " +
                (IS_MAC ? "left-24 " : "left-2 ") +
                (sidebarOpen ? "opacity-0 pointer-events-none" : "opacity-100")
              }
            >
              <PanelLeftOpen className="w-4 h-4" />
            </button>
          </Tooltip>
          <div
            data-tauri-drag-region
            className={
              "flex items-center gap-2 min-w-0 " +
              (sidebarOpen ? "" : IS_MAC ? "pl-[112px] " : "pl-9 ")
            }
          >
            {selected && activeMessageMeta && sidebarOpen && (
              <HeaderMessageMetaButton
                label={`${activeMessageMeta.partial ? "~" : ""}${t("header.messages_count", { count: activeMessageMeta.count })}`}
                open={metaPopoverOpen}
                onToggle={() => setMetaPopoverOpen((open) => !open)}
              />
            )}
            {selected && !sidebarOpen && (
              <>
                <span
                  data-tauri-drag-region
                  className="flex h-5 w-5 shrink-0"
                >
                  <AgentGlyph
                    agent={selected.agent}
                    className="h-5 w-5 pointer-events-none"
                  />
                </span>
                <div
                  data-tauri-drag-region
                  className="min-w-0 max-w-[min(42vw,520px)]"
                >
                  <div
                    data-tauri-drag-region
                    className="truncate text-body font-medium leading-tight text-ink/85"
                  >
                    {detailTitle}
                  </div>
                  {activeMessageMeta && (
                    <HeaderMessageMetaButton
                      label={`${activeMessageMeta.partial ? "~" : ""}${t("header.messages_count", { count: activeMessageMeta.count })}`}
                      open={metaPopoverOpen}
                      onToggle={() => setMetaPopoverOpen((open) => !open)}
                      compact
                    />
                  )}
                </div>
              </>
            )}
          </div>
          <div data-tauri-drag-region="false" className="justify-self-center">
            <HeaderModeTabs mode={detailMode} onChange={setDetailMode} />
          </div>
          <div className="justify-self-end" data-tauri-drag-region="false">
            {memoryBackendMissing ? (
              <MemoryBackendMissingButton
                status={memoryBackendStatus}
                placement="bottom"
                onRefresh={() => refreshMemoryBackendStatus(setMemoryBackendStatus)}
              />
            ) : (
              <Tooltip content={t("header.search")} placement="bottom">
                <button
                  type="button"
                  aria-label={t("header.search")}
                  onClick={() => setMemorySearchOpen(true)}
                  disabled={projectGroups.length === 0}
                  className="p-1 text-ink/55 hover:text-ink disabled:opacity-35 disabled:hover:text-ink/55 transition rounded-md"
                >
                  <Search className="w-4 h-4" />
                </button>
              </Tooltip>
            )}
          </div>
          <div className="absolute top-0 right-0 z-20">
            <WindowControls />
          </div>
        </div>

        {selected && metaPopoverMounted && (
          <>
            <button
              type="button"
              data-tauri-drag-region="false"
              aria-label="Close metadata"
              className={
                "absolute inset-x-0 top-12 bottom-0 z-30 bg-bg/35 backdrop-blur-sm transition-opacity duration-150 " +
                (metaPopoverOpen ? "opacity-100" : "opacity-0")
              }
              onClick={() => setMetaPopoverOpen(false)}
              onTransitionEnd={(e) => {
                if (!metaPopoverOpen && e.currentTarget === e.target) {
                  setMetaPopoverMounted(false);
                }
              }}
            />
            <div
              data-tauri-drag-region="false"
              className={
                "absolute left-1/2 top-12 z-40 w-[520px] max-w-[calc(100vw-80px)] -translate-x-1/2 transition-[opacity,transform] duration-150 ease-out " +
                (metaPopoverOpen ? "translate-y-0 opacity-100" : "-translate-y-3 opacity-0")
              }
            >
              <SessionMetaList session={selected} />
            </div>
          </>
        )}

        {error ? (
          <div className="m-5 p-3 rounded bg-status-error/10 text-status-error text-body-sm">
            {error}
          </div>
        ) : selected ? (
          <div className="relative flex-1 min-h-0">
            <div
              className={
                "absolute inset-0 " +
                (detailMode === "chat" ? "visible" : "invisible pointer-events-none")
              }
              aria-hidden={detailMode !== "chat"}
            >
              <SessionDetail
                session={selected}
                viewMode={viewMode}
                liveState={liveRuntimeState}
                runtimeSessionAliases={runtimeSessionAliases}
                dispatchLiveEvent={dispatchLiveRuntimeEvent}
                onMessageCount={handleMessageCount}
                onActiveMessageMeta={handleActiveMessageMeta}
              />
            </div>
            <div
              className={
                "absolute inset-0 " +
                (detailMode === "memory" ? "visible" : "invisible pointer-events-none")
              }
              aria-hidden={detailMode !== "memory"}
            >
              <SessionMemory session={selected} />
            </div>
          </div>
        ) : (
          <NewChatView
            projects={projectGroups}
            runtimeAgents={runtimeAgents}
            liveState={liveRuntimeState}
            dispatchLiveEvent={dispatchLiveRuntimeEvent}
            onError={setError}
            onPendingSession={(pending) => {
              setPendingNewChats((prev) => ({
                ...prev,
                [pending.sessioRuntimeSessionId]: pending,
              }));
            }}
          />
        )}

        {memorySearchMounted && projectSearchInitialKey && (
          <ProjectMemorySearchDialog
            open={memorySearchOpen}
            initialProjectKey={projectSearchInitialKey}
            projects={projectGroups}
            activeProjectKey={filter.kind === "project" ? filter.key : null}
            onClose={() => setMemorySearchOpen(false)}
            onExited={() => setMemorySearchMounted(false)}
          />
        )}

        {deleteTarget && (
          <ConfirmPopover
            title={t("delete.title")}
            body={
              deleteTarget.kind === "session"
                ? t("delete.session_body")
                : t("delete.scope_body")
            }
            pos={deleteTarget.pos}
            onCancel={() => setDeleteTarget(null)}
            onConfirm={() => {
              void confirmDelete();
            }}
          />
        )}
      </main>
    </div>
  );
}

function IndexStatusDot({ indexing }: { indexing: boolean }) {
  const { t } = useI18n();
  // Decouple the ripple lifecycle from `indexing` so a quick true→false flip
  // still plays at least MIN_ITERATIONS full ring iterations; while indexing
  // stays true the animation loops infinitely via CSS.
  const MIN_ITERATIONS = 2;
  const [animating, setAnimating] = useState(indexing);
  const indexingRef = useRef(indexing);
  const iterRef = useRef(0);
  useEffect(() => {
    indexingRef.current = indexing;
    if (indexing) {
      iterRef.current = 0;
      setAnimating(true);
    }
  }, [indexing]);

  const tip = (
    <div className="flex flex-col gap-2 py-0.5">
      <div className="flex items-center gap-2.5">
        <StatusDot />
        <span>{t("sidebar.status_idle")}</span>
      </div>
      <div className="flex items-center gap-2.5">
        <StatusDot ripple />
        <span>{t("sidebar.status_indexing")}</span>
      </div>
    </div>
  );
  return (
    <Tooltip content={tip} placement="top">
      <span
        aria-label={
          animating ? t("sidebar.status_indexing") : t("sidebar.status_idle")
        }
        className="inline-flex items-center justify-center p-1.5 -m-1.5"
      >
        <StatusDot
          ripple={animating}
          onIterationEnd={() => {
            iterRef.current += 1;
            if (!indexingRef.current && iterRef.current >= MIN_ITERATIONS) {
              setAnimating(false);
            }
          }}
        />
      </span>
    </Tooltip>
  );
}

function StatusDot({
  ripple,
  onIterationEnd,
}: {
  ripple?: boolean;
  onIterationEnd?: () => void;
}) {
  return (
    <span className="relative inline-block w-1.5 h-1.5 shrink-0">
      {ripple && (
        <span
          onAnimationIteration={onIterationEnd}
          className="absolute inset-0 rounded-full animate-ping"
          style={{ background: "rgb(var(--color-emerald))" }}
        />
      )}
      <span
        className="absolute inset-0 rounded-full"
        style={{ background: "rgb(var(--color-emerald))" }}
      />
    </span>
  );
}

function SidebarFooter({
  sidebarOpen,
  lang,
  onLangChange,
  themeMode,
  onThemeModeChange,
  update,
  rebuilding,
  indexing,
  onError,
  onRebuildFinished,
}: {
  sidebarOpen: boolean;
  lang: Lang;
  onLangChange: (lang: Lang) => void;
  themeMode: ThemeMode;
  onThemeModeChange: (mode: ThemeMode) => void;
  update: ReturnType<typeof useUpdateCheck>;
  rebuilding: boolean;
  indexing: boolean;
  onError: (error: string | null) => void;
  onRebuildFinished: () => Promise<void> | void;
}) {
  const { t } = useI18n();
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    if (!sidebarOpen) setSettingsOpen(false);
  }, [sidebarOpen]);

  return (
    <div className="relative w-64 border-t border-ink/10">
      <div
        className={
          "absolute left-0 bottom-full w-64 origin-bottom transition-[opacity,transform] duration-200 ease-out " +
          (settingsOpen
            ? "translate-y-0 scale-y-100 opacity-100 pointer-events-auto"
            : "translate-y-2 scale-y-95 opacity-0 pointer-events-none")
        }
      >
        <div className="border-y border-ink/10 bg-surface">
          <div className="px-3 pt-3 pb-2 flex items-center justify-between gap-3">
            <span className="text-caption uppercase tracking-[0.12em] text-ink/40">
              {t("sidebar.settings")}
              <Tooltip content={t("sidebar.check_update")} placement="top">
                <button
                  type="button"
                  aria-label={t("sidebar.check_update")}
                  onClick={() => update.check()}
                  disabled={update.checking}
                  className={
                    "ml-2 normal-case tracking-normal transition " +
                    (update.checking
                      ? "text-ink/50 animate-pulse"
                      : "text-ink/30 hover:text-ink/70 cursor-pointer")
                  }
                >
                  v{__APP_VERSION__}
                </button>
              </Tooltip>
            </span>
            <button
              type="button"
              aria-label={t("sidebar.close_settings")}
              onClick={() => setSettingsOpen(false)}
              className="p-1 -m-1 text-ink/40 hover:text-ink transition rounded-md"
            >
              <X className="w-3.5 h-3.5" />
            </button>
          </div>
          <div className="mx-3 border-t border-ink/10" />
          <div className="px-3 pt-3 pb-3 flex flex-col gap-3">
            <div className="flex items-center justify-between gap-3">
              <span className="text-body-sm text-ink/55">{t("sidebar.language")}</span>
              <LanguageSwitcher lang={lang} onChange={onLangChange} />
            </div>
            <div className="flex items-center justify-between gap-3">
              <span className="text-body-sm text-ink/55">{t("sidebar.theme")}</span>
              <ThemeSwitcher mode={themeMode} onChange={onThemeModeChange} />
            </div>
            <div className="flex items-center justify-between gap-3 text-body-sm text-ink/55">
              <span>{t("sidebar.rebuild_index")}</span>
              <button
                type="button"
                aria-label={t("sidebar.rebuild_index")}
                onClick={() => {
                  rebuildSessionIndex()
                    .catch((err) => {
                      onError(String(err));
                    })
                    .finally(() => {
                      void onRebuildFinished();
                    });
                }}
                className="p-1 -m-1 text-ink/55 transition hover:text-ink rounded-md"
              >
                <RefreshCw className={"w-4 h-4 shrink-0 " + (rebuilding ? "animate-spin" : "")} />
              </button>
            </div>
          </div>
        </div>
      </div>

      <div className="px-3 py-2 flex items-center justify-between gap-2">
        <div className="shrink-0 flex items-center gap-1">
          <Tooltip content={t("sidebar.settings")} placement="top">
            <button
              type="button"
              aria-label={t("sidebar.settings")}
              onClick={() => setSettingsOpen((v) => !v)}
              className="p-1 text-ink/55 hover:text-ink transition rounded-md"
            >
              <Settings
                className={
                  "w-4 h-4 transition-transform duration-200 " +
                  (settingsOpen ? "rotate-90" : "")
                }
              />
            </button>
          </Tooltip>
          {update.hasUpdate && update.latestVersion && (
            <Tooltip
              content={t("sidebar.update_available", {
                version: update.latestVersion,
              })}
              placement="top"
            >
              <button
                type="button"
                aria-label={t("sidebar.update_available", {
                  version: update.latestVersion,
                })}
                onClick={() => {
                  openReleasePage(update.releaseUrl).catch((err) => {
                    onError(String(err));
                  });
                }}
                className="relative p-1 text-ink/55 hover:text-ink transition rounded-md"
              >
                <Download className="w-4 h-4" />
                <span className="absolute top-0.5 right-0.5 w-1.5 h-1.5 rounded-full bg-accent-purple" />
              </button>
            </Tooltip>
          )}
        </div>
        <IndexStatusDot indexing={indexing} />
      </div>
    </div>
  );
}

function ProjectMemorySearchDialog({
  open,
  initialProjectKey,
  projects,
  activeProjectKey,
  onClose,
  onExited,
}: {
  open: boolean;
  initialProjectKey: string;
  projects: Array<{ key: string; label: string }>;
  activeProjectKey: string | null;
  onClose: () => void;
  onExited: () => void;
}) {
  const { t } = useI18n();
  const [selectedProjectKey, setSelectedProjectKey] = useState(initialProjectKey);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [results, setResults] = useState<ProjectMemorySearchResult[]>([]);
  const [searched, setSearched] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const lockedProject = activeProjectKey !== null;
  const selectedProject =
    projects.find((project) => project.key === selectedProjectKey) ?? projects[0];

  useEffect(() => {
    if (!open) return;
    const id = requestAnimationFrame(() => inputRef.current?.focus());
    return () => cancelAnimationFrame(id);
  }, [open]);

  useEffect(() => {
    const nextProjectKey =
      activeProjectKey ??
      (projects.some((project) => project.key === selectedProjectKey)
        ? selectedProjectKey
        : projects[0]?.key);
    if (nextProjectKey && nextProjectKey !== selectedProjectKey) {
      setSelectedProjectKey(nextProjectKey);
      setResults([]);
      setError(null);
      setLoading(false);
      setSearched(false);
    }
  }, [activeProjectKey, projects, selectedProjectKey]);

  const runSearch = () => {
    const text = query.trim();
    setError(null);
    if (!text) {
      setResults([]);
      setLoading(false);
      setSearched(false);
      return;
    }
    setLoading(true);
    setSearched(true);
    searchProjectMemory(selectedProjectKey, text)
      .then((rows) => {
        setResults(rows);
      })
      .catch((err) => {
        setResults([]);
        setError(String(err));
      })
      .finally(() => setLoading(false));
  };

  const selectProject = (key: string) => {
    setSelectedProjectKey(key);
    setResults([]);
    setError(null);
    setLoading(false);
    setSearched(false);
  };

  const clearOrClose = () => {
    if (query.trim()) {
      setQuery("");
      setResults([]);
      setError(null);
      setLoading(false);
      setSearched(false);
      return;
    }
    onClose();
  };

  return (
    <div
      className={
        "project-memory-search-dialog absolute inset-x-0 top-12 bottom-0 z-30 bg-black/35 backdrop-blur-sm flex items-start justify-center pt-10 px-4 " +
        (open ? "project-memory-search-dialog-in" : "project-memory-search-dialog-out")
      }
      onClick={onClose}
      onAnimationEnd={(e) => {
        if (!open && e.currentTarget === e.target) {
          onExited();
        }
      }}
    >
      <div
        className={
          "project-memory-search-panel w-full max-w-[680px] bg-surface-panel border border-ink/10 shadow-[0_24px_80px_rgba(0,0,0,0.22)] rounded-lg overflow-hidden " +
          (open ? "project-memory-search-panel-in" : "project-memory-search-panel-out")
        }
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 px-3 py-2 border-b border-ink/10">
          {!lockedProject && (
            <InlineMenuSelect
              value={selectedProjectKey}
              options={projects.map(
                (project): InlineMenuSelectOption => ({
                  value: project.key,
                  label: project.label,
                }),
              )}
              onChange={selectProject}
              menuAlign="parent"
              placeholder={t("list.unknown_project")}
              ariaLabel={t("memory_search.project_selector")}
              className="max-w-[128px]"
            />
          )}
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setSearched(false);
              setResults([]);
              setError(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") onClose();
              if (e.key === "Enter") runSearch();
            }}
            placeholder={t("memory_search.placeholder", { project: selectedProject?.label ?? "" })}
            className="flex-1 min-w-0 bg-transparent outline-none text-body text-ink placeholder:text-ink/35"
          />
          <button
            type="button"
            aria-label={t("header.search")}
            onClick={runSearch}
            disabled={loading || !query.trim()}
            className="p-1 text-ink/45 hover:text-ink disabled:opacity-35 disabled:hover:text-ink/45 rounded-md transition"
          >
            <Search className="w-4 h-4" />
          </button>
          <button
            type="button"
            aria-label={query.trim() ? t("list.clear") : t("detail.close")}
            onClick={clearOrClose}
            className="p-1 text-ink/45 hover:text-ink rounded-md transition"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
        <ScrollArea className="max-h-[55vh]">
          {loading && (
            <div className="px-4 py-6 text-center text-body-sm text-ink/45">
              {t("memory_search.searching")}
            </div>
          )}
          {!loading && error && (
            <div className="m-4 p-3 rounded bg-status-error/10 text-status-error text-body-sm">
              {error}
            </div>
          )}
          {!loading && !error && searched && query.trim() && results.length === 0 && (
            <div className="px-4 py-6 text-center text-body-sm text-ink/45">
              {t("memory_search.empty")}
            </div>
          )}
          {!loading && !error && results.length > 0 && (
            <ul className="divide-y divide-ink/5">
              {results.map((result, idx) => (
                <li key={`${result.recordId ?? result.artifactUri ?? idx}`} className="px-4 py-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="text-body-sm font-medium text-ink truncate">
                        {result.title ?? result.recordId ?? result.artifactUri ?? t("memory_search.result")}
                      </div>
                      {result.snippet && (
                        <div className="mt-1 text-body-sm text-ink/60 overflow-hidden [display:-webkit-box] [-webkit-line-clamp:3] [-webkit-box-orient:vertical]">
                          {result.snippet}
                        </div>
                      )}
                      {(result.recordId || result.artifactUri) && (
                        <div className="mt-1 text-meta text-ink/35 truncate">
                          {result.recordId ?? result.artifactUri}
                        </div>
                      )}
                    </div>
                    {result.score !== null && (
                      <span className="shrink-0 text-meta tabular-nums text-ink/40">
                        {result.score.toFixed(3)}
                      </span>
                    )}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </ScrollArea>
      </div>
    </div>
  );
}

function MemoryBackendMissingButton({
  status,
  placement = "top",
  onRefresh,
}: {
  status: MemoryBackendStatus | null;
  placement?: "top" | "bottom";
  onRefresh?: () => Promise<void> | void;
}) {
  const { t } = useI18n();
  const [state, setState] = useState<"idle" | "copied" | "error">("idle");
  const timerRef = useRef<number | null>(null);
  const installCommand =
    (status?.details as { installCommand?: string } | undefined)?.installCommand ??
    "";
  const backendName = status?.backend ?? "memory backend";

  useEffect(
    () => () => {
      if (timerRef.current) window.clearTimeout(timerRef.current);
    },
    [],
  );

  const resetSoon = (next: "copied" | "error") => {
    setState(next);
    if (timerRef.current) window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => setState("idle"), 1500);
  };

  const handleClick = async () => {
    let nextState: "copied" | "error" = "copied";
    try {
      await navigator.clipboard.writeText(installCommand);
    } catch (err) {
      console.error("qmd install command copy failed", err);
      nextState = "error";
    } finally {
      resetSoon(nextState);
      void onRefresh?.();
    }
  };

  const tip =
    state === "copied" ? (
      t("list.copied")
    ) : state === "error" ? (
      t("list.copy_failed")
    ) : (
      <div className="flex max-w-full flex-col gap-1.5 py-0.5">
        <span>{t("sidebar.memory_backend_required", { backend: backendName })}</span>
        <code className="block max-w-full truncate whitespace-nowrap font-mono text-[11px] text-ink/85">
          {installCommand}
        </code>
        <span className="text-ink/55">
          {t("sidebar.click_to_copy", { backend: backendName })}
        </span>
        <span className="text-ink/55">
          {t("sidebar.click_to_check_backend", { backend: backendName })}
        </span>
      </div>
    );

  return (
    <Tooltip content={tip} placement={placement}>
      <button
        type="button"
        onClick={handleClick}
        aria-label={t("sidebar.copy_memory_backend_install")}
        className="inline-flex p-1 text-ink/55 hover:text-status-error hover:bg-status-error/10 focus-visible:text-status-error focus-visible:bg-status-error/10 focus-visible:outline-none transition rounded-md"
      >
        <Skull className="w-4 h-4" />
      </button>
    </Tooltip>
  );
}

function SectionHeader({
  label,
  collapsed,
  onToggle,
}: {
  label: string;
  collapsed: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      onClick={onToggle}
      className="group flex items-center justify-between w-full px-2 mt-3 mb-1 text-caption uppercase text-ink/55 hover:text-ink/85 transition"
    >
      <span>{label}</span>
      <Chevron collapsed={collapsed} />
    </button>
  );
}

function Chevron({ collapsed }: { collapsed: boolean }) {
  return (
    <ChevronDown
      className={
        "w-3.5 h-3.5 transition-transform duration-150 " +
        (collapsed ? "-rotate-90" : "")
      }
    />
  );
}

type ProjectGroup = {
  key: string;
  label: string;
  count: number;
  path: string | null;
  latest: number;
  sessions: SessionInfo[];
};

function ProjectSidebarGroup({
  project,
  expanded,
  sessionsExpanded,
  selectedKey,
  selectedIdentityKey,
  liveState,
  runtimeSessionAliases,
  unreadSessionIds,
  onSelectProject,
  onSelectSession,
  onToggleSessionLimit,
  onProjectContextMenu,
  onSessionContextMenu,
}: {
  project: ProjectGroup;
  expanded: boolean;
  sessionsExpanded: boolean;
  selectedKey: string | null;
  selectedIdentityKey: string | null;
  liveState: LiveRuntimeState;
  runtimeSessionAliases: Record<string, string>;
  unreadSessionIds: Set<string>;
  onSelectProject: () => void;
  onSelectSession: (session: SessionInfo) => void;
  onToggleSessionLimit: () => void;
  onProjectContextMenu: (e: React.MouseEvent) => void;
  onSessionContextMenu: (
    session: SessionInfo,
    pos: { x: number; y: number },
  ) => void;
}) {
  const { t } = useI18n();
  const projectButtonRef = useRef<HTMLButtonElement>(null);
  const FolderIcon = expanded ? FolderOpen : Folder;
  const visibleSessions = sessionsExpanded
    ? project.sessions
    : project.sessions.slice(0, SIDEBAR_SESSION_PREVIEW_LIMIT);
  const canToggleSessionLimit =
    project.sessions.length > SIDEBAR_SESSION_PREVIEW_LIMIT;
  const toggleSessionLimit = () => {
    const collapsing = sessionsExpanded;
    onToggleSessionLimit();
    if (collapsing) {
      requestAnimationFrame(() => {
        projectButtonRef.current?.scrollIntoView({ block: "nearest" });
      });
    }
  };
  return (
    <div>
      <button
        ref={projectButtonRef}
        type="button"
        onClick={onSelectProject}
        title={project.path ?? project.label}
        className={
          "group flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-ink/70 transition hover:bg-ink/5 hover:text-ink"
        }
        onContextMenu={onProjectContextMenu}
      >
        <FolderIcon
          className={
            "w-3.5 h-3.5 shrink-0 text-ink/55 transition-transform duration-200 " +
            (expanded ? "scale-105" : "scale-100")
          }
        />
        <span className="flex-1 truncate text-body">{project.label}</span>
        <span className="text-meta text-ink/40 tabular-nums">{project.count}</span>
      </button>
      <div
        className={
          "grid overflow-hidden transition-[grid-template-rows,opacity] duration-200 ease-out " +
          (expanded ? "grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0")
        }
      >
        <div className="min-h-0 overflow-hidden">
          <div
            className={
              "mt-0.5 flex flex-col gap-0.5 transition-transform duration-200 ease-out " +
              (expanded ? "translate-y-0" : "-translate-y-1")
            }
          >
            {visibleSessions.map((session) => {
              const key = sessionKey(session);
              const runtimeSessionId = runtimeSessionAliases[`${session.agent}:${session.id}`] ?? session.id;
              const liveActivity = liveSessionActivity(liveState.sessions[runtimeSessionId]);
              return (
                <SidebarSessionItem
                  key={key}
                  item={session}
                  active={selectedKey === key || selectedIdentityKey === sessionIdentityKey(session)}
                  liveActivity={liveActivity}
                  unread={unreadSessionIds.has(session.id)}
                  onSelect={() => onSelectSession(session)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    onSessionContextMenu(session, { x: e.clientX, y: e.clientY });
                  }}
                />
              );
            })}
          </div>
          {canToggleSessionLimit && (
            <button
              type="button"
              onClick={toggleSessionLimit}
              className="mt-0.5 ml-7 px-1 py-1 text-left text-body-sm text-ink/40 transition hover:text-ink/65"
            >
              {t(sessionsExpanded ? "list.show_less" : "list.show_more")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function SidebarSessionItem({
  item,
  active,
  liveActivity,
  unread,
  onSelect,
  onContextMenu,
}: {
  item: SessionInfo;
  active: boolean;
  liveActivity: ReturnType<typeof liveSessionActivity>;
  unread: boolean;
  onSelect: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
}) {
  const { t } = useI18n();
  const title = item.title ?? item.firstUserMessage ?? t("list.no_user_message");
  const relativeTime = formatShortRelativeTime(item.updatedAt ?? item.startedAt, t);
  return (
    <button
      type="button"
      onClick={onSelect}
      onContextMenu={onContextMenu}
      title={title}
      className={
        "group relative flex w-full items-center gap-2 rounded-md py-1.5 pl-7 pr-2 text-left transition " +
        (active
          ? "bg-ink/10 text-ink"
          : "text-ink/65 hover:bg-ink/5 hover:text-ink")
      }
    >
      <SidebarSessionStatus activity={liveActivity} unread={unread} />
      <span className="flex h-3.5 w-3.5 shrink-0 items-center justify-center">
        <AgentGlyph agent={item.agent} className="h-3.5 w-3.5" />
      </span>
      <span
        className={
          "min-w-0 flex-1 truncate text-body-sm leading-snug " +
          (item.archived ? "text-ink/45" : "text-inherit")
        }
      >
        {item.title ?? item.firstUserMessage ?? (
          <span className="text-ink/30">{t("list.no_user_message")}</span>
        )}
      </span>
      <span className="shrink-0 text-meta tabular-nums text-ink/35">
        {relativeTime}
      </span>
    </button>
  );
}

function SidebarSessionStatus({
  activity,
  unread,
}: {
  activity: ReturnType<typeof liveSessionActivity>;
  unread: boolean;
}) {
  if (activity === "permission") {
    return (
      <span className="pointer-events-none absolute left-2 top-1/2 flex h-3.5 w-3.5 -translate-y-1/2 items-center justify-center text-status-warn">
        <KeyRound className="h-3 w-3" />
      </span>
    );
  }
  if (activity === "failed") {
    return (
      <span className="pointer-events-none absolute left-2 top-1/2 flex h-3.5 w-3.5 -translate-y-1/2 items-center justify-center text-status-error">
        <CircleAlert className="h-3 w-3" />
      </span>
    );
  }
  if (activity === "running") {
    return (
      <span className="pointer-events-none absolute left-2 top-1/2 flex h-3.5 w-3.5 -translate-y-1/2 items-center justify-center text-emerald">
        <RefreshCw className="h-3 w-3 animate-spin" />
      </span>
    );
  }
  if (!unread) return null;
  return (
    <span className="pointer-events-none absolute left-2 top-1/2 flex h-3.5 w-3.5 -translate-y-1/2 items-center justify-center text-accent-purple">
      <MailPlus className="h-3 w-3" />
    </span>
  );
}

function formatShortRelativeTime(ts: number | null, t: (key: string, vars?: Record<string, string | number>) => string): string {
  if (!ts) return "";
  const diffMs = Math.max(0, Date.now() - ts);
  const minute = 60 * 1000;
  const hour = 60 * minute;
  const day = 24 * hour;
  const week = 7 * day;
  const month = 30 * day;
  if (diffMs < hour) {
    return t("time.minute", { count: Math.max(1, Math.floor(diffMs / minute)) });
  }
  if (diffMs < day) return t("time.hour", { count: Math.floor(diffMs / hour) });
  if (diffMs < week) return t("time.day", { count: Math.floor(diffMs / day) });
  if (diffMs < month) return t("time.week", { count: Math.floor(diffMs / week) });
  return t("time.month", { count: Math.floor(diffMs / month) });
}

function NewChatView({
  projects,
  runtimeAgents,
  liveState,
  dispatchLiveEvent,
  onError,
  onPendingSession,
}: {
  projects: ProjectGroup[];
  runtimeAgents: RuntimeAgentMetadata[];
  liveState: LiveRuntimeState;
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onError: (error: string | null) => void;
  onPendingSession: (session: PendingNewChatSession) => void;
}) {
  const { t } = useI18n();
  const [text, setText] = useState("");
  const [projectKeyValue, setProjectKeyValue] = useState(() => projects[0]?.key ?? "");
  const [agent, setAgent] = useState<Agent>(
    () => runtimeAgents[0]?.agent ?? "codex",
  );
  const [attachments, setAttachments] = useState<ComposerAttachment[]>([]);
  const [attachmentMenuOpen, setAttachmentMenuOpen] = useState(false);
  const [sending, setSending] = useState(false);
  const [composerError, setComposerError] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const attachmentButtonRef = useRef<HTMLButtonElement>(null);
  const fallbackRuntimeSequenceRef = useRef(0);
  const project = projects.find((p) => p.key === projectKeyValue) ?? projects[0] ?? null;
  const workspacePath = project?.path ?? null;
  const agentOptions: InlineMenuSelectOption[] = runtimeAgents.map((runtimeAgent) => ({
    value: runtimeAgent.agent,
    label:
      runtimeAgent.agent === "codex"
        ? "Codex"
        : runtimeAgent.agent === "claude"
          ? "Claude"
        : "Gemini",
    icon: <AgentGlyph agent={runtimeAgent.agent} className="h-4 w-4" />,
  }));
  const selectedRuntimeAgent =
    runtimeAgents.find((runtimeAgent) => runtimeAgent.agent === agent) ?? null;
  const supportsAttachments =
    selectedRuntimeAgent?.capabilities?.supportsAttachments ?? false;
  const supportsImageAttachments =
    selectedRuntimeAgent?.capabilities?.supportsImageAttachments ?? false;
  const supportsEmbeddedContext =
    selectedRuntimeAgent?.capabilities?.supportsEmbeddedContext ?? false;
  const canSend =
    text.trim().length > 0 &&
    Boolean(workspacePath) &&
    agentOptions.length > 0 &&
    !sending;
  const attachmentMenuOptions = [
    supportsImageAttachments
      ? {
          key: "images" as const,
          label: t("new_chat.add_images"),
          icon: <ImageIcon className="h-4 w-4" />,
        }
      : null,
    supportsEmbeddedContext
      ? {
          key: "files" as const,
          label: t("new_chat.add_files"),
          icon: <FileText className="h-4 w-4" />,
        }
      : null,
  ].filter((option): option is NonNullable<typeof option> => Boolean(option));

  useEffect(() => {
    if (projectKeyValue && projects.some((p) => p.key === projectKeyValue)) return;
    setProjectKeyValue(projects[0]?.key ?? "");
  }, [projectKeyValue, projects]);

  useEffect(() => {
    if (agentOptions.some((option) => option.value === agent)) return;
    setAgent((runtimeAgents[0]?.agent ?? "codex") as Agent);
  }, [agent, agentOptions, runtimeAgents]);

  useEffect(() => {
    setAttachments((current) =>
      current.filter((attachment) => {
        if (attachment.kind === "image") return supportsImageAttachments;
        return supportsEmbeddedContext;
      }),
    );
    if (!supportsAttachments) {
      setAttachmentMenuOpen(false);
    }
  }, [supportsAttachments, supportsEmbeddedContext, supportsImageAttachments]);

  useEffect(() => {
    window.requestAnimationFrame(() => textareaRef.current?.focus());
  }, []);

  useEffect(() => {
    if (!attachmentMenuOpen) return;
    const close = () => setAttachmentMenuOpen(false);
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("resize", close);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [attachmentMenuOpen]);

  const addAttachmentPaths = useCallback(
    (paths: string[], kind: ComposerAttachment["kind"]) => {
      if (paths.length === 0) return;
      const next = paths.map((path) => ({
        kind,
        path,
        mimeType: null,
        name: basename(path),
      }));
      setAttachments((current) => dedupeComposerAttachments([...current, ...next]));
      setAttachmentMenuOpen(false);
    },
    [],
  );

  const removeAttachment = useCallback((path: string) => {
    setAttachments((current) => current.filter((attachment) => attachment.path !== path));
  }, []);

  const pickAttachments = useCallback(
    async (kind: "images" | "files") => {
      try {
        const selection = await open({
          multiple: true,
          directory: false,
          filters:
            kind === "images"
              ? [
                  {
                    name: "Images",
                    extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "heif"],
                  },
                ]
              : [
                  {
                    name: "Documents and code",
                    extensions: [
                      ...TEXT_ATTACHMENT_EXTENSIONS,
                    ],
                  },
                ],
        });
        if (!selection) return;
        const paths = Array.isArray(selection) ? selection : [selection];
        addAttachmentPaths(paths, kind === "images" ? "image" : "file");
      } catch (error) {
        const message = `Failed to open file picker: ${String(error)}`;
        setComposerError(message);
        onError(message);
      }
    },
    [addAttachmentPaths, onError],
  );

  const handleSend = async () => {
    const prompt = text.trim();
    if (!prompt || sending) return;
    if (!workspacePath || !project) {
      setComposerError(t("new_chat.no_project"));
      return;
    }
    if (!agentOptions.some((option) => option.value === agent)) {
      setComposerError("No configured runtime agent available");
      return;
    }
    setSending(true);
    setComposerError(null);
    onError(null);
    try {
      const handle = await startAgentSession({
        agent,
        workspacePath,
        options: { transport: "acp" },
      });
      const timestamp = Date.now();
      const localTurnId = `local-turn-${timestamp}`;
      const existingLiveSession = liveState.sessions[handle.sessioRuntimeSessionId];
      if (!existingLiveSession) {
        fallbackRuntimeSequenceRef.current += 1;
        dispatchLiveEvent({
          type: "runtime-event",
          event: {
            kind: "sessionStarted",
            sequence: liveState.lastSequence + fallbackRuntimeSequenceRef.current,
            timestamp,
            agent: handle.agent,
            sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
            agentRuntimeSessionId: handle.agentRuntimeSessionId,
            transport: handle.transport,
            workspacePath: handle.workspacePath,
            capabilities: handle.capabilities,
          },
        });
      }
      dispatchLiveEvent({
        type: "optimistic-user-message",
        sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
        turnId: localTurnId,
        text: prompt,
        timestamp,
      });
      onPendingSession({
        sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
        agent: handle.agent,
        projectPath: workspacePath,
        projectName: project.label,
        prompt,
        timestamp,
      });
      const turn = await sendAgentInput(handle.sessioRuntimeSessionId, {
        text: prompt,
        attachments: attachments.map(({ path, mimeType, kind }) => ({ path, mimeType, kind })),
      });
      dispatchLiveEvent({
        type: "replace-turn-id",
        sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
        from: localTurnId,
        to: turn.turnId,
      });
      setText("");
      setAttachments([]);
    } catch (err) {
      const message = String(err);
      setComposerError(message);
      onError(message);
    } finally {
      setSending(false);
    }
  };

  const activeSessionCount = Object.values(liveState.sessions).filter(
    (session) => !session.ended,
  ).length;

  return (
    <div className="flex flex-1 min-h-0 flex-col bg-surface-panel">
      <div className="flex flex-1 min-h-0 items-center justify-center px-6 pb-16">
        <div className="w-full max-w-[730px]">
          <h1 className="mb-11 text-center text-[28px] font-medium leading-tight tracking-normal text-ink/92">
            {t("new_chat.title")}
          </h1>
          {composerError && (
            <div className="mb-2 rounded-md border border-status-error/25 bg-status-error/10 px-3 py-2 text-body-sm text-status-error">
              {composerError}
            </div>
          )}
          <div
            className={
              "overflow-hidden rounded-2xl bg-ink/[0.055] shadow-[inset_0_0_0_1px_rgb(var(--color-ink)/0.08)] transition-shadow " +
              (composerError
                ? "shadow-[inset_0_0_0_1px_rgb(var(--color-status-error)/0.35)]"
                : "focus-within:shadow-[inset_0_0_0_1px_rgb(var(--color-ink)/0.20)]")
            }
          >
            {attachments.length > 0 && (
              <div className="flex flex-wrap gap-2 border-b border-ink/5 px-3.5 pt-3 pb-2">
                {attachments.map((attachment) => {
                  const Icon = attachment.kind === "image" ? ImageIcon : FileText;
                  return (
                    <span
                      key={attachment.path}
                      className="relative inline-flex min-w-[142px] max-w-[220px] items-center gap-2 rounded-lg border border-ink/8 bg-bg-panel px-3 py-2 pr-8 text-body-sm text-ink/78 shadow-sm"
                    >
                      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-emerald/10 text-emerald">
                        <Icon className="h-4 w-4" />
                      </span>
                      <span className="min-w-0">
                        <span className="block truncate font-medium leading-4">{attachment.name}</span>
                        <span className="block text-caption uppercase leading-4 text-ink/45">
                          {attachment.kind === "image" ? "Image" : "Text"}
                        </span>
                      </span>
                      <button
                        type="button"
                        onClick={() => removeAttachment(attachment.path)}
                        className="absolute right-1.5 top-1.5 rounded-full bg-ink text-[rgb(var(--color-bg-panel))] p-0.5 transition hover:bg-ink/75"
                        aria-label={`Remove ${attachment.name}`}
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </span>
                  );
                })}
              </div>
            )}
            <textarea
              ref={textareaRef}
              value={text}
              placeholder={t("new_chat.placeholder")}
              rows={2}
              onChange={(event) => {
                resizeTextareaToContent(event.currentTarget);
                setText(event.target.value);
              }}
              onInput={(event) => resizeTextareaToContent(event.currentTarget)}
              onKeyDown={(event) => {
                if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) {
                  return;
                }
                event.preventDefault();
                if (canSend) void handleSend();
              }}
              className="chat-composer-textarea block w-full resize-none bg-transparent px-3.5 py-3.5 text-body leading-5 text-ink/88 placeholder:text-ink/38 outline-none"
            />
            <div className="flex h-12 items-center justify-between gap-3 border-b border-ink/5 px-3 pb-2">
              <div className="flex min-w-0 items-center gap-3">
                {supportsAttachments && (
                  <Tooltip content={t("new_chat.add_context")} placement="top">
                    <button
                      ref={attachmentButtonRef}
                      type="button"
                      onClick={() => setAttachmentMenuOpen((open) => !open)}
                      className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-ink/55 transition hover:bg-ink/8 hover:text-ink"
                      aria-label={t("new_chat.add_context")}
                      aria-expanded={attachmentMenuOpen}
                      aria-haspopup="menu"
                    >
                      <Plus className="h-5 w-5" />
                    </button>
                  </Tooltip>
                )}
                <NewChatMenuButton icon={Hand} label="Default permissions" text />
              </div>
              <div className="flex shrink-0 items-center gap-2.5">
                <span className="rounded-md bg-ink/8 px-2.5 py-1 text-body-sm font-medium text-ink/64">
                  free
                </span>
                <NewChatMenuButton icon={Mic} label={t("new_chat.voice")} />
                <Tooltip content={sending ? t("new_chat.sending") : t("new_chat.send")} placement="top">
                  <button
                    type="button"
                    disabled={!canSend}
                    onClick={() => void handleSend()}
                    className="flex h-7 w-7 items-center justify-center rounded-full bg-ink/70 text-[rgb(var(--color-bg-panel))] transition hover:bg-ink disabled:cursor-not-allowed disabled:bg-ink/25 disabled:text-[rgb(var(--color-bg-panel)/0.7)]"
                    aria-label={sending ? t("new_chat.sending") : t("new_chat.send")}
                  >
                    <ArrowUp className="h-5 w-5" />
                  </button>
                </Tooltip>
              </div>
            </div>
            <div className="flex h-10 items-center gap-2 px-3 text-body-sm text-ink/55">
              <NewChatSelect
                ariaLabel={t("new_chat.project")}
                value={projectKeyValue}
                onChange={setProjectKeyValue}
                disabled={projects.length === 0}
                options={projects.map((p) => ({
                  value: p.key,
                  label: p.label,
                  icon: <Folder className="h-4 w-4 text-ink/55" />,
                }))}
              />
              <NewChatSelect
                ariaLabel={t("new_chat.agent")}
                value={agent}
                onChange={(value) => setAgent(value as Agent)}
                disabled={agentOptions.length === 0}
                options={agentOptions}
              />
              <NewChatMenuButton icon={Cpu} label={activeSessionCount > 0 ? `${activeSessionCount}` : t("new_chat.work_locally")} text />
              <NewChatMenuButton icon={GitBranch} label="main" text />
            </div>
          </div>
          {attachmentMenuOpen && attachmentButtonRef.current &&
            createPortal(
              <NewChatAttachmentMenu
                anchor={attachmentButtonRef.current}
                options={attachmentMenuOptions}
                onClose={() => setAttachmentMenuOpen(false)}
                onSelect={(key) => {
                  void pickAttachments(key);
                }}
              />,
              document.body,
            )}
        </div>
      </div>
    </div>
  );
}

function NewChatSelect({
  ariaLabel,
  value,
  options,
  disabled,
  onChange,
}: {
  ariaLabel: string;
  value: string;
  options: InlineMenuSelectOption[];
  disabled?: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <div className="flex min-w-0 max-w-[220px] items-center rounded-md text-ink/55 transition hover:bg-ink/8 hover:text-ink">
      <InlineMenuSelect
        value={value}
        options={disabled ? options.map((option) => ({ ...option, disabled: true })) : options}
        onChange={onChange}
        menuAlign="trigger"
        placeholder={ariaLabel}
        ariaLabel={ariaLabel}
        className="h-7 max-w-[220px] border-r-0 px-1.5 py-1 text-ink/60 hover:text-ink"
        menuClassName="bg-surface-panel"
        minMenuWidth={180}
        emptyContent={ariaLabel}
      />
    </div>
  );
}

function NewChatMenuButton({
  icon: Icon,
  label,
  text,
}: {
  icon: LucideIcon;
  label: string;
  text?: boolean;
}) {
  return (
    <button
      type="button"
      className={
        "flex min-w-0 items-center gap-1.5 rounded-md py-1 text-body-sm text-ink/55 transition hover:bg-ink/8 hover:text-ink " +
        (text ? "max-w-[220px] px-1.5" : "h-7 w-7 justify-center px-0")
      }
      aria-label={label}
    >
      <Icon className="h-4 w-4 shrink-0" />
      {text && <span className="truncate">{label}</span>}
      {text && <ChevronDown className="h-3.5 w-3.5 shrink-0" />}
    </button>
  );
}

function NewChatAttachmentMenu({
  anchor,
  options,
  onSelect,
  onClose,
}: {
  anchor: HTMLButtonElement;
  options: Array<{
    key: "images" | "files";
    label: string;
    icon: React.ReactNode;
  }>;
  onSelect: (key: "images" | "files") => void;
  onClose: () => void;
}) {
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const updatePosition = useCallback(() => {
    const rect = anchor.getBoundingClientRect();
    const menuWidth = menuRef.current?.offsetWidth ?? 192;
    const menuHeight = menuRef.current?.offsetHeight ?? 8 + options.length * 40;
    const left = Math.round(
      Math.max(8, Math.min(rect.left + rect.width / 2 - menuWidth / 2, window.innerWidth - menuWidth - 8)),
    );
    const top = Math.round(Math.max(8, rect.top - menuHeight - 10));
    setPos({ top, left });
  }, [anchor, options.length]);

  useLayoutEffect(() => {
    updatePosition();
  }, [updatePosition]);

  useEffect(() => {
    const reposition = () => updatePosition();
    window.addEventListener("scroll", reposition, true);
    window.addEventListener("resize", reposition);
    return () => {
      window.removeEventListener("scroll", reposition, true);
      window.removeEventListener("resize", reposition);
    };
  }, [updatePosition]);

  return (
    <>
      <div className="fixed inset-0 z-[39] bg-transparent" onMouseDown={onClose} />
      <div
        ref={menuRef}
        className="fixed z-40 min-w-[192px] rounded-xl border border-ink/10 bg-surface-panel p-1.5 shadow-[0_20px_60px_rgba(0,0,0,0.22)]"
        style={{
          top: pos?.top ?? -9999,
          left: pos?.left ?? -9999,
          visibility: pos ? "visible" : "hidden",
        }}
        role="menu"
      >
        {options.map((option) => (
          <button
            key={option.key}
            type="button"
            className="flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-body-sm text-ink/72 transition hover:bg-ink/6 hover:text-ink"
            role="menuitem"
            onClick={() => {
              onSelect(option.key);
              onClose();
            }}
          >
            <span className="shrink-0 text-ink/55">{option.icon}</span>
            <span>{option.label}</span>
          </button>
        ))}
      </div>
    </>
  );
}

function HeaderMessageMetaButton({
  label,
  open,
  onToggle,
  compact = false,
}: {
  label: string;
  open: boolean;
  onToggle: () => void;
  compact?: boolean;
}) {
  const Icon = open ? ListChevronsDownUp : ListChevronsUpDown;
  return (
    <div
      data-tauri-drag-region
      className={
        "inline-flex items-center gap-1.5 " +
        (compact
          ? "text-caption text-ink/40"
          : "text-body font-medium text-ink/45")
      }
    >
      <span data-tauri-drag-region className="tabular-nums leading-tight">
        {label}
      </span>
      <button
        type="button"
        data-tauri-drag-region="false"
        onClick={onToggle}
        className="group -m-1 rounded-md p-1 text-ink/35 transition-colors hover:bg-ink/[0.05] hover:text-ink/65"
      >
        <Icon
          className={
            "shrink-0 transition-[transform,opacity] duration-200 " +
            (compact ? "h-3.5 w-3.5" : "h-4 w-4") +
            (open ? " rotate-0 scale-110" : " rotate-0 scale-100")
          }
        />
      </button>
    </div>
  );
}

function HeaderModeTabs({
  mode,
  onChange,
}: {
  mode: DetailMode;
  onChange: (mode: DetailMode) => void;
}) {
  const items: { value: DetailMode; label: string }[] = [
    { value: "chat", label: "Chat" },
    { value: "memory", label: "Memory" },
  ];
  const activeIndex = Math.max(
    0,
    items.findIndex((item) => item.value === mode),
  );
  const BTN_W = 72;
  return (
    <div className="relative flex items-center rounded-md bg-ink/[0.14] p-0.5">
      <div
        aria-hidden
        className="absolute top-0.5 left-0.5 h-[26px] rounded bg-surface shadow-[0_1px_2px_rgba(0,0,0,0.18)] transition-transform duration-300 ease-out"
        style={{
          width: `${BTN_W}px`,
          transform: `translateX(${activeIndex * BTN_W}px)`,
        }}
      />
      {items.map(({ value, label }, index) => {
        const active = index === activeIndex;
        return (
          <button
            key={label}
            type="button"
            onClick={() => onChange(value)}
            style={{ width: `${BTN_W}px` }}
            className={
              "relative z-10 h-[26px] flex items-center justify-center rounded text-body-sm leading-none transition-colors duration-150 " +
              (active ? "text-ink" : "text-ink/55 hover:text-ink/85")
            }
          >
            {label}
          </button>
        );
      })}
    </div>
  );
}

function ThemeSwitcher({
  mode,
  onChange,
}: {
  mode: ThemeMode;
  onChange: (m: ThemeMode) => void;
}) {
  const { t } = useI18n();
  const items: { value: ThemeMode; icon: typeof Sun; label: string }[] = [
    { value: "light", icon: Sun, label: t("theme.light") },
    { value: "dark", icon: Moon, label: t("theme.dark") },
    { value: "system", icon: Monitor, label: t("theme.system") },
  ];
  return (
    <div className="flex items-center gap-0.5 rounded-md bg-ink/[0.04] p-0.5">
      {items.map(({ value, icon: Icon, label }) => {
        const active = mode === value;
        return (
          <Tooltip key={value} content={label} placement="top">
            <button
              type="button"
              onClick={() => onChange(value)}
              aria-label={label}
              className={
                "p-1 rounded transition " +
                (active
                  ? "bg-ink/10 text-ink"
                  : "text-ink/45 hover:text-ink/80")
              }
            >
              <Icon className="w-3.5 h-3.5" />
            </button>
          </Tooltip>
        );
      })}
    </div>
  );
}

function LanguageSwitcher({
  lang,
  onChange,
}: {
  lang: Lang;
  onChange: (l: Lang) => void;
}) {
  const { t } = useI18n();
  const items: { value: Lang; label: string; tip: string }[] = [
    { value: "en", label: "EN", tip: t("lang.english") },
    { value: "zh", label: "中", tip: t("lang.chinese") },
  ];
  return (
    <div className="flex items-center gap-0.5 rounded-md bg-ink/[0.04] p-0.5">
      {items.map(({ value, label, tip }) => {
        const active = lang === value;
        return (
          <Tooltip key={value} content={tip} placement="top">
            <button
              type="button"
              onClick={() => onChange(value)}
              aria-label={tip}
              className={
                "px-1.5 h-[22px] min-w-[22px] flex items-center justify-center rounded text-caption font-medium leading-none transition " +
                (active
                  ? "bg-ink/10 text-ink"
                  : "text-ink/45 hover:text-ink/80")
              }
            >
              {label}
            </button>
          </Tooltip>
        );
      })}
    </div>
  );
}
