import {
  ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Search, PanelLeftClose, PanelLeftOpen, Folder, FolderOpen, Sun, Moon, Monitor, ChevronDown, RefreshCw, Settings, X, BotMessageSquare, Download, Skull } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { Menu } from "@tauri-apps/api/menu/menu";
import { MenuItem } from "@tauri-apps/api/menu/menuItem";
import { cursorPosition, getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalPosition } from "@tauri-apps/api/dpi";
import {
  AGENT_LABEL,
  Agent,
  getIndexStatus,
  IndexPhase,
  getMemoryBackendStatus,
  MemoryBackendStatus,
  ProjectMemorySearchResult,
  SessionInfo,
  rebuildSessionIndex,
  listSessions,
  removeSessionsByScope,
  removeSessionFiles,
  searchProjectMemory,
  type SessionScope,
} from "./api";
import { syncTrayMenu } from "./tray";
import SessionDetail from "./components/SessionDetail";
import { AgentBadge, AgentGlyph } from "./components/AgentIcon";
import ScrollArea from "./components/ScrollArea";
import ConfirmPopover from "./components/ConfirmPopover";
import InlineMenuSelect, { type InlineMenuSelectOption } from "./components/InlineMenuSelect";
import Tooltip from "./components/Tooltip";
import WindowControls from "./components/WindowControls";
import { ThemeMode, useTheme } from "./theme";
import { Lang, useI18n } from "./i18n";
import { useUpdateCheck, openReleasePage } from "./updater";

type Filter =
  | SessionScope
  | { kind: "project"; key: string; label: string };

function scopeForFilter(filter: Filter): SessionScope {
  if (filter.kind === "project") return { kind: "project", key: filter.key };
  return filter;
}

export type ViewMode = "native" | "cross";

const VIEW_MODE_STORAGE_KEY = "sessio.viewMode";

function readViewMode(): ViewMode {
  if (typeof localStorage === "undefined") return "native";
  const v = localStorage.getItem(VIEW_MODE_STORAGE_KEY);
  return v === "cross" ? "cross" : "native";
}

const AGENT_ORDER: Agent[] = ["codex", "claude", "gemini"];
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

