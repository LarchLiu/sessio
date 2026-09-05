import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Menu } from "@tauri-apps/api/menu/menu";
import { MenuItem } from "@tauri-apps/api/menu/menuItem";
import { cursorPosition, getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalPosition } from "@tauri-apps/api/dpi";
import {
  Agent,
  archiveProject,
  getDebugConfig,
  getProjectGitSummary,
  listSessioApps,
  listThreadIndex,
  openAppshotPermissionsPanel,
  SessionInfo,
  removeSessionsByScope,
  removeSessionFiles,
  takeConfigRecoveryNotice,
  updateSessionRenameTitle,
  type SessionScope,
  type SessioAppInfo,
  type ThreadIndexItemInfo,
} from "./api";
import { syncTrayMenu, type TrayRecentEntry } from "./tray";
import AppLayout from "./layouts/AppLayout";
import { type ActiveMessageMeta } from "./pages/ChatPage";
import AppHeader from "./components/AppHeader";
import AppMain from "./components/AppMain";
import AppOverlays, { type DeleteTarget } from "./components/AppOverlays";
import AppSidebar from "./components/AppSidebar";
import AppRightSidebar from "./components/AppRightSidebar";
import TerminalDock from "./components/TerminalDock";
import ToastStack from "./components/ToastStack";
import UpdateConfirmDialog from "./components/UpdateConfirmDialog";
import SettingsPage from "./pages/SettingsPage";
import AutoTasksPage from "./pages/AutoTasksPage";
import AppsPage from "./pages/AppsPage";
import type { ToastStackMessage } from "./components/ToastStack";
import type { CanvasKey } from "./canvasTypes";
import { useAppData } from "./hooks/useAppData";
import { usePendingNewChats } from "./hooks/usePendingNewChats";
import { usePlanTaskRuntimeCompletion } from "./hooks/usePlanTaskRuntimeCompletion";
import { useProjectGroups } from "./hooks/useProjectGroups";
import { useRuntimeEventSubscription } from "./hooks/useRuntimeEventSubscription";
import { useSystemNotifications } from "./hooks/useSystemNotifications";
import { useSelectedSessionSync } from "./hooks/useSelectedSessionSync";
import { useSessionAncestors } from "./hooks/useSessionAncestors";
import { useUnreadSessions } from "./hooks/useUnreadSessions";
import { useTheme } from "./theme";
import { useI18n } from "./i18n";
import { useUpdateCheck } from "./updater";
import {
  applyRuntimeAction,
  emptyLiveRuntimeState,
} from "./runtimeChat";
import { useRuntimeAgents } from "./runtimeAgents";
import { CalendarClock, Folder, Goal, Hash, Kanban, MessagesSquare, MessageSquare, MessageSquareText } from "lucide-react";
import type { ChatFilesSubview } from "./components/ChatFilesView";
import type { ChatView, DetailMode, PendingNewChatSession, ViewMode } from "./navigation";
import {
  isSubagentOnly,
  matchesScope,
  mergeRuntimeSessionAliases,
  projectFilterKey,
  sessionDisplayTitle,
  sessionIdentityKey,
  sessionKey,
  deleteUnreadKeys,
  threadUnreadKeys,
  type Filter,
  type ProjectSelection,
} from "./appUtils";
import { appendAppshotToActiveComposer } from "./appshot";

const VIEW_MODE_STORAGE_KEY = "sessio.viewMode";
const RIGHT_SIDEBAR_OPEN_STORAGE_KEY = "sessio.rightSidebarOpen";

type ThreadSelection = { projectId: string; threadId: string; goal: string } | null;
type ProjectFileSelectionRequest = {
  path: string;
  requestId: number;
};
type CanvasFileSelectionRequest = { paths: string[]; requestId: number };
type UtilityView = "autoTasks" | "apps" | null;

function readViewMode(): ViewMode {
  if (typeof localStorage === "undefined") return "native";
  const v = localStorage.getItem(VIEW_MODE_STORAGE_KEY);
  return v === "cross" ? "cross" : "native";
}

function readRightSidebarOpen(): boolean {
  if (typeof localStorage === "undefined") return false;
  return localStorage.getItem(RIGHT_SIDEBAR_OPEN_STORAGE_KEY) === "1";
}

const IS_MAC =
  typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);

async function revealMainWindow(): Promise<void> {
  try {
    await invoke("reveal_main_window");
    return;
  } catch {
    const win = getCurrentWindow();
    await Promise.allSettled([
      win.unminimize(),
      win.show(),
      win.setFocus(),
    ]);
  }
}

