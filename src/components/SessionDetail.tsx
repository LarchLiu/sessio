import {
  startTransition,
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { MultiFileDiff, PatchDiff } from "@pierre/diffs/react";
import { ChevronDown, ChevronRight, FileDiff } from "lucide-react";
import ReactMarkdown, { type Components } from "react-markdown";
import rehypeKatex from "rehype-katex";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import type { Options as SanitizeSchema } from "rehype-sanitize";
import "katex/dist/katex.min.css";
import {
  SessionInfo,
  SessionMessage,
  SubagentInfo,
  getSessionMessages,
  readLocalImageDataUrl,
  updateSessionMessageCount,
} from "../api";
import ScrollArea from "./ScrollArea";
import Tooltip from "./Tooltip";
import { localeTag, useI18n } from "../i18n";
import type { ViewMode } from "../App";

interface Props {
  session: SessionInfo;
  viewMode: ViewMode;
  onMessageCount: (filePath: string, count: number) => boolean;
  onActiveMessageMeta: (meta: ActiveMessageMeta) => void;
}

export interface ActiveMessageMeta {
  filePath: string;
  count: number;
  partial: boolean;
}

// 与后端 src-tauri/src/models.rs:strip_injected_context 保持一致：
// 剥离 IDE 注入的上下文块，仅在展示 user 消息预览时使用，展开仍保留原文。
function stripInjectedContext(s: string): string {
  let text = s;
  for (;;) {
    const trimmed = text.trimStart();
    if (!trimmed.startsWith("<ide_")) break;
    const afterLt = trimmed.slice("<ide_".length);
    const closeIdx = afterLt.indexOf(">");
    if (closeIdx < 0) break;
    const tag = afterLt.slice(0, closeIdx);
    const close = `</ide_${tag}>`;
    const afterOpen = afterLt.slice(closeIdx + 1);
    const endIdx = afterOpen.indexOf(close);
    if (endIdx < 0) break;
    text = afterOpen.slice(endIdx + close.length);
  }
  const MARKER = "## My request for Codex:";
  const idx = text.indexOf(MARKER);
  if (idx >= 0) text = text.slice(idx + MARKER.length);
  return stripImagePlaceholders(text).trim();
}

function stripImagePlaceholders(s: string): string {
  return s
    .replace(/<image\b[^>]*>[\s\S]*?<\/image>/gi, "")
    .replace(/^\s*<image\b[^>]*>\s*$/gim, "")
    .replace(/^\s*<\/image>\s*$/gim, "")
    .replace(/\n{3,}/g, "\n\n");
}

type Tab =
  | { kind: "main" }
  | { kind: "sub"; sub: SubagentInfo };

const ROLE_NAV_SHOW_DELAY_MS = 800;

interface MessageCacheEntry {
  messages: SessionMessage[];
  messageCount: number;
  loadedAt: number;
}

interface ScrollAnchor {
  key: string;
  offset: number;
}

interface ScrollCacheEntry {
  scrollTop: number;
  anchor: ScrollAnchor | null;
  atBottom: boolean;
}

const messageCache = new Map<string, MessageCacheEntry>();
const scrollCache = new Map<string, ScrollCacheEntry>();

function messageSourceKey(agent: SessionInfo["agent"], filePath: string, sessionId: string): string {
  return `${agent}:${sessionId}:${filePath}`;
}

export default function SessionDetail({
  session,
  viewMode,
  onMessageCount,
  onActiveMessageMeta,
}: Props) {
  const { t } = useI18n();
  const defaultTab: Tab = useMemo(
    () =>
      session.available
        ? { kind: "main" }
        : session.subagents.length > 0
          ? { kind: "sub", sub: session.subagents[0] }
          : { kind: "main" },
    [session.available, session.id]
  );
  const [tab, setTab] = useState<Tab>(defaultTab);

  useEffect(() => {
    setTab((current) => {
      if (current.kind === "main") {
        return session.available ? current : defaultTab;
      }
      const nextSub = session.subagents.find((s) => s.id === current.sub.id);
      return nextSub ? { kind: "sub", sub: nextSub } : defaultTab;
    });
  }, [defaultTab, session.available, session.subagents]);

  const [previewImage, setPreviewImage] = useState<MarkdownImage | null>(null);
  const activeMessageMeta =
    tab.kind === "main"
      ? {
          filePath: session.filePath,
          count: session.messageCount,
          partial: session.partial,
        }
      : {
          filePath: tab.sub.filePath,
          count: tab.sub.messageCount,
          partial: tab.sub.partial,
        };

  useEffect(() => {
    onActiveMessageMeta(activeMessageMeta);
  }, [
    activeMessageMeta.filePath,
    activeMessageMeta.count,
    activeMessageMeta.partial,
    onActiveMessageMeta,
  ]);

  return (
    <div className="h-full min-h-0 bg-surface-panel flex flex-col">
        {session.subagents.length > 0 && (
          <ScrollArea
            className="shrink-0 border-b border-ink/5 bg-surface-panel-alt"
            viewportClassName="px-3 pt-1 pb-px"
            orientation="horizontal"
            persistScrollbars
          >
            <div className="flex min-w-max gap-1">
              <TabButton
                active={tab.kind === "main"}
                disabled={!session.available}
                onClick={() => setTab({ kind: "main" })}
                label={t("detail.main")}
              />
              {session.subagents.map((s) => (
                <TabButton
                  key={s.id}
                  active={tab.kind === "sub" && tab.sub.id === s.id}
                  onClick={() => setTab({ kind: "sub", sub: s })}
                  label={
                    s.description ??
                    s.agentType ??
                    t("detail.default_subagent_type")
                  }
                  accent="rgb(var(--color-accent-purple))"
                  tooltip={s.agentType ? `${s.agentType} · ${s.id}` : s.id}
                />
              ))}
            </div>
          </ScrollArea>
        )}

        <MessageStream
          key={
            tab.kind === "main"
              ? messageSourceKey(session.agent, session.filePath, session.id)
              : messageSourceKey(session.agent, tab.sub.filePath, `${session.id}:${tab.sub.id}`)
          }
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
          viewMode={viewMode}
          onPreviewImage={setPreviewImage}
          onMessageCount={onMessageCount}
          messageCount={activeMessageMeta.count}
        />

        {previewImage && (
          <ImagePreviewOverlay
            image={previewImage}
            onClose={() => setPreviewImage(null)}
          />
        )}
    </div>
  );
}

function TabButton({
  active,
  disabled,
  onClick,
  label,
  accent,
  tooltip,
}: {
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
  label: string;
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
        "relative shrink-0 px-3 py-1 text-left text-body-sm transition border-b-2 " +
        (active
          ? "border-ink/55 text-ink"
          : disabled
            ? "border-transparent text-ink/25 cursor-not-allowed"
            : "border-transparent text-ink/60 hover:text-ink")
      }
    >
      <div className="flex items-center gap-1.5">
        <span
          className="w-1.5 h-1.5 rounded-full"
          style={{ background: color }}
        />
        <span className="font-medium">{label}</span>
      </div>
    </button>
  );
}

