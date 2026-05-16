import { ReactNode, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { Search, PanelLeftClose, PanelLeftOpen, Folder, Sun, Moon, Monitor, ChevronDown, RefreshCw, Settings, X, BotMessageSquare } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import {
  AGENT_LABEL,
  Agent,
  getIndexStatus,
  getSessionMessages,
  SessionInfo,
  SessionMessage,
  rebuildSessionIndex,
  listSessions,
  writeCrossPrompt,
} from "./api";
import SessionDetail from "./components/SessionDetail";
import { AgentBadge, AgentGlyph } from "./components/AgentIcon";
import ScrollArea from "./components/ScrollArea";
import Tag from "./components/Tag";
import Tooltip from "./components/Tooltip";
import { ThemeMode, useTheme } from "./theme";
import { Lang, localeTag, useI18n } from "./i18n";

type Filter =
  | { kind: "all" }
  | { kind: "agent"; agent: Agent }
  | { kind: "project"; key: string; label: string };

export type ViewMode = "native" | "cross";

const VIEW_MODE_STORAGE_KEY = "sessio.viewMode";

function readViewMode(): ViewMode {
  if (typeof localStorage === "undefined") return "native";
  const v = localStorage.getItem(VIEW_MODE_STORAGE_KEY);
  return v === "cross" ? "cross" : "native";
}

const AGENT_ORDER: Agent[] = ["codex", "claude", "gemini"];

const RESUME_CMD: Record<Agent, (id: string) => string> = {
  codex: (id) => `codex resume ${id}`,
  claude: (id) => `claude --resume ${id}`,
  gemini: (id) => `gemini --resume ${id}`,
};

const CROSS_PROMPT_MAX = 16 * 1024;

const IS_WIN =
  typeof navigator !== "undefined" && /Win/i.test(navigator.platform);

function bashQuote(s: string): string {
  return `'${s.replace(/'/g, "'\\''")}'`;
}

function pwshQuote(s: string): string {
  return `'${s.replace(/'/g, "''")}'`;
}

function buildCrossPrompt(messages: SessionMessage[]): string {
  const filtered = messages.filter(
    (m) => m.role === "user" || m.role === "thinking" || m.role === "assistant",
  );
  if (filtered.length === 0) return "";
  const SEP = "\n\n";
  const formatted = filtered.map((m) => `[${m.role}]\n${m.text}`);
  let size = 0;
  let startIdx = filtered.length;
  for (let i = filtered.length - 1; i >= 0; i--) {
    const extra = formatted[i].length + (i === filtered.length - 1 ? 0 : SEP.length);
    if (size + extra > CROSS_PROMPT_MAX) break;
    size += extra;
    startIdx = i;
  }
  while (startIdx < filtered.length && filtered[startIdx].role !== "user") {
    startIdx++;
  }
  if (startIdx >= filtered.length) return "";
  const header =
    `\n\n# Continued session from agent\n` +
    `The dialogue below is the recent context of an in-progress session ` +
    `(oldest → latest). Pick up from the last turn and continue helping ` +
    `the user.\n\n`;
  return header + formatted.slice(startIdx).join(SEP);
}

function buildCrossCommand(
  targetAgent: Agent,
  filePath: string,
  placeholder: string,
): string {
  if (IS_WIN) {
    return `${targetAgent} "<${placeholder}>$(Get-Content -Raw ${pwshQuote(filePath)})"`;
  }
  return `${targetAgent} "<${placeholder}>$(cat ${bashQuote(filePath)})"`;
}

const IS_MAC =
  typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);

function projectKey(s: SessionInfo): string {
  return s.projectPath ?? `__unknown__:${s.agent}`;
}

// Orphan main session that only exists to carry subagents (Claude cleaned
// the main jsonl, no index entry either). Don't count it as a "real" session
// but still show it in the list so subagents stay reachable.
function isSubagentOnly(s: SessionInfo): boolean {
  return s.archived && s.messageCount === 0 && s.subagents.length > 0;
}