type DeleteTarget =
  | { kind: "session"; session: SessionInfo; pos: { x: number; y: number } }
  | { kind: "scope"; scope: SessionScope; pos: { x: number; y: number } };

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
  const [expandAgent, setExpandAgent] = useState(true);
  const [expandProject, setExpandProject] = useState(true);
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(
    () => new Set(),
  );
  const [expandedProjectSessions, setExpandedProjectSessions] = useState<Set<string>>(
    () => new Set(),
  );
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [memoryBackendStatus, setMemoryBackendStatus] =
    useState<MemoryBackendStatus | null>(null);
  const [memorySearchOpen, setMemorySearchOpen] = useState(false);
  const [memorySearchMounted, setMemorySearchMounted] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>(() => readViewMode());
  const [deleteTarget, setDeleteTarget] = useState<DeleteTarget | null>(null);
  const { mode, setMode } = useTheme();
  const { lang, setLang, t } = useI18n();
  const update = useUpdateCheck(__APP_VERSION__);
  const listScrollRef = useRef<HTMLDivElement>(null);
  const indexing = indexPhase !== "idle";
  const rebuilding = indexPhase === "rebuilding";

  const availableSessions = useMemo(
    () => sessions.filter((s) => s.available),
    [sessions]
  );

  useEffect(() => {
    listScrollRef.current?.scrollTo(0, 0);
  }, [filter]);

  useEffect(() => {
    if (!sidebarOpen) setSettingsOpen(false);
  }, [sidebarOpen]);

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
    if (!selected) return;
    const next = availableSessions.find((s) => sessionKey(s) === sessionKey(selected));
    if (!next) {
      setSelected(null);
      return;
    }
    if (next !== selected) {
      setSelected(next);
    }
  }, [availableSessions, selected]);

  useEffect(() => {
    if (selected || availableSessions.length === 0) return;
    const next = availableSessions.find((s) => !isSubagentOnly(s));
    if (next) setSelected(next);
  }, [availableSessions, selected]);

  const agentStats = useMemo(() => {
    const m: Record<Agent, { count: number; latest: number }> = {
      codex: { count: 0, latest: 0 },
      claude: { count: 0, latest: 0 },
      gemini: { count: 0, latest: 0 },
    };
    for (const s of availableSessions) {
      if (isSubagentOnly(s)) continue;
      m[s.agent].count += 1;
      const t = s.updatedAt ?? s.startedAt ?? 0;
      if (t > m[s.agent].latest) m[s.agent].latest = t;
    }
    return m;
  }, [availableSessions]);

  const agentOrdered = useMemo(
    () =>
      AGENT_ORDER.map((a) => ({ agent: a, ...agentStats[a] })).sort(
        (x, y) => y.latest - x.latest
      ),
    [agentStats]
  );

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
        sessions: v.sessions.sort(
          (a, b) =>
            (b.updatedAt ?? b.startedAt ?? 0) -
            (a.updatedAt ?? a.startedAt ?? 0),
        ),
      }))
      .sort((a, b) => b.latest - a.latest || a.label.localeCompare(b.label));
  }, [availableSessions, t]);

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

  const totalRealSessions = useMemo(
    () => availableSessions.filter((s) => !isSubagentOnly(s)).length,
    [availableSessions]
  );

  const recentForMenu = useMemo(
    () => availableSessions.filter((s) => !isSubagentOnly(s)).slice(0, 5),
    [availableSessions]
  );

  const selectedKey = selected ? sessionKey(selected) : null;

  useEffect(() => {
    syncTrayMenu(recentForMenu, {
      show: t("menubar.show"),
      quit: t("menubar.quit"),
      noSessions: t("menubar.no_sessions"),
      noMessage: t("list.no_user_message"),
      resumeCommand: t("menubar.resume_command"),
      crossCommand: t("menubar.cross_command"),
      crossPromptPlaceholder: t("list.cross_prompt_placeholder"),
    });
  }, [recentForMenu, t]);

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

  const visible = useMemo(() => {
    return availableSessions.filter((s) => {
      if (filter.kind === "agent" && s.agent !== filter.agent) return false;
      if (filter.kind === "project" && projectKey(s) !== filter.key) return false;
      return true;
    });
  }, [availableSessions, filter]);

  const visibleCount = useMemo(
    () => visible.filter((s) => !isSubagentOnly(s)).length,
    [visible]
  );

  const headerLabel =
    filter.kind === "all"
      ? t("sidebar.all_sessions")
      : filter.kind === "agent"
        ? AGENT_LABEL[filter.agent]
        : "label" in filter
          ? filter.label
          : filter.key;
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
          <div className="shrink-0 flex flex-col gap-0.5">
            <SidebarItem
              label={t("sidebar.all_sessions")}
              count={totalRealSessions}
              active={filter.kind === "all"}
              onClick={() => setFilter({ kind: "all" })}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    void openScopeMenu(
                      { kind: "all" },
                      { x: e.clientX, y: e.clientY },
                    );
                  }}
              icon={<BotMessageSquare className="w-3.5 h-3.5 shrink-0 text-ink/55" />}
            />

            <SectionHeader
              label={t("sidebar.by_agent")}
              collapsed={!expandAgent}
              onToggle={() => setExpandAgent((v) => !v)}
            />
            {expandAgent &&
              agentOrdered.map(({ agent, count }) => (
                <SidebarItem
                  key={agent}
                  label={AGENT_LABEL[agent]}
                  count={count}
                  active={filter.kind === "agent" && filter.agent === agent}
                  onClick={() => setFilter({ kind: "agent", agent })}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    void openScopeMenu(
                      { kind: "agent", agent },
                      { x: e.clientX, y: e.clientY },
                    );
                  }}
                  icon={<AgentBadge agent={agent} className="w-3.5 h-3.5" />}
                />
              ))}

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

        <div className="relative w-64 border-t border-ink/10">
          <div
            className={
              "absolute left-0 bottom-full w-64 overflow-hidden transition-all duration-250 ease-out " +
              (settingsOpen
                ? "translate-y-0 opacity-100 max-h-80 pointer-events-auto"
                : "translate-y-2 opacity-0 max-h-0 pointer-events-none")
            }
          >
            <div className="border-t border-ink/10 bg-surface shadow-[0_20px_60px_rgba(0,0,0,0.18)] backdrop-blur">
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
                  <LanguageSwitcher lang={lang} onChange={setLang} />
                </div>
                <div className="flex items-center justify-between gap-3">
                  <span className="text-body-sm text-ink/55">{t("sidebar.theme")}</span>
                  <ThemeSwitcher mode={mode} onChange={setMode} />
                </div>
                <div className="flex items-center justify-between gap-3 text-body-sm text-ink/55">
                  <span>{t("sidebar.rebuild_index")}</span>
                  <button
                    type="button"
                    aria-label={t("sidebar.rebuild_index")}
                    onClick={() => {
                      rebuildSessionIndex().catch((err) => {
                        setError(String(err));
                      }).finally(() => {
                        refreshMemoryBackendStatus(setMemoryBackendStatus);
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
                        setError(String(err));
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
            className={
              "flex items-center gap-2 min-w-0 pointer-events-none " +
              (sidebarOpen ? "" : IS_MAC ? "pl-[112px] " : "pl-9 ")
            }
          >
            {!sidebarOpen && (
              <>
                {filter.kind === "all" && (
                  <BotMessageSquare className="w-5 h-5 shrink-0 text-ink/55" />
                )}
                {filter.kind === "agent" && (
                  <AgentBadge agent={filter.agent} className="w-5 h-5" />
                )}
                {filter.kind === "project" && (
                  <Folder className="w-5 h-5 shrink-0 text-ink/55" />
                )}
                <div className="text-title font-medium truncate">{headerLabel}</div>
              </>
            )}
            <span className="text-ink/40 text-body-sm tabular-nums shrink-0">
              {t("header.sessions_count", { count: visibleCount })}
            </span>
          </div>
          <div
            data-tauri-drag-region="false"
            className="justify-self-center"
          >
            <ViewModeSwitcher mode={viewMode} onChange={setViewMode} />
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

        {error ? (
          <div className="m-5 p-3 rounded bg-status-error/10 text-status-error text-body-sm">
            {error}
          </div>
        ) : selected ? (
          <SessionDetail
            session={selected}
            viewMode={viewMode}
            onRemoved={() => setSelected(null)}
          />
        ) : (
          <ScrollArea ref={listScrollRef} className="flex-1 min-h-0">
            <EmptyDetailState
              indexing={indexing}
              visibleCount={visibleCount}
            />
          </ScrollArea>
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

function SidebarItem({
  label,
  count,
  active,
  onClick,
  onContextMenu,
  icon,
  title,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  icon?: ReactNode;
  title?: string;
}) {
  return (
    <button
      onClick={onClick}
      onContextMenu={onContextMenu}
      title={title}
      className={
        "flex items-center gap-2 px-2.5 py-1.5 rounded-md text-left text-body transition " +
        (active
          ? "bg-ink/10 text-ink"
          : "text-ink/70 hover:bg-ink/5 hover:text-ink")
      }
    >
      {icon !== undefined ? (
        icon
      ) : (
        <span className="w-2 shrink-0" />
      )}
      <span className="flex-1 truncate">{label}</span>
      <span className="text-meta text-ink/40 tabular-nums">{count}</span>
    </button>
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
            {visibleSessions.map((session) => (
              <SidebarSessionItem
                key={sessionKey(session)}
                item={session}
                active={selectedKey === sessionKey(session)}
                onSelect={() => onSelectSession(session)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  onSessionContextMenu(session, { x: e.clientX, y: e.clientY });
                }}
              />
            ))}
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
  onSelect,
  onContextMenu,
}: {
  item: SessionInfo;
  active: boolean;
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
        "group flex w-full items-center gap-2 rounded-md py-1.5 pl-7 pr-2 text-left transition " +
        (active
          ? "bg-ink/10 text-ink"
          : "text-ink/65 hover:bg-ink/5 hover:text-ink")
      }
    >
      <AgentGlyph agent={item.agent} className="h-3.5 w-3.5 shrink-0" />
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

function EmptyDetailState({
  indexing,
  visibleCount,
}: {
  indexing: boolean;
  visibleCount: number;
}) {
  const { t } = useI18n();
  return (
    <div className="flex min-h-full items-center justify-center p-10 text-center text-body text-ink/40">
      {indexing ? (
        <div className="flex items-center gap-2">
          <StatusDot ripple />
          <span>{t("sidebar.status_indexing")}</span>
        </div>
      ) : visibleCount === 0 ? (
        t("list.empty")
      ) : (
        t("list.select_session")
      )}
    </div>
  );
}

function ViewModeSwitcher({
  mode,
  onChange,
}: {
  mode: ViewMode;
  onChange: (m: ViewMode) => void;
}) {
  const { t } = useI18n();
  const items: { value: ViewMode; label: string }[] = [
    { value: "native", label: t("header.mode_native") },
    { value: "cross", label: t("header.mode_cross") },
  ];
  const activeIndex = Math.max(
    0,
    items.findIndex((it) => it.value === mode),
  );
  const BTN_W = 72;
  return (
    <div className="relative flex items-center rounded-md bg-ink/[0.14] p-0.5">
      <div
        aria-hidden
        className="absolute top-0.5 left-0.5 h-[26px] rounded bg-surface shadow-[0_1px_2px_rgba(0,0,0,0.18)] transition-transform duration-200 ease-out"
        style={{
          width: `${BTN_W}px`,
          transform: `translateX(${activeIndex * BTN_W}px)`,
        }}
      />
      {items.map(({ value, label }) => {
        const active = mode === value;
        return (
          <button
            key={value}
            type="button"
            onClick={() => onChange(value)}
            data-tauri-drag-region="false"
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