function MessageStream({
  agent,
  filePath,
  sessionId,
  available,
  emptyHint,
  viewMode,
  onPreviewImage,
  onMessageCount,
  messageCount,
}: {
  agent: SessionInfo["agent"];
  filePath: string;
  sessionId: string;
  available: boolean;
  emptyHint: string;
  viewMode: ViewMode;
  onPreviewImage: (image: MarkdownImage) => void;
  onMessageCount: (filePath: string, count: number) => boolean;
  messageCount: number;
}) {
  const { t } = useI18n();
  const sourceKey = messageSourceKey(agent, filePath, sessionId);
  const cachedEntry = messageCache.get(sourceKey);
  const isFreshCache =
    Boolean(cachedEntry) && cachedEntry?.messageCount === messageCount;
  const [messages, setMessages] = useState<SessionMessage[]>(
    cachedEntry?.messages ?? [],
  );
  const [loading, setLoading] = useState(() => !isFreshCache);
  const [error, setError] = useState<string | null>(null);
  const bubbleRefs = useRef<(HTMLDivElement | null)[]>([]);
  const viewportRef = useRef<HTMLDivElement>(null);
  const pendingInitialPositionRef = useRef<"bottom" | "restore" | null>(null);
  const initialPositionAppliedRef = useRef(false);

  useLayoutEffect(() => {
    if (!available || !filePath) {
      setMessages([]);
      setLoading(false);
      setError(null);
      pendingInitialPositionRef.current = null;
      initialPositionAppliedRef.current = false;
      return;
    }
    const cached = messageCache.get(sourceKey);
    if (cached) {
      setMessages(cached.messages);
      setLoading(cached.messageCount !== messageCount);
      setError(null);
    } else {
      setMessages([]);
      setLoading(true);
      setError(null);
    }
    pendingInitialPositionRef.current = scrollCache.has(sourceKey)
      ? "restore"
      : "bottom";
    initialPositionAppliedRef.current = false;
  }, [available, filePath, messageCount, sourceKey]);

  useEffect(() => {
    if (!available || !filePath) return;
    const cached = messageCache.get(sourceKey);
    if (cached && cached.messageCount === messageCount) return;

    let cancelled = false;
    let frameId: number | null = null;
    let timerId: number | null = null;
    frameId = window.requestAnimationFrame(() => {
      timerId = window.setTimeout(() => {
        getSessionMessages(agent, filePath, sessionId)
          .then((result) => {
            if (cancelled) return;
            messageCache.set(sourceKey, {
              messages: result.messages,
              messageCount: result.messageCount,
              loadedAt: Date.now(),
            });
            startTransition(() => {
              setMessages(result.messages);
              setLoading(false);
            });
            if (!onMessageCount(filePath, result.messageCount)) return;
            window.setTimeout(() => {
              updateSessionMessageCount(
                agent,
                filePath,
                result.messageCount,
                sessionId,
              ).catch((err) => console.warn("update message count failed", err));
            }, 0);
          })
          .catch((err) => {
            if (cancelled) return;
            setError(String(err));
            setLoading(false);
          });
      }, 0);
    });
    return () => {
      cancelled = true;
      if (frameId !== null) window.cancelAnimationFrame(frameId);
      if (timerId !== null) window.clearTimeout(timerId);
    };
  }, [agent, filePath, sessionId, available, messageCount, onMessageCount, sourceKey]);

  const displayItems = useMemo(() => {
    const all = messages.map((m, srcIdx) => ({ m, srcIdx }));
    const filtered =
      viewMode === "native"
        ? all
        : all.filter(({ m }) => isConversationRole(m.role));
    return moveFileEditsToTurnEnd(pairToolMessages(filtered));
  }, [messages, viewMode]);

  bubbleRefs.current.length = displayItems.length;

  const saveScrollSnapshot = useCallback(
    (vp: HTMLDivElement | null = viewportRef.current) => {
      if (
        !vp ||
        !available ||
        !filePath ||
        !initialPositionAppliedRef.current
      ) {
        return;
      }
      const atBottom = vp.scrollTop + vp.clientHeight >= vp.scrollHeight - 1;
      let anchor: ScrollAnchor | null = null;
      if (!atBottom && displayItems.length > 0) {
        const vpRect = vp.getBoundingClientRect();
        let bestIdx = -1;
        let bestOffset = Number.NEGATIVE_INFINITY;
        let fallbackIdx = -1;
        let fallbackOffset = Number.POSITIVE_INFINITY;
        for (let i = 0; i < displayItems.length; i += 1) {
          const el = bubbleRefs.current[i];
          if (!el) continue;
          const offset = el.getBoundingClientRect().top - vpRect.top;
          if (offset <= 0 && offset > bestOffset) {
            bestOffset = offset;
            bestIdx = i;
          }
          if (offset >= 0 && offset < fallbackOffset) {
            fallbackOffset = offset;
            fallbackIdx = i;
          }
        }
        const idx = bestIdx >= 0 ? bestIdx : fallbackIdx;
        const el = idx >= 0 ? bubbleRefs.current[idx] : null;
        if (el) {
          const offset = el.getBoundingClientRect().top - vpRect.top;
          anchor = { key: displayItems[idx].key, offset };
        }
      }
      scrollCache.set(sourceKey, {
        scrollTop: vp.scrollTop,
        anchor,
        atBottom,
      });
    },
    [available, filePath, displayItems, sourceKey],
  );

  useLayoutEffect(() => {
    return () => {
      if (initialPositionAppliedRef.current) saveScrollSnapshot();
    };
  }, [saveScrollSnapshot]);

  useLayoutEffect(() => {
    const vp = viewportRef.current;
    const mode = pendingInitialPositionRef.current;
    if (!vp || mode === null || loading || messages.length === 0) return;
    const snapshot = scrollCache.get(sourceKey);
    if (mode === "restore" && snapshot?.anchor) {
      const idx = displayItems.findIndex((item) => item.key === snapshot.anchor?.key);
      const el = idx >= 0 ? bubbleRefs.current[idx] : null;
      if (el) {
        const vpRect = vp.getBoundingClientRect();
        const top = el.getBoundingClientRect().top - vpRect.top + vp.scrollTop;
        vp.scrollTop = Math.max(0, top - snapshot.anchor.offset);
        pendingInitialPositionRef.current = null;
        initialPositionAppliedRef.current = true;
        return;
      }
    }
    if (mode === "restore" && snapshot) {
      vp.scrollTop = Math.max(
        0,
        Math.min(snapshot.scrollTop, vp.scrollHeight - vp.clientHeight),
      );
    } else {
      const last = bubbleRefs.current[displayItems.length - 1];
      if (last) {
        last.scrollIntoView({ block: "end" });
      } else {
        vp.scrollTop = Math.max(0, vp.scrollHeight - vp.clientHeight);
      }
    }
    pendingInitialPositionRef.current = null;
    initialPositionAppliedRef.current = true;
  }, [displayItems, loading, messages.length, sourceKey]);

  return (
    <div className="relative flex-1 min-h-0 flex flex-col">
      <ScrollArea
        ref={viewportRef}
        className="flex-1 min-h-0"
        viewportClassName="px-10 py-4 session-chat-scroll-viewport"
        onScroll={saveScrollSnapshot}
      >
        {!available && (
          <div className="text-status-warn text-body bg-status-warn/[0.10] border border-status-warn/30 rounded p-3 leading-relaxed">
            {emptyHint}
          </div>
        )}
        {error && (
          <div className="text-status-error text-body-sm bg-status-error/10 rounded p-3">
            {error}
          </div>
        )}
        {!loading && !error && available && displayItems.length === 0 && (
          <div className="text-ink/40 text-body">{t("detail.no_messages")}</div>
        )}
        <div className="flex flex-col gap-2">
          {displayItems.map((item, i) => (
            <div
              key={item.key}
              ref={(el) => {
                bubbleRefs.current[i] = el;
              }}
              className={
                "message-render-contain " +
                (item.message.role === "user" ? "flex justify-end" : "")
              }
            >
              <MessageBubble
                msg={item.message}
                toolResult={item.toolResult}
                onPreviewImage={onPreviewImage}
              />
            </div>
          ))}
        </div>
      </ScrollArea>
      <RoleNav
        role="assistant"
        side="left"
        messages={displayItems.map((d) => d.message)}
        refs={bubbleRefs}
        viewportRef={viewportRef}
      />
      <RoleNav
        role="user"
        side="right"
        messages={displayItems.map((d) => d.message)}
        refs={bubbleRefs}
        viewportRef={viewportRef}
      />
    </div>
  );
}

