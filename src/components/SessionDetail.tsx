import {
  startTransition,
  forwardRef,
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
import { ArrowUp, ChevronDown, ChevronRight, FileDiff, Plus, Square } from "lucide-react";
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
  RuntimeError,
  SubagentInfo,
  ensureAgentRuntimeSession,
  getSessionMessages,
  readLocalImageDataUrl,
  cancelAgentTurn,
  respondAgentPermission,
  sendAgentInput,
  updateSessionMessageCount,
} from "../api";
import ScrollArea from "./ScrollArea";
import Tooltip from "./Tooltip";
import { localeTag, useI18n } from "../i18n";
import type { ViewMode } from "../App";
import {
  type AcpContentBlock,
  type AcpPermissionRequest,
  type AcpRenderBlock,
  type AcpToolCall,
  type LiveRuntimeAction,
  type LiveRuntimeState,
  type LiveRuntimeSession,
  type LiveTurn,
} from "../runtimeChat";

interface Props {
  session: SessionInfo;
  viewMode: ViewMode;
  liveState: LiveRuntimeState;
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onMessageCount: (
    agent: SessionInfo["agent"],
    filePath: string,
    sessionId: string,
    count: number,
  ) => boolean;
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
const BOTTOM_FOLLOW_THRESHOLD_PX = 24;
const PROGRAMMATIC_SCROLL_SETTLE_MS = 120;

function messageSourceKey(agent: SessionInfo["agent"], filePath: string, sessionId: string): string {
  return `${agent}:${sessionId}:${filePath}`;
}

function isNearScrollBottom(vp: HTMLDivElement): boolean {
  return (
    vp.scrollTop + vp.clientHeight >=
    vp.scrollHeight - BOTTOM_FOLLOW_THRESHOLD_PX
  );
}

export default function SessionDetail({
  session,
  viewMode,
  liveState,
  dispatchLiveEvent,
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
          liveState={liveState}
          dispatchLiveEvent={dispatchLiveEvent}
          onPreviewImage={setPreviewImage}
          onMessageCount={onMessageCount}
          messageCount={activeMessageMeta.count}
          workspacePath={session.projectPath}
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
  liveState,
  dispatchLiveEvent,
  onPreviewImage,
  onMessageCount,
  messageCount,
  workspacePath,
}: {
  agent: SessionInfo["agent"];
  filePath: string;
  sessionId: string;
  available: boolean;
  emptyHint: string;
  viewMode: ViewMode;
  liveState: LiveRuntimeState;
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onPreviewImage: (image: MarkdownImage) => void;
  onMessageCount: (
    agent: SessionInfo["agent"],
    filePath: string,
    sessionId: string,
    count: number,
  ) => boolean;
  messageCount: number;
  workspacePath: string | null;
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
  const runtimeSessionId = sessionId;
  const [composerText, setComposerText] = useState("");
  const [composerError, setComposerError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [runtimeNow, setRuntimeNow] = useState(() => Date.now());
  const bubbleRefs = useRef<(HTMLDivElement | null)[]>([]);
  const viewportRef = useRef<HTMLDivElement>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const followLiveStreamRef = useRef(false);
  const liveStreamingRef = useRef(false);
  const activeRuntimeTurnIdRef = useRef<string | null>(null);
  const programmaticScrollUntilRef = useRef(0);
  const pendingInitialPositionRef = useRef<"bottom" | "restore" | null>(null);
  const initialPositionAppliedRef = useRef(false);
  const liveSession = runtimeSessionId
    ? liveState.sessions[runtimeSessionId]
    : null;

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
            const indexedThrough = latestMessageTimestamp(result.messages);
            if (indexedThrough !== null) {
              dispatchLiveEvent({
                type: "reconcile-indexed-session",
                sessioRuntimeSessionId: runtimeSessionId,
                indexedThrough,
              });
            }
            if (!onMessageCount(agent, filePath, sessionId, result.messageCount)) return;
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
  }, [
    agent,
    filePath,
    sessionId,
    available,
    messageCount,
    onMessageCount,
    sourceKey,
    dispatchLiveEvent,
    runtimeSessionId,
  ]);

  const displayItems = useMemo(() => {
    if (liveSession && liveSession.turns.length > 0) {
      return liveTurnsToRenderItems(liveSession.turns);
    }
    const all = messages.map((m, srcIdx) => ({ m, srcIdx }));
    const filtered =
      viewMode === "native"
        ? all
        : all.filter(({ m }) => isConversationRole(m.role));
    return moveFileEditsToTurnEnd(pairToolMessages(filtered));
  }, [liveSession, messages, viewMode]);

  const liveStreamingKey = useMemo(() => {
    if (!liveSession) return "";
    return liveSession.turns
      .filter((turn) => turn.status === "streaming")
      .map((turn) => `${turn.turnId}:${turn.blocks.length}:${turn.updatedAt}`)
      .join("|");
  }, [liveSession]);
  const liveActiveKey = useMemo(() => {
    if (!liveSession) return "";
    return liveSession.turns
      .filter((turn) => turn.status === "pending" || turn.status === "streaming" || turn.status === "cancelling")
      .map((turn) => turn.turnId)
      .join("|");
  }, [liveSession]);
  const liveCacheKey = useMemo(() => {
    if (!liveSession) return "";
    return liveSession.turns
      .map((turn) =>
        [
          turn.turnId,
          turn.status,
          turn.blocks.length,
          turn.tools.length,
          turn.permissions.length,
          turn.updatedAt,
        ].join(":"),
      )
      .join("|");
  }, [liveSession]);

  void liveCacheKey;
  const activeTurnId = useMemo(() => {
    if (!liveSession) return null;
    return liveSession.turns.find((turn) =>
      turn.status === "pending" ||
      turn.status === "streaming" ||
      turn.status === "cancelling"
    )?.turnId ?? null;
  }, [liveSession]);
  liveStreamingRef.current = Boolean(liveStreamingKey);

  useEffect(() => {
    if (!liveActiveKey) return;
    setRuntimeNow(Date.now());
    const timer = window.setInterval(() => setRuntimeNow(Date.now()), 500);
    return () => window.clearInterval(timer);
  }, [liveActiveKey]);

  useEffect(() => {
    if (!activeTurnId) activeRuntimeTurnIdRef.current = null;
  }, [activeTurnId]);

  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;
    const handleUserScrollIntent = () => {
      programmaticScrollUntilRef.current = 0;
    };
    vp.addEventListener("wheel", handleUserScrollIntent, { passive: true });
    vp.addEventListener("touchmove", handleUserScrollIntent, { passive: true });
    vp.addEventListener("keydown", handleUserScrollIntent);
    return () => {
      vp.removeEventListener("wheel", handleUserScrollIntent);
      vp.removeEventListener("touchmove", handleUserScrollIntent);
      vp.removeEventListener("keydown", handleUserScrollIntent);
    };
  }, []);

  const scrollChatToBottom = useCallback(() => {
    const scroll = () => {
      const vp = viewportRef.current;
      if (!vp) return;
      programmaticScrollUntilRef.current =
        performance.now() + PROGRAMMATIC_SCROLL_SETTLE_MS;
      vp.scrollTop = Math.max(0, vp.scrollHeight - vp.clientHeight);
    };
    scroll();
    window.requestAnimationFrame(() => {
      scroll();
      window.requestAnimationFrame(scroll);
    });
    window.setTimeout(scroll, 80);
  }, []);

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
      const atBottom = isNearScrollBottom(vp);
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
      const isProgrammaticScroll =
        performance.now() < programmaticScrollUntilRef.current;
      if (!isProgrammaticScroll) followLiveStreamRef.current = atBottom;
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
    if (!vp || mode === null || loading || displayItems.length === 0) return;
    const snapshot = scrollCache.get(sourceKey);
    if (mode === "restore" && snapshot?.atBottom) {
      followLiveStreamRef.current = true;
      scrollChatToBottom();
      pendingInitialPositionRef.current = null;
      initialPositionAppliedRef.current = true;
      return;
    }
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
      followLiveStreamRef.current = true;
      scrollChatToBottom();
    }
    pendingInitialPositionRef.current = null;
    initialPositionAppliedRef.current = true;
  }, [displayItems, loading, scrollChatToBottom, sourceKey]);

  useLayoutEffect(() => {
    const vp = viewportRef.current;
    if (!vp || displayItems.length === 0 || !initialPositionAppliedRef.current) {
      return;
    }
    const snapshot = scrollCache.get(sourceKey);
    if (snapshot && !snapshot.atBottom && !followLiveStreamRef.current) return;
    scrollChatToBottom();
  }, [displayItems, scrollChatToBottom, sourceKey]);

  useEffect(() => {
    if (!liveStreamingKey || !followLiveStreamRef.current) return;
    scrollChatToBottom();
  }, [liveStreamingKey, scrollChatToBottom]);

  const handleSend = useCallback(async () => {
    const text = composerText.trim();
    if (!text || sending) return;
    if (!workspacePath) {
      setComposerError("This session has no workspace path, so live chat cannot start yet.");
      return;
    }
    setSending(true);
    setComposerError(null);
    activeRuntimeTurnIdRef.current = null;
    followLiveStreamRef.current = true;
    const timestamp = Date.now();
    const optimisticTurnId = `local-turn-${timestamp}`;
    const optimisticSessionId = runtimeSessionId;
    let pendingRuntimeSessionId = optimisticSessionId;
    console.info("[sessio-runtime:frontend:send]", {
      text,
      runtimeSessionId,
      optimisticSessionId,
      workspacePath,
      sourceSessionId: sessionId,
    });
    if (!liveState.sessions[optimisticSessionId]) {
      dispatchLiveEvent({
        type: "ensure-session",
        session: pendingLiveSession({
          sessioRuntimeSessionId: optimisticSessionId,
          agent,
          workspacePath: workspacePath ?? "",
        }),
      });
    }
    dispatchLiveEvent({
      type: "optimistic-user-message",
      sessioRuntimeSessionId: optimisticSessionId,
      turnId: optimisticTurnId,
      text,
      timestamp,
    });
    scrollChatToBottom();
    try {
      await ensureAgentRuntimeSession({
        agent,
        sessioRuntimeSessionId: runtimeSessionId,
        workspacePath,
        agentRuntimeSessionId: sessionId,
      });
      pendingRuntimeSessionId = runtimeSessionId;
      const turn = await sendAgentInput(runtimeSessionId, { text });
      activeRuntimeTurnIdRef.current = turn.turnId;
      dispatchLiveEvent({
        type: "replace-turn-id",
        sessioRuntimeSessionId: runtimeSessionId,
        from: optimisticTurnId,
        to: turn.turnId,
      });
      setComposerText("");
      window.requestAnimationFrame(() => composerRef.current?.focus());
    } catch (err) {
      const message = String(err);
      setComposerError(message);
      const failedRuntimeSessionId = pendingRuntimeSessionId;
      if (failedRuntimeSessionId) {
        dispatchLiveEvent({
          type: "turn-error",
          sessioRuntimeSessionId: failedRuntimeSessionId,
          turnId: optimisticTurnId,
          error: { code: "send_failed", message, data: null },
          timestamp: Date.now(),
        });
      }
    } finally {
      setSending(false);
    }
  }, [agent, composerText, liveState.sessions, runtimeSessionId, scrollChatToBottom, sending, sessionId, workspacePath]);

  const handleCancelTurn = useCallback(async () => {
    if (!activeTurnId) return;
    const turnId = activeRuntimeTurnIdRef.current ?? activeTurnId;
    setComposerError(null);
    try {
      await cancelAgentTurn(runtimeSessionId, turnId);
      activeRuntimeTurnIdRef.current = null;
    } catch (err) {
      setComposerError(String(err));
    }
  }, [activeTurnId, runtimeSessionId]);

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
                (renderItemRole(item) === "user" ? "flex justify-end" : "")
              }
            >
              {item.acp ? (
                <AcpLiveItem
                  item={item.acp}
                  sessioRuntimeSessionId={runtimeSessionId}
                  now={runtimeNow}
                  onPreviewImage={onPreviewImage}
                  onPermissionResponse={respondAgentPermission}
                />
              ) : item.message ? (
                <MessageBubble
                  msg={item.message}
                  toolResult={item.toolResult}
                  onPreviewImage={onPreviewImage}
                  onPermissionResponse={respondAgentPermission}
                />
              ) : null}
            </div>
          ))}
        </div>
      </ScrollArea>
      <ChatComposer
        ref={composerRef}
        value={composerText}
        disabled={sending}
        active={Boolean(activeTurnId)}
        sending={sending}
        error={composerError}
        contextLabel="52% used"
        placeholder="Ask, Search or Chat..."
        onChange={setComposerText}
        onSend={handleSend}
        onCancel={handleCancelTurn}
      />
      <RoleNav
        role="assistant"
        side="left"
        messages={displayItems.map(renderItemNavMessage)}
        refs={bubbleRefs}
        viewportRef={viewportRef}
      />
      <RoleNav
        role="user"
        side="right"
        messages={displayItems.map(renderItemNavMessage)}
        refs={bubbleRefs}
        viewportRef={viewportRef}
      />
    </div>
  );
}

