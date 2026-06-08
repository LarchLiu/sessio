import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
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
  getDebugConfig,
  listThreadChatSummaries,
  refreshThreadChatSummaries,
  SessionInfo,
  removeSessionsByScope,
  removeSessionFiles,
  updateSessionRenameTitle,
  type SessionScope,
  type ThreadChatSummaryInfo,
} from "./api";
import { syncTrayMenu, type TrayRecentEntry } from "./tray";
import AppLayout from "./layouts/AppLayout";
import { type ActiveMessageMeta } from "./pages/ChatPage";
import AppHeader from "./components/AppHeader";
import AppMain from "./components/AppMain";
import AppOverlays, { type DeleteTarget } from "./components/AppOverlays";
import AppSidebar from "./components/AppSidebar";
import MemoryBackendMissingButton from "./components/MemoryBackendMissingButton";
import ToastStack from "./components/ToastStack";
import UpdateConfirmDialog from "./components/UpdateConfirmDialog";
import SettingsPage from "./pages/SettingsPage";
import { useAppData } from "./hooks/useAppData";
import { usePendingNewChats } from "./hooks/usePendingNewChats";
import { usePlanTaskRuntimeCompletion } from "./hooks/usePlanTaskRuntimeCompletion";
import { useProjectGroups } from "./hooks/useProjectGroups";
import { useRuntimeEventSubscription } from "./hooks/useRuntimeEventSubscription";
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
import { Folder, Goal, Hash, Kanban, MessagesSquare, MessageSquare, MessageSquareText } from "lucide-react";
import type { DetailMode, PendingNewChatSession, ViewMode } from "./navigation";
import {
  isSubagentOnly,
  matchesScope,
  mergeRuntimeSessionAliases,
  projectFilterKey,
  scopeForFilter,
  sessionDisplayTitle,
  sessionIdentityKey,
  sessionKey,
  type Filter,
  type ProjectSelection,
} from "./appUtils";

const VIEW_MODE_STORAGE_KEY = "sessio.viewMode";

type ThreadSelection = { projectId: string; threadId: string; goal: string } | null;