function RoleNav({
  role,
  side,
  messages,
  refs,
  viewportRef,
}: {
  role: "assistant" | "user";
  side: "left" | "right";
  messages: SessionMessage[];
  refs: React.RefObject<(HTMLDivElement | null)[]>;
  viewportRef: React.RefObject<HTMLDivElement | null>;
}) {
  const { t } = useI18n();
  const showTimerRef = useRef<number | undefined>(undefined);
  const roleIndices = useMemo(
    () =>
      messages
        .map((m, i) => (m.role === role ? i : -1))
        .filter((i) => i >= 0),
    [messages, role],
  );

  const [activeIdx, setActiveIdx] = useState<number | null>(null);
  const activeRef = useRef<number | null>(null);
  const [positions, setPositions] = useState<Map<number, number>>(new Map());
  const [navVisible, setNavVisible] = useState(false);

  useEffect(() => {
    return () => {
      if (showTimerRef.current !== undefined) {
        window.clearTimeout(showTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp || roleIndices.length === 0) {
      setActiveIdx(null);
      activeRef.current = null;
      setPositions(new Map());
      return;
    }
    let measureFrame: number | null = null;

    // 滞回判定:进入线在视口顶部下方 1/4,退出线在 3/4,中间为死区
    // 向下滚:下一条顶端越过 1/4 → 切到下一条(此时它已占视口 ≈ 3/4)
    // 向上滚:当前条顶端退过 3/4 → 回到上一条(此时下一条只剩 ≈ 1/4)
    const computeActive = () => {
      const vpRect = vp.getBoundingClientRect();
      const enter = vpRect.top + vpRect.height * 0.25;
      const exit = vpRect.top + vpRect.height * 0.75;
      const atBottom =
        vp.scrollTop + vp.clientHeight >= vp.scrollHeight - 1;
      const atTop = vp.scrollTop <= 0;

      let active = activeRef.current;
      if (active === null || !roleIndices.includes(active)) {
        // 初始化沿用单线规则:取顶端已越过 enter 线的最后一条
        let init: number | null = null;
        for (const idx of roleIndices) {
          const el = refs.current[idx];
          if (!el) continue;
          if (el.getBoundingClientRect().top <= enter) init = idx;
          else break;
        }
        active = init ?? roleIndices[0];
      } else {
        // 向下推进:跳跃式滚动可能一次跨过多条
        const pos = roleIndices.indexOf(active);
        for (let i = pos + 1; i < roleIndices.length; i++) {
          const el = refs.current[roleIndices[i]];
          if (!el) break;
          if (el.getBoundingClientRect().top <= enter) active = roleIndices[i];
          else break;
        }
        // 向上回退:同样支持连续回退多条
        while (true) {
          const i = roleIndices.indexOf(active);
          if (i <= 0) break;
          const el = refs.current[active];
          if (!el) break;
          if (el.getBoundingClientRect().top > exit) active = roleIndices[i - 1];
          else break;
        }
      }
      // 已经滚到底部:最后一条若因后续内容不足无法越过 enter,强制置为 active
      if (atBottom) active = roleIndices[roleIndices.length - 1];
      // 已经滚到顶部:第一条若因前面内容不足无法把第二条挤出 exit,强制置为 active
      if (atTop) active = roleIndices[0];
      activeRef.current = active;
      setActiveIdx(active);
    };

    const computePositions = () => {
      const sh = vp.scrollHeight;
      if (sh <= 0) return;
      const vpRect = vp.getBoundingClientRect();
      const m = new Map<number, number>();
      for (const idx of roleIndices) {
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
    const scheduleMeasure = () => {
      if (measureFrame !== null) return;
      measureFrame = window.requestAnimationFrame(() => {
        measureFrame = null;
        computePositions();
        computeActive();
      });
    };
    const ro = new ResizeObserver(() => {
      scheduleMeasure();
    });
    ro.observe(vp);
    for (const child of Array.from(vp.children)) ro.observe(child);
    return () => {
      vp.removeEventListener("scroll", computeActive);
      if (measureFrame !== null) window.cancelAnimationFrame(measureFrame);
      ro.disconnect();
    };
  }, [viewportRef, refs, roleIndices]);

  if (roleIndices.length === 0) return null;
  const isLeft = side === "left";
  const handleWheel = (event: React.WheelEvent<HTMLDivElement>) => {
    const vp = viewportRef.current;
    if (!vp) return;
    event.preventDefault();
    const unit =
      event.deltaMode === 1
        ? 16
        : event.deltaMode === 2
          ? vp.clientHeight
          : 1;
    vp.scrollTop += event.deltaY * unit;
    vp.scrollLeft += event.deltaX * unit;
  };
  const handleMouseEnter = () => {
    if (showTimerRef.current !== undefined) {
      window.clearTimeout(showTimerRef.current);
    }
    showTimerRef.current = window.setTimeout(() => {
      showTimerRef.current = undefined;
      setNavVisible(true);
    }, ROLE_NAV_SHOW_DELAY_MS);
  };
  const handleMouseLeave = () => {
    if (showTimerRef.current !== undefined) {
      window.clearTimeout(showTimerRef.current);
      showTimerRef.current = undefined;
    }
    setNavVisible(false);
  };
  return (
    <div
      onWheel={handleWheel}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      className={
        "group/role-nav absolute top-2 bottom-2 z-10 w-10 " +
        (isLeft ? "left-0" : "right-0")
      }
    >
      {roleIndices.map((idx) => {
        const ratio = positions.get(idx);
        if (ratio === undefined) return null;
        const cleaned = previewTextForRole(messages[idx]);
        const preview = cleaned.replace(/\s+/g, " ").trim().slice(0, 200);
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
            {cleaned.length > 200 ? "…" : ""}
          </div>
        );
        return (
          <Tooltip
            key={idx}
            content={tip}
            placement={isLeft ? "right" : "left"}
            offset={12}
            delayMs={100}
          >
            <button
              type="button"
              onClick={() =>
                refs.current[idx]?.scrollIntoView({
                  behavior: "smooth",
                  block: "start",
                })
              }
              style={{ top: `${ratio * 100}%`, transform: "translateY(-50%)" }}
              className={
                "group absolute cursor-pointer p-1.5 transition-opacity duration-150 focus-visible:opacity-100 " +
                (navVisible ? "opacity-100 " : "pointer-events-none opacity-0 ") +
                (isLeft ? "left-1.5" : "right-1.5")
              }
              aria-label={t(
                role === "assistant"
                  ? "detail.jump_to_assistant_msg"
                  : "detail.jump_to_user_msg",
                { n: idx + 1 },
              )}
            >
              <span
                className={
                  "block w-1.5 h-1.5 rounded-full transition-[background-color,transform,opacity] duration-150 ease-out group-focus-visible:translate-x-0 " +
                  (navVisible
                    ? "translate-x-0 "
                    : isLeft
                      ? "-translate-x-1 "
                      : "translate-x-1 ") +
                  " " +
                  (idx === activeIdx
                    ? "bg-ink scale-100 group-focus-visible:scale-125 " +
                      (navVisible ? "scale-125" : "")
                    : "bg-ink/25 group-hover:bg-ink group-focus-visible:scale-100 " +
                      (navVisible ? "scale-100" : "scale-75"))
                }
              />
            </button>
          </Tooltip>
        );
      })}
    </div>
  );
}

function previewTextForRole(message: SessionMessage): string {
  return message.role === "user"
    ? stripInjectedContext(message.text)
    : stripImagePlaceholders(message.text);
}

function findScroller(el: HTMLElement | null): HTMLElement | null {
  let node = el?.parentElement ?? null;
  while (node) {
    const oy = getComputedStyle(node).overflowY;
    if (oy === "auto" || oy === "scroll") return node;
    node = node.parentElement;
  }
  return null;
}

const MessageBubble = memo(function MessageBubble({
  msg,
  toolResult,
  onPreviewImage,
}: {
  msg: SessionMessage;
  toolResult?: SessionMessage;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  const { lang, t } = useI18n();
  const LONG_TOOL_THRESHOLD = 500;
  const MESSAGE_LINE_LIMIT = 20;
  const thinkingCollapsible = msg.role === "thinking";
  const todoCollapsible = msg.role === "todo";
  const meta = messageMeta(msg);
  const bodyText = meta.bodyText;
  const userMedia = useMemo(
    () => (msg.role === "user" ? splitMarkdownImages(bodyText) : null),
    [bodyText, msg.role],
  );
  const renderText = userMedia?.text ?? bodyText;
  const renderLines = useMemo(() => renderText.split(/\r?\n/), [renderText]);
  const conversationLineCollapsible =
    (msg.role === "user" || msg.role === "assistant") &&
    renderLines.length > MESSAGE_LINE_LIMIT;
  const collapseText = msg.text + (toolResult?.text ?? "");
  const pairedToolCollapsible = Boolean(toolResult);
  const longCollapsible =
    !pairedToolCollapsible &&
    !thinkingCollapsible &&
    !todoCollapsible &&
    msg.role !== "user" &&
    msg.role !== "assistant" &&
    collapseText.length > LONG_TOOL_THRESHOLD;
  const collapsible =
    thinkingCollapsible ||
    todoCollapsible ||
    conversationLineCollapsible ||
    longCollapsible ||
    pairedToolCollapsible;
  const [collapsed, setCollapsed] = useState(collapsible);
  const previewSource =
    msg.role === "user" ? stripInjectedContext(msg.text) : msg.text;
  const preview = collapsible
    ? previewSource.replace(/\s+/g, " ").slice(0, 200)
    : "";
  const bubbleRef = useRef<HTMLDivElement>(null);
  const anchorTopRef = useRef<number | null>(null);

  useEffect(() => {
    setCollapsed(collapsible);
  }, [collapsible, msg.role, msg.text]);

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

  const handleBlockClick = (event: React.MouseEvent<HTMLDivElement>) => {
    if (!collapsible) return;
    const target = event.target as HTMLElement | null;
    if (
      target?.closest(
        "button,a,input,textarea,select,summary,label,[data-no-toggle]",
      )
    ) {
      return;
    }
    toggle();
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

  const toolSummary = isToolCallRole(msg.role) ? parseToolSummary(msg.text) : null;
  const visibleRenderText =
    conversationLineCollapsible && collapsed
      ? renderLines.slice(0, MESSAGE_LINE_LIMIT).join("\n")
      : renderText;
  const contentClass =
    toolResult && isToolCallRole(msg.role) ? "text-body-sm" : meta.contentClass;

  if (msg.role === "file_edit") {
    return (
      <div ref={bubbleRef} className="text-body leading-relaxed break-words py-1">
        <FileEditContent text={msg.text} />
      </div>
    );
  }

  return (
    <div
      ref={bubbleRef}
      onClick={handleBlockClick}
      className={
        "text-body leading-relaxed break-words " +
        (msg.role === "user"
          ? "w-fit max-w-[75%] rounded-lg px-4 py-3 bg-ink/[0.06] border border-ink/[0.04]"
          : meta.compact
            ? "py-1.5"
            : "px-0 py-1")
        + (collapsible ? " cursor-pointer select-none" : "")
      }
    >
      <div
        className={
          "flex items-center gap-2 leading-none " +
          (meta.compact ? "mb-1 " : "mb-2 ") +
          (collapsible
            ? "hover:text-ink/70"
            : "")
        }
        role={collapsible ? "button" : undefined}
        aria-expanded={collapsible ? !collapsed : undefined}
      >
        {meta.compact && (
          <span
            className={
              "h-1.5 w-1.5 shrink-0 rounded-full " +
              (meta.tone === "tool"
                ? "bg-[rgb(var(--color-emerald)/0.7)]"
                : "bg-ink/45")
            }
          />
        )}
        <span
          className={
            "text-caption font-medium " +
            (meta.compact
              ? "normal-case text-ink/65"
              : "uppercase text-ink/40")
          }
        >
          {meta.label}
        </span>
        {toolSummary?.description && (
          <span className="text-caption text-ink/45">
            {toolSummary.description}
          </span>
        )}
        {(thinkingCollapsible || todoCollapsible) && (
          collapsed ? (
            <ChevronRight className="w-3.5 h-3.5 text-ink/35" />
          ) : (
            <ChevronDown className="w-3.5 h-3.5 text-ink/35" />
          )
        )}
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
      <div
        className={
          contentClass +
          (meta.compact ? " ml-3.5" : "") +
          (meta.compact && !renderText.trim() ? " hidden" : "")
        }
      >
        {longCollapsible && collapsed ? (
          <span className="text-ink/60">
            {preview}
            {previewSource.length > 200 ? "…" : ""}
          </span>
        ) : (thinkingCollapsible || todoCollapsible) && collapsed ? null : (
          <>
            {userMedia && userMedia.images.length > 0 && (
              <MarkdownImageStrip
                images={userMedia.images}
                align="right"
                onPreviewImage={onPreviewImage}
              />
            )}
            {toolResult ? (
              <ToolPairContent
                input={renderText}
                output={toolResult.text}
                collapsed={collapsed}
                onPreviewImage={onPreviewImage}
              />
            ) : meta.renderMode === "file_edit" ? (
              <FileEditContent text={visibleRenderText} />
            ) : meta.renderMode === "todo" ? (
              <TodoContent text={visibleRenderText} />
            ) : meta.renderMode === "plain" ? (
              <PlainTextContent text={visibleRenderText} />
            ) : (
              <MarkdownContent
                text={visibleRenderText}
                onPreviewImage={onPreviewImage}
              />
            )}
            {conversationLineCollapsible && (
              <button
                type="button"
                className="mt-2 flex items-center gap-1 text-left text-body-sm text-ink/60 hover:text-ink/85"
                data-no-toggle
                onClick={toggle}
              >
                <span>{t(collapsed ? "detail.expand" : "detail.collapse")}</span>
                <ChevronDown
                  className={
                    "h-3.5 w-3.5 transition-transform " +
                    (collapsed ? "" : "rotate-180")
                  }
                />
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
});

function isConversationRole(role: string): boolean {
  return [
    "user",
    "assistant",
    "thinking",
    "file_edit",
    "tool",
    "tool_call",
    "tool_use",
    "function_call",
    "tool_result",
    "function_call_output",
    "todo",
  ].includes(role);
}

interface MessageRenderItem {
  key: string;
  message: SessionMessage;
  toolResult?: SessionMessage;
}

function pairToolMessages(
  entries: { m: SessionMessage; srcIdx: number }[],
): MessageRenderItem[] {
  const consumedResults = new Set<number>();
  const items: MessageRenderItem[] = [];

  for (let i = 0; i < entries.length; i += 1) {
    const { m, srcIdx } = entries[i];
    if (isToolCallRole(m.role)) {
      const resultIdx = findToolResultIndex(entries, i);
      if (resultIdx !== null) {
        consumedResults.add(resultIdx);
      }
      items.push({
        key: `${srcIdx}:${m.toolCallId ?? "tool"}`,
        message: m,
        toolResult: resultIdx !== null ? entries[resultIdx].m : undefined,
      });
      continue;
    }
    if (isToolResultRole(m.role)) {
      if (consumedResults.has(i)) continue;
      items.push({
        key: `${srcIdx}:${m.toolCallId ?? "result"}`,
        message: m,
      });
      continue;
    }
    items.push({ key: String(srcIdx), message: m });
  }

  return items;
}

function moveFileEditsToTurnEnd(items: MessageRenderItem[]): MessageRenderItem[] {
  const out: MessageRenderItem[] = [];
  let turn: MessageRenderItem[] = [];
  const flush = () => {
    if (turn.length === 0) return;
    const edits = turn.filter((item) => item.message.role === "file_edit");
    const rest = turn.filter((item) => item.message.role !== "file_edit");
    const mergedEdit = mergeFileEditItems(edits);
    out.push(...rest);
    if (mergedEdit) out.push(mergedEdit);
    turn = [];
  };
  for (const item of items) {
    if (item.message.role === "user" && turn.length > 0) {
      flush();
    }
    turn.push(item);
  }
  flush();
  return out;
}

function mergeFileEditItems(items: MessageRenderItem[]): MessageRenderItem | null {
  if (items.length === 0) return null;
  const summaries = items
    .map((item) => parseFileEditSummary(item.message.text))
    .filter((summary): summary is FileEditSummary => Boolean(summary));
  if (summaries.length === 0) return items[items.length - 1];
  const byPath = new Map<string, FileEditItem>();
  for (const summary of summaries) {
    for (const edit of summary.edits ?? []) {
      const key = edit.path || edit.displayPath || "(unknown file)";
      const existing = byPath.get(key);
      if (existing) {
        existing.additions = (existing.additions ?? 0) + (edit.additions ?? 0);
        existing.deletions = (existing.deletions ?? 0) + (edit.deletions ?? 0);
        existing.kind = existing.kind === edit.kind ? existing.kind : "mixed";
        existing.detail = mergeEditDetail(existing.detail, edit.detail);
        existing.patches = [
          ...normalizeEditPatches(existing),
          ...normalizeEditPatches(edit),
        ];
        existing.patch = undefined;
        existing.contentDiffs = [
          ...normalizeContentDiffs(existing),
          ...normalizeContentDiffs(edit),
        ];
        existing.oldContent = undefined;
        existing.newContent = undefined;
      } else {
        byPath.set(key, { ...edit });
      }
    }
  }
  const edits = Array.from(byPath.values());
  const first = items[0].message;
  const last = items[items.length - 1].message;
  const source = summaries.find((summary) => summary.source)?.source ?? "session";
  const text = JSON.stringify({
    source,
    files: edits.length,
    additions: sumEditNumber(edits, "additions"),
    deletions: sumEditNumber(edits, "deletions"),
    edits,
  });
  return {
    key: `${items[0].key}:merged-file-edits`,
    message: {
      ...first,
      role: "file_edit",
      text,
      timestamp: last.timestamp ?? first.timestamp,
    },
  };
}

function findToolResultIndex(
  entries: { m: SessionMessage; srcIdx: number }[],
  callIndex: number,
): number | null {
  const call = entries[callIndex].m;
  if (call.toolCallId) {
    for (let i = callIndex + 1; i < entries.length; i += 1) {
      const candidate = entries[i].m;
      if (isToolResultRole(candidate.role) && candidate.toolCallId === call.toolCallId) {
        return i;
      }
      if (isToolCallRole(candidate.role) && candidate.toolCallId === call.toolCallId) {
        break;
      }
    }
    return null;
  }
  const next = entries[callIndex + 1]?.m;
  return next && isToolResultRole(next.role) && !next.toolCallId
    ? callIndex + 1
    : null;
}

function isToolCallRole(role: string): boolean {
  return ["tool", "tool_call", "tool_use", "function_call"].includes(role);
}

function isToolResultRole(role: string): boolean {
  return ["tool_result", "function_call_output"].includes(role);
}

function messageMeta(msg: SessionMessage): {
  label: string;
  bodyText: string;
  compact: boolean;
  contentClass: string;
  tone: "normal" | "thinking" | "tool";
  renderMode: "markdown" | "plain" | "todo" | "file_edit";
} {
  const role = msg.role;
  if (role === "file_edit") {
    return {
      label: "Edited Files",
      bodyText: msg.text,
      compact: false,
      contentClass: "text-ink/70 text-body-sm",
      tone: "tool",
      renderMode: "file_edit",
    };
  }
  if (role === "user") {
    return {
      label: role,
      bodyText: stripInjectedContext(msg.text),
      compact: false,
      contentClass: "text-ink/85",
      tone: "normal",
      renderMode: "markdown",
    };
  }
  if (role === "thinking") {
    return {
      label: "Thinking",
      bodyText: msg.text,
      compact: true,
      contentClass: "text-ink/55 text-body-sm",
      tone: "thinking",
      renderMode: "markdown",
    };
  }
  if (["tool", "tool_call", "tool_use", "function_call"].includes(role)) {
    const parsed = parseToolCall(msg.text);
    return {
      label: parsed.name,
      bodyText: parsed.body,
      compact: true,
      contentClass:
        "text-ink/80 text-body-sm bg-ink/[0.055] border border-ink/[0.08] rounded-md px-3 py-2 overflow-hidden",
      tone: "tool",
      renderMode: "plain",
    };
  }
  if (["tool_result", "function_call_output"].includes(role)) {
    return {
      label: "Tool Result",
      bodyText: msg.text,
      compact: true,
      contentClass:
        "text-ink/70 text-body-sm bg-ink/[0.04] border border-ink/[0.06] rounded-md px-3 py-2 overflow-hidden",
      tone: "tool",
      renderMode: "plain",
    };
  }
  if (role === "todo") {
    return {
      label: "Update Todos",
      bodyText: msg.text,
      compact: true,
      contentClass: "text-ink/45 text-body-sm",
      tone: "tool",
      renderMode: "todo",
    };
  }
  return {
    label: role,
    bodyText: msg.text,
    compact: false,
    contentClass: "text-ink/85",
    tone: "normal",
    renderMode: "markdown",
  };
}

function parseToolCall(text: string): { name: string; body: string } {
  const m = text.match(/^\[([^\]\n]+)\]\s*\n?([\s\S]*)$/);
  if (!m) return { name: "Tool Use", body: text };
  return { name: m[1], body: m[2] ?? "" };
}

function parseToolSummary(text: string): { description: string | null } {
  const parsed = parseToolCall(text);
  try {
    const value = JSON.parse(parsed.body) as unknown;
    if (value && typeof value === "object" && !Array.isArray(value)) {
      const description = (value as Record<string, unknown>).description;
      if (typeof description === "string" && description.trim()) {
        return { description };
      }
    }
  } catch {
    // Non-JSON tool calls are still valid; they just do not have a summary.
  }
  return { description: null };
}

interface FileEditSummary {
  source?: string;
  files?: number;
  additions?: number;
  deletions?: number;
  edits?: FileEditItem[];
}

interface FileEditItem {
  path?: string;
  displayPath?: string;
  kind?: string;
  additions?: number;
  deletions?: number;
  detail?: string;
  patch?: string | null;
  patches?: string[];
  oldContent?: string | null;
  newContent?: string | null;
  contentDiffs?: FileEditContentDiff[];
}

interface FileEditContentDiff {
  oldContent?: string | null;
  newContent?: string | null;
}

function FileEditContent({ text }: { text: string }) {
  const summary = parseFileEditSummary(text);
  if (!summary) return <PlainTextContent text={text} />;
  const edits = summary.edits ?? [];
  const fileCount = summary.files ?? edits.length;
  const additions = summary.additions ?? sumEditNumber(edits, "additions");
  const deletions = summary.deletions ?? sumEditNumber(edits, "deletions");
  const [expandedState, setExpandedState] = useState(() => ({
    text,
    expanded: edits.length <= 3,
  }));
  const expanded =
    expandedState.text === text ? expandedState.expanded : edits.length <= 3;
  const [openDetails, setOpenDetails] = useState<Set<string>>(() => new Set());
  const pendingScrollKeyRef = useRef<string | null>(null);
  const detailRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const visibleEdits = expanded ? edits : edits.slice(0, 3);
  const hiddenCount = Math.max(0, edits.length - visibleEdits.length);
  const setExpanded = (nextExpanded: boolean) => {
    setExpandedState({ text, expanded: nextExpanded });
  };

  useEffect(() => {
    setOpenDetails(new Set());
    pendingScrollKeyRef.current = null;
    detailRefs.current.clear();
  }, [text]);

  const toggleDetail = (key: string) => {
    setOpenDetails((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        pendingScrollKeyRef.current = key;
        next.add(key);
      }
      return next;
    });
  };

  useLayoutEffect(() => {
    const key = pendingScrollKeyRef.current;
    if (!key || !openDetails.has(key)) return;
    pendingScrollKeyRef.current = null;
    const node = detailRefs.current.get(key);
    if (!node) return;
    window.requestAnimationFrame(() => {
      const scroller = findScroller(node);
      if (!scroller) {
        node.scrollIntoView({ block: "end", behavior: "smooth" });
        return;
      }
      const nodeBottom = node.getBoundingClientRect().bottom;
      const scrollerBottom = scroller.getBoundingClientRect().bottom;
      const delta = nodeBottom - scrollerBottom + 12;
      if (delta > 0) {
        scroller.scrollTo({
          top: scroller.scrollTop + delta,
          behavior: "smooth",
        });
      }
    });
  }, [openDetails]);

  return (
    <div className="overflow-hidden rounded-md bg-ink/[0.035]">
      <div className="flex items-center justify-between gap-3 px-2.5 py-2">
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-bg-panel text-ink/70">
            <FileDiff className="h-4 w-4" />
          </span>
          <div className="min-w-0">
            <div className="text-body-sm font-medium text-ink/80">
              Edited {fileCount} {fileCount === 1 ? "file" : "files"}
            </div>
            <div className="font-mono text-caption leading-tight">
              <span className="text-[rgb(var(--color-emerald))]">
                +{additions}
              </span>
              <span className="text-ink/25"> </span>
              <span className="text-status-error">-{deletions}</span>
            </div>
          </div>
        </div>
      </div>
      {edits.length > 0 && (
        <div className="border-t border-ink/[0.07]">
          {visibleEdits.map((edit, i) => {
            const label = edit.displayPath || edit.path || "(unknown file)";
            const detailKey = edit.path || edit.displayPath || `${label}-${i}`;
            const detail = typeof edit.detail === "string" ? edit.detail : "";
            const hasDetail = hasRenderableEditDetail(edit);
            const detailOpen = openDetails.has(detailKey);
            const rowContent = (
              <>
                <span className="min-w-0 truncate text-ink/80">
                  {label}
                </span>
                <div className="flex shrink-0 items-center gap-2">
                  <span className="font-mono text-caption">
                    <span className="text-[rgb(var(--color-emerald))]">
                      +{edit.additions ?? 0}
                    </span>
                    <span className="text-ink/25"> </span>
                    <span className="text-status-error">
                      -{edit.deletions ?? 0}
                    </span>
                  </span>
                  {hasDetail && (
                    <ChevronDown
                      className={
                        "h-3.5 w-3.5 text-ink/55 transition-transform " +
                        (detailOpen ? "rotate-180" : "")
                      }
                    />
                  )}
                </div>
              </>
            );
            return (
              <div key={`${label}-${i}`}>
                {hasDetail ? (
                  <button
                    type="button"
                    className="grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-2.5 py-1.5 text-left text-body-sm hover:bg-ink/[0.04]"
                    data-no-toggle
                    aria-expanded={detailOpen}
                    aria-label={
                      detailOpen
                        ? `Hide changes for ${label}`
                        : `Show changes for ${label}`
                    }
                    onClick={() => toggleDetail(detailKey)}
                  >
                    {rowContent}
                  </button>
                ) : (
                  <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-2.5 py-1.5 text-body-sm">
                    {rowContent}
                  </div>
                )}
                {detailOpen && hasDetail && (
                  <div
                    ref={(node) => {
                      if (node) {
                        detailRefs.current.set(detailKey, node);
                      } else {
                        detailRefs.current.delete(detailKey);
                      }
                    }}
                  >
                    <DiffPreview edit={edit} fallback={detail} />
                  </div>
                )}
              </div>
            );
          })}
          {hiddenCount > 0 && (
            <button
              type="button"
              className="flex w-full items-center gap-1 px-2.5 py-1.5 text-left text-body-sm text-ink/75 hover:bg-ink/[0.04]"
              data-no-toggle
              onClick={() => setExpanded(true)}
            >
              <span>
                Show {hiddenCount} more {hiddenCount === 1 ? "file" : "files"}
              </span>
              <ChevronDown className="h-3.5 w-3.5" />
            </button>
          )}
          {expanded && edits.length > 3 && (
            <button
              type="button"
              className="flex w-full items-center gap-1 px-2.5 py-1.5 text-left text-body-sm text-ink/75 hover:bg-ink/[0.04]"
              data-no-toggle
              onClick={() => setExpanded(false)}
            >
              <span>Collapse files</span>
              <ChevronDown className="h-3.5 w-3.5 rotate-180" />
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function parseFileEditSummary(text: string): FileEditSummary | null {
  try {
    const parsed = JSON.parse(text) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return null;
    }
    const record = parsed as FileEditSummary;
    return Array.isArray(record.edits) ? record : null;
  } catch {
    return null;
  }
}

function sumEditNumber(edits: FileEditItem[], key: "additions" | "deletions"): number {
  return edits.reduce((sum, edit) => {
    const value = edit[key];
    return sum + (typeof value === "number" ? value : 0);
  }, 0);
}

function hasRenderableEditDetail(edit: FileEditItem): boolean {
  return Boolean(
    (typeof edit.patch === "string" && edit.patch.trim()) ||
      normalizeEditPatches(edit).length > 0 ||
      (typeof edit.detail === "string" && edit.detail.trim()) ||
      normalizeContentDiffs(edit).length > 0 ||
      typeof edit.oldContent === "string" ||
      typeof edit.newContent === "string",
  );
}

function diffPreviewOptions(themeType: "light" | "dark") {
  return {
  diffStyle: "unified" as const,
  overflow: "scroll" as const,
  theme: {
    dark: "github-dark",
    light: "github-light",
  },
    themeType,
  disableFileHeader: true,
  hunkSeparators: "line-info-basic" as const,
  lineDiffType: "word" as const,
  };
}

function DiffPreview({
  edit,
  fallback,
}: {
  edit: FileEditItem;
  fallback: string;
}) {
  const name = edit.displayPath || edit.path || "file";
  const themeType = useEffectiveThemeType();
  const options = useMemo(() => diffPreviewOptions(themeType), [themeType]);
  const contentDiffs = normalizeContentDiffs(edit);
  const patch = typeof edit.patch === "string" ? edit.patch : "";
  const patches = normalizeEditPatches(edit);
  return (
    <ScrollArea
      className="mx-2.5 mb-2 max-h-72 rounded bg-bg-panel-alt"
      viewportClassName="p-0"
      orientation="both"
      persistScrollbars
    >
      <div className="min-w-max text-caption sessio-diff-preview">
        {patches.length > 0 || patch.trim() ? (
          <>
            {(patches.length > 0 ? patches : [patch]).map((patchItem, i) => (
              <PatchDiff
                key={i}
                patch={patchItem}
                options={options}
                disableWorkerPool
              />
            ))}
          </>
        ) : contentDiffs.length > 0 ? (
          <>
            {contentDiffs.map((contentDiff, i) => (
              <MultiFileDiff
                key={i}
                oldFile={{ name, contents: contentDiff.oldContent ?? "" }}
                newFile={{ name, contents: contentDiff.newContent ?? "" }}
                options={options}
                disableWorkerPool
              />
            ))}
          </>
        ) : (
          <pre className="px-2.5 py-2 font-mono text-caption leading-relaxed text-ink/75">
            <code>{fallback}</code>
          </pre>
        )}
      </div>
    </ScrollArea>
  );
}

function normalizeEditPatches(edit: FileEditItem): string[] {
  const patches = Array.isArray(edit.patches)
    ? edit.patches.filter((item): item is string => Boolean(item.trim()))
    : [];
  if (typeof edit.patch === "string" && edit.patch.trim()) {
    patches.push(edit.patch);
  }
  return patches;
}

function normalizeContentDiffs(edit: FileEditItem): FileEditContentDiff[] {
  const diffs = Array.isArray(edit.contentDiffs)
    ? edit.contentDiffs.filter(
        (item) =>
          typeof item.oldContent === "string" ||
          typeof item.newContent === "string",
      )
    : [];
  if (
    typeof edit.oldContent === "string" ||
    typeof edit.newContent === "string"
  ) {
    diffs.push({
      oldContent: edit.oldContent,
      newContent: edit.newContent,
    });
  }
  return diffs;
}

function useEffectiveThemeType(): "light" | "dark" {
  const [themeType, setThemeType] = useState<"light" | "dark">(() =>
    document.documentElement.getAttribute("data-theme") === "light"
      ? "light"
      : "dark",
  );
  useEffect(() => {
    const root = document.documentElement;
    const update = () => {
      setThemeType(root.getAttribute("data-theme") === "light" ? "light" : "dark");
    };
    update();
    const observer = new MutationObserver(update);
    observer.observe(root, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    return () => observer.disconnect();
  }, []);
  return themeType;
}

function mergeEditDetail(a?: string, b?: string): string | undefined {
  const left = typeof a === "string" ? a.trim() : "";
  const right = typeof b === "string" ? b.trim() : "";
  if (!left) return right || undefined;
  if (!right) return left;
  return `${left}\n\n${right}`;
}

interface MarkdownImage {
  alt: string;
  src: string;
}

function PlainTextContent({ text }: { text: string }) {
  if (!text.trim()) return null;
  return (
    <pre className="overflow-x-auto whitespace-pre-wrap break-words font-mono text-caption leading-relaxed">
      <code>{text}</code>
    </pre>
  );
}

function ToolPairContent({
  input,
  output,
  collapsed,
  onPreviewImage,
}: {
  input: string;
  output: string;
  collapsed: boolean;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  return (
    <div className="overflow-hidden rounded-md border border-ink/[0.08] bg-bg-panel-alt">
      <ToolPairRow
        label="IN"
        text={formatToolInput(input)}
        collapsed={collapsed}
        onPreviewImage={onPreviewImage}
      />
      <div className="border-t border-ink/[0.07]" />
      <ToolPairRow
        label="OUT"
        text={output}
        collapsed={collapsed}
        onPreviewImage={onPreviewImage}
      />
    </div>
  );
}

function ToolPairRow({
  label,
  text,
  collapsed,
  onPreviewImage,
}: {
  label: string;
  text: string;
  collapsed: boolean;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  const media = splitMarkdownImages(text);
  return (
    <div className="grid grid-cols-[2.25rem_minmax(0,1fr)] gap-2 px-3 py-2">
      <div className="font-mono text-[10px] leading-relaxed text-ink/35">
        {label}
      </div>
      <div className="min-w-0">
        {media.images.length > 0 && (
          <div className="mb-1.5 flex flex-wrap gap-2">
            {media.images.map((image, i) => (
              <MarkdownImageButton
                key={`${image.src}-${i}`}
                image={image}
                onPreviewImage={onPreviewImage}
              />
            ))}
          </div>
        )}
        {media.text.trim() && (
          <pre
            className={
              "min-w-0 overflow-x-auto whitespace-pre-wrap break-words font-mono text-caption leading-relaxed text-ink/75 " +
              (collapsed ? "line-clamp-3" : "")
            }
          >
            <code>{media.text}</code>
          </pre>
        )}
      </div>
    </div>
  );
}

function formatToolInput(text: string): string {
  const parsed = parseToolCall(text);
  const body = parsed.body.trim();
  if (!body) return "";
  try {
    const value = JSON.parse(body) as unknown;
    if (value && typeof value === "object" && !Array.isArray(value)) {
      const record = value as Record<string, unknown>;
      const command = record.command ?? record.cmd;
      if (typeof command === "string" && command.trim()) {
        return command;
      }
      const filePath = record.file_path ?? record.path;
      if (typeof filePath === "string" && filePath.trim()) {
        return filePath;
      }
    }
    return JSON.stringify(value, null, 2);
  } catch {
    return body;
  }
}

type TodoStatus = "pending" | "in_progress" | "completed";

interface ClaudeTodo {
  content: string;
  activeForm?: string;
  status: TodoStatus | string;
}

function TodoContent({ text }: { text: string }) {
  const todos = parseTodos(text);
  if (todos.length === 0) return <PlainTextContent text={text} />;
  return (
    <ul className="mt-2 space-y-1.5">
      {todos.map((todo, i) => {
        const completed = todo.status === "completed";
        const active = todo.status === "in_progress";
        return (
          <li
            key={`${todo.content}-${i}`}
            className={
              "flex items-start gap-2 text-caption leading-relaxed " +
              (completed
                ? "text-ink/35 line-through decoration-ink/30"
                : active
                  ? "text-ink/65"
                  : "text-ink/45")
            }
          >
            <span
              className={
                "mt-0.5 inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-sm border text-[10px] leading-none " +
                (completed
                  ? "border-ink/20 bg-ink/[0.035] text-ink/35"
                  : active
                    ? "border-[rgb(var(--color-emerald)/0.45)] bg-[rgb(var(--color-emerald)/0.12)] text-[rgb(var(--color-emerald))]"
                    : "border-ink/18 bg-bg-panel-alt text-transparent")
              }
              aria-hidden="true"
            >
              {completed ? "✓" : active ? "•" : ""}
            </span>
            <span>{todo.content}</span>
          </li>
        );
      })}
    </ul>
  );
}

function parseTodos(text: string): ClaudeTodo[] {
  try {
    const parsed = JSON.parse(text) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.flatMap<ClaudeTodo>((item) => {
      if (!item || typeof item !== "object") return [];
      const record = item as Record<string, unknown>;
      const content = record.content;
      if (typeof content !== "string" || !content.trim()) return [];
      const todo: ClaudeTodo = {
        content,
        status: typeof record.status === "string" ? record.status : "pending",
      };
      if (typeof record.activeForm === "string") {
        todo.activeForm = record.activeForm;
      }
      return [todo];
    });
  } catch {
    return [];
  }
}

function MarkdownContent({
  text,
  onPreviewImage,
}: {
  text: string;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  const safeText = stripImagePlaceholders(text);
  if (!safeText.trim()) return null;
  const media = splitMarkdownImages(safeText);
  const components = useMemo(
    () => createMarkdownComponents(onPreviewImage),
    [onPreviewImage],
  );
  return (
    <div className="markdown-content">
      {media.images.length > 0 && (
        <MarkdownImageStrip
          images={media.images}
          onPreviewImage={onPreviewImage}
        />
      )}
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkBreaks, remarkMath]}
        rehypePlugins={[rehypeRaw, [rehypeSanitize, markdownSanitizeSchema], rehypeKatex]}
        components={components}
        urlTransform={markdownUrlTransform}
      >
        {media.text}
      </ReactMarkdown>
    </div>
  );
}

const markdownSanitizeSchema: SanitizeSchema = {
  ...defaultSchema,
  tagNames: [
    ...(defaultSchema.tagNames ?? []),
    "details",
    "summary",
    "input",
    "section",
    "article",
  ],
  attributes: {
    ...defaultSchema.attributes,
    "*": [
      ...(defaultSchema.attributes?.["*"] ?? []),
      "className",
      "data*",
      "ariaLabel",
      "ariaHidden",
    ],
    a: [
      ...(defaultSchema.attributes?.a ?? []),
      "href",
      "title",
      "target",
      "rel",
    ],
    img: [
      ...(defaultSchema.attributes?.img ?? []),
      "alt",
      "src",
      "title",
      "width",
      "height",
    ],
    input: [["type", "checkbox"], "checked", "disabled"],
    code: [...(defaultSchema.attributes?.code ?? []), "className"],
    pre: [...(defaultSchema.attributes?.pre ?? []), "className"],
    span: [...(defaultSchema.attributes?.span ?? []), "className"],
    div: [...(defaultSchema.attributes?.div ?? []), "className"],
  },
  protocols: {
    ...defaultSchema.protocols,
    href: ["http", "https", "mailto"],
    src: ["http", "https", "data", "asset", "blob"],
  },
};

function createMarkdownComponents(
  onPreviewImage: (image: MarkdownImage) => void,
): Components {
  return {
    p: ({ children }) => <p className="my-2 first:mt-0 last:mb-0">{children}</p>,
    h1: ({ children }) => (
      <h1 className="font-semibold text-ink mt-3 mb-1 first:mt-0">{children}</h1>
    ),
    h2: ({ children }) => (
      <h2 className="font-semibold text-ink mt-3 mb-1 first:mt-0">{children}</h2>
    ),
    h3: ({ children }) => (
      <h3 className="font-semibold text-ink mt-3 mb-1 first:mt-0">{children}</h3>
    ),
    h4: ({ children }) => (
      <h4 className="font-semibold text-ink mt-3 mb-1 first:mt-0">{children}</h4>
    ),
    blockquote: ({ children }) => (
      <blockquote className="border-l-2 border-ink/20 pl-3 my-2 text-ink/65">
        {children}
      </blockquote>
    ),
    ul: ({ children }) => <ul className="list-disc pl-5 my-2 space-y-1">{children}</ul>,
    ol: ({ children }) => <ol className="list-decimal pl-5 my-2 space-y-1">{children}</ol>,
    li: ({ children }) => <li>{children}</li>,
    hr: () => <hr className="border-ink/10 my-3" />,
    pre: ({ children }) => (
      <pre className="overflow-x-auto rounded-md bg-bg-panel-alt border border-ink/[0.08] px-3 py-2 text-caption leading-relaxed my-2">
        {children}
      </pre>
    ),
    code: ({ children, className }) => {
      if (className) {
        return <code className={className}>{children}</code>;
      }
      return (
        <code className="rounded bg-ink/[0.08] px-1 py-0.5 font-mono text-[0.92em] text-ink">
          {children}
        </code>
      );
    },
    a: ({ children, href }) => {
      const safe = safeHref(href ?? "");
      if (!safe) return <>{children}</>;
      return (
        <a
          href={safe}
          target="_blank"
          rel="noreferrer"
          className="text-[rgb(var(--color-blue))] underline underline-offset-2"
        >
          {children}
        </a>
      );
    },
    img: ({ src, alt }) => (
      <MarkdownImageButton
        image={{ src: src ?? "", alt: alt ?? "image" }}
        onPreviewImage={onPreviewImage}
      />
    ),
  };
}

function MarkdownImageStrip({
  images,
  align,
  onPreviewImage,
}: {
  images: MarkdownImage[];
  align?: "left" | "right";
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  return (
    <div
      className={
        "mb-3 flex flex-wrap gap-2 " +
        (align === "right" ? "justify-end" : "")
      }
    >
      {images.map((image, i) => (
        <MarkdownImageButton
          key={`${image.src}-${i}`}
          image={image}
          onPreviewImage={onPreviewImage}
        />
      ))}
    </div>
  );
}

function MarkdownImageButton({
  image,
  onPreviewImage,
}: {
  image: MarkdownImage;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  const resolvedSrc = useResolvedImageSrc(image.src);
  const previewImage = useMemo(
    () => ({ ...image, src: resolvedSrc }),
    [image, resolvedSrc],
  );
  return (
    <button
      type="button"
      onClick={() => onPreviewImage(previewImage)}
      className="my-1 block overflow-hidden rounded-md border border-ink/10 bg-bg-panel-alt hover:border-ink/25 focus:outline-none focus:ring-2 focus:ring-ink/20 transition"
      title={image.alt}
    >
      <img
        src={resolvedSrc}
        alt={image.alt}
        className="h-28 w-36 object-contain"
        loading="lazy"
      />
    </button>
  );
}

function ImagePreviewOverlay({
  image,
  onClose,
}: {
  image: MarkdownImage;
  onClose: () => void;
}) {
  const src = useResolvedImageSrc(image.src);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
    >
      <div className="max-h-full max-w-full" onClick={(e) => e.stopPropagation()}>
        <img
          src={src}
          alt={image.alt}
          className="max-h-[calc(100vh-48px)] max-w-[calc(100vw-48px)] rounded-md bg-bg-panel-alt object-contain shadow-2xl"
        />
      </div>
    </div>
  );
}

function splitMarkdownImages(text: string): { text: string; images: MarkdownImage[] } {
  const images: MarkdownImage[] = [];
  const stripped = stripImagePlaceholders(text)
    .split(/\r?\n/)
    .filter((line) => {
      const image = parseStandaloneMarkdownImage(line);
      if (!image) return true;
      images.push(image);
      return false;
    })
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  return { text: stripped, images };
}

function parseStandaloneMarkdownImage(line: string): MarkdownImage | null {
  const trimmed = line.trim();
  const match = trimmed.match(/^!\[([^\]]*)\]\((.+)\)$/);
  if (!match) return null;
  const rawSrc = match[2].trim();
  const withoutTitle = rawSrc.replace(/\s+"[^"]*"$/, "").trim();
  const src = withoutTitle.replace(/^<|>$/g, "");
  if (!src) return null;
  return { alt: match[1] || "image", src };
}

function markdownUrlTransform(url: string): string {
  const raw = url.trim().replace(/^<|>$/g, "");
  if (/^(https?:|mailto:|data:|asset:|blob:)/i.test(raw)) return raw;
  if (/^\/|^[A-Za-z]:[\\/]/.test(raw)) return convertFileSrc(raw);
  return "";
}

function useResolvedImageSrc(rawSrc: string): string {
  const fallback = useMemo(() => resolveImageSrc(rawSrc), [rawSrc]);
  const [src, setSrc] = useState(fallback);

  useEffect(() => {
    let cancelled = false;
    setSrc(fallback);
    const localPath = localImagePath(rawSrc);
    if (!localPath) return;
    readLocalImageDataUrl(localPath)
      .then((dataUrl) => {
        if (!cancelled) setSrc(dataUrl);
      })
      .catch(() => {
        if (!cancelled) setSrc(fallback);
      });
    return () => {
      cancelled = true;
    };
  }, [fallback, rawSrc]);

  return src;
}

function resolveImageSrc(rawSrc: string): string {
  const src = rawSrc.trim().replace(/^<|>$/g, "");
  if (/^(https?:|data:|asset:|blob:)/i.test(src)) return src;
  if (/^\/|^[A-Za-z]:[\\/]/.test(src)) return convertFileSrc(src);
  return src;
}

function localImagePath(rawSrc: string): string | null {
  const src = rawSrc.trim().replace(/^<|>$/g, "");
  if (/^\/|^[A-Za-z]:[\\/]/.test(src)) return src;
  return null;
}

function safeHref(rawHref: string): string | null {
  const href = rawHref.trim().replace(/^<|>$/g, "");
  if (/^(https?:|mailto:)/i.test(href)) return href;
  return null;
}
