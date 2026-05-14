import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import {
  AGENT_ACCENT,
  AGENT_LABEL,
  SessionInfo,
  SessionMessage,
  SubagentInfo,
  agentTint,
  getSessionMessages,
} from "../api";
import Tag from "./Tag";
import ScrollArea from "./ScrollArea";
import Tooltip from "./Tooltip";
import { localeTag, useI18n } from "../i18n";

interface Props {
  session: SessionInfo;
  onClose: () => void;
}

type Tab =
  | { kind: "main" }
  | { kind: "sub"; sub: SubagentInfo };

export default function SessionDetail({ session, onClose }: Props) {
  const { t } = useI18n();
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

  return (
    <div
      className="absolute inset-0 bg-black/40 backdrop-blur-sm flex items-stretch justify-end z-10"
      onClick={onClose}
    >
      <div
        className="w-[720px] max-w-[85vw] h-full bg-surface-panel border-l border-ink/10 flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="px-5 py-4 border-b border-ink/5 flex items-start gap-3">
          <div className="flex-1 min-w-0">
            <div className="text-subtitle font-medium truncate">
              {session.firstUserMessage ?? (
                <span className="text-ink/30">{t("list.no_user_message")}</span>
              )}
            </div>
            {session.projectPath && (
              <div className="text-meta font-mono text-ink/30 truncate mt-0.5">
                {session.projectPath}
              </div>
            )}
            <div className="flex items-center gap-2 mt-1.5">
              <Tag
                label={AGENT_LABEL[session.agent]}
                style={{ background: agentTint(session.agent, 0.13), color: AGENT_ACCENT[session.agent] }}
              />
              <span className="text-body-sm text-ink/40 truncate font-mono">
                {session.id}
              </span>
              {session.archived && (
                <Tag
                  label={t("list.archived")}
                  color="var(--color-muted)"
                  title={t(
                    session.available
                      ? "list.archived_tooltip_by_user"
                      : "list.archived_tooltip"
                  )}
                />
              )}
            </div>
          </div>
          <button
            onClick={onClose}
            className="text-ink/40 hover:text-ink text-2xl leading-none px-2"
            aria-label={t("detail.close")}
          >
            ×
          </button>
        </header>

        {session.subagents.length > 0 && (
          <div className="px-3 py-2 border-b border-ink/5 flex gap-1 overflow-x-auto bg-surface-panel-alt">
            <TabButton
              active={tab.kind === "main"}
              disabled={!session.available}
              onClick={() => setTab({ kind: "main" })}
              label={t("detail.main")}
              sub={
                session.available
                  ? `${session.partial ? "~" : ""}${t("detail.msgs", { count: session.messageCount })}`
                  : t("detail.no_jsonl")
              }
            />
            {session.subagents.map((s) => (
              <TabButton
                key={s.id}
                active={tab.kind === "sub" && tab.sub.id === s.id}
                onClick={() => setTab({ kind: "sub", sub: s })}
                label={s.agentType ?? t("detail.default_subagent_type")}
                sub={t("detail.msgs", { count: s.messageCount })}
                accent="rgb(var(--color-accent-purple))"
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
              ? t("detail.session_archived")
              : t("detail.subagent_unreadable")
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
  const color = accent ?? "currentColor";
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      title={tooltip}
      className={
        "shrink-0 px-3 py-2 rounded-md text-left text-body-sm transition border " +
        (active
          ? "bg-ink/[0.08] border-ink/15"
          : disabled
            ? "bg-transparent border-transparent text-ink/25 cursor-not-allowed"
            : "bg-transparent border-transparent text-ink/60 hover:bg-ink/5 hover:text-ink")
      }
    >
      <div className="flex items-center gap-1.5">
        <span
          className="w-1.5 h-1.5 rounded-full"
          style={{ background: color }}
        />
        <span className="font-medium">{label}</span>
      </div>
      <div className="text-caption text-ink/40 mt-0.5">{sub}</div>
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
  const { t } = useI18n();
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
          <div className="text-body-sm text-accent-purple bg-accent-purple/[0.08] border border-accent-purple/20 rounded p-3 mb-4 leading-relaxed">
            <span className="text-accent-purple/70 uppercase text-caption mr-2 font-medium">
              {t("detail.task")}
            </span>
            {subagentDesc}
          </div>
        )}
        {!available && (
          <div className="text-status-warn text-body bg-status-warn/[0.10] border border-status-warn/30 rounded p-3 leading-relaxed">
            {emptyHint}
          </div>
        )}
        {loading && (
          <div className="text-ink/40 text-body">{t("detail.loading_messages")}</div>
        )}
        {error && (
          <div className="text-status-error text-body-sm bg-status-error/10 rounded p-3">
            {error}
          </div>
        )}
        {!loading && !error && available && messages.length === 0 && (
          <div className="text-ink/40 text-body">{t("detail.no_messages")}</div>
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
  refs: React.RefObject<(HTMLDivElement | null)[]>;
  viewportRef: React.RefObject<HTMLDivElement | null>;
}) {
  const { t } = useI18n();
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
        const ratio = positions.get(idx);
        if (ratio === undefined) return null;
        const text = messages[idx].text;
        const preview = text.replace(/\s+/g, " ").trim().slice(0, 200);
        const tip = (
          <div
            className="w-72 whitespace-normal"
            style={{
              display: "-webkit-box",
              WebkitLineClamp: 3,
              WebkitBoxOrient: "vertical",
              overflow: "hidden",
            }}
          >
            {preview}
            {text.length > 200 ? "…" : ""}
          </div>
        );
        return (
          <Tooltip key={idx} content={tip} placement="left" offset={12}>
            <button
              type="button"
              onClick={() =>
                refs.current[idx]?.scrollIntoView({
                  behavior: "smooth",
                  block: "start",
                })
              }
              style={{ top: `${ratio * 100}%`, transform: "translateY(-50%)" }}
              className="group absolute right-0 cursor-pointer p-1.5"
              aria-label={t("detail.jump_to_user_msg", { n: idx + 1 })}
            >
              <span
                className={
                  "block w-1.5 h-1.5 rounded-full transition " +
                  (idx === activeIdx
                    ? "bg-ink scale-125"
                    : "bg-ink/25 group-hover:bg-ink")
                }
              />
            </button>
          </Tooltip>
        );
      })}
    </div>
  );
}

function MessageBubble({ msg }: { msg: SessionMessage }) {
  const { lang, t } = useI18n();
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
  const bubbleRef = useRef<HTMLDivElement>(null);
  const anchorTopRef = useRef<number | null>(null);

  function findScroller(el: HTMLElement | null): HTMLElement | null {
    let node = el?.parentElement ?? null;
    while (node) {
      const oy = getComputedStyle(node).overflowY;
      if (oy === "auto" || oy === "scroll") return node;
      node = node.parentElement;
    }
    return null;
  }

  const toggle = () => {
    const bubble = bubbleRef.current;
    const scroller = findScroller(bubble);
    if (bubble && scroller) {
      anchorTopRef.current =
        bubble.getBoundingClientRect().top -
        scroller.getBoundingClientRect().top;
    }
    setCollapsed((v) => !v);
  };

  useLayoutEffect(() => {
    const before = anchorTopRef.current;
    if (before === null) return;
    anchorTopRef.current = null;
    const bubble = bubbleRef.current;
    const scroller = findScroller(bubble);
    if (!bubble || !scroller) return;
    const after =
      bubble.getBoundingClientRect().top -
      scroller.getBoundingClientRect().top;
    // 折叠时若 title 已被滚出视口顶部，拉回到视口顶部；其余保持原位
    const target = collapsed && before < 0 ? 0 : before;
    const delta = after - target;
    if (delta !== 0) scroller.scrollTop += delta;
  }, [collapsed]);

  return (
    <div
      ref={bubbleRef}
      className="rounded-lg px-4 py-3 text-body leading-relaxed whitespace-pre-wrap break-words bg-ink/[0.06] border border-ink/[0.04]"
    >
      <div
        className={
          "flex items-center gap-2 mb-2 " +
          (collapsible
            ? "cursor-pointer select-none hover:text-ink/70"
            : "")
        }
        onClick={collapsible ? toggle : undefined}
        role={collapsible ? "button" : undefined}
        aria-expanded={collapsible ? !collapsed : undefined}
      >
        <span className="text-caption uppercase text-ink/40 font-medium">
          {msg.role}
        </span>
        {msg.timestamp && (
          <span className="text-caption text-ink/30">
            {new Date(msg.timestamp).toLocaleString(localeTag(lang), {
              hour: "2-digit",
              minute: "2-digit",
              month: "short",
              day: "numeric",
            })}
          </span>
        )}
      </div>
      <div className="text-ink/85">
        {collapsible && collapsed ? (
          <span className="text-ink/60">
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
            onClick={toggle}
            aria-label={collapsed ? t("detail.expand") : t("detail.collapse")}
            className="text-ink/70 hover:text-ink leading-none px-4 py-1 rounded hover:bg-ink/5 transition"
          >
            {collapsed ? (
              <ChevronDown className="w-4 h-4" />
            ) : (
              <ChevronUp className="w-4 h-4" />
            )}
          </button>
        </div>
      )}
    </div>
  );
}
