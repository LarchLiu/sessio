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
  ProjectInfo,
  KanbanItem,
  createPendingSession,
  getIndexStatus,
  getDebugConfig,
  IndexPhase,
  getMemoryBackendStatus,
  MemoryBackendStatus,
  linkKanbanItemSession,
  SessionInfo,
  listProjects,
  listSessions,
  removeSessionsByScope,
  removeSessionFiles,
  type SessionScope,
  updateKanbanItemStatus,
} from "./api";
import { syncTrayMenu } from "./tray";
import AppLayout from "./layouts/AppLayout";
import ChatPage, { type ActiveMessageMeta } from "./pages/ChatPage";
import NewChatPage from "./pages/NewChatPage";
import { ProjectWorkbenchPage } from "./pages/ProjectPage";
import AppHeader from "./components/AppHeader";
import AppOverlays, { type DeleteTarget } from "./components/AppOverlays";
import AppSidebar from "./components/AppSidebar";
import MemoryBackendMissingButton from "./components/MemoryBackendMissingButton";
import { useProjectGroups } from "./hooks/useProjectGroups";
import { useSessionAncestors } from "./hooks/useSessionAncestors";
import { useTheme } from "./theme";
import { useI18n } from "./i18n";
import { useUpdateCheck } from "./updater";
import {
  applyRuntimeAction,
  emptyLiveRuntimeState,
  normalizeAgentRuntimeEvent,
} from "./runtimeChat";
import { useRuntimeAgents } from "./runtimeAgents";
import type { DetailMode, PendingNewChatSession, ViewMode } from "./navigation";
import {
  addUnreadKeys,
  deleteUnreadKeys,
  intersectsSet,
  isSubagentOnly,
  matchesScope,
  mergePendingSession,
  messageCountKey,
  projectFilterKey,
  runtimeEventUnreadKeys,
  scopeForFilter,
  sessionIdentityKey,
  sessionKey,
  sessionUnreadKeys,
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

export default function App() {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [indexPhase, setIndexPhase] = useState<IndexPhase>("indexing");
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>({ kind: "all" });
  const [selectedProject, setSelectedProject] = useState<ProjectSelection>(null);
  const [selected, setSelected] = useState<SessionInfo | null>(null);
  const [newChatProjectKey, setNewChatProjectKey] = useState<string | null>(null);
  const [expandProject, setExpandProject] = useState(true);
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
  const [debugAcpConfig, setDebugAcpConfig] = useState(false);
  const update = useUpdateCheck(__APP_VERSION__);
  const messageCountBySourceRef = useRef<Map<string, number>>(new Map());
  const selectedUnreadKeysRef = useRef<Set<string>>(new Set());
  const runtimeSessionAliasesRef = useRef<Record<string, string>>({});
  const sessionsLoadedRef = useRef(false);
  const pendingNewChatWritesRef = useRef<Set<string>>(new Set());
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

  const refreshProjects = useCallback(() => {
    return listProjects()
      .then(setProjects)
      .catch((err) => {
        setError(String(err));
      });
  }, []);

  const refreshSessions = useCallback(() => {
    return listSessions()
      .then(setSessions)
      .catch((err) => {
        setError(String(err));
      });
  }, []);

  useEffect(() => {
    runtimeSessionAliasesRef.current = runtimeSessionAliases;
  }, [runtimeSessionAliases]);

  useEffect(() => {
    const previous = messageCountBySourceRef.current;
    const next = new Map<string, number>();
    const changedSessions = new Map<string, SessionInfo>();
    for (const session of sessions) {
      const mainKey = messageCountKey(session.agent, session.filePath, session.id);
      next.set(mainKey, session.messageCount);
      const previousMainCount = previous.get(mainKey);
      if (
        sessionsLoadedRef.current &&
        previousMainCount !== undefined &&
        session.messageCount > previousMainCount
      ) {
        changedSessions.set(sessionIdentityKey(session), session);
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
          changedSessions.set(sessionIdentityKey(session), session);
        }
      }
    }
    messageCountBySourceRef.current = next;
    sessionsLoadedRef.current = true;
    if (changedSessions.size > 0) {
      const selectedKeys = new Set(
        selected ? sessionUnreadKeys(selected, runtimeSessionAliases) : [],
      );
      setUnreadSessionIds((prev) => {
        let next = prev;
        for (const session of changedSessions.values()) {
          const keys = sessionUnreadKeys(session, runtimeSessionAliases);
          if (intersectsSet(keys, selectedKeys)) continue;
          next = addUnreadKeys(next, keys);
        }
        return next;
      });
    }
  }, [runtimeSessionAliases, selected, sessions]);

  useEffect(() => {
    const selectedKeys = selected
      ? sessionUnreadKeys(selected, runtimeSessionAliases)
      : [];
    selectedUnreadKeysRef.current = new Set(selectedKeys);
    if (!selected) return;
    setUnreadSessionIds((prev) => {
      return deleteUnreadKeys(prev, selectedKeys);
    });
  }, [runtimeSessionAliases, selected]);

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
    Promise.all([listSessions(), listProjects()])
      .then(([sessionRows, projectRows]) => {
        if (cancelled) return;
        setSessions(sessionRows);
        setProjects(projectRows);
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
      void refreshSessions();
      void refreshProjects();
      refreshMemoryBackendStatus(setMemoryBackendStatus);
    });
    const projectsUnlisten = listen("projects_updated", () => {
      void refreshProjects();
      void refreshSessions();
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
      projectsUnlisten.then((f) => f()).catch(() => {});
      statusUnlisten.then((f) => f()).catch(() => {});
    };
  }, [refreshProjects, refreshSessions]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<unknown>("agent-runtime-event", (event) => {
      if (cancelled) return;
      const payload = normalizeAgentRuntimeEvent(event.payload);
      console.info("[sessio-runtime:frontend:event]", payload);
      const unreadKeys = runtimeEventUnreadKeys(
        payload,
        runtimeSessionAliasesRef.current,
      );
      if (
        !intersectsSet(unreadKeys, selectedUnreadKeysRef.current) &&
        payload.kind !== "sessionEnded"
      ) {
        setUnreadSessionIds((prev) => {
          return addUnreadKeys(prev, unreadKeys);
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
    const project = projects.find((item) => item.path === next.projectPath);
    if (project) {
      setSelectedProject(null);
      setFilter({ kind: "project", key: projectFilterKey(project), label: project.name });
    }
    setExpandedProjects((prev) => {
      const expanded = new Set(prev);
      if (project) expanded.add(project.id);
      return expanded;
    });
    setPendingSelectSession(null);
  }, [availableSessions, pendingSelectSession, projects]);

  useEffect(() => {
    for (const pending of Object.values(pendingNewChats)) {
      const liveSession = liveRuntimeState.sessions[pending.sessioRuntimeSessionId];
      if (!liveSession) continue;
      const agentSessionId = liveSession.agentRuntimeSessionId;
      if (
        !agentSessionId ||
        agentSessionId === "pending" ||
        agentSessionId.startsWith("fake-agent-session")
      ) {
        continue;
      }
      if (pendingNewChatWritesRef.current.has(pending.sessioRuntimeSessionId)) continue;

      pendingNewChatWritesRef.current.add(pending.sessioRuntimeSessionId);
      const pendingSession: SessionInfo = {
        id: agentSessionId,
        agent: pending.agent,
        forkedFromAgent: pending.forkedFromAgent ?? null,
        forkedFromId: pending.forkedFromId ?? null,
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
      };
      createPendingSession(pendingSession)
        .then(async () => {
          let linkedKanbanItem: KanbanItem | null = null;
          if (pending.kanbanItemId) {
            linkedKanbanItem = await linkKanbanItemSession(
              pending.kanbanItemId,
              pending.agent,
              agentSessionId,
            );
            if (pending.kanbanItemStatus === "todo") {
              linkedKanbanItem = await updateKanbanItemStatus(
                pending.kanbanItemId,
                "in_progress",
              );
            }
          }
          setRuntimeSessionAliases((prev) => ({
            ...prev,
            [`${pending.agent}:${agentSessionId}`]: pending.sessioRuntimeSessionId,
          }));
          setSessions((prev) => mergePendingSession(prev, pendingSession));
          if (linkedKanbanItem) {
            setSelectedProject((current) => (current ? { ...current } : current));
          }
          setSelected(pendingSession);
          setDetailMode("chat");
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

  const projectWorkbenchProps = (project: ProjectInfo) => ({
    project,
    sessions: availableSessions.filter((session) => session.projectPath === project.path),
    runtimeAgents,
    debugAcpConfig,
    liveState: liveRuntimeState,
    dispatchLiveEvent: dispatchLiveRuntimeEvent,
    onProjectUpdated: (updatedProject: ProjectInfo) => {
      setProjects((prev) => prev.map((item) => (item.id === updatedProject.id ? updatedProject : item)));
      setFilter({ kind: "project", key: projectFilterKey(updatedProject), label: updatedProject.name });
    },
    onProjectArchived: (projectId: string) => {
      setProjects((prev) => prev.filter((item) => item.id !== projectId));
      setSelectedProject(null);
      setFilter({ kind: "all" });
      void refreshSessions();
    },
    onSelectSession: (session: SessionInfo) => {
      setSelectedProject(null);
      setSelected(session);
      setDetailMode("chat");
    },
    onPendingSession: (pending: PendingNewChatSession) => {
      setPendingNewChats((prev) => ({
        ...prev,
        [pending.sessioRuntimeSessionId]: pending,
      }));
    },
    onChatStarted: () => {
      setSelectedProject(null);
      setDetailMode("chat");
    },
    onError: setError,
  });

  const sidebar = (
    <AppSidebar
      isMac={IS_MAC}
      sidebarOpen={sidebarOpen}
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
      lang={lang}
      themeMode={mode}
      update={update}
      rebuilding={rebuilding}
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
      onLangChange={setLang}
      onThemeModeChange={setMode}
      onError={setError}
      onRebuildFinished={() => refreshMemoryBackendStatus(setMemoryBackendStatus)}
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
      onRefreshMemoryBackend={() => refreshMemoryBackendStatus(setMemoryBackendStatus)}
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

  const mainContent = (
    <>
        {error ? (
          <div className="m-5 p-3 rounded bg-status-error/10 text-status-error text-body-sm">
            {error}
          </div>
        ) : activeProject ? (
          <ProjectWorkbenchPage {...projectWorkbenchProps(activeProject)} />
        ) : selected ? (
          <div className="relative flex-1 min-h-0">
            <div
              className={
                "absolute inset-0 " +
                (detailRoute === "chat" ? "visible" : "invisible pointer-events-none")
              }
              aria-hidden={detailRoute !== "chat"}
            >
              <ChatPage
                session={selected}
                viewMode={viewMode}
                liveState={liveRuntimeState}
                runtimeAgents={runtimeAgents}
                debugAcpConfig={debugAcpConfig}
                runtimeSessionAliases={runtimeSessionAliases}
                ancestorSessions={selectedAncestorSessions}
                dispatchLiveEvent={dispatchLiveRuntimeEvent}
                onPendingSession={(pending) => {
                  setPendingNewChats((prev) => ({
                    ...prev,
                    [pending.sessioRuntimeSessionId]: pending,
                  }));
                }}
                onMessageCount={handleMessageCount}
                onActiveMessageMeta={handleActiveMessageMeta}
              />
            </div>
            <div
              className={
                "absolute inset-0 " +
                (detailRoute === "project" ? "visible" : "invisible pointer-events-none")
              }
              aria-hidden={detailRoute !== "project"}
            >
              {selectedSessionProject ? (
                <ProjectWorkbenchPage {...projectWorkbenchProps(selectedSessionProject)} />
              ) : (
                <div className="flex h-full min-h-0 items-center justify-center bg-surface-panel px-6 text-body-sm text-ink/45">
                  No project is linked to this session.
                </div>
              )}
            </div>
          </div>
        ) : (
          <NewChatPage
            projects={projectGroups}
            initialProjectKey={newChatProjectKey}
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
    </>
  );

  return (
    <AppLayout sidebar={sidebar} header={header} overlays={overlays}>
      {mainContent}
    </AppLayout>
  );
}