export default function App() {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [indexing, setIndexing] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>({ kind: "all" });
  const [selected, setSelected] = useState<SessionInfo | null>(null);
  const [expandAgent, setExpandAgent] = useState(true);
  const [expandProject, setExpandProject] = useState(true);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>(() => readViewMode());
  const deferredViewMode = useDeferredValue(viewMode);
  const { mode, setMode } = useTheme();
  const { lang, setLang, t } = useI18n();
  const listScrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    listScrollRef.current?.scrollTo(0, 0);
  }, [filter]);

  useEffect(() => {
    if (!sidebarOpen) setSettingsOpen(false);
  }, [sidebarOpen]);

  useEffect(() => {
    localStorage.setItem(VIEW_MODE_STORAGE_KEY, viewMode);
  }, [viewMode]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    listSessions()
      .then((rows) => {
        if (cancelled) return;
        setSessions(rows);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(String(err));
      })
      .finally(() => {
        if (cancelled) return;
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    getIndexStatus()
      .then((status) => {
        setIndexing(status.indexing);
        if (status.lastError) setError(status.lastError);
      })
      .catch(() => {});

    const unlisten = listen("sessions_index_updated", () => {
      setIndexing(false);
      listSessions()
        .then(setSessions)
        .catch(() => {});
    });
    const statusUnlisten = listen("sessions_index_status", (event) => {
      const payload = event.payload as { indexing?: boolean; lastError?: string | null };
      if (typeof payload.indexing === "boolean") setIndexing(payload.indexing);
      if (payload.lastError !== undefined) setError(payload.lastError);
    });
    return () => {
      unlisten.then((f) => f()).catch(() => {});
      statusUnlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  const agentStats = useMemo(() => {
    const m: Record<Agent, { count: number; latest: number }> = {
      codex: { count: 0, latest: 0 },
      claude: { count: 0, latest: 0 },
      gemini: { count: 0, latest: 0 },
    };
    for (const s of sessions) {
      if (isSubagentOnly(s)) continue;
      m[s.agent].count += 1;
      const t = s.updatedAt ?? s.startedAt ?? 0;
      if (t > m[s.agent].latest) m[s.agent].latest = t;
    }
    return m;
  }, [sessions]);

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
      { label: string; count: number; path: string | null; latest: number }
    >();
    const unknown = t("list.unknown_project");
    for (const s of sessions) {
      if (isSubagentOnly(s)) continue;
      const key = projectKey(s);
      const ts = s.updatedAt ?? s.startedAt ?? 0;
      const e = m.get(key);
      if (e) {
        e.count += 1;
        if (ts > e.latest) e.latest = ts;
      } else {
        m.set(key, {
          label: s.projectName ?? s.projectPath ?? unknown,
          count: 1,
          path: s.projectPath,
          latest: ts,
        });
      }
    }
    return [...m.entries()]
      .map(([key, v]) => ({ key, ...v }))
      .sort((a, b) => b.latest - a.latest || a.label.localeCompare(b.label));
  }, [sessions, t]);

  const totalRealSessions = useMemo(
    () => sessions.filter((s) => !isSubagentOnly(s)).length,
    [sessions]
  );

  const visible = useMemo(() => {
    return sessions.filter((s) => {
      if (filter.kind === "agent" && s.agent !== filter.agent) return false;
      if (filter.kind === "project" && projectKey(s) !== filter.key) return false;
      return true;
    });
  }, [sessions, filter]);

  const visibleCount = useMemo(
    () => visible.filter((s) => !isSubagentOnly(s)).length,
    [visible]
  );

  const headerLabel =
    filter.kind === "all"
      ? t("sidebar.all_sessions")
      : filter.kind === "agent"
        ? AGENT_LABEL[filter.agent]
        : filter.label;

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
              viewportClassName="pr-3 flex flex-col gap-0.5"
            >
              {projectGroups.map((p) => (
                <SidebarItem
                  key={p.key}
                  label={p.label}
                  count={p.count}
                  active={filter.kind === "project" && filter.key === p.key}
                  onClick={() =>
                    setFilter({ kind: "project", key: p.key, label: p.label })
                  }
                  icon={<Folder className="w-3.5 h-3.5 shrink-0 text-ink/55" />}
                  title={p.path ?? p.label}
                />
              ))}
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
                      setIndexing(true);
                      rebuildSessionIndex().catch((err) => {
                        setError(String(err));
                        setIndexing(false);
                      });
                    }}
                    className="p-1 -m-1 text-ink/55 transition hover:text-ink rounded-md"
                  >
                    <RefreshCw className={"w-4 h-4 shrink-0 " + (indexing ? "animate-spin" : "")} />
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
            </div>
            <span className="text-meta text-ink/30 truncate text-right">
              {indexing
                ? t("sidebar.indexing")
                : loading
                  ? t("sidebar.loading")
                  : ""}
            </span>
          </div>
        </div>
      </aside>

      <main className="flex-1 flex flex-col min-w-0">
        <div
          data-tauri-drag-region
          className="relative h-12 shrink-0 grid grid-cols-3 items-center px-5 bg-surface border-b border-ink/10 select-none"
        >
          <Tooltip content={t("sidebar.open")} placement="bottom">
            <button
              type="button"
              aria-label={t("sidebar.open")}
              data-tauri-drag-region="false"
              onClick={() => setSidebarOpen(true)}
              className={
                "absolute top-1/2 -translate-y-1/2 p-1 text-ink/55 hover:text-ink rounded-md transition-opacity duration-300 " +
                (IS_MAC ? "left-24 " : "left-3 ") +
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
          <div className="justify-self-end">
            <Tooltip content={t("header.search")} placement="bottom">
              <button
                type="button"
                aria-label={t("header.search")}
                data-tauri-drag-region="false"
                className="p-1 text-ink/55 hover:text-ink transition rounded-md"
              >
                <Search className="w-4 h-4" />
              </button>
            </Tooltip>
          </div>
        </div>

        <ScrollArea ref={listScrollRef} className="flex-1 min-h-0">
          {deferredViewMode === "native" ? (
            <NativeSessionList
              visible={visible}
              filter={filter}
              error={error}
              loading={loading}
              indexing={indexing}
              onSelect={setSelected}
            />
          ) : (
            <CrossSessionList
              visible={visible}
              filter={filter}
              error={error}
              loading={loading}
              indexing={indexing}
              onSelect={setSelected}
            />
          )}
        </ScrollArea>
      </main>

      {selected && (
        <SessionDetail
          session={selected}
          viewMode={viewMode}
          onClose={() => setSelected(null)}
        />
      )}
    </div>
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
  icon,
  title,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
  icon?: ReactNode;
  title?: string;
}) {
  return (
    <button
      onClick={onClick}
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

function NativeSessionList({
  visible,
  filter,
  error,
  loading,
  indexing,
  onSelect,
}: {
  visible: SessionInfo[];
  filter: Filter;
  error: string | null;
  loading: boolean;
  indexing: boolean;
  onSelect: (s: SessionInfo) => void;
}) {
  const { t } = useI18n();
  return (
    <>
      {error && (
        <div className="m-5 p-3 rounded bg-status-error/10 text-status-error text-body-sm">
          {error}
        </div>
      )}
      {!error && !loading && !indexing && visible.length === 0 && (
        <div className="p-10 text-center text-ink/40 text-body">
          {t("list.empty")}
        </div>
      )}
      <ul className="divide-y divide-ink/5">
        {visible.map((s) => (
          <li
            key={`${s.agent}:${s.filePath}:${s.id}`}
            className="px-5 py-3.5 hover:bg-ink/[0.03] transition"
          >
            <SessionRow
              item={s}
              filter={filter}
              onOpenDetail={() => onSelect(s)}
            />
          </li>
        ))}
      </ul>
    </>
  );
}

function CrossSessionList({
  visible,
  filter,
  error,
  loading,
  indexing,
  onSelect,
}: {
  visible: SessionInfo[];
  filter: Filter;
  error: string | null;
  loading: boolean;
  indexing: boolean;
  onSelect: (s: SessionInfo) => void;
}) {
  const { t } = useI18n();
  return (
    <>
      {error && (
        <div className="m-5 p-3 rounded bg-status-error/10 text-status-error text-body-sm">
          {error}
        </div>
      )}
      {!error && !loading && !indexing && visible.length === 0 && (
        <div className="p-10 text-center text-ink/40 text-body">
          {t("list.empty")}
        </div>
      )}
      <ul className="divide-y divide-ink/5">
        {visible.map((s) => (
          <li
            key={`${s.agent}:${s.filePath}:${s.id}`}
            className="px-5 py-3.5 hover:bg-ink/[0.03] transition"
          >
            <SessionRow
              item={s}
              filter={filter}
              showOtherAgents
              onOpenDetail={() => onSelect(s)}
            />
          </li>
        ))}
      </ul>
    </>
  );
}

function SessionRow({
  item,
  filter,
  showOtherAgents,
  onOpenDetail,
}: {
  item: SessionInfo;
  filter: Filter;
  showOtherAgents?: boolean;
  onOpenDetail?: () => void;
}) {
  const { lang, t } = useI18n();
  const subCount = item.subagents.length;
  return (
    <div className="min-w-0">
      <div
        onClick={onOpenDetail}
        className={
          "pl-4 text-body line-clamp-3 cursor-pointer" +
          (item.archived ? " text-ink/55" : " text-ink/90")
        }
      >
        {item.firstUserMessage ?? (
          <span className="text-ink/30">{t("list.no_user_message")}</span>
        )}
      </div>
      <div className="pl-4 mt-1.5 flex items-center gap-2 text-meta text-ink/40 leading-none">
        {filter.kind !== "project" && (
          <>
            <Folder className="w-3.5 h-3.5 shrink-0" />
            <span className="font-medium truncate text-ink/55">
              {item.projectName ?? item.projectPath ?? t("list.unknown_project")}
            </span>
            <MetaDivider />
          </>
        )}
        <span className="shrink-0">
          {formatTime(item.updatedAt ?? item.startedAt, localeTag(lang))}
        </span>
        <span className="shrink-0">·</span>
        <span className="shrink-0">
          {item.partial && !item.archived ? "~" : ""}
          {t("list.msgs", { count: item.messageCount })}
        </span>
        {subCount > 0 && (
          <Tag
            label={t("list.subagent_count", {
              count: subCount,
              s: subCount > 1 ? "s" : "",
            })}
            color="var(--color-accent-purple)"
            title={t("list.subagent_tooltip", {
              count: subCount,
              s: subCount > 1 ? "s" : "",
            })}
          />
        )}
        {item.archived && (
          <Tag
            label={t("list.archived")}
            color="var(--color-muted)"
            title={t(
              item.available
                ? "list.archived_tooltip_by_user"
                : "list.archived_tooltip"
            )}
          />
        )}
        <MetaDivider />
        {showOtherAgents
          ? AGENT_ORDER.filter((a) => a !== item.agent).map((a) => (
              <CrossAgentButton key={a} item={item} targetAgent={a} />
            ))
          : <ResumeAgentButton item={item} />}
      </div>
    </div>
  );
}

function MetaDivider() {
  return <span aria-hidden className="shrink-0 w-px h-3 bg-ink/15" />;
}

function CrossAgentButton({
  item,
  targetAgent,
}: {
  item: SessionInfo;
  targetAgent: Agent;
}) {
  const { t } = useI18n();
  const [state, setState] = useState<"idle" | "loading" | "copied" | "error">(
    "idle",
  );
  const timerRef = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (timerRef.current) window.clearTimeout(timerRef.current);
    },
    [],
  );

  const handleClick = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (state === "loading") return;
    setState("loading");
    try {
      const messages = await getSessionMessages(item.agent, item.filePath, item.id);
      const prompt = buildCrossPrompt(messages);
      if (!prompt) {
        setState("error");
        if (timerRef.current) window.clearTimeout(timerRef.current);
        timerRef.current = window.setTimeout(() => setState("idle"), 1500);
        return;
      }
      const path = await writeCrossPrompt(item.id, prompt);
      await navigator.clipboard.writeText(
        buildCrossCommand(targetAgent, path, t("list.cross_prompt_placeholder")),
      );
      setState("copied");
      if (timerRef.current) window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => setState("idle"), 1500);
    } catch (err) {
      console.error("cross copy failed", err);
      setState("error");
      if (timerRef.current) window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => setState("idle"), 1500);
    }
  };

  const tipText =
    state === "loading"
      ? t("list.copying")
      : state === "copied"
        ? IS_WIN
          ? t("list.copied_powershell")
          : t("list.copied")
        : state === "error"
          ? t("list.copy_failed")
          : t("list.copy_cross_to", { agent: AGENT_LABEL[targetAgent] });

  return (
    <Tooltip content={tipText} placement="top">
      <button
        type="button"
        onClick={handleClick}
        disabled={state === "loading"}
        aria-label={t("list.copy_cross_to", { agent: AGENT_LABEL[targetAgent] })}
        className="appearance-none p-0 bg-transparent border-0 rounded transition hover:opacity-70 disabled:opacity-50"
      >
        <Tag
          label={AGENT_LABEL[targetAgent]}
          color={`var(--color-agent-${targetAgent})`}
          icon={<AgentGlyph agent={targetAgent} className="w-3.5 h-3.5 shrink-0" />}
        />
      </button>
    </Tooltip>
  );
}

function ResumeAgentButton({ item }: { item: SessionInfo }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (timerRef.current) window.clearTimeout(timerRef.current);
    },
    [],
  );

  const handleClick = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(RESUME_CMD[item.agent](item.id));
      setCopied(true);
      if (timerRef.current) window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => setCopied(false), 1500);
    } catch (err) {
      console.error("clipboard write failed", err);
    }
  };

  return (
    <Tooltip
      content={copied ? t("list.copied") : t("list.copy_resume")}
      placement="top"
    >
      <button
        type="button"
        onClick={handleClick}
        aria-label={t("list.copy_resume")}
        className="appearance-none p-0 bg-transparent border-0 rounded transition hover:opacity-70"
      >
        <Tag
          label={AGENT_LABEL[item.agent]}
          color={`var(--color-agent-${item.agent})`}
          icon={<AgentGlyph agent={item.agent} className="w-3.5 h-3.5 shrink-0" />}
        />
      </button>
    </Tooltip>
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

function formatTime(ts: number | null, locale: string): string {
  if (!ts) return "—";
  const d = new Date(ts);
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) {
    return d.toLocaleTimeString(locale, {
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  return d.toLocaleDateString(locale, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}
