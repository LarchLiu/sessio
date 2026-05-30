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
  SessionInfo,
  removeSessionsByScope,
  removeSessionFiles,
  type SessionScope,
} from "./api";
import { syncTrayMenu } from "./tray";
import AppLayout from "./layouts/AppLayout";
import { type ActiveMessageMeta } from "./pages/ChatPage";
import AppHeader from "./components/AppHeader";
import AppMain from "./components/AppMain";
import AppOverlays, { type DeleteTarget } from "./components/AppOverlays";
import AppSidebar from "./components/AppSidebar";
import MemoryBackendMissingButton from "./components/MemoryBackendMissingButton";
import SettingsPage from "./pages/SettingsPage";
import { useAppData } from "./hooks/useAppData";
import { usePendingNewChats } from "./hooks/usePendingNewChats";
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
import type { DetailMode, PendingNewChatSession, ViewMode } from "./navigation";
import {
  isSubagentOnly,
  matchesScope,
  projectFilterKey,
  scopeForFilter,
  sessionIdentityKey,
  sessionKey,
  type Filter,
  type ProjectSelection,
} from "./appUtils";

const VIEW_MODE_STORAGE_KEY = "sessio.viewMode";

function readViewMode(): ViewMode {
  if (typeof localStorage === "undefined") return "native";
  const v = localStorage.getItem(VIEW_MODE_STORAGE_KEY);
  return v === "cross" ? "cross" : "native";
}

