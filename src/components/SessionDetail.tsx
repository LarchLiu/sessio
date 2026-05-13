import { useEffect, useMemo, useRef, useState } from "react";
import {
  AGENT_ACCENT,
  AGENT_LABEL,
  SessionInfo,
  SessionMessage,
  SubagentInfo,
  getSessionMessages,
} from "../api";
import Tag from "./Tag";
import ScrollArea from "./ScrollArea";

interface Props {
  session: SessionInfo;
  onClose: () => void;
}

type Tab =
  | { kind: "main" }
  | { kind: "sub"; sub: SubagentInfo };

export default function SessionDetail({ session, onClose }: Props) {
  const defaultTab: Tab = useMemo(
    () =>
      session.available
        ? { kind: "main" }
        : session.subagents.length > 0
          ? { kind: "sub", sub: session.subagents[0] }
          : { kind: "main" },
    [session.available, session.subagents]
  );
  const [tab, setTab] = useState<Tab>(defaultTab);

  useEffect(() => {
    setTab(defaultTab);
  }, [defaultTab]);

  const accent = AGENT_ACCENT[session.agent];

  return (
    <div
      className="absolute inset-0 bg-black/40 backdrop-blur-sm flex items-stretch justify-end z-10"
      onClick={onClose}
    >
      <div
        className="w-[720px] max-w-[85vw] h-full bg-[#0b0c10] border-l border-white/10 flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="px-5 py-4 border-b border-white/5 flex items-start gap-3">
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 mb-1.5">
              <Tag
                label={AGENT_LABEL[session.agent]}
                style={{ background: `${accent}22`, color: accent }}
              />
              <span className="text-body-sm text-white/40 truncate font-mono">
                {session.id}
              </span>
              {session.archived && (
                <Tag
                  label="archived"
                  className="bg-white/5 text-white/40 border border-white/5"
                />
              )}
            </div>
            <div className="text-subtitle font-medium truncate">
              {session.projectName ?? session.projectPath ?? "(unknown)"}
            </div>
            {session.projectPath && (
              <div className="text-meta font-mono text-white/30 truncate mt-0.5">
                {session.projectPath}
              </div>
            )}
          </div>
          <button
            onClick={onClose}
            className="text-white/40 hover:text-white text-2xl leading-none px-2"
            aria-label="Close"
          >
            ×
          </button>
        </header>

        {session.subagents.length > 0 && (
          <div className="px-3 py-2 border-b border-white/5 flex gap-1 overflow-x-auto bg-[#0a0b0f]">
            <TabButton
              active={tab.kind === "main"}
              disabled={!session.available}
              onClick={() => setTab({ kind: "main" })}
              label="Main"
              sub={
                session.available
                  ? `${session.messageCount}${session.partial ? "~" : ""} msgs`
                  : "no jsonl"
              }
            />
            {session.subagents.map((s) => (
              <TabButton
                key={s.id}
                active={tab.kind === "sub" && tab.sub.id === s.id}
                onClick={() => setTab({ kind: "sub", sub: s })}
                label={s.agentType ?? "agent"}
                sub={`${s.messageCount} msgs`}
                accent="#a78bfa"
                tooltip={s.description ?? s.id}
              />
            ))}
          </div>
        )}

        <MessageStream
          key={tab.kind === "main" ? "main" : tab.sub.id}
          agent={session.agent}
          filePath={tab.kind === "main" ? session.filePath : tab.sub.filePath}
          sessionId={session.id}
          available={
            tab.kind === "main" ? session.available : tab.sub.filePath !== ""
          }
          emptyHint={
            tab.kind === "main"
              ? "Session content is no longer on disk — the agent removed the JSONL file and only metadata remains."
              : "Subagent jsonl unreadable."
          }
          subagentDesc={tab.kind === "sub" ? tab.sub.description : null}
        />
      </div>
    </div>
  );
}

