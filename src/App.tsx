import { ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { Search, PanelLeftClose, PanelLeftOpen, Bot, Folder, Sun, Moon, Monitor, ChevronDown } from "lucide-react";
import {
  AGENT_ACCENT,
  AGENT_LABEL,
  Agent,
  SessionInfo,
  listSessions,
} from "./api";
import SessionDetail from "./components/SessionDetail";
import ScrollArea from "./components/ScrollArea";
import Tag from "./components/Tag";
import Tooltip from "./components/Tooltip";
import { ThemeMode, useTheme } from "./theme";
import { Lang, localeTag, useI18n } from "./i18n";

type Filter =
  | { kind: "all" }
  | { kind: "agent"; agent: Agent }
  | { kind: "project"; key: string; label: string };

const AGENT_ORDER: Agent[] = ["codex", "claude", "gemini"];

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
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>({ kind: "all" });
  const [selected, setSelected] = useState<SessionInfo | null>(null);
  const [expandAgent, setExpandAgent] = useState(true);
  const [expandProject, setExpandProject] = useState(true);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const { mode, setMode } = useTheme();
  const { lang, setLang, t } = useI18n();
  const listScrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    listScrollRef.current?.scrollTo(0, 0);
  }, [filter]);

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
              dot="rgb(var(--color-fg) / 0.4)"
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
                  dot={AGENT_ACCENT[agent]}
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

        <div className="w-64 px-3 py-2 flex items-center justify-between gap-2 border-t border-ink/5">
          <span className="text-meta text-ink/30 truncate">
            {loading
              ? t("sidebar.loading")
              : t("sidebar.sessions_count", { count: totalRealSessions })}
          </span>
          <div className="shrink-0 flex items-center gap-1">
            <LanguageSwitcher lang={lang} onChange={setLang} />
            <ThemeSwitcher mode={mode} onChange={setMode} />
          </div>
        </div>
      </aside>

      <main className="flex-1 flex flex-col min-w-0">
        <div
          data-tauri-drag-region
          className="relative h-12 shrink-0 flex items-center justify-center px-5 bg-surface border-b border-ink/10"
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
          <div className="flex items-center gap-2 min-w-0">
            {!sidebarOpen && (
              <>
                {filter.kind === "agent" && (
                  <Bot
                    className="w-6 h-6 shrink-0"
                    style={{ color: AGENT_ACCENT[filter.agent] }}
                  />
                )}
                {filter.kind === "project" && (
                  <Folder className="w-5 h-5 shrink-0 text-ink/55" />
                )}
                <h1 className="text-title font-medium truncate">{headerLabel}</h1>
              </>
            )}
            <span className="text-ink/40 text-body-sm tabular-nums">
              {t("header.sessions_count", { count: visibleCount })}
            </span>
          </div>
          <Tooltip content={t("header.search")} placement="bottom">
            <button
              type="button"
              aria-label={t("header.search")}
              data-tauri-drag-region="false"
              className="absolute right-5 top-1/2 -translate-y-1/2 p-1 text-ink/55 hover:text-ink transition rounded-md"
            >
              <Search className="w-4 h-4" />
            </button>
          </Tooltip>
        </div>

        <ScrollArea ref={listScrollRef} className="flex-1 min-h-0">
          {error && (
            <div className="m-5 p-3 rounded bg-status-error/10 text-status-error text-body-sm">
              {error}
            </div>
          )}
          {!error && !loading && visible.length === 0 && (
            <div className="p-10 text-center text-ink/40 text-body">
              {t("list.empty")}
            </div>
          )}
          <ul className="divide-y divide-ink/5">
            {visible.map((s) => (
              <li
                key={`${s.agent}:${s.filePath}:${s.id}`}
                onClick={() => setSelected(s)}
                className="px-5 py-3.5 cursor-pointer hover:bg-ink/[0.03] transition"
              >
                <SessionRow item={s} filter={filter} />
              </li>
            ))}
          </ul>
        </ScrollArea>
      </main>

      {selected && (
        <SessionDetail
          session={selected}
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
  dot,
  icon,
  title,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
  dot?: string;
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
      ) : dot !== undefined ? (
        <span
          className="w-2 h-2 rounded-full shrink-0"
          style={{ background: dot }}
        />
      ) : (
        <span className="w-2 shrink-0" />
      )}
      <span className="flex-1 truncate">{label}</span>
      <span className="text-meta text-ink/40 tabular-nums">{count}</span>
    </button>
  );
}

function SessionRow({ item, filter }: { item: SessionInfo, filter: Filter }) {
  const { lang, t } = useI18n();
  const subCount = item.subagents.length;
  return (
    <div className="min-w-0">
      <div className={"pl-4 text-body line-clamp-3" + (item.archived ? " text-ink/55" : " text-ink/90")}>
        {item.firstUserMessage ?? (
          <span className="text-ink/30">{t("list.no_user_message")}</span>
        )}
      </div>
      <div className="pl-4 mt-1.5 flex items-center gap-2 text-meta text-ink/40">
      {filter.kind !== "project" && (
        <>
        <Folder
          className="w-3.5 h-3.5 shrink-0"
        />
        <span
          className="text-body font-medium truncate text-ink/55"
        >
          {item.projectName ?? item.projectPath ?? t("list.unknown_project")}
        </span>
        </>
      )}
      {filter.kind !== "agent" && (
        <Tag
          label={AGENT_LABEL[item.agent]}
          color={`var(--color-agent-${item.agent})`}
        />
      )}
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
        <span>{formatTime(item.updatedAt ?? item.startedAt, localeTag(lang))}</span>
        <span>·</span>
        <span>
          {item.partial && !item.archived ? "~" : ""}
          {t("list.msgs", { count: item.messageCount })}
        </span>
      </div>
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