function readViewMode(): ViewMode {
  if (typeof localStorage === "undefined") return "native";
  const v = localStorage.getItem(VIEW_MODE_STORAGE_KEY);
  return v === "cross" ? "cross" : "native";
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
  const {
    sessions,
    setSessions,
    projects,
    setProjects,
    indexPhase,
    memoryBackendStatus,
    refreshSessions,
    refreshMemoryBackend,
  } = useAppData({ setError });
  const [filter, setFilter] = useState<Filter>({ kind: "all" });
  const [selectedProject, setSelectedProject] = useState<ProjectSelection>(null);
  const [selectedThread, setSelectedThread] = useState<ThreadSelection>(null);
  const [selected, setSelected] = useState<SessionInfo | null>(null);
  const [newChatProjectKey, setNewChatProjectKey] = useState<string | null>(null);
  const [expandProject, setExpandProject] = useState(true);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [memorySearchOpen, setMemorySearchOpen] = useState(false);
  const [memorySearchMounted, setMemorySearchMounted] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [updateConfirmOpen, setUpdateConfirmOpen] = useState(false);
  const [updateConfirmMounted, setUpdateConfirmMounted] = useState(false);
  const [viewMode] = useState<ViewMode>(() => readViewMode());
  const [detailMode, setDetailMode] = useState<DetailMode>("chat");
  const [metaPopoverOpen, setMetaPopoverOpen] = useState(false);
  const [metaPopoverMounted, setMetaPopoverMounted] = useState(false);
  const [activeMessageMeta, setActiveMessageMeta] =
    useState<ActiveMessageMeta | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<DeleteTarget | null>(null);
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
  const [threadChatSummaries, setThreadChatSummaries] = useState<ThreadChatSummaryInfo[]>([]);
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

  useSelectedSessionSync({
    availableSessions,
    selected,
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

  const refreshThreadSummaries = useCallback((projectId?: string | null) => {
    return refreshThreadChatSummaries(projectId).then((rows) => {
      setThreadChatSummaries((prev) => {
        if (!projectId) return rows;
        const next = prev.filter((summary) => summary.projectId !== projectId);
        return [...next, ...rows].sort((a, b) => b.time - a.time);
      });
    });
  }, []);

  useEffect(() => {
    if (projects.length === 0) {
      setThreadChatSummaries([]);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      listThreadChatSummaries()
        .then((rows) => {
          if (!cancelled) setThreadChatSummaries(rows);
        })
        .catch((err) => {
          if (!cancelled) console.warn("load thread chat summaries failed", err);
        });
    };
    refresh();
    const unlistenThreads = listen<{
      projectId?: string | null;
      threadId?: string | null;
    }>("threads_updated", (event) => {
      const projectId = event.payload?.projectId ?? null;
      refreshThreadSummaries(projectId).catch((err) => {
        if (!cancelled) console.warn("refresh thread chat summaries failed", err);
      });
    });
    const unlistenSessions = listen("sessions_index_updated", () => {
      refreshThreadSummaries(null).catch((err) => {
        if (!cancelled) console.warn("refresh thread chat summaries failed", err);
      });
    });
    return () => {
      cancelled = true;
      unlistenThreads.then((f) => f()).catch(() => {});
      unlistenSessions.then((f) => f()).catch(() => {});
    };
  }, [projects.length, refreshThreadSummaries]);

  const recentForMenu = useMemo<TrayRecentEntry[]>(() => {
    const linkedSessionKeys = new Set<string>();
    const entries: TrayRecentEntry[] = [];
    for (const summary of threadChatSummaries) {
      for (const key of summary.sessionKeys) linkedSessionKeys.add(key);
      entries.push({
        kind: "thread",
        thread: summary,
        sessions: summary.sessions,
        time: summary.time,
      });
    }
    for (const session of availableSessions) {
      if (isSubagentOnly(session)) continue;
      if (linkedSessionKeys.has(sessionIdentityKey(session))) continue;
      entries.push({
        kind: "session",
        session,
        time: session.updatedAt ?? session.startedAt ?? 0,
      });
    }
    return entries.sort((a, b) => b.time - a.time).slice(0, 5);
  }, [availableSessions, threadChatSummaries]);

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
    syncTrayMenu(recentForMenu, {
      show: t("menubar.show"),
      quit: t("menubar.quit"),
      noSessions: t("menubar.no_sessions"),
      noMessage: t("list.no_user_message"),
      resumeCommand: t("menubar.resume_command"),
      crossCommand: t("menubar.cross_command"),
      crossPromptPlaceholder: t("list.cross_prompt_placeholder"),
      updateAvailable: t("menubar.update_available"),
      updateInstalling: t("sidebar.update_installing"),
    }, systemAppearance, {
      hasUpdate: update.hasUpdate,
      latestVersion: update.latestVersion,
      installing: update.installing,
      install: openUpdateConfirm,
    });
  }, [
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

  const openScopeMenu = async (scope: SessionScope, pos: { x: number; y: number }) => {
    await openDeleteMenu(pos, (clickPos) => setDeleteTarget({ kind: "scope", scope, pos: clickPos }));
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

  const memoryBackendMissing =
    memoryBackendStatus !== null && memoryBackendStatus.available === false;
  const projectSearchInitialKey = filter.kind === "project" ? filter.key : projects[0]?.path;
  const detailRoute: DetailMode = detailMode;
  const headerContextTitle = selected
    ? { label: t("header.chat"), icon: MessageSquareText }
    : selectedThreadId
      ? detailMode === "threadMultiSessionChat" || detailMode === "threadChat"
        ? { label: t("thread.multi_session_chat"), icon: MessagesSquare }
        : { label: t("thread.detail"), icon: Hash }
      : activeProject
        ? { label: t("project.workbench"), icon: Kanban }
        : { label: t("sidebar.new_chat"), icon: MessageSquare };
  const headerEntityTitle = selectedThread
    ? { kind: "thread" as const, title: selectedThread.goal, icon: Goal }
    : activeProject
      ? { kind: "project" as const, title: activeProject.name, icon: Folder, pill: activeProject.workflowId }
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
        setSelectedProject(null);
        setSelectedThread(null);
        setNewChatProjectKey(null);
        setSelected(null);
        setDetailMode("chat");
      }}
      onToggleProjectSection={() => setExpandProject((value) => !value)}
      onProjectAdded={(project) => {
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
        setSelected(null);
        setSelectedThread(null);
        setSelectedProject({ kind: "project", projectId: projectGroup.project.id });
        setNewChatProjectKey(null);
        setDetailMode("project");
        setFilter({ kind: "project", key: projectFilterKey(projectGroup.project), label: projectGroup.label });
      }}
      onNewProjectChat={(projectGroup) => {
        setSelectedProject(null);
        setSelectedThread(null);
        setNewChatProjectKey(projectGroup.key);
        setSelected(null);
        setDetailMode("chat");
        setFilter({ kind: "project", key: projectFilterKey(projectGroup.project), label: projectGroup.label });
      }}
      onSelectSession={(projectGroup, session) => {
        setSelectedProject(null);
        setSelectedThread(null);
        setNewChatProjectKey(null);
        setFilter({ kind: "project", key: projectFilterKey(projectGroup.project), label: projectGroup.label });
        setSelected(session);
        setDetailMode("chat");
      }}
      onSelectThread={(projectGroup, thread, source) => {
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
        void openScopeMenu(
          scopeForFilter({ kind: "project", key: projectFilterKey(projectGroup.project), label: projectGroup.label }),
          { x: event.clientX, y: event.clientY },
        );
      }}
      onSessionContextMenu={(session, pos) => {
        void openSessionMenu(session, pos);
      }}
      onOpenSettings={() => setSettingsOpen(true)}
      onInstallUpdate={openUpdateConfirm}
      onError={setError}
    />
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
      memoryBackendStatus={memoryBackendStatus}
      memoryBackendMissing={memoryBackendMissing}
      projectCount={projectGroups.length}
      onOpenSidebar={() => setSidebarOpen(true)}
      onToggleMetaPopover={() => setMetaPopoverOpen((open) => !open)}
      onOpenSearch={() => setMemorySearchOpen(true)}
      onRefreshMemoryBackend={refreshMemoryBackend}
      MemoryBackendMissingButton={MemoryBackendMissingButton}
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
      </div>
    );
  }

  return (
    <div className="relative h-screen">
      <AppLayout
        sidebar={sidebar}
        header={header}
        sidebarOpen={sidebarOpen}
        overlays={overlays}
      >
        <AppMain
          activeProject={activeProject ?? activeThreadProject}
          selectedThreadId={selectedThreadId}
          selected={selected}
          selectedSessionProject={selectedSessionProject}
          detailRoute={detailRoute}
          viewMode={viewMode}
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
      </AppLayout>
      <ToastStack message={error} onMessageConsumed={() => setError(null)} />
    </div>
  );
}