function TabButton({
  active,
  disabled,
  onClick,
  label,
  sub,
  accent,
  tooltip,
}: {
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
  label: string;
  sub: string;
  accent?: string;
  tooltip?: string;
}) {
  const color = accent ?? "#e6e7ea";
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      title={tooltip}
      className={
        "shrink-0 px-3 py-2 rounded-md text-left text-body-sm transition border " +
        (active
          ? "bg-white/[0.08] border-white/15"
          : disabled
            ? "bg-transparent border-transparent text-white/25 cursor-not-allowed"
            : "bg-transparent border-transparent text-white/60 hover:bg-white/5 hover:text-white")
      }
    >
      <div className="flex items-center gap-1.5">
        <span
          className="w-1.5 h-1.5 rounded-full"
          style={{ background: color }}
        />
        <span className="font-medium">{label}</span>
      </div>
      <div className="text-caption text-white/40 mt-0.5">{sub}</div>
    </button>
  );
}

function MessageStream({
  agent,
  filePath,
  sessionId,
  available,
  emptyHint,
  subagentDesc,
}: {
  agent: SessionInfo["agent"];
  filePath: string;
  sessionId: string;
  available: boolean;
  emptyHint: string;
  subagentDesc: string | null;
}) {
  const [messages, setMessages] = useState<SessionMessage[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const bubbleRefs = useRef<(HTMLDivElement | null)[]>([]);
  const viewportRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    if (!available || !filePath) {
      setMessages([]);
      setLoading(false);
      setError(null);
      return;
    }
    setLoading(true);
    setError(null);
    setMessages([]);
    getSessionMessages(agent, filePath, sessionId)
      .then((rows) => !cancelled && setMessages(rows))
      .catch((err) => !cancelled && setError(String(err)))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [agent, filePath, sessionId, available]);

  bubbleRefs.current.length = messages.length;

  return (
    <div className="relative flex-1 min-h-0 flex flex-col">
      <ScrollArea
        ref={viewportRef}
        className="flex-1 min-h-0"
        viewportClassName="px-5 py-4"
      >
        {subagentDesc && (
          <div className="text-body-sm text-purple-300/85 bg-purple-500/[0.06] border border-purple-500/15 rounded p-3 mb-4 leading-relaxed">
            <span className="text-purple-200/60 uppercase text-caption mr-2 font-medium">
              task
            </span>
            {subagentDesc}
          </div>
        )}
        {!available && (
          <div className="text-amber-200/80 text-body bg-amber-500/[0.08] border border-amber-500/20 rounded p-3 leading-relaxed">
            {emptyHint}
          </div>
        )}
        {loading && (
          <div className="text-white/40 text-body">Loading messages…</div>
        )}
        {error && (
          <div className="text-red-300 text-body-sm bg-red-500/10 rounded p-3">
            {error}
          </div>
        )}
        {!loading && !error && available && messages.length === 0 && (
          <div className="text-white/40 text-body">No messages.</div>
        )}
        <div className="flex flex-col gap-4">
          {messages.map((m, i) => (
            <div
              key={i}
              ref={(el) => {
                bubbleRefs.current[i] = el;
              }}
            >
              <MessageBubble msg={m} />
            </div>
          ))}
        </div>
      </ScrollArea>
      <UserNav messages={messages} refs={bubbleRefs} viewportRef={viewportRef} />
    </div>
  );
}

