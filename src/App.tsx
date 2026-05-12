import { useEffect, useMemo, useState } from "react";
import {
  AGENT_ACCENT,
  AGENT_LABEL,
  Agent,
  SessionInfo,
  listSessions,
} from "./api";
import SessionDetail from "./components/SessionDetail";
import Tag from "./components/Tag";

type Filter =
  | { kind: "all" }
  | { kind: "agent"; agent: Agent }
  | { kind: "project"; key: string; label: string };

const AGENT_ORDER: Agent[] = ["codex", "claude", "gemini"];

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
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<SessionInfo | null>(null);
  const [expandAgent, setExpandAgent] = useState(true);
  const [expandProject, setExpandProject] = useState(true);

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
    const q = query.trim().toLowerCase();
    return sessions.filter((s) => {
      if (filter.kind === "agent" && s.agent !== filter.agent) return false;
      if (filter.kind === "project" && projectKey(s) !== filter.key) return false;
      if (!q) return true;
      const hay = [
        s.projectPath ?? "",
        s.projectName ?? "",
        s.firstUserMessage ?? "",
        s.id,
      ]
        .join(" ")
        .toLowerCase();
      return hay.includes(q);
    });
  }, [sessions, filter, query]);

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
      <aside className="w-64 shrink-0 border-r border-white/5 bg-[#0b0c10] flex flex-col">
        <div className="px-4 py-4 border-b border-white/5">
          <div className="text-title font-semibold tracking-tight">Sessio</div>
          <div className="text-meta text-white/40 mt-0.5">
            Agent sessions manager
          </div>
        </div>

        <nav className="flex-1 overflow-y-scroll p-2 flex flex-col gap-0.5">
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
          {expandProject &&
            projectGroups.map((p) => (
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
        </nav>

        <div className="p-3 text-meta text-white/30 border-t border-white/5">
          {loading ? "Loading…" : `${totalRealSessions} sessions`}
        </div>
      </aside>

      <main className="flex-1 flex flex-col min-w-0">
        <header className="flex items-center gap-3 px-5 py-3.5 border-b border-white/5 bg-[#0f1014]">
          <h1 className="text-title font-medium truncate">{headerLabel}</h1>
          <span className="text-white/40 text-body-sm tabular-nums">
            {visibleCount}
          </span>
          <div className="flex-1" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search project, message, id…"
            className="w-72 bg-white/5 border border-white/5 rounded-md px-3 py-1.5 text-body placeholder:text-white/30 focus:outline-none focus:border-white/20"
          />
        </header>

        <div className="flex-1 overflow-y-auto">
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
                <SessionRow item={s} />
              </li>
            ))}
          </ul>
        </div>
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

function SessionRow({ item }: { item: SessionInfo }) {
  const accent = AGENT_ACCENT[item.agent];
  return (
    <div className="min-w-0">
      <div className="flex items-center gap-2 mb-1">
        <span
          className="w-2 h-2 rounded-full shrink-0"
          style={{ background: accent }}
          title={AGENT_LABEL[item.agent]}
        />
        <span
          className={
            "text-body font-medium truncate " +
            (item.archived ? "text-white/55" : "text-white/90")
          }
        >
          {item.projectName ?? item.projectPath ?? "(unknown project)"}
        </span>
        <Tag
          label={AGENT_LABEL[item.agent]}
          style={{ background: `${accent}22`, color: accent }}
        />
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
      </div>
      <div className="pl-4 text-body-sm text-white/55 truncate">
        {item.firstUserMessage ?? (
          <span className="text-white/30">(no user message)</span>
        )}
      </div>
      <div className="pl-4 mt-1.5 flex items-center gap-3 text-meta text-white/40">
        <span>{formatTime(item.updatedAt ?? item.startedAt)}</span>
        <span>·</span>
        <span>
          {item.partial && !item.archived ? "~" : ""}
          {item.messageCount} msgs
        </span>
        {item.projectPath && (
          <>
            <span>·</span>
            <span className="truncate font-mono text-caption text-white/30">
              {item.projectPath}
            </span>
          </>
        )}
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