const IS_MAC =
  typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);

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
  const [selected, setSelected] = useState<SessionInfo | null>(null);
  const [newChatProjectKey, setNewChatProjectKey] = useState<string | null>(null);
  const [expandProject, setExpandProject] = useState(true);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [memorySearchOpen, setMemorySearchOpen] = useState(false);
  const [memorySearchMounted, setMemorySearchMounted] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
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
  const { mode, setMode } = useTheme();
  const [systemAppearance, setSystemAppearance] = useState<"light" | "dark">("dark");
  const { lang, setLang, t } = useI18n();
  const { agents: runtimeAgents } = useRuntimeAgents();
  const [debugAcpConfig, setDebugAcpConfig] = useState(false);
  const update = useUpdateCheck(__APP_VERSION__);
  const indexing = indexPhase !== "idle";
  const rebuilding = indexPhase === "rebuilding";

  const availableSessions = useMemo(
    () => sessions.filter((s) => s.available),
    [sessions]
  );

  useEffect(() => {
    let cancelled = false;
    getDebugConfig()
      .then((config) => {
        if (!cancelled) setDebugAcpConfig(config.acpConfig);
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
    setSelected,
    setDetailMode,
    setPendingSelectSession,
    setPendingNewChats,
    setError,
  });

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
    setExpandedProjects,
    setPendingSelectSession,
  });

  const recentForMenu = useMemo(
    () => availableSessions.filter((s) => !isSubagentOnly(s)).slice(0, 5),
    [availableSessions]
  );

  useEffect(() => {
    if (!selectedProject) return;
    if (projects.some((project) => project.id === selectedProject.projectId)) return;
    setSelectedProject(null);
  }, [projects, selectedProject]);

  const activeProject = selectedProject
    ? projects.find((project) => project.id === selectedProject.projectId) ?? null
    : null;
  const selectedSessionProject =
    selected?.projectPath
      ? projects.find((project) => project.path === selected.projectPath) ?? null
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
  const projectSearchInitialKey = filter.kind === "project" ? filter.key : projects[0]?.path;
  const detailRoute: DetailMode = detailMode;

  useEffect(() => {
    if (memorySearchOpen && projectSearchInitialKey) {
      setMemorySearchMounted(true);
    }
    if (!projectSearchInitialKey) {
      setMemorySearchMounted(false);
    }
  }, [memorySearchOpen, projectSearchInitialKey]);

  const handleDetailModeChange = (mode: DetailMode) => {
    setDetailMode(mode);
    if (mode === "chat") {
      setSelectedProject(null);
      return;
    }
    if (selectedSessionProject) {
      setSelectedProject({ kind: "project", projectId: selectedSessionProject.id });
      setFilter({
        kind: "project",
        key: projectFilterKey(selectedSessionProject),
        label: selectedSessionProject.name,
      });
    }
  };

  const sidebar = (
    <AppSidebar
      isMac={IS_MAC}
      projectSectionExpanded={expandProject}
      projectGroups={projectGroups}
      expandedProjects={expandedProjects}
      expandedProjectSessions={expandedProjectSessions}
      selectedKey={selectedKey}
      selectedIdentityKey={selectedIdentityKey}
      hasSelectedSession={Boolean(selected)}
      liveState={liveRuntimeState}
      runtimeSessionAliases={runtimeSessionAliases}
      unreadSessionIds={unreadSessionIds}
      update={update}
      indexing={indexing}
      onCloseSidebar={() => setSidebarOpen(false)}
      onNewChat={() => {
        setSelectedProject(null);
        setNewChatProjectKey(null);
        setSelected(null);
        setDetailMode("chat");
      }}
      onToggleProjectSection={() => setExpandProject((value) => !value)}
      onProjectAdded={(project) => {
        setProjects((prev) => [project, ...prev.filter((p) => p.id !== project.id)]);
        setSelectedProject({ kind: "project", projectId: project.id });
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
        setSelectedProject({ kind: "project", projectId: projectGroup.project.id });
        setNewChatProjectKey(null);
        setDetailMode("project");
        setFilter({ kind: "project", key: projectFilterKey(projectGroup.project), label: projectGroup.label });
      }}
      onNewProjectChat={(projectGroup) => {
        setSelectedProject(null);
        setNewChatProjectKey(projectGroup.key);
        setSelected(null);
        setDetailMode("chat");
        setFilter({ kind: "project", key: projectFilterKey(projectGroup.project), label: projectGroup.label });
      }}
      onSelectSession={(projectGroup, session) => {
        setSelectedProject(null);
        setNewChatProjectKey(null);
        setFilter({ kind: "project", key: projectFilterKey(projectGroup.project), label: projectGroup.label });
        setSelected(session);
        setDetailMode("chat");
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
      onError={setError}
    />
  );

  const header = (
    <AppHeader
      isMac={IS_MAC}
      sidebarOpen={sidebarOpen}
      selected={selected}
      detailTitle={detailTitle}
      detailMode={detailMode}
      showDetailTabs={Boolean(selected)}
      activeMessageMeta={activeMessageMeta}
      metaPopoverOpen={metaPopoverOpen}
      memoryBackendStatus={memoryBackendStatus}
      memoryBackendMissing={memoryBackendMissing}
      projectCount={projectGroups.length}
      onOpenSidebar={() => setSidebarOpen(true)}
      onDetailModeChange={handleDetailModeChange}
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
      onCloseMetaPopover={() => setMetaPopoverOpen(false)}
      onMetaPopoverExited={() => setMetaPopoverMounted(false)}
      onCloseMemorySearch={() => setMemorySearchOpen(false)}
      onMemorySearchExited={() => setMemorySearchMounted(false)}
      onCancelDelete={() => setDeleteTarget(null)}
      onConfirmDelete={() => {
        void confirmDelete();
      }}
    />
  );

  if (settingsOpen) {
    return (
      <div className="h-screen text-body">
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
        />
      </div>
    );
  }

  return (
    <AppLayout
      sidebar={sidebar}
      header={header}
      sidebarOpen={sidebarOpen}
      onSidebarOpenChange={setSidebarOpen}
      overlays={overlays}
    >
      <AppMain
        error={error}
        activeProject={activeProject}
        selected={selected}
        selectedSessionProject={selectedSessionProject}
        detailRoute={detailRoute}
        viewMode={viewMode}
        liveState={liveRuntimeState}
        runtimeAgents={runtimeAgents}
        debugAcpConfig={debugAcpConfig}
        runtimeSessionAliases={runtimeSessionAliases}
        selectedAncestorSessions={selectedAncestorSessions}
        newChatProjectKey={newChatProjectKey}
        projectGroups={projectGroups}
        availableSessions={availableSessions}
        dispatchLiveEvent={dispatchLiveRuntimeEvent}
        setProjects={setProjects}
        setFilter={setFilter}
        setSelectedProject={setSelectedProject}
        setSelected={setSelected}
        setDetailMode={setDetailMode}
        setPendingNewChats={setPendingNewChats}
        refreshSessions={refreshSessions}
        onMessageCount={handleMessageCount}
        onActiveMessageMeta={handleActiveMessageMeta}
        onError={setError}
      />
    </AppLayout>
  );
}