function UserNav({
  messages,
  refs,
  viewportRef,
}: {
  messages: SessionMessage[];
  refs: React.MutableRefObject<(HTMLDivElement | null)[]>;
  viewportRef: React.RefObject<HTMLDivElement | null>;
}) {
  const userIndices = useMemo(
    () =>
      messages
        .map((m, i) => (m.role === "user" ? i : -1))
        .filter((i) => i >= 0),
    [messages],
  );

  const [activeIdx, setActiveIdx] = useState<number | null>(null);
  const [positions, setPositions] = useState<Map<number, number>>(new Map());

  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp || userIndices.length === 0) {
      setActiveIdx(null);
      setPositions(new Map());
      return;
    }

    const computeActive = () => {
      const vpRect = vp.getBoundingClientRect();
      const threshold = vpRect.top + vpRect.height * 0.33;
      let active: number | null = null;
      for (const idx of userIndices) {
        const el = refs.current[idx];
        if (!el) continue;
        const r = el.getBoundingClientRect();
        if (r.top <= threshold) active = idx;
        else break;
      }
      setActiveIdx(active ?? userIndices[0]);
    };

    const computePositions = () => {
      const sh = vp.scrollHeight;
      if (sh <= 0) return;
      const vpRect = vp.getBoundingClientRect();
      const m = new Map<number, number>();
      for (const idx of userIndices) {
        const el = refs.current[idx];
        if (!el) continue;
        const r = el.getBoundingClientRect();
        const top = r.top - vpRect.top + vp.scrollTop;
        m.set(idx, Math.min(Math.max(top / sh, 0), 1));
      }
      setPositions(m);
    };

    computePositions();
    computeActive();
    vp.addEventListener("scroll", computeActive, { passive: true });
    const ro = new ResizeObserver(() => {
      computePositions();
      computeActive();
    });
    ro.observe(vp);
    for (const child of Array.from(vp.children)) ro.observe(child);
    return () => {
      vp.removeEventListener("scroll", computeActive);
      ro.disconnect();
    };
  }, [viewportRef, refs, userIndices]);

  if (userIndices.length === 0) return null;
  return (
    <div className="absolute right-0.5 top-2 bottom-2 z-10 w-5">
      {userIndices.map((idx) => {
        const preview = messages[idx].text
          .replace(/\s+/g, " ")
          .trim()
          .slice(0, 200);
        const ratio = positions.get(idx);
        if (ratio === undefined) return null;
        return (
          <button
            key={idx}
            type="button"
            onClick={() =>
              refs.current[idx]?.scrollIntoView({
                behavior: "smooth",
                block: "start",
              })
            }
            style={{ top: `${ratio * 100}%`, transform: "translateY(-50%)" }}
            className="group absolute right-0 cursor-pointer p-1.5"
            aria-label={`Jump to user message ${idx + 1}`}
          >
            <span
              className={
                "block w-1.5 h-1.5 rounded-full transition " +
                (idx === activeIdx
                  ? "bg-white scale-125"
                  : "bg-white/25 group-hover:bg-white")
              }
            />
            <div className="hidden group-hover:block absolute right-full mr-3 top-1/2 -translate-y-1/2 w-72 bg-black/90 border border-white/10 text-white/90 text-body-sm px-3 py-2 rounded shadow-lg leading-relaxed pointer-events-none">
              <span
                style={{
                  display: "-webkit-box",
                  WebkitLineClamp: 3,
                  WebkitBoxOrient: "vertical",
                  overflow: "hidden",
                }}
              >
                {preview}
                {messages[idx].text.length > 200 ? "…" : ""}
              </span>
            </div>
          </button>
        );
      })}
    </div>
  );
}

function MessageBubble({ msg }: { msg: SessionMessage }) {
  const LONG_TOOL_THRESHOLD = 500;
  const isToolLike =
    msg.role === "tool_call" || msg.role === "tool_result";
  const collapsible =
    msg.role === "developer" ||
    msg.role === "system" ||
    (isToolLike && msg.text.length > LONG_TOOL_THRESHOLD);
  const [collapsed, setCollapsed] = useState(collapsible);
  const preview = collapsible
    ? msg.text.replace(/\s+/g, " ").slice(0, 200)
    : "";
  return (
    <div className="rounded-lg px-4 py-3 text-body leading-relaxed whitespace-pre-wrap break-words bg-white/[0.06] border border-white/[0.04]">
      <div className="flex items-center gap-2 mb-2">
        <span className="text-caption uppercase text-white/40 font-medium">
          {msg.role}
        </span>
        {msg.timestamp && (
          <span className="text-caption text-white/30">
            {new Date(msg.timestamp).toLocaleString([], {
              hour: "2-digit",
              minute: "2-digit",
              month: "short",
              day: "numeric",
            })}
          </span>
        )}
      </div>
      <div className="text-white/85">
        {collapsible && collapsed ? (
          <span className="text-white/60">
            {preview}
            {msg.text.length > 200 ? "…" : ""}
          </span>
        ) : (
          msg.text
        )}
      </div>
      {collapsible && (
        <div className="mt-2 flex justify-center">
          <button
            type="button"
            onClick={() => setCollapsed((v) => !v)}
            aria-label={collapsed ? "Expand" : "Collapse"}
            className="text-white/70 hover:text-white text-lg leading-none px-4 py-1 rounded hover:bg-white/5 transition"
          >
            {collapsed ? "▾" : "▴"}
          </button>
        </div>
      )}
    </div>
  );
}