export default function App() {
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<ToastStackMessage | null>(null);
  const {
    sessions,
    setSessions,
    projects,
    setProjects,
    indexPhase,
    refreshSessions,
    refreshMemoryBackend,
  } = useAppData({ setError });
  const [filter, setFilter] = useState<Filter>({ kind: "all" });
  const [selectedProject, setSelectedProject] = useState<ProjectSelection>(null);
  const [selectedThread, setSelectedThread] = useState<ThreadSelection>(null);
  const [selected, setSelected] = useState<SessionInfo | null>(null);
  const [newChatProjectKey, setNewChatProjectKey] = useState<string | null>(null);
  const [lastSelectedProjectKey, setLastSelectedProjectKey] = useState<string | null>(null);
  const [expandProject, setExpandProject] = useState(true);
  const [expandApps, setExpandApps] = useState(true);
  const [sessioApps, setSessioApps] = useState<SessioAppInfo[]>([]);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [rightSidebarOpen, setRightSidebarOpen] = useState<boolean>(() =>
    readRightSidebarOpen(),
  );
  const [terminalDockOpen, setTerminalDockOpen] = useState(false);
  const [rightSidebarFilesReloadKey, setRightSidebarFilesReloadKey] = useState(0);
  const [memorySearchOpen, setMemorySearchOpen] = useState(false);
  const [memorySearchMounted, setMemorySearchMounted] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [utilityView, setUtilityView] = useState<UtilityView>(null);
  const [selectedApp, setSelectedApp] = useState<SessioAppInfo | null>(null);
  const [appRuntimeSessions, setAppRuntimeSessions] = useState<Record<string, string>>({});
  const [updateConfirmOpen, setUpdateConfirmOpen] = useState(false);
  const [updateConfirmMounted, setUpdateConfirmMounted] = useState(false);
  const [viewMode] = useState<ViewMode>(() => readViewMode());
  const [detailMode, setDetailMode] = useState<DetailMode>("chat");
  const [metaPopoverOpen, setMetaPopoverOpen] = useState(false);
  const [metaPopoverMounted, setMetaPopoverMounted] = useState(false);
  const [activeMessageMeta, setActiveMessageMeta] =
    useState<ActiveMessageMeta | null>(null);
  const [chatView, setChatView] = useState<ChatView>("chat");
  const [filesSubview, setFilesSubview] = useState<ChatFilesSubview>("code");
  const [deleteTarget, setDeleteTarget] = useState<DeleteTarget | null>(null);
  const [projectGitRepos, setProjectGitRepos] = useState<Record<string, boolean>>({});
  const projectGitReposRef = useRef<Record<string, boolean>>({});
  const projectGitRepoProbeRef = useRef<Set<string>>(new Set());
  const [liveRuntimeState, dispatchLiveRuntimeEvent] = useReducer(
    applyRuntimeAction,
    emptyLiveRuntimeState,
  );
  const [pendingSelectSession, setPendingSelectSession] = useState<{
    agent: Agent;
    sessionId: string;
    detailMode?: DetailMode;
  } | null>(null);
  const [pendingNewChats, setPendingNewChats] = useState<Record<string, PendingNewChatSession>>({});
  const [runtimeSessionAliases, setRuntimeSessionAliases] = useState<Record<string, string>>({});
  const [projectFileSelectionBySession, setProjectFileSelectionBySession] = useState<Record<string, ProjectFileSelectionRequest>>({});
  const [canvasFileSelectionBySession, setCanvasFileSelectionBySession] = useState<Record<string, CanvasFileSelectionRequest>>({});
  const [threadIndexItems, setThreadIndexItems] = useState<ThreadIndexItemInfo[]>([]);
  const { mode, setMode } = useTheme();
  const [systemAppearance, setSystemAppearance] = useState<"light" | "dark">("dark");
  const { lang, setLang, t } = useI18n();
  const {
    agents: runtimeAgents,
    lastSelection: lastRuntimeAgentSelection,
    rememberSelection: rememberRuntimeAgentSelection,
  } = useRuntimeAgents();
  const [debugAcpConfig, setDebugAcpConfig] = useState(false);
  const [debugUpdatePreview, setDebugUpdatePreview] = useState(false);
  const update = useUpdateCheck(__APP_VERSION__, debugUpdatePreview);
  const indexing = indexPhase !== "idle";
  const rebuilding = indexPhase === "rebuilding";
  const refreshSessioApps = useCallback(async () => {
    try {
      const catalog = await listSessioApps();
      setSessioApps(catalog.apps);
      setSelectedApp((current) =>
        current
          ? catalog.apps.find((app) => app.directoryPath === current.directoryPath) ?? current
          : null,
      );
    } catch (error) {
      setError(String(error));
    }
  }, []);
  const openUpdateConfirm = useCallback(() => {
    if (!update.hasUpdate || !update.latestVersion || update.installing) return;
    void revealMainWindow();
    setUpdateConfirmOpen(true);
    setUpdateConfirmMounted(true);
  }, [update.hasUpdate, update.installing, update.latestVersion]);

  const handleInstallUpdate = useCallback(async () => {
    try {
      if (update.updateReady) {
        await update.restart();
        return;
      }
      await update.install();
    } catch (err) {
      setError(String(err));
    }
  }, [update.install, update.restart, update.updateReady]);

  const availableSessions = useMemo(
    () => sessions.filter((s) => s.available),
    [sessions]
  );

  useEffect(() => {
    let cancelled = false;
    getDebugConfig()
      .then((config) => {
        if (!cancelled) {
          setDebugAcpConfig(config.acpConfig);
          setDebugUpdatePreview(config.updatePreview);
        }
      })
      .catch((err) => console.warn("load debug config failed", err));
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    void refreshSessioApps();
  }, [refreshSessioApps]);

  useEffect(() => {
    let cancelled = false;
    takeConfigRecoveryNotice()
      .then((notice) => {
        if (cancelled || !notice) return;
        const line = notice.lineNumber
          ? notice.lineText
            ? `第 ${notice.lineNumber} 行：${notice.lineText}`
            : `第 ${notice.lineNumber} 行`
          : null;
        const backup = notice.backupPath ? `\n已备份到：${notice.backupPath}` : "";
        const details = line ? `\n问题位置：${line}` : "";
        setToast({
          message:
            `配置文件解析失败，已回退到默认配置。\n文件：${notice.path}${details}${backup}\n${notice.error}`,
          tone: "error",
        });
      })
      .catch((err) => {
        if (cancelled) return;
        console.warn("load config recovery notice failed", err);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlistenPromise = listen<{ path: string; shortcut: string }>(
      "appshot_captured",
      (event) => {
        void (async () => {
          try {
            const inserted = await appendAppshotToActiveComposer(event.payload.path);
            setToast({
              message: inserted
                ? t("appshot.inserted")
                : t("appshot.no_active_composer"),
              tone: inserted ? "info" : "error",
            });
          } catch (err) {
            if (disposed) return;
            setToast({
              message: t("appshot.capture_failed", { error: String(err) }),
              tone: "error",
            });
          }
        })();
      },
    );
    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [t]);

  useEffect(() => {
    let disposed = false;
    const unlistenPromise = listen("appshot_permission_required", () => {
      if (disposed) return;
      setToast({
        message: t("appshot.permission.required"),
        tone: "error",
      });
    });
    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [t]);

  const handleProjectGitRepoDetected = useCallback((projectPath: string, isRepo: boolean) => {
    if (!projectPath) return;
    setProjectGitRepos((current) => {
      if (current[projectPath] === isRepo) return current;
      const next = { ...current, [projectPath]: isRepo };
      projectGitReposRef.current = next;
      return next;
    });
  }, []);

  const projectGitProbePathsKey = useMemo(
    () =>
      Array.from(
        new Set([
          ...projects.map((project) => project.path).filter(Boolean),
          ...(utilityView === "apps" && selectedApp?.directoryPath
            ? [selectedApp.directoryPath]
            : []),
        ]),
      )
        .sort()
        .join("\n"),
    [projects, selectedApp?.directoryPath, utilityView],
  );

  useEffect(() => {
    const paths = projectGitProbePathsKey ? projectGitProbePathsKey.split("\n") : [];
    if (paths.length === 0) {
      setProjectGitRepos((current) => {
        if (Object.keys(current).length === 0) return current;
        projectGitReposRef.current = {};
        return {};
      });
      projectGitRepoProbeRef.current.clear();
      return;
    }

    setProjectGitRepos((current) => {
      const pathSet = new Set(paths);
      let changed = false;
      const next: Record<string, boolean> = {};
      for (const path of paths) {
        if (Object.prototype.hasOwnProperty.call(current, path)) {
          next[path] = current[path];
        }
      }
      for (const path of Object.keys(current)) {
        if (!pathSet.has(path)) changed = true;
      }
      for (const path of Array.from(projectGitRepoProbeRef.current)) {
        if (!pathSet.has(path)) projectGitRepoProbeRef.current.delete(path);
      }
      if (!changed) return current;
      projectGitReposRef.current = next;
      return next;
    });

    const missing = paths.filter(
      (path) => projectGitReposRef.current[path] === undefined && !projectGitRepoProbeRef.current.has(path),
    );
    if (missing.length === 0) return;

    for (const path of missing) {
      projectGitRepoProbeRef.current.add(path);
      getProjectGitSummary(path)
        .then((summary) => {
          handleProjectGitRepoDetected(path, summary.isRepo);
        })
        .catch(() => {
          handleProjectGitRepoDetected(path, false);
        })
        .finally(() => {
          projectGitRepoProbeRef.current.delete(path);
        });
    }
  }, [handleProjectGitRepoDetected, projectGitProbePathsKey]);

  const {
    unreadSessionIds,
    setUnreadSessionIds,
    handleMessageCount,
  } = useUnreadSessions({
    sessions,
    selected,
    runtimeSessionAliases,
    setSessions,
    setSelected,
    setActiveMessageMeta,
  });

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
    localStorage.setItem(
      RIGHT_SIDEBAR_OPEN_STORAGE_KEY,
      rightSidebarOpen ? "1" : "0",
    );
  }, [rightSidebarOpen]);

  useEffect(() => {
    if (!rightSidebarOpen) return;
    setRightSidebarFilesReloadKey((current) => current + 1);
  }, [rightSidebarOpen]);

  useRuntimeEventSubscription({
    selected,
    runtimeSessionAliases,
    dispatchLiveRuntimeEvent,
    setUnreadSessionIds,
    setError,
  });

  usePendingNewChats({
    pendingNewChats,
    liveSessions: liveRuntimeState.sessions,
    setRuntimeSessionAliases,
    setSessions,
    setSelectedProject,
    setSelectedThread,
    setSelected,
    setDetailMode,
    setPendingSelectSession,
    setPendingNewChats,
    setError,
  });

  usePlanTaskRuntimeCompletion({
    pendingNewChats,
    liveSessions: liveRuntimeState.sessions,
    setError,
  });

  useEffect(() => {
    setRuntimeSessionAliases((prev) =>
      mergeRuntimeSessionAliases(prev, liveRuntimeState.sessions),
    );
  }, [liveRuntimeState.sessions]);

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

  const {
    projectGroups,
    expandedProjects,
    setExpandedProjects,
    expandedProjectSessions,
    setExpandedProjectSessions,
  } = useProjectGroups({
    availableSessions,
    projects,
    liveSessions: liveRuntimeState.sessions,
    runtimeSessionAliases,
    selected,
  });

  useEffect(() => {
    if (!lastSelectedProjectKey) return;
    if (projectGroups.some((project) => project.key === lastSelectedProjectKey)) return;
    setLastSelectedProjectKey(null);
  }, [lastSelectedProjectKey, projectGroups]);

  const rememberSidebarProject = useCallback((projectGroup: { key: string } | null | undefined) => {
    if (!projectGroup) return;
    setLastSelectedProjectKey(projectGroup.key);
  }, []);

  const clearThreadUnread = useCallback((thread: ThreadIndexItemInfo) => {
    setUnreadSessionIds((prev) =>
      deleteUnreadKeys(
        prev,
        threadUnreadKeys(thread, runtimeSessionAliases, liveRuntimeState.sessions),
      ),
    );
  }, [liveRuntimeState.sessions, runtimeSessionAliases, setUnreadSessionIds]);

  useSelectedSessionSync({
    availableSessions,
    selected,
    detailMode,
    pendingSelectSession,
    projects,
    setSelected,
    setDetailMode,
    setFilter,
    setSelectedProject,
    setSelectedThread,
    setExpandedProjects,
    setPendingSelectSession,
  });

  const refreshThreadIndex = useCallback((projectId?: string | null) => {
    return listThreadIndex(projectId).then((rows) => {
      setThreadIndexItems((prev) => {
        if (!projectId) return rows;
        const next = prev.filter((item) => item.projectId !== projectId);
        return [...next, ...rows].sort((a, b) => b.time - a.time);
      });
    });
  }, []);

  useEffect(() => {
    if (projects.length === 0) {
      setThreadIndexItems([]);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      listThreadIndex()
        .then((rows) => {
          if (!cancelled) setThreadIndexItems(rows);
        })
        .catch((err) => {
          if (!cancelled) console.warn("load thread index failed", err);
        });
    };
    refresh();
    const unlistenThreads = listen<{
      projectId?: string | null;
      threadId?: string | null;
    }>("threads_updated", (event) => {
      const projectId = event.payload?.projectId ?? null;
      refreshThreadIndex(projectId).catch((err) => {
        if (!cancelled) console.warn("refresh thread index failed", err);
      });
    });
    return () => {
      cancelled = true;
      unlistenThreads.then((f) => f()).catch(() => {});
    };
  }, [projects.length, refreshThreadIndex]);

  const recentForMenu = useMemo<TrayRecentEntry[]>(() => {
    // SQL-side filtering already drops thread/auxiliary sessions, so the tray
    // recent list only needs to dedupe subagent-only rows. Threads still
    // arrive separately through threadIndexItems.
    const entries: TrayRecentEntry[] = threadIndexItems.map((item) => ({
      kind: "thread",
      thread: item,
      time: item.time,
    }));
    for (const session of availableSessions) {
      if (isSubagentOnly(session)) continue;
      entries.push({
        kind: "session",
        session,
        time: session.updatedAt ?? session.startedAt ?? 0,
      });
    }
    return entries.sort((a, b) => b.time - a.time).slice(0, 5);
  }, [availableSessions, threadIndexItems]);

  useEffect(() => {
    if (!selectedProject) return;
    if (projects.some((project) => project.id === selectedProject.projectId)) return;
    setSelectedProject(null);
  }, [projects, selectedProject]);

  useEffect(() => {
    if (!selectedThread) return;
    if (projects.some((project) => project.id === selectedThread.projectId)) return;
    setSelectedThread(null);
  }, [projects, selectedThread]);

  const activeProject = selectedProject
    ? projects.find((project) => project.id === selectedProject.projectId) ?? null
    : null;
  const activeThreadProject = selectedThread
    ? projects.find((project) => project.id === selectedThread.projectId) ?? null
    : null;
  const selectedProjectId = selectedProject?.projectId ?? null;
  const selectedThreadId = selectedThread?.threadId ?? null;
  const selectedSessionProject =
    selected?.projectPath
      ? projects.find((project) => project.path === selected.projectPath) ?? null
      : null;
  const newChatProject = newChatProjectKey
    ? projects.find((project) => project.id === newChatProjectKey || project.path === newChatProjectKey) ?? null
    : null;

  const selectedKey = selected ? sessionKey(selected) : null;
  const selectedIdentityKey = selected ? sessionIdentityKey(selected) : null;
  const selectedAncestorSessions = useSessionAncestors(
    selected,
    sessions,
    selectedIdentityKey,
  );

  useEffect(() => {
    if (!selectedThreadId) return;
    const thread = threadIndexItems.find((item) => item.threadId === selectedThreadId);
    if (thread) clearThreadUnread(thread);
  }, [clearThreadUnread, selectedThreadId, threadIndexItems, unreadSessionIds]);

  const openSessionSelection = useCallback((
    session: SessionInfo,
    options?: {
      detailMode?: DetailMode;
      revealWindow?: boolean;
      projectLabel?: string;
    },
  ) => {
    const project = session.projectPath
      ? projects.find((item) => item.path === session.projectPath) ?? null
      : null;
    if (project) {
      rememberSidebarProject({ key: project.id });
      setFilter({
        kind: "project",
        key: projectFilterKey(project),
        label: options?.projectLabel ?? project.name,
      });
    }
    setUtilityView(null);
    setSelectedProject(null);
    setSelectedThread(null);
    setNewChatProjectKey(null);
    setSelected(session);
    setDetailMode(options?.detailMode ?? "chat");
    if (options?.revealWindow) void revealMainWindow();
  }, [projects, rememberSidebarProject, setFilter]);

  const openThreadSelection = useCallback((
    thread: ThreadIndexItemInfo,
    options?: {
      detailMode?: DetailMode;
      revealWindow?: boolean;
      projectLabel?: string;
      clearUnread?: boolean;
    },
  ) => {
    const project = projects.find((item) => item.id === thread.projectId) ?? null;
    if (project) {
      rememberSidebarProject({ key: project.id });
      setFilter({
        kind: "project",
        key: projectFilterKey(project),
        label: options?.projectLabel ?? project.name,
      });
    }
    if (options?.clearUnread !== false) clearThreadUnread(thread);
    setUtilityView(null);
    setSelected(null);
    setSelectedProject(null);
    setSelectedThread({ projectId: thread.projectId, threadId: thread.threadId, goal: thread.goal });
    setNewChatProjectKey(null);
    setDetailMode(options?.detailMode ?? "threadMultiSessionChat");
    if (options?.revealWindow) void revealMainWindow();
  }, [clearThreadUnread, projects, rememberSidebarProject, setFilter]);

  useSystemNotifications({
    t,
    sessions,
    threadIndexItems,
    liveSessions: liveRuntimeState.sessions,
    runtimeSessionAliases,
    pendingNewChats,
    unreadSessionIds,
    selected,
    selectedThreadId,
    detailMode,
  });

  useEffect(() => {
    syncTrayMenu(recentForMenu, {
      show: t("menubar.show"),
      quit: t("menubar.quit"),
      noSessions: t("menubar.no_sessions"),
      noMessage: t("list.no_user_message"),
      updateAvailable: t("menubar.update_available"),
      updateInstalling: t("sidebar.update_installing"),
    }, systemAppearance, {
      hasUpdate: update.hasUpdate,
      latestVersion: update.latestVersion,
      installing: update.installing,
      install: openUpdateConfirm,
    }, {
      onSelectSession: (session) => {
        openSessionSelection(session, { revealWindow: true });
      },
      onSelectThread: (thread) => {
        openThreadSelection(thread, { revealWindow: true, detailMode: "threadMultiSessionChat" });
      },
    });
  }, [
    openSessionSelection,
    openThreadSelection,
    recentForMenu,
    t,
    systemAppearance,
    update.hasUpdate,
    update.latestVersion,
    update.installing,
    openUpdateConfirm,
  ]);

  const removeSessionsInScope = async (scope: SessionScope) => {
    const targets = availableSessions.filter(
      (s) => !isSubagentOnly(s) && matchesScope(scope, s),
    );
    if (targets.length === 0) return;
    await removeSessionsByScope(scope);
    if (selected && targets.some((s) => sessionKey(s) === sessionKey(selected))) {
      setSelected(null);
      setSelectedThread(null);
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
          setSelectedThread(null);
        }
      } else if (target.kind === "project") {
        await removeSessionsInScope(target.scope);
        await archiveProject(target.projectId);
        setProjects((prev) => prev.filter((project) => project.id !== target.projectId));
        setSelectedProject((current) =>
          current?.projectId === target.projectId ? null : current,
        );
        setSelectedThread((current) =>
          current?.projectId === target.projectId ? null : current,
        );
        setNewChatProjectKey((current) =>
          current === target.projectId ? null : current,
        );
        setLastSelectedProjectKey((current) =>
          current === target.projectId ? null : current,
        );
        setFilter((current) =>
          current.kind === "project" && current.key === target.scope.key
            ? { kind: "all" }
            : current,
        );
      } else {
        await removeSessionsInScope(target.scope);
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setDeleteTarget(null);
    }
  };

  const renameSession = async (session: SessionInfo) => {
    const currentTitle = session.renameTitle ?? sessionDisplayTitle(session) ?? "";
    const nextTitle = window.prompt(t("session.rename_title"), currentTitle);
    if (nextTitle === null) return;
    try {
      const trimmed = nextTitle.trim();
      await updateSessionRenameTitle(session.agent, session.id, trimmed || null);
      setSessions((prev) =>
        prev.map((item) =>
          item.agent === session.agent && item.id === session.id
            ? { ...item, renameTitle: trimmed || null }
            : item,
        ),
      );
      setSelected((prev) =>
        prev && prev.agent === session.agent && prev.id === session.id
          ? { ...prev, renameTitle: trimmed || null }
          : prev,
      );
      void refreshSessions();
    } catch (err) {
      setError(String(err));
    }
  };

  const openDeleteMenu = async (
    pos: { x: number; y: number },
    onDelete: (pos: { x: number; y: number }) => void,
    session?: SessionInfo,
  ) => {
    try {
      const renameItem = session
        ? await MenuItem.new({
            id: "rename",
            text: t("session.rename"),
            action: async () => {
              await renameSession(session);
            },
          })
        : null;
      const removeItem = await MenuItem.new({
        id: "delete",
        text: t("sidebar.remove"),
        action: async () => onDelete(await getMenuClickPos()),
      });
      const menu = await Menu.new({
        items: renameItem ? [renameItem, removeItem] : [removeItem],
      });
      await menu.popup(
        new LogicalPosition(pos.x + 1, pos.y),
        getCurrentWindow(),
      );
    } catch (err) {
      setError(String(err));
    }
  };

  const openProjectMenu = async (
    projectId: string,
    projectPath: string,
    pos: { x: number; y: number },
  ) => {
    await openDeleteMenu(pos, (clickPos) =>
      setDeleteTarget({
        kind: "project",
        projectId,
        scope: { kind: "project", key: projectPath },
        pos: clickPos,
      }),
    );
  };

  const openSessionMenu = async (session: SessionInfo, pos: { x: number; y: number }) => {
    await openDeleteMenu(
      pos,
      (clickPos) => setDeleteTarget({ kind: "session", session, pos: clickPos }),
      session,
    );
  };

  const detailTitle =
    (selected ? sessionDisplayTitle(selected) : null) ??
    t("list.no_user_message");

  const projectSearchInitialKey = filter.kind === "project" ? filter.key : projects[0]?.path;
  const detailRoute: DetailMode = detailMode;
  const headerContextTitle = selected
    ? detailMode === "threadChat"
      ? { label: t("thread.chat"), icon: MessageSquareText }
      : { label: t("header.chat"), icon: MessageSquareText }
    : selectedThreadId
      ? detailMode === "threadMultiSessionChat"
        ? { label: t("thread.multi_session_chat"), icon: MessagesSquare }
        : { label: t("thread.detail"), icon: Hash }
      : activeProject
        ? { label: t("project.workbench"), icon: Kanban }
        : { label: t("sidebar.new_chat"), icon: MessageSquare };
  const headerEntityTitle = selectedThread
    ? { kind: "thread" as const, title: selectedThread.goal, icon: Goal }
    : activeProject
      ? { kind: "project" as const, title: activeProject.name, icon: Folder }
      : null;
  const headerProjectContext =
    selectedSessionProject ??
    activeThreadProject ??
    activeProject ??
    newChatProject;

  useEffect(() => {
    if (memorySearchOpen && projectSearchInitialKey) {
      setMemorySearchMounted(true);
    }
    if (!projectSearchInitialKey) {
      setMemorySearchMounted(false);
    }
  }, [memorySearchOpen, projectSearchInitialKey]);

  const sidebar = (
    <AppSidebar
      isMac={IS_MAC}
      projectSectionExpanded={expandProject}
      projectGroups={projectGroups}
      expandedProjects={expandedProjects}
      expandedProjectSessions={expandedProjectSessions}
      threadIndexItems={threadIndexItems}
      selectedKey={selectedKey}
      selectedIdentityKey={selectedIdentityKey}
      selectedProjectId={selectedProjectId}
      selectedThreadId={selectedThreadId}
      hasActiveSelection={Boolean(selected || selectedProject || selectedThread)}
      liveState={liveRuntimeState}
      runtimeSessionAliases={runtimeSessionAliases}
      unreadSessionIds={unreadSessionIds}
      update={update}
      onCloseSidebar={() => setSidebarOpen(false)}
      onNewChat={() => {
        setUtilityView(null);
        setSelectedProject(null);
        setSelectedThread(null);
        setNewChatProjectKey(lastSelectedProjectKey);
        setSelected(null);
        setDetailMode("chat");
      }}
      onToggleProjectSection={() => setExpandProject((value) => !value)}
      onProjectAdded={(project) => {
        setUtilityView(null);
        setProjects((prev) => [project, ...prev.filter((p) => p.id !== project.id)]);
        setSelectedProject({ kind: "project", projectId: project.id });
        setSelectedThread(null);
        setSelected(null);
        setFilter({ kind: "project", key: projectFilterKey(project), label: project.name });
        setExpandedProjects((prev) => new Set(prev).add(project.id));
        void refreshSessions();
      }}
      onToggleProjectExpanded={(projectKeyValue) => {
        setExpandedProjects((prev) => {
          const next = new Set(prev);
          if (next.has(projectKeyValue)) next.delete(projectKeyValue);
          else next.add(projectKeyValue);
          return next;
        });
      }}
      onOpenKanban={(projectGroup) => {
        setUtilityView(null);
        setSelected(null);
        setSelectedThread(null);
        setSelectedProject({ kind: "project", projectId: projectGroup.project.id });
        setNewChatProjectKey(null);
        setDetailMode("project");
        setFilter({ kind: "project", key: projectFilterKey(projectGroup.project), label: projectGroup.label });
      }}
      onNewProjectChat={(projectGroup) => {
        setUtilityView(null);
        setSelectedProject(null);
        setSelectedThread(null);
        setNewChatProjectKey(projectGroup.key);
        setSelected(null);
        setDetailMode("chat");
        setFilter({ kind: "project", key: projectFilterKey(projectGroup.project), label: projectGroup.label });
      }}
      onSelectSession={(projectGroup, session) => {
        setUtilityView(null);
        rememberSidebarProject(projectGroup);
        setSelectedProject(null);
        setSelectedThread(null);
        setNewChatProjectKey(null);
        setFilter({ kind: "project", key: projectFilterKey(projectGroup.project), label: projectGroup.label });
        setSelected(session);
        setDetailMode("chat");
      }}
      onSelectThread={(projectGroup, thread, source) => {
        const indexItem = threadIndexItems.find((item) => item.threadId === thread.id);
        if (indexItem) clearThreadUnread(indexItem);
        setUtilityView(null);
        rememberSidebarProject(projectGroup);
        setSelected(null);
        setSelectedProject(null);
        setSelectedThread({ projectId: projectGroup.project.id, threadId: thread.id, goal: thread.goal });
        setNewChatProjectKey(null);
        setDetailMode(source === "threadChat" ? "threadMultiSessionChat" : "project");
        setFilter({ kind: "project", key: projectFilterKey(projectGroup.project), label: projectGroup.label });
      }}
      onToggleProjectSessions={(projectKeyValue) => {
        setExpandedProjectSessions((prev) => {
          const next = new Set(prev);
          if (next.has(projectKeyValue)) next.delete(projectKeyValue);
          else next.add(projectKeyValue);
          return next;
        });
      }}
      onProjectContextMenu={(projectGroup, event) => {
        event.preventDefault();
        void openProjectMenu(
          projectGroup.project.id,
          projectFilterKey(projectGroup.project),
          { x: event.clientX, y: event.clientY },
        );
      }}
      onSessionContextMenu={(session, pos) => {
        void openSessionMenu(session, pos);
      }}
      onOpenSettings={() => setSettingsOpen(true)}
      onOpenAutoTasks={() => setUtilityView("autoTasks")}
      autoTasksActive={utilityView === "autoTasks"}
      appsSectionExpanded={expandApps}
      apps={sessioApps}
      selectedAppPath={selectedApp?.directoryPath ?? null}
      onToggleAppsSection={() => {
        const next = !expandApps;
        setExpandApps(next);
        if (next) void refreshSessioApps();
      }}
      onSelectApp={(app) => {
        setSelectedApp(app);
        setSelected(null);
        setSelectedProject(null);
        setSelectedThread(null);
        setNewChatProjectKey(null);
        setPendingSelectSession(null);
        setFilter({ kind: "all" });
        setDetailMode("chat");
        setUtilityView("apps");
      }}
      appsActive={utilityView === "apps"}
      onInstallUpdate={openUpdateConfirm}
      onError={setError}
    />
  );

  const currentChatView: ChatView = chatView;
  const chatViewToggleVisible =
    (Boolean(selected) && detailMode === "chat") ||
    (Boolean(selectedThreadId) && detailMode === "threadMultiSessionChat");
  const terminalDockVisible = utilityView !== "autoTasks";
  const handleChatViewChange = useCallback(
    (next: ChatView) => {
      setChatView(next);
    },
    [],
  );
  const currentSessionIdentity = selected
    ? sessionIdentityKey(selected)
    : selectedThreadId && detailMode === "threadMultiSessionChat"
      ? selectedThreadId
      : null;
  const activeCanvasKey: CanvasKey | null = selected
    ? { kind: "session", id: selected.id }
    : selectedThreadId && detailMode === "threadMultiSessionChat"
      ? { kind: "thread", id: selectedThreadId }
      : null;
  const currentProjectFileSelection =
    currentSessionIdentity ? projectFileSelectionBySession[currentSessionIdentity] ?? null : null;
  const currentCanvasFileSelection =
    currentSessionIdentity ? canvasFileSelectionBySession[currentSessionIdentity] ?? null : null;
  const handleOpenProjectFile = useCallback(
    (path: string) => {
      const identity = selected
        ? sessionIdentityKey(selected)
        : selectedThreadId && activeThreadProject && detailMode === "threadMultiSessionChat"
          ? selectedThreadId
          : null;
      if (!identity) return;
      if (!selected && detailMode !== "threadMultiSessionChat") return;
      if (selected && detailMode !== "chat" && detailMode !== "threadChat") return;
      if (currentChatView !== "file") setChatView("file");
      setProjectFileSelectionBySession((prev) => {
        const currentSelection = prev[identity];
        return {
          ...prev,
          [identity]: {
            path,
            requestId: (currentSelection?.requestId ?? 0) + 1,
          },
        };
      });
    },
    [activeThreadProject, currentChatView, detailMode, selected, selectedThreadId],
  );
  const handleAddProjectFileToCanvas = useCallback(
    (paths: string[] | string) => {
      const identity = selected
        ? sessionIdentityKey(selected)
        : selectedThreadId && activeThreadProject && detailMode === "threadMultiSessionChat"
          ? selectedThreadId
          : null;
      if (!identity) return;
      if (!selected && detailMode !== "threadMultiSessionChat") return;
      if (selected && detailMode !== "chat" && detailMode !== "threadChat") return;
      const nextPaths = (Array.isArray(paths) ? paths : [paths]).map((path) => path.trim()).filter(Boolean);
      if (nextPaths.length === 0) return;
      if (currentChatView !== "canvas") setChatView("canvas");
      setCanvasFileSelectionBySession((prev) => {
        const currentSelection = prev[identity];
        return {
          ...prev,
          [identity]: {
            paths: nextPaths,
            requestId: (currentSelection?.requestId ?? 0) + 1,
          },
        };
      });
    },
    [activeThreadProject, currentChatView, detailMode, selected, selectedThreadId],
  );

  const header = (
    <AppHeader
      isMac={IS_MAC}
      sidebarOpen={sidebarOpen}
      selected={selected}
      detailTitle={detailTitle}
      contextTitle={headerContextTitle}
      entityTitle={headerEntityTitle}
      projectContext={headerProjectContext}
      activeMessageMeta={activeMessageMeta}
      metaPopoverOpen={metaPopoverOpen}
      rightSidebarOpen={rightSidebarOpen}
      terminalDockOpen={terminalDockOpen}
      terminalDockVisible={terminalDockVisible}
      chatView={currentChatView}
      chatViewVisible={chatViewToggleVisible}
      onOpenSidebar={() => setSidebarOpen(true)}
      onToggleMetaPopover={() => setMetaPopoverOpen((open) => !open)}
      onToggleTerminalDock={() => setTerminalDockOpen((open) => !open)}
      onToggleRightSidebar={() => setRightSidebarOpen((open) => !open)}
      onChatViewChange={handleChatViewChange}
    />
  );

  const autoTasksHeader = (
    <AppHeader
      isMac={IS_MAC}
      sidebarOpen={sidebarOpen}
      selected={null}
      detailTitle=""
      contextTitle={{ label: t("autoTasks.title"), icon: CalendarClock }}
      entityTitle={null}
      projectContext={null}
      activeMessageMeta={null}
      metaPopoverOpen={false}
      rightSidebarOpen={rightSidebarOpen}
      terminalDockOpen={false}
      terminalDockVisible={false}
      onOpenSidebar={() => setSidebarOpen(true)}
      onToggleMetaPopover={() => {}}
      onToggleTerminalDock={() => {}}
      onToggleRightSidebar={() => setRightSidebarOpen((open) => !open)}
    />
  );

  const appsHeader = (
    <AppHeader
      isMac={IS_MAC}
      sidebarOpen={sidebarOpen}
      selected={null}
      detailTitle=""
      contextTitle={null}
      entityTitle={null}
      projectContext={null}
      activeMessageMeta={null}
      metaPopoverOpen={false}
      rightSidebarOpen={rightSidebarOpen}
      terminalDockOpen={terminalDockOpen}
      terminalDockVisible={Boolean(selectedApp)}
      onOpenSidebar={() => setSidebarOpen(true)}
      onToggleMetaPopover={() => {}}
      onToggleTerminalDock={() => setTerminalDockOpen((open) => !open)}
      onToggleRightSidebar={() => setRightSidebarOpen((open) => !open)}
    />
  );

  const overlays = (
    <AppOverlays
      selected={selected}
      metaPopoverMounted={metaPopoverMounted}
      metaPopoverOpen={metaPopoverOpen}
      memorySearchMounted={memorySearchMounted}
      memorySearchOpen={memorySearchOpen}
      projectSearchInitialKey={projectSearchInitialKey}
      memorySearchProjects={projects.map((project) => ({
        key: project.path,
        label: project.name,
      }))}
      activeMemorySearchProjectKey={filter.kind === "project" ? filter.key : null}
      deleteTarget={deleteTarget}
      updateConfirmMounted={updateConfirmMounted}
      updateConfirmOpen={updateConfirmOpen}
      updateCurrentVersion={__APP_VERSION__}
      updateLatestVersion={update.latestVersion}
      updateReleaseNotes={update.releaseNotes}
      updateCanInstall={update.canInstall}
      updateInstalling={update.installing}
      updateReady={update.updateReady}
      updateDownloadedBytes={update.downloadedBytes}
      updateTotalBytes={update.totalBytes}
      onCloseMetaPopover={() => setMetaPopoverOpen(false)}
      onMetaPopoverExited={() => setMetaPopoverMounted(false)}
      onCloseMemorySearch={() => setMemorySearchOpen(false)}
      onMemorySearchExited={() => setMemorySearchMounted(false)}
      onCancelDelete={() => setDeleteTarget(null)}
      onConfirmDelete={() => {
        void confirmDelete();
      }}
      onCancelUpdateConfirm={() => {
        if (!update.installing) setUpdateConfirmOpen(false);
      }}
      onConfirmUpdate={() => {
        void handleInstallUpdate();
      }}
      onUpdateConfirmExited={() => setUpdateConfirmMounted(false)}
    />
  );

  if (settingsOpen) {
    return (
      <div className="relative h-screen text-body">
        <SettingsPage
          lang={lang}
          onLangChange={setLang}
          themeMode={mode}
          onThemeModeChange={setMode}
          rebuilding={rebuilding}
          indexing={indexing}
          onBack={() => setSettingsOpen(false)}
          onError={setError}
          onRebuildFinished={refreshMemoryBackend}
          appVersion={__APP_VERSION__}
          update={update}
          onOpenUpdate={openUpdateConfirm}
          onOpenAppshotPermissions={async () => {
            await openAppshotPermissionsPanel();
          }}
        />
        {updateConfirmMounted && update.latestVersion && (
          <UpdateConfirmDialog
            open={updateConfirmOpen}
            currentVersion={__APP_VERSION__}
            latestVersion={update.latestVersion}
            releaseNotes={update.releaseNotes}
            canInstall={update.canInstall}
            installing={update.installing}
            updateReady={update.updateReady}
            downloadedBytes={update.downloadedBytes}
            totalBytes={update.totalBytes}
            onCancel={() => {
              if (!update.installing) setUpdateConfirmOpen(false);
            }}
            onConfirm={() => {
              void handleInstallUpdate();
            }}
            onExited={() => setUpdateConfirmMounted(false)}
          />
        )}
        <ToastStack message={error} onMessageConsumed={() => setError(null)} />
        <ToastStack message={toast} onMessageConsumed={() => setToast(null)} />
      </div>
    );
  }

  return (
    <div className="relative h-screen">
      <AppLayout
        sidebar={sidebar}
        header={
          utilityView === "autoTasks"
            ? autoTasksHeader
            : utilityView === "apps"
              ? appsHeader
              : header
        }
        sidebarOpen={sidebarOpen}
        rightSidebar={
          <AppRightSidebar
            selectedThread={selectedThread}
            selectedSessionProject={selectedSessionProject}
            selectedThreadProject={activeThreadProject}
            activeApp={utilityView === "apps" ? selectedApp : null}
            open={rightSidebarOpen}
            isCanvasViewActive={currentChatView === "canvas"}
            activeCanvasKey={activeCanvasKey}
            liveState={liveRuntimeState}
            filesReloadKey={rightSidebarFilesReloadKey}
            projectGitRepos={projectGitRepos}
            onProjectGitRepoDetected={handleProjectGitRepoDetected}
            onSelectThreadChatSession={(session) => {
              setSelectedProject(null);
              setSelectedThread(null);
              setSelected(session);
              setDetailMode("threadChat");
            }}
            onOpenThreadMultiSessionChat={() => setDetailMode("threadMultiSessionChat")}
            onOpenProjectFile={handleOpenProjectFile}
            onAddProjectFileToCanvas={handleAddProjectFileToCanvas}
            onClose={() => setRightSidebarOpen(false)}
            onError={setError}
          />
        }
        rightSidebarOpen={rightSidebarOpen}
        overlays={overlays}
      >
        {utilityView === "autoTasks" ? (
          <AutoTasksPage onError={setError} />
        ) : utilityView === "apps" && selectedApp ? (
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
            <AppsPage
              key={selectedApp.directoryPath}
              app={selectedApp}
              runtimeSessionId={appRuntimeSessions[selectedApp.id] ?? null}
              onRuntimeSessionIdChange={(runtimeSessionId) => {
                setAppRuntimeSessions((current) => ({
                  ...current,
                  [selectedApp.id]: runtimeSessionId,
                }));
              }}
              runtimeAgents={runtimeAgents}
              lastRuntimeAgentSelection={lastRuntimeAgentSelection}
              rememberRuntimeAgentSelection={rememberRuntimeAgentSelection}
              liveState={liveRuntimeState}
              dispatchLiveEvent={dispatchLiveRuntimeEvent}
              onError={setError}
            />
            <TerminalDock
              open={terminalDockOpen}
              defaultCwd={selectedApp?.directoryPath ?? "~"}
              onOpenChange={setTerminalDockOpen}
            />
          </div>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
            <AppMain
              activeProject={activeProject ?? activeThreadProject}
              selectedThreadId={selectedThreadId}
              selected={selected}
              selectedSessionProject={selectedSessionProject}
              detailRoute={detailRoute}
              viewMode={viewMode}
              chatView={currentChatView}
              filesSubview={filesSubview}
              onFilesSubviewChange={setFilesSubview}
              projectFilesReloadKey={rightSidebarFilesReloadKey}
              projectGitRepos={projectGitRepos}
              onProjectGitRepoDetected={handleProjectGitRepoDetected}
              selectedProjectFileRequest={currentProjectFileSelection}
              selectedCanvasFileRequest={currentCanvasFileSelection}
              onOpenProjectFile={handleOpenProjectFile}
              onAddProjectFileToCanvas={handleAddProjectFileToCanvas}
              liveState={liveRuntimeState}
              runtimeAgents={runtimeAgents}
              lastRuntimeAgentSelection={lastRuntimeAgentSelection}
              rememberRuntimeAgentSelection={rememberRuntimeAgentSelection}
              debugAcpConfig={debugAcpConfig}
              runtimeSessionAliases={runtimeSessionAliases}
              selectedAncestorSessions={selectedAncestorSessions}
              newChatProjectKey={newChatProjectKey}
              pendingNewChats={pendingNewChats}
              setNewChatProjectKey={setNewChatProjectKey}
              projectGroups={projectGroups}
              availableSessions={availableSessions}
              dispatchLiveEvent={dispatchLiveRuntimeEvent}
              setProjects={setProjects}
              setFilter={setFilter}
              setSelectedProject={setSelectedProject}
              setSelectedThread={setSelectedThread}
              setSelected={setSelected}
              setDetailMode={setDetailMode}
              setPendingNewChats={setPendingNewChats}
              refreshSessions={refreshSessions}
              onMessageCount={handleMessageCount}
              onActiveMessageMeta={handleActiveMessageMeta}
              onError={setError}
            />
            <TerminalDock
              open={terminalDockOpen}
              defaultCwd={
                activeThreadProject?.path ??
                activeProject?.path ??
                selectedSessionProject?.path ??
                "~"
              }
              onOpenChange={setTerminalDockOpen}
            />
          </div>
        )}
      </AppLayout>
      <ToastStack message={error} onMessageConsumed={() => setError(null)} />
      <ToastStack message={toast} onMessageConsumed={() => setToast(null)} />
    </div>
  );
}
