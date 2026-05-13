import { useEffect, useMemo, useState } from "react";
import { Search, PanelLeftClose, PanelLeftOpen, Bot, Folder } from "lucide-react";
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

function projectLabel(s: SessionInfo): string {
  return s.projectName ?? s.projectPath ?? "(unknown project)";
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
    for (const s of sessions) {
      if (isSubagentOnly(s)) continue;
      const key = projectKey(s);
      const t = s.updatedAt ?? s.startedAt ?? 0;
      const e = m.get(key);
      if (e) {
        e.count += 1;
        if (t > e.latest) e.latest = t;
      } else {
        m.set(key, {
          label: projectLabel(s),
          count: 1,
          path: s.projectPath,
          latest: t,
        });
      }
    }
    return [...m.entries()]
      .map(([key, v]) => ({ key, ...v }))
      .sort((a, b) => b.latest - a.latest || a.label.localeCompare(b.label));
  }, [sessions]);

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
      ? "All Sessions"
      : filter.kind === "agent"
        ? AGENT_LABEL[filter.agent]
        : filter.label;

  return (
    <div className="flex h-screen text-body">
      <aside
        className={
          "shrink-0 border-r border-white/5 bg-[#22252e] flex flex-col overflow-hidden transition-[width] duration-600 ease-in-out " +
          (sidebarOpen ? "w-64" : "w-0")
        }
      >
        <div
          data-tauri-drag-region
          className="relative h-12 shrink-0 w-64"
        >
          <button
            type="button"
            aria-label="Close sidebar"
            data-tauri-drag-region="false"
            onClick={() => setSidebarOpen(false)}
            className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-white/55 hover:text-white transition rounded-md"
          >
            <PanelLeftClose className="w-4 h-4" />
          </button>
        </div>

        <nav className="flex-1 min-h-0 w-64 p-2 flex flex-col gap-0.5">
          <div className="shrink-0 flex flex-col gap-0.5">
            <SidebarItem
              label="All Sessions"
              count={totalRealSessions}
              active={filter.kind === "all"}
              onClick={() => setFilter({ kind: "all" })}
              dot="#888"
            />

            <SectionHeader
              label="By Agent"
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
              label="By Project"
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
                  title={p.path ?? p.label}
                />
              ))}
            </ScrollArea>
          )}
        </nav>

        <div className="w-64 p-3 text-meta text-white/30 border-t border-white/5">
          {loading ? "Loading…" : `${totalRealSessions} sessions`}
        </div>
      </aside>

      <main className="flex-1 flex flex-col min-w-0">
        <div
          data-tauri-drag-region
          className="relative h-12 shrink-0 flex items-center justify-center px-5 bg-[#0f1014] border-b border-white/10"
        >
          <button
            type="button"
            aria-label="Open sidebar"
            data-tauri-drag-region="false"
            onClick={() => setSidebarOpen(true)}
            className={
              "absolute top-1/2 -translate-y-1/2 p-1 text-white/55 hover:text-white rounded-md transition-opacity duration-300 " +
              (IS_MAC ? "left-24 " : "left-3 ") +
              (sidebarOpen ? "opacity-0 pointer-events-none" : "opacity-100")
            }
          >
            <PanelLeftOpen className="w-4 h-4" />
          </button>
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
                  <Folder className="w-5 h-5 shrink-0 text-white/55" />
                )}
                <h1 className="text-title font-medium truncate">{headerLabel}</h1>
              </>
            )}
            <span className="text-white/40 text-body-sm tabular-nums">
              {visibleCount} sessions
            </span>
          </div>
          <button
            type="button"
            aria-label="Search"
            data-tauri-drag-region="false"
            className="absolute right-5 top-1/2 -translate-y-1/2 p-1 text-white/55 hover:text-white transition rounded-md"
          >
            <Search className="w-4 h-4" />
          </button>
        </div>

        <ScrollArea className="flex-1 min-h-0">
          {error && (
            <div className="m-5 p-3 rounded bg-red-500/10 text-red-300 text-body-sm">
              {error}
            </div>
          )}
          {!error && !loading && visible.length === 0 && (
            <div className="p-10 text-center text-white/40 text-body">
              No sessions found.
            </div>
          )}
          <ul className="divide-y divide-white/5">
            {visible.map((s) => (
              <li
                key={`${s.agent}:${s.filePath}:${s.id}`}
                onClick={() => setSelected(s)}
                className="px-5 py-3.5 cursor-pointer hover:bg-white/[0.03] transition"
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
      className="group flex items-center justify-between w-full px-2 mt-3 mb-1 text-caption uppercase text-white/55 hover:text-white/85 transition"
    >
      <span>{label}</span>
      <Chevron collapsed={collapsed} />
    </button>
  );
}

function Chevron({ collapsed }: { collapsed: boolean }) {
  return (
    <svg
      viewBox="0 0 16 16"
      fill="none"
      className={
        "w-3.5 h-3.5 transition-transform duration-150 " +
        (collapsed ? "-rotate-90" : "")
      }
    >
      <path
        d="M4 6l4 4 4-4"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function SidebarItem({
  label,
  count,
  active,
  onClick,
  dot,
  title,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
  dot?: string;
  title?: string;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={
        "flex items-center gap-2 px-2.5 py-1.5 rounded-md text-left text-body transition " +
        (active
          ? "bg-white/10 text-white"
          : "text-white/70 hover:bg-white/5 hover:text-white")
      }
    >
      {dot !== undefined ? (
        <span
          className="w-2 h-2 rounded-full shrink-0"
          style={{ background: dot }}
        />
      ) : (
        <span className="w-2 shrink-0" />
      )}
      <span className="flex-1 truncate">{label}</span>
      <span className="text-meta text-white/40 tabular-nums">{count}</span>
    </button>
  );
}

function SessionRow({ item, filter }: { item: SessionInfo, filter: Filter }) {
  const accent = AGENT_ACCENT[item.agent];
  return (
    <div className="min-w-0">
      <div className={"pl-4 text-body line-clamp-3" + (item.archived ? " text-white/55" : " text-white/90")}>
        {item.firstUserMessage ?? (
          <span className="text-white/30">(no user message)</span>
        )}
      </div>
      <div className="pl-4 mt-1.5 flex items-center gap-2 text-meta text-white/40">
      {filter.kind !== "project" && (
        <>
        <Folder
          className="w-4 h-4 shrink-0"
        />
        <span
          className="text-body font-medium truncate text-white/55"
        >
          {item.projectName ?? item.projectPath ?? "(unknown project)"}
        </span>
        </>
      )}
      {filter.kind !== "agent" && (
        <Tag
          label={AGENT_LABEL[item.agent]}
          style={{ background: `${accent}22`, color: accent }}
        />
      )}
        {item.subagents.length > 0 && (
          <Tag
            label={`+${item.subagents.length} subagent${item.subagents.length > 1 ? "s" : ""}`}
            className="bg-purple-500/10 text-purple-300/90 border border-purple-500/20"
            title={`${item.subagents.length} subagent invocation${item.subagents.length > 1 ? "s" : ""}`}
          />
        )}
        {item.archived && (
          <Tag
            label="archived"
            className="bg-white/5 text-white/40 border border-white/5"
            title="JSONL file was removed by the agent; only index metadata remains."
          />
        )}
        <span>{formatTime(item.updatedAt ?? item.startedAt)}</span>
        <span>·</span>
        <span>
          {item.partial && !item.archived ? "~" : ""}
          {item.messageCount} msgs
        </span>
      </div>
    </div>
  );
}

function formatTime(ts: number | null): string {
  if (!ts) return "—";
  const d = new Date(ts);
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) {
    return d.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  return d.toLocaleDateString([], {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}