function pendingLiveSession(handle: {
  sessioRuntimeSessionId: string;
  agent: SessionInfo["agent"];
  workspacePath: string;
}): LiveRuntimeSession {
  return {
    sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
    agent: handle.agent,
    agentRuntimeSessionId: "pending",
    transport: "fake",
    workspacePath: handle.workspacePath,
    capabilities: {
      supportsCancel: true,
      supportsPermissions: true,
      supportsToolDeltas: true,
      supportsResume: true,
      supportsAttachments: false,
      supportsModes: false,
    },
    turns: [],
    sessionState: {
      plan: null,
      availableCommands: [],
      currentModeId: null,
      configOptions: [],
      sessionInfo: null,
    },
    protocolMessages: [],
    ended: false,
  };
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

const ChatComposer = forwardRef<HTMLTextAreaElement, {
  value: string;
  disabled: boolean;
  active: boolean;
  sending: boolean;
  error: string | null;
  contextLabel: string;
  placeholder: string;
  onChange: (value: string) => void;
  onSend: () => void;
  onCancel: () => void;
}>(function ChatComposer(
  {
    value,
    disabled,
    active,
    sending,
    error,
    contextLabel,
    placeholder,
    onChange,
    onSend,
    onCancel,
  },
  ref,
) {
  const canSend = value.trim().length > 0 && !disabled && !sending && !active;
  const canCancel = active && !disabled && !sending;
  const disabledTitle = disabled ? "Select a project-backed session to chat" : undefined;
  const innerRef = useRef<HTMLTextAreaElement | null>(null);
  useLayoutEffect(() => {
    if (innerRef.current) resizeTextareaToContent(innerRef.current);
  }, [value]);
  const setTextareaRef = (el: HTMLTextAreaElement | null) => {
    innerRef.current = el;
    if (typeof ref === "function") {
      ref(el);
    } else if (ref) {
      ref.current = el;
    }
    if (el) resizeTextareaToContent(el);
  };
  return (
    <div className="shrink-0 px-10 pb-4 pt-2 bg-gradient-to-t from-surface-panel via-surface-panel to-surface-panel/80">
      <div className="w-full">
        {error && (
          <div className="mb-2 rounded-md border border-status-error/25 bg-status-error/10 px-3 py-2 text-body-sm text-status-error">
            {error}
          </div>
        )}
        <div
          className={
            "rounded-lg border bg-ink/[0.045] transition-colors " +
            (error
              ? "border-status-error/35"
              : "border-ink/10 focus-within:border-ink/24")
          }
          title={disabledTitle}
        >
          <textarea
            ref={setTextareaRef}
            value={value}
            disabled={disabled}
            placeholder={placeholder}
            rows={2}
            onChange={(event) => {
              resizeTextareaToContent(event.currentTarget);
              onChange(event.target.value);
            }}
            onInput={(event) => resizeTextareaToContent(event.currentTarget)}
            onKeyDown={(event) => {
              if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) {
                return;
              }
              event.preventDefault();
              if (canSend) onSend();
            }}
            className="chat-composer-textarea block w-full resize-none bg-transparent px-3 py-3 text-body leading-5 text-ink/85 placeholder:text-ink/45 outline-none disabled:cursor-not-allowed disabled:opacity-55"
          />
          <div className="flex h-11 items-center justify-between gap-3 px-2.5 pb-2">
            <div className="flex min-w-0 items-center gap-3">
              <Tooltip content="Add context" placement="top">
                <button
                  type="button"
                  disabled={disabled}
                  className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-ink/20 text-ink/55 transition hover:border-ink/35 hover:text-ink disabled:cursor-not-allowed disabled:opacity-45"
                  aria-label="Add context"
                >
                  <Plus className="h-4 w-4" />
                </button>
              </Tooltip>
              <button
                type="button"
                disabled={disabled}
                className="text-body-sm font-medium text-ink/62 transition hover:text-ink disabled:cursor-not-allowed disabled:opacity-45"
              >
                Auto
              </button>
            </div>
            <div className="flex shrink-0 items-center gap-3">
              <span className="text-body-sm font-medium text-ink/50">
                {contextLabel}
              </span>
              <button
                type="button"
                disabled={active ? !canCancel : !canSend}
                onClick={active ? onCancel : onSend}
                className="flex h-6 w-6 items-center justify-center rounded-full bg-ink text-[rgb(var(--color-bg-panel))] transition hover:bg-ink/85 disabled:cursor-not-allowed disabled:bg-ink/25 disabled:text-[rgb(var(--color-bg-panel)/0.7)]"
                aria-label={active ? "Stop" : sending ? "Sending" : "Send"}
              >
                {active ? <Square className="h-3 w-3 fill-current" /> : <ArrowUp className="h-4 w-4" />}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
});

function resizeTextareaToContent(el: HTMLTextAreaElement) {
  el.style.height = "auto";
  const lineHeight = parseFloat(getComputedStyle(el).lineHeight) || 20;
  const minHeight = lineHeight * 2;
  const maxHeight = lineHeight * 6;
  const nextHeight = Math.min(Math.max(el.scrollHeight, minHeight), maxHeight);
  el.style.height = `${nextHeight}px`;
  el.style.overflowY = el.scrollHeight > maxHeight ? "auto" : "hidden";
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

function AcpLiveItem({
  item,
  sessioRuntimeSessionId,
  now,
  onPreviewImage,
  onPermissionResponse,
}: {
  item: AcpRenderItem;
  sessioRuntimeSessionId: string;
  now: number;
  onPreviewImage: (image: MarkdownImage) => void;
  onPermissionResponse: (
    sessioRuntimeSessionId: string,
    requestId: string,
    approved: boolean,
  ) => Promise<void>;
}) {
  if (item.kind === "turnStatus") {
    return <RuntimeStatusContent text={liveTurnStatusText(item.turn, now)} />;
  }
  if (item.kind === "tool") {
    return <AcpToolCard tool={item.tool} onPreviewImage={onPreviewImage} />;
  }
  if (item.kind === "permission") {
    return (
      <AcpPermissionCard
        sessioRuntimeSessionId={sessioRuntimeSessionId}
        permission={item.permission}
        onRespond={onPermissionResponse}
      />
    );
  }
  if (item.kind === "error") {
    return (
      <div className="rounded-md border border-status-error/25 bg-status-error/10 px-3 py-2 text-body-sm text-status-error">
        {item.error.message}
      </div>
    );
  }
  return (
    <AcpContentBlockGroup
      block={item.block}
      timestamp={item.turn.updatedAt}
      onPreviewImage={onPreviewImage}
    />
  );
}

function AcpContentBlockGroup({
  block,
  timestamp,
  onPreviewImage,
}: {
  block: AcpRenderBlock;
  timestamp: number;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  if (block.kind !== "user" && block.kind !== "assistant" && block.kind !== "thought") {
    return null;
  }
  const isUser = block.kind === "user";
  const label =
    block.kind === "thought" ? "Thinking" : block.kind === "assistant" ? "assistant" : "user";
  return (
    <div
      className={
        "text-body leading-relaxed break-words " +
        (isUser
          ? "w-fit max-w-[75%] rounded-lg border border-ink/[0.04] bg-ink/[0.06] px-4 py-3"
          : block.kind === "thought"
            ? "py-1.5 text-ink/55 text-body-sm"
            : "px-0 py-1 text-ink/85")
      }
    >
      <div className="mb-2 flex items-center gap-2 leading-none">
        {block.kind === "thought" && (
          <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-ink/35" />
        )}
        <span className="text-caption font-medium uppercase text-ink/40">
          {label}
        </span>
        <span className="text-caption text-ink/30">
          {new Date(timestamp).toLocaleTimeString([], {
            hour: "2-digit",
            minute: "2-digit",
          })}
        </span>
      </div>
      <div className={block.kind === "thought" ? "ml-3.5" : ""}>
        <AcpContentBlocks blocks={block.blocks} onPreviewImage={onPreviewImage} />
      </div>
    </div>
  );
}

function AcpContentBlocks({
  blocks,
  onPreviewImage,
}: {
  blocks: AcpContentBlock[];
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  return (
    <div className="space-y-2">
      {blocks.map((block, index) => (
        <AcpContentBlockView
          key={index}
          block={block}
          onPreviewImage={onPreviewImage}
        />
      ))}
    </div>
  );
}

function AcpContentBlockView({
  block,
  onPreviewImage,
}: {
  block: AcpContentBlock;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  const type = String(block.type ?? "unknown");
  if (type === "text") {
    return (
      <MarkdownContent
        text={typeof block.text === "string" ? block.text : ""}
        onPreviewImage={onPreviewImage}
      />
    );
  }
  if (type === "image") {
    const mimeType = typeof block.mimeType === "string" ? block.mimeType : "image";
    const uri = typeof block.uri === "string" ? block.uri : "";
    const data = typeof block.data === "string" ? block.data : "";
    const src = uri || (data ? `data:${mimeType};base64,${data}` : "");
    return src ? (
      <MarkdownImageButton
        image={{ alt: mimeType, src }}
        onPreviewImage={onPreviewImage}
      />
    ) : (
      <PlainTextContent text={JSON.stringify(block, null, 2)} />
    );
  }
  if (type === "audio") {
    return <PlainTextContent text={`Audio: ${String(block.mimeType ?? "unknown")}`} />;
  }
  if (type === "resource_link") {
    return (
      <div className="rounded-md border border-ink/[0.08] bg-ink/[0.035] px-3 py-2 text-body-sm">
        <div className="font-medium text-ink/75">{String(block.name ?? "Resource")}</div>
        <div className="truncate font-mono text-caption text-ink/45">{String(block.uri ?? "")}</div>
      </div>
    );
  }
  return <PlainTextContent text={JSON.stringify(block, null, 2)} />;
}

function AcpToolCard({
  tool,
  onPreviewImage,
}: {
  tool: AcpToolCall;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  const input = tool.rawInput === null ? "" : JSON.stringify(tool.rawInput, null, 2);
  const output =
    tool.rawOutput !== null
      ? JSON.stringify(tool.rawOutput, null, 2)
      : tool.content.length > 0
        ? tool.content.map(formatAcpToolContent).join("\n\n")
        : "";
  return (
    <div className="overflow-hidden rounded-md border border-ink/[0.08] bg-ink/[0.045] text-body-sm">
      <div className="flex items-center justify-between gap-3 border-b border-ink/[0.07] px-3 py-2">
        <div className="min-w-0">
          <div className="truncate font-medium text-ink/80">{tool.title}</div>
          <div className="text-caption text-ink/45">{tool.kind} · {formatToolStatus(tool.status)}</div>
        </div>
      </div>
      {(input || output || tool.locations.length > 0) && (
        <div className="space-y-2 px-3 py-2">
          {input && <ToolPairRow label="IN" text={input} collapsed={false} onPreviewImage={onPreviewImage} />}
          {output && <ToolPairRow label="OUT" text={output} collapsed={false} onPreviewImage={onPreviewImage} />}
          {tool.locations.length > 0 && (
            <PlainTextContent text={JSON.stringify(tool.locations, null, 2)} />
          )}
        </div>
      )}
    </div>
  );
}

function AcpPermissionCard({
  sessioRuntimeSessionId,
  permission,
  onRespond,
}: {
  sessioRuntimeSessionId: string;
  permission: AcpPermissionRequest;
  onRespond: (
    sessioRuntimeSessionId: string,
    requestId: string,
    approved: boolean,
  ) => Promise<void>;
}) {
  const [pendingChoice, setPendingChoice] = useState<"allow" | "reject" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const resolved = Boolean(permission.selectedOptionId || permission.cancelled);
  const respond = (approved: boolean) => {
    if (resolved || pendingChoice) return;
    setPendingChoice(approved ? "allow" : "reject");
    setError(null);
    onRespond(sessioRuntimeSessionId, permission.requestId, approved).catch((err) => {
      setError(String(err));
      setPendingChoice(null);
    });
  };
  return (
    <div className="rounded-md border border-ink/[0.08] bg-ink/[0.045] px-3 py-2 text-body-sm">
      <div className="mb-2 flex items-center justify-between gap-3">
        <div>
          <div className="font-medium text-ink/80">Permission · {permission.toolName}</div>
          <div className="text-caption text-ink/45">
            {resolved ? "Resolved" : "Waiting for approval"}
          </div>
        </div>
      </div>
      {permission.input !== null && (
        <PlainTextContent text={JSON.stringify(permission.input, null, 2)} />
      )}
      {!resolved && (
        <div className="mt-2 flex items-center gap-2">
          <button
            type="button"
            disabled={Boolean(pendingChoice)}
            onClick={() => respond(true)}
            className="rounded-md border border-[rgb(var(--color-emerald)/0.28)] bg-[rgb(var(--color-emerald)/0.12)] px-2.5 py-1 text-caption font-medium text-[rgb(var(--color-emerald))] transition hover:bg-[rgb(var(--color-emerald)/0.18)] disabled:cursor-not-allowed disabled:opacity-55"
          >
            {pendingChoice === "allow" ? "Allowing..." : "Allow"}
          </button>
          <button
            type="button"
            disabled={Boolean(pendingChoice)}
            onClick={() => respond(false)}
            className="rounded-md border border-ink/12 bg-ink/[0.04] px-2.5 py-1 text-caption font-medium text-ink/60 transition hover:bg-ink/[0.07] hover:text-ink/80 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {pendingChoice === "reject" ? "Rejecting..." : "Reject"}
          </button>
        </div>
      )}
      {error && <div className="mt-2 text-caption text-status-error">{error}</div>}
    </div>
  );
}

function contentBlocksText(blocks: AcpContentBlock[]): string {
  return blocks.map((block) => {
    if (block.type === "text" && typeof block.text === "string") return block.text;
    if (block.type === "image") return `[image: ${String(block.mimeType ?? "")}]`;
    if (block.type === "resource_link") return `[resource: ${String(block.uri ?? "")}]`;
    return JSON.stringify(block);
  }).join("\n");
}

function formatAcpToolContent(content: unknown): string {
  if (!content || typeof content !== "object") return String(content ?? "");
  const record = content as Record<string, unknown>;
  if (record.type === "content") {
    const inner = record.content as AcpContentBlock | undefined;
    return inner ? contentBlocksText([inner]) : JSON.stringify(record, null, 2);
  }
  if (record.type === "diff") return JSON.stringify(record, null, 2);
  if (record.type === "terminal") return `Terminal: ${String(record.terminalId ?? "")}`;
  return JSON.stringify(record, null, 2);
}

const MessageBubble = memo(function MessageBubble({
  msg,
  toolResult,
  onPreviewImage,
  onPermissionResponse,
}: {
  msg: SessionMessage;
  toolResult?: SessionMessage;
  onPreviewImage: (image: MarkdownImage) => void;
  onPermissionResponse: (
    sessioRuntimeSessionId: string,
    requestId: string,
    approved: boolean,
  ) => Promise<void>;
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
  const showHeader = meta.label.length > 0 || msg.timestamp;

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

  if (msg.role === "runtime_status") {
    return (
      <div ref={bubbleRef} className="py-1">
        <RuntimeStatusContent text={msg.text} />
      </div>
    );
  }

  if (msg.role === "turn_note") {
    return (
      <div ref={bubbleRef} className="py-1">
        <TurnNoteContent msg={msg} locale={localeTag(lang)} />
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
      {showHeader && (
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
          {meta.compact && meta.label && (
            <span
              className={
                "h-1.5 w-1.5 shrink-0 rounded-full " +
                (meta.tone === "tool"
                  ? "bg-[rgb(var(--color-emerald)/0.7)]"
                  : "bg-ink/45")
              }
            />
          )}
          {meta.label && (
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
          )}
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
      )}
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
            ) : meta.renderMode === "runtime_status" ? (
              <RuntimeStatusContent text={visibleRenderText} />
            ) : meta.renderMode === "permission" ? (
              <PermissionRequestContent
                text={visibleRenderText}
                metadata={msg.toolCallId ?? null}
                onRespond={onPermissionResponse}
              />
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
    "runtime_status",
    "permission_request",
    "turn_note",
  ].includes(role);
}

function latestMessageTimestamp(messages: SessionMessage[]): number | null {
  let latest: number | null = null;
  for (const message of messages) {
    if (message.timestamp === null) continue;
    latest = latest === null ? message.timestamp : Math.max(latest, message.timestamp);
  }
  return latest;
}

interface MessageRenderItem {
  key: string;
  kind?: "legacy" | "acp";
  message?: SessionMessage;
  acp?: AcpRenderItem;
  toolResult?: SessionMessage;
}

type AcpRenderItem =
  | { kind: "turnStatus"; turn: LiveTurn }
  | { kind: "block"; turn: LiveTurn; block: AcpRenderBlock }
  | { kind: "tool"; turn: LiveTurn; tool: AcpToolCall }
  | { kind: "permission"; turn: LiveTurn; permission: AcpPermissionRequest }
  | { kind: "error"; turn: LiveTurn; error: RuntimeError };

function liveTurnsToRenderItems(turns: LiveTurn[]): MessageRenderItem[] {
  const items: MessageRenderItem[] = [];
  for (const turn of turns) {
    items.push({
      key: `live:${turn.turnId}:status`,
      kind: "acp",
      acp: { kind: "turnStatus", turn },
    });
    const renderedTools = new Set<string>();
    const renderedPermissions = new Set<string>();
    turn.blocks.forEach((block, index) => {
      if (block.kind === "tool") {
        const tool = turn.tools.find((item) => item.toolId === block.toolId);
        if (!tool || renderedTools.has(tool.toolId)) return;
        renderedTools.add(tool.toolId);
        items.push({
          key: `live:${turn.turnId}:tool:${tool.toolId}`,
          kind: "acp",
          acp: { kind: "tool", turn, tool },
        });
        return;
      }
      if (block.kind === "permission") {
        const permission = turn.permissions.find((item) => item.requestId === block.requestId);
        if (!permission || renderedPermissions.has(permission.requestId)) return;
        renderedPermissions.add(permission.requestId);
        items.push({
          key: `live:${turn.turnId}:permission:${permission.requestId}`,
          kind: "acp",
          acp: { kind: "permission", turn, permission },
        });
        return;
      }
      if (block.kind === "error") return;
      items.push({
        key: `live:${turn.turnId}:block:${index}`,
        kind: "acp",
        acp: { kind: "block", turn, block },
      });
    });
    if (turn.error) {
      items.push({
        key: `live:${turn.turnId}:error`,
        kind: "acp",
        acp: { kind: "error", turn, error: turn.error },
      });
    }
  }
  return items;
}

function liveTurnStatusText(turn: LiveTurn, now: number): string {
  const running =
    turn.status === "pending" ||
    turn.status === "streaming" ||
    turn.status === "cancelling";
  const elapsedMs = Math.max(0, (running ? now : turn.updatedAt) - turn.startedAt);
  return `${running ? "running" : "done"}|${formatDuration(elapsedMs)}`;
}

function renderItemRole(item: MessageRenderItem): string {
  if (item.message) return item.message.role;
  const acp = item.acp;
  if (!acp) return "";
  if (acp.kind === "block") {
    if (acp.block.kind === "user") return "user";
    if (acp.block.kind === "assistant") return "assistant";
    if (acp.block.kind === "thought") return "thinking";
  }
  return "tool_call";
}

function renderItemNavMessage(item: MessageRenderItem): SessionMessage {
  if (item.message) return item.message;
  const acp = item.acp;
  if (!acp || acp.kind !== "block") return { role: "", text: "", timestamp: null };
  if (acp.block.kind !== "user" && acp.block.kind !== "assistant") {
    return { role: "", text: "", timestamp: null };
  }
  return {
    role: acp.block.kind === "user" ? "user" : "assistant",
    text: contentBlocksText(acp.block.blocks),
    timestamp: acp.turn.updatedAt,
  };
}

function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.round(ms / 1000));
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}m ${seconds}s`;
}

function formatToolStatus(status: string): string {
  return status.replace(/_/g, " ");
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
    const edits = turn.filter((item) => item.message?.role === "file_edit");
    const rest = turn.filter((item) => item.message?.role !== "file_edit");
    const mergedEdit = mergeFileEditItems(edits);
    out.push(...rest);
    if (mergedEdit) out.push(mergedEdit);
    turn = [];
  };
  for (const item of items) {
    if (item.message?.role === "user" && turn.length > 0) {
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
    .map((item) => item.message ? parseFileEditSummary(item.message.text) : null)
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
  if (!first || !last) return null;
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
  renderMode: "markdown" | "plain" | "todo" | "file_edit" | "runtime_status" | "permission";
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
  if (role === "runtime_status") {
    return {
      label: "",
      bodyText: msg.text,
      compact: true,
      contentClass: "text-ink/45 text-body-sm",
      tone: "thinking",
      renderMode: "runtime_status",
    };
  }
  if (role === "turn_note") {
    return {
      label: "",
      bodyText: msg.text,
      compact: true,
      contentClass: "text-ink/40 text-body-sm italic",
      tone: "thinking",
      renderMode: "plain",
    };
  }
  if (role === "permission_request") {
    const parsed = parseToolCall(msg.text);
    return {
      label: `Permission · ${parsed.name}`,
      bodyText: parsed.body,
      compact: true,
      contentClass:
        "text-ink/75 text-body-sm bg-ink/[0.045] border border-ink/[0.08] rounded-md px-3 py-2 overflow-hidden",
      tone: "tool",
      renderMode: "permission",
    };
  }
  if (["tool", "tool_call", "tool_use", "function_call"].includes(role)) {
    const parsed = parseToolCall(msg.text);
    const label = toolDisplayName(parsed.name);
    return {
      label,
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

function toolDisplayName(name: string): string {
  if (name === "web_search") return "Searching web";
  return name;
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

function RuntimeStatusContent({ text }: { text: string }) {
  const [state = "running", duration = "0s"] = text.split("|");
  const running = state === "running";
  return (
    <div className="flex items-center gap-2 text-body-sm text-ink/50">
      <span
        className={
          "h-1.5 w-1.5 shrink-0 rounded-full " +
          (running ? "bg-[rgb(var(--color-emerald))]" : "bg-ink/30")
        }
      />
      <span>{running ? "Working for" : "Worked for"} {duration}</span>
    </div>
  );
}

function TurnNoteContent({
  msg,
  locale,
}: {
  msg: SessionMessage;
  locale: string;
}) {
  return (
    <div className="flex items-center gap-2 text-body-sm italic text-ink/40">
      <span>{msg.text}</span>
      {msg.timestamp && (
        <span className="text-caption not-italic text-ink/30">
          {new Date(msg.timestamp).toLocaleString(locale, {
            hour: "2-digit",
            minute: "2-digit",
            month: "short",
            day: "numeric",
          })}
        </span>
      )}
    </div>
  );
}

function PermissionRequestContent({
  text,
  metadata,
  onRespond,
}: {
  text: string;
  metadata: string | null;
  onRespond: (
    sessioRuntimeSessionId: string,
    requestId: string,
    approved: boolean,
  ) => Promise<void>;
}) {
  const parsed = parseToolCall(text);
  const meta = parsePermissionMetadata(metadata);
  const [pendingChoice, setPendingChoice] = useState<"allow" | "reject" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const canRespond = Boolean(meta && meta.pending && !pendingChoice);
  const respond = (approved: boolean) => {
    if (!meta || !canRespond) return;
    setPendingChoice(approved ? "allow" : "reject");
    setError(null);
    onRespond(meta.sessioRuntimeSessionId, meta.requestId, approved).catch((err) => {
      console.warn("respond permission failed", err);
      setError(String(err));
      setPendingChoice(null);
    });
  };
  return (
    <div className="space-y-2">
      {parsed.body.trim() && <PlainTextContent text={parsed.body} />}
      {meta?.pending && (
        <div className="flex items-center gap-2">
          <button
            type="button"
            disabled={!canRespond}
            onClick={() => respond(true)}
            className="rounded-md border border-[rgb(var(--color-emerald)/0.28)] bg-[rgb(var(--color-emerald)/0.12)] px-2.5 py-1 text-caption font-medium text-[rgb(var(--color-emerald))] transition hover:bg-[rgb(var(--color-emerald)/0.18)] disabled:cursor-not-allowed disabled:opacity-55"
          >
            {pendingChoice === "allow" ? "Allowing..." : "Allow"}
          </button>
          <button
            type="button"
            disabled={!canRespond}
            onClick={() => respond(false)}
            className="rounded-md border border-ink/12 bg-ink/[0.04] px-2.5 py-1 text-caption font-medium text-ink/60 transition hover:bg-ink/[0.07] hover:text-ink/80 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {pendingChoice === "reject" ? "Rejecting..." : "Reject"}
          </button>
        </div>
      )}
      {error && <div className="text-caption text-status-error">{error}</div>}
    </div>
  );
}

function parsePermissionMetadata(metadata: string | null):
  | { sessioRuntimeSessionId: string; requestId: string; pending: boolean }
  | null {
  const parts = metadata?.split(":") ?? [];
  if (parts[0] !== "permission" || parts.length < 4) return null;
  return {
    sessioRuntimeSessionId: parts[1],
    requestId: parts[2],
    pending: parts[3] === "pending",
  };
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
