import {
  memo,
  startTransition,
  forwardRef,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { MultiFileDiff, PatchDiff } from "@pierre/diffs/react";
import { ArrowDownToLine, ArrowUp, BookOpen, Brain, CheckSquare, ChevronDown, ChevronRight, ClipboardList, Code2, FileDiff, FileSearch, FileText, FolderOpen, Globe, Image as ImageIcon, ListChecks, ListTodo, LoaderCircle, MessageCircleQuestionMark, Mic, MoveRight, Plus, Search, SearchCheck, Square, Pen, SquareTerminal, Trash2, UserKey, Wrench, type LucideIcon } from "lucide-react";
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
  type AgentAttachment,
  type Agent,
  type SessionHistorySnapshotGroup,
  type SessionHistoryTurn,
  SessionInfo,
  RuntimeAgentMetadata,
  RuntimeCapabilitySet,
  RuntimeError,
  SubagentInfo,
  ensureAgentRuntimeSession,
  getSessionHistory,
  getSessionHistorySnapshots,
  readLocalImageDataUrl,
  readLocalTextFile,
  cancelAgentTurn,
  respondAgentPermission,
  sendAgentInput,
  startAgentSession,
  setAgentSessionConfigOption,
  updateRuntimeAgentPreferences,
  updateSessionHistoryCount,
  writeCrossPrompt,
} from "../api";
import ScrollArea from "../components/ScrollArea";
import Tooltip from "../components/Tooltip";
import {
  agentModelSelectOptions,
  agentModelSelectValue,
  initialRuntimeEffort,
  parseAgentModelSelectValue,
  runtimeEffortOptions,
} from "../components/AgentSelect";
import type { InlineMenuSelectOption } from "../components/InlineMenuSelect";
import { RuntimeEffortControl, RuntimeMenuSelect, runtimePermissionModeOptions } from "../components/RuntimeMenuSelect";
import {
  attachmentMenuOptions,
  type ComposerAttachment,
  ComposerAttachmentMenu,
  ComposerAttachmentPreviewList,
  useComposerAttachments,
} from "../components/ComposerAttachments";
import {
  hasMessageStreamScrollSnapshot,
  isNearScrollBottom,
  useMessageStreamScrollController,
} from "../components/useMessageStreamScrollController";
import { localeTag, useI18n } from "../i18n";
import type { ViewMode } from "../navigation";
import {
  type AcpViewModel,
  type AcpAvailableCommand,
  type AcpContentBlock,
  type AcpPermissionRequest,
  type AcpRenderBlock,
  type AcpSessionConfigOption,
  type AcpSessionState,
  type AcpToolCall,
  dispatchSessionStartedFallback,
  historyTurnsToAcpViewModel,
  liveSessionToAcpViewModel,
  type LiveRuntimeAction,
  type LiveRuntimeState,
  type LiveRuntimeSession,
  type LiveTurn,
} from "../runtimeChat";
import { buildCrossPromptFromTurns } from "../cross";
import {
  contentBlocksText,
  forkVisibleHistoryTurns,
  mergeHistoryWithLiveTurns,
  stripImagePlaceholders,
  stripInjectedContext,
  stripSessioUploadWrapper,
} from "../historyMerge";
import { getCachedSessionHistorySnapshots } from "../sessionHistorySnapshots";

interface Props {
  session: SessionInfo;
  viewMode: ViewMode;
  liveState: LiveRuntimeState;
  runtimeAgents: RuntimeAgentMetadata[];
  debugAcpConfig?: boolean;
  runtimeSessionAliases?: Record<string, string>;
  ancestorSessions?: SessionInfo[];
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onPendingSession: (session: PendingAgentSession) => void;
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

interface FilePreview {
  title: string;
  text: string;
}

export interface PendingAgentSession {
  sessioRuntimeSessionId: string;
  agent: Agent;
  projectPath: string;
  projectName: string;
  prompt: string;
  timestamp: number;
  forkedFromAgent?: Agent | null;
  forkedFromId?: string | null;
  historySnapshots?: SessionHistorySnapshotGroup[];
}

function initialRuntimeModel(agent: RuntimeAgentMetadata | null): string {
  return agent?.model ?? agent?.models[0]?.value ?? "";
}

function initialRuntimePermission(agent: RuntimeAgentMetadata | null): string {
  return agent?.permissionMode ?? agent?.permissionModes[0]?.value ?? "";
}

function runtimeSessionOptions(model: string, permissionMode: string, effort = ""): Record<string, unknown> {
  return {
    transport: "acp",
    ...(model ? { model } : {}),
    ...(effort ? { effort } : {}),
    ...(permissionMode ? { permissionMode } : {}),
  };
}

function runtimeEffortConfigId(agent: Agent): string {
  return agent === "codex" ? "reasoning_effort" : "effort";
}

type Tab =
  | { kind: "main" }
  | { kind: "sub"; sub: SubagentInfo };

const ROLE_NAV_SHOW_DELAY_MS = 800;

interface HistoryCacheEntry {
  turns: LiveTurn[];
  indexedThrough: number | null;
  messageCount: number;
  loadedAt: number;
}

interface AncestorHistoryGroup {
  session: SessionInfo;
  turns: LiveTurn[];
}

interface AncestorSnapshotState {
  loaded: boolean;
  hasSnapshot: boolean;
  groups: AncestorHistoryGroup[];
}

interface HistoryViewCacheEntry {
  sourceKey: string;
  viewMode: ViewMode;
  turns: LiveTurn[];
  viewModel: AcpViewModel;
}

const historyCache = new Map<string, HistoryCacheEntry>();
const historyViewCache = new Map<string, HistoryViewCacheEntry>();
const renderItemsCache = new WeakMap<AcpViewModel, Map<string, AcpRenderItem[]>>();
const INITIAL_HISTORY_RENDER_ITEMS = 120;

function historySourceKey(agent: SessionInfo["agent"], filePath: string, sessionId: string): string {
  return `${agent}:${sessionId}:${filePath}`;
}

function cachedAncestorHistoryGroups(sessions: SessionInfo[]): AncestorHistoryGroup[] {
  return sessions.map((session) => ({
    session,
    turns:
      historyCache.get(historySourceKey(session.agent, session.filePath, session.id))
        ?.turns ?? [],
  }));
}

function snapshotGroupsToAncestorHistoryGroups(
  groups: SessionHistorySnapshotGroup[],
  ancestorSessions: SessionInfo[],
): AncestorHistoryGroup[] {
  const byIdentity = new Map(
    ancestorSessions.map((session) => [`${session.agent}:${session.id}`, session]),
  );
  return groups.map((group) => ({
    session: byIdentity.get(`${group.ancestorAgent}:${group.ancestorSessionId}`) ?? {
      id: group.ancestorSessionId,
      agent: group.ancestorAgent,
      forkedFromAgent: null,
      forkedFromId: null,
      projectPath: null,
      projectName: null,
      startedAt: null,
      updatedAt: null,
      messageCount: group.turns.length,
      title: null,
      firstUserMessage: null,
      filePath: "",
      fileSize: 0,
      partial: false,
      available: true,
      archived: false,
      subagents: [],
    },
    turns: normalizeSessionHistoryTurns(group.turns),
  }));
}

function ChatPage({
  session,
  viewMode,
  liveState,
  runtimeAgents,
  debugAcpConfig = false,
  runtimeSessionAliases = {},
  ancestorSessions = [],
  dispatchLiveEvent,
  onPendingSession,
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
  const [previewFile, setPreviewFile] = useState<FilePreview | null>(null);
  const [filePreviewNotice, setFilePreviewNotice] = useState<string | null>(null);
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
              ? historySourceKey(session.agent, session.filePath, session.id)
              : historySourceKey(session.agent, tab.sub.filePath, `${session.id}:${tab.sub.id}`)
          }
          agent={session.agent}
          filePath={tab.kind === "main" ? session.filePath : tab.sub.filePath}
          sessionId={session.id}
          ancestorSessions={tab.kind === "main" ? ancestorSessions : []}
          available={
            tab.kind === "main"
              ? session.available || Boolean(liveState.sessions[runtimeSessionAliases[`${session.agent}:${session.id}`] ?? session.id])
              : tab.sub.filePath !== ""
          }
          emptyHint={
            tab.kind === "main"
              ? t("detail.session_archived")
              : t("detail.subagent_unreadable")
          }
          viewMode={viewMode}
          liveState={liveState}
          runtimeAgents={runtimeAgents}
          debugAcpConfig={debugAcpConfig}
          runtimeSessionAliases={runtimeSessionAliases}
          dispatchLiveEvent={dispatchLiveEvent}
          onPendingSession={onPendingSession}
          onPreviewImage={setPreviewImage}
          onPreviewFile={setPreviewFile}
          onFilePreviewError={setFilePreviewNotice}
          onMessageCount={onMessageCount}
          messageCount={activeMessageMeta.count}
          workspacePath={session.projectPath}
          skipHistoryLoad={tab.kind === "main" && !session.filePath && Boolean(liveState.sessions[runtimeSessionAliases[`${session.agent}:${session.id}`] ?? session.id])}
        />

        {previewImage && (
          <ImagePreviewOverlay
            image={previewImage}
            onClose={() => setPreviewImage(null)}
          />
        )}
        {previewFile && (
          <FilePreviewOverlay
            file={previewFile}
            onClose={() => setPreviewFile(null)}
          />
        )}
        {filePreviewNotice && (
          <FilePreviewNotice
            message={filePreviewNotice}
            onClose={() => setFilePreviewNotice(null)}
          />
        )}
    </div>
  );
}

export default memo(ChatPage);

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
  ancestorSessions = [],
  available,
  emptyHint,
  viewMode,
  liveState,
  runtimeAgents,
  debugAcpConfig,
  runtimeSessionAliases,
  dispatchLiveEvent,
  onPendingSession,
  onPreviewImage,
  onPreviewFile,
  onFilePreviewError,
  onMessageCount,
  messageCount,
  workspacePath,
  skipHistoryLoad = false,
}: {
  agent: SessionInfo["agent"];
  filePath: string;
  sessionId: string;
  ancestorSessions?: SessionInfo[];
  available: boolean;
  emptyHint: string;
  viewMode: ViewMode;
  liveState: LiveRuntimeState;
  runtimeAgents: RuntimeAgentMetadata[];
  debugAcpConfig: boolean;
  runtimeSessionAliases: Record<string, string>;
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onPendingSession: (session: PendingAgentSession) => void;
  onPreviewImage: (image: MarkdownImage) => void;
  onPreviewFile: (file: FilePreview) => void;
  onFilePreviewError: (message: string) => void;
  onMessageCount: (
    agent: SessionInfo["agent"],
    filePath: string,
    sessionId: string,
    count: number,
  ) => boolean;
  messageCount: number;
  workspacePath: string | null;
  skipHistoryLoad?: boolean;
}) {
  const { t } = useI18n();
  const sourceKey = historySourceKey(agent, filePath, sessionId);
  const readableAncestorSessions = useMemo(
    () => ancestorSessions.filter((session) => session.available && session.filePath),
    [ancestorSessions],
  );
  const ancestorSnapshotSessionKey = `${agent}:${sessionId}`;
  const cachedAncestorSnapshots = getCachedSessionHistorySnapshots(agent, sessionId);
  const [ancestorSnapshotState, setAncestorSnapshotState] = useState<AncestorSnapshotState>({
    loaded: Boolean(cachedAncestorSnapshots),
    hasSnapshot: Boolean(cachedAncestorSnapshots),
    groups: cachedAncestorSnapshots
      ? snapshotGroupsToAncestorHistoryGroups(cachedAncestorSnapshots, ancestorSessions)
      : [],
  });
  const ancestorSourceKeys = useMemo(
    () =>
      ancestorSnapshotState.hasSnapshot
        ? ancestorSnapshotState.groups.map((group) =>
            `${group.session.agent}:${group.session.id}:snapshot:${ancestorSnapshotSessionKey}`,
          )
        : readableAncestorSessions.map((session) => historySourceKey(session.agent, session.filePath, session.id)),
    [ancestorSnapshotSessionKey, ancestorSnapshotState, readableAncestorSessions],
  );
  const ancestorCacheKey = ancestorSourceKeys.join("->");
  const allAncestorCacheFresh = !ancestorSnapshotState.hasSnapshot && readableAncestorSessions.every((session) => {
      const cached = historyCache.get(historySourceKey(session.agent, session.filePath, session.id));
      return cached?.messageCount === session.messageCount;
    });
  const cachedEntry = historyCache.get(sourceKey);
  const isFreshCache =
    Boolean(cachedEntry) && cachedEntry?.messageCount === messageCount;
  const hasCachedHistory = Boolean(cachedEntry);
  const [historyTurns, setHistoryTurns] = useState<LiveTurn[]>(
    cachedEntry?.turns ?? [],
  );
  const [ancestorHistoryGroups, setAncestorHistoryGroups] = useState<AncestorHistoryGroup[]>(() =>
    allAncestorCacheFresh
      ? cachedAncestorHistoryGroups(readableAncestorSessions)
      : [],
  );
  const [loading, setLoading] = useState(() => !isFreshCache);
  const [ancestorsLoading, setAncestorsLoading] = useState(() =>
    readableAncestorSessions.length > 0 && !allAncestorCacheFresh,
  );
  const [error, setError] = useState<string | null>(null);
  const runtimeSessionId = runtimeSessionAliases[`${agent}:${sessionId}`] ?? sessionId;
  const [composerText, setComposerText] = useState("");
  const [composerError, setComposerError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [composerAgent, setComposerAgent] = useState<Agent>(agent);
  const [composerModel, setComposerModel] = useState("");
  const [composerEffort, setComposerEffort] = useState("");
  const [composerPermissionMode, setComposerPermissionMode] = useState("");
  const [historyRenderReady, setHistoryRenderReady] = useState(hasCachedHistory);
  const [runtimeNow, setRuntimeNow] = useState(() => Date.now());
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const activeRuntimeTurnIdRef = useRef<string | null>(null);
  const fallbackRuntimeSequenceRef = useRef(0);
  const liveSession = runtimeSessionId
    ? liveState.sessions[runtimeSessionId]
    : null;
  const mergedAncestorTurns = useMemo(
    () => ancestorHistoryGroups.flatMap((group) => group.turns),
    [ancestorHistoryGroups],
  );
  const activeTurnId = useMemo(() => {
    if (!liveSession) return null;
    return liveSession.turns.find((turn) =>
      turn.status === "pending" ||
      turn.status === "streaming" ||
      turn.status === "cancelling"
    )?.turnId ?? null;
  }, [liveSession]);
  const selectedAgentModelValue = agentModelSelectValue(composerAgent, composerModel);
  const selectedComposerAgent =
    runtimeAgents.find((item) => item.agent === composerAgent) ?? null;
  const handleComposerEffortChange = useCallback(async (targetAgent: Agent, nextValue: string) => {
    if (targetAgent === composerAgent) setComposerEffort(nextValue);
    try {
      await updateRuntimeAgentPreferences({ agent: targetAgent, effort: nextValue });
    } catch (err) {
      setComposerError(String(err));
    }
  }, [composerAgent]);
  const agentModelOptions = useMemo(
    () =>
      agentModelSelectOptions(
        runtimeAgents,
        Object.fromEntries(
          runtimeAgents.map((runtimeAgent) => [
            runtimeAgent.agent,
            <RuntimeEffortControl
              value={runtimeAgent.agent === composerAgent ? composerEffort : initialRuntimeEffort(runtimeAgent)}
              options={runtimeEffortOptions(runtimeAgent)}
              onChange={(value) => void handleComposerEffortChange(runtimeAgent.agent, value)}
              disabled={sending || Boolean(activeTurnId)}
            />,
          ]),
        ) as Partial<Record<Agent, ReactNode>>,
        { [composerAgent]: composerEffort },
      ),
    [activeTurnId, composerAgent, composerEffort, handleComposerEffortChange, runtimeAgents, sending],
  );
  const composerPermissionOptions = runtimePermissionModeOptions(
    selectedComposerAgent?.permissionModes ?? [],
    composerPermissionMode,
    selectedComposerAgent?.agent,
  );
  const fallbackRuntimeAgent = runtimeAgents.find((item) => item.agent === agent) ?? null;
  const fallbackCapabilities = fallbackRuntimeAgent?.capabilities ?? null;
  const fallbackComposerCapabilities =
    selectedComposerAgent?.capabilities ?? (composerAgent === agent ? fallbackCapabilities : null);
  const attachmentCapabilities =
    (composerAgent === agent ? liveSession?.capabilities : null) ?? fallbackComposerCapabilities;
  const {
    attachments,
    supportsAttachments,
    supportsImageAttachments,
    supportsEmbeddedContext,
    removeAttachment,
    clearAttachments,
    pickAttachments,
  } = useComposerAttachments({
    capabilities: attachmentCapabilities,
    onError: setComposerError,
  });

  useEffect(() => {
    if (!available || !filePath || skipHistoryLoad) return;
    const cached = historyCache.get(sourceKey);
    if (cached && cached.messageCount === messageCount) return;

    let cancelled = false;
    let frameId: number | null = null;
    let timerId: number | null = null;
    frameId = window.requestAnimationFrame(() => {
      timerId = window.setTimeout(() => {
        getSessionHistory(agent, filePath, sessionId)
          .then((result) => {
            if (cancelled) return;
            const turns = normalizeSessionHistoryTurns(result.turns);
            historyCache.set(sourceKey, {
              turns,
              indexedThrough: result.indexedThrough,
              messageCount: result.messageCount,
              loadedAt: Date.now(),
            });
            startTransition(() => {
              setHistoryTurns(turns);
              setLoading(false);
            });
            if (result.indexedThrough !== null) {
              dispatchLiveEvent({
                type: "reconcile-indexed-session",
                sessioRuntimeSessionId: runtimeSessionId,
                indexedThrough: result.indexedThrough,
              });
            }
            if (!onMessageCount(agent, filePath, sessionId, result.messageCount)) return;
            window.setTimeout(() => {
              updateSessionHistoryCount(
                agent,
                filePath,
                result.messageCount,
                sessionId,
              ).catch((err) => console.warn("update history count failed", err));
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
    skipHistoryLoad,
  ]);

  useEffect(() => {
    let cancelled = false;
    const cached = getCachedSessionHistorySnapshots(agent, sessionId);
    if (cached) {
      setAncestorSnapshotState({
        loaded: true,
        hasSnapshot: true,
        groups: snapshotGroupsToAncestorHistoryGroups(cached, ancestorSessions),
      });
      return () => {
        cancelled = true;
      };
    }
    setAncestorSnapshotState({ loaded: false, hasSnapshot: false, groups: [] });
    getSessionHistorySnapshots(agent, sessionId)
      .then((result) => {
        if (cancelled) return;
        setAncestorSnapshotState({
          loaded: true,
          hasSnapshot: result.hasSnapshot,
          groups: snapshotGroupsToAncestorHistoryGroups(result.groups, ancestorSessions),
        });
      })
      .catch((err) => {
        if (cancelled) return;
        console.warn("load session history snapshots failed", err);
        setAncestorSnapshotState({ loaded: true, hasSnapshot: false, groups: [] });
      });
    return () => {
      cancelled = true;
    };
  }, [agent, ancestorSessions, sessionId]);

  useEffect(() => {
    if (!ancestorSnapshotState.loaded) {
      setAncestorHistoryGroups([]);
      setAncestorsLoading(true);
      return;
    }
    if (ancestorSnapshotState.hasSnapshot) {
      setAncestorHistoryGroups(ancestorSnapshotState.groups);
      setAncestorsLoading(false);
      return;
    }
    if (readableAncestorSessions.length === 0) {
      setAncestorHistoryGroups([]);
      setAncestorsLoading(false);
      return;
    }
    if (allAncestorCacheFresh) {
      setAncestorHistoryGroups(cachedAncestorHistoryGroups(readableAncestorSessions));
      setAncestorsLoading(false);
      return;
    }

    let cancelled = false;
    setAncestorsLoading(true);
    Promise.all(
      readableAncestorSessions.map(async (session) => {
        const key = historySourceKey(session.agent, session.filePath, session.id);
        const cached = historyCache.get(key);
        if (cached && cached.messageCount === session.messageCount) {
          return { session, turns: cached.turns };
        }
        const result = await getSessionHistory(session.agent, session.filePath, session.id);
        const turns = normalizeSessionHistoryTurns(result.turns);
        historyCache.set(key, {
          turns,
          indexedThrough: result.indexedThrough,
          messageCount: result.messageCount,
          loadedAt: Date.now(),
        });
        return { session, turns };
      }),
    )
      .then((results) => {
        if (cancelled) return;
        startTransition(() => {
          setAncestorHistoryGroups(results);
          setAncestorsLoading(false);
        });
      })
      .catch((err) => {
        if (cancelled) return;
        setError(String(err));
        setAncestorsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [allAncestorCacheFresh, ancestorCacheKey, ancestorSnapshotState, readableAncestorSessions]);

  const acpViewModel = useMemo<AcpViewModel>(() => {
    const historyTurnsForView = forkVisibleHistoryTurns(
      mergedAncestorTurns,
      historyTurns,
    );
    const historyKey = ancestorCacheKey ? `${ancestorCacheKey}->${sourceKey}` : sourceKey;
    const historyViewModel = cachedHistoryViewModel(historyKey, viewMode, historyTurnsForView);
    if (!liveSession || liveSession.turns.length === 0) return historyViewModel;
    return mergeHistoryAndLiveViewModels(
      historyViewModel,
      liveSessionToAcpViewModel(liveSession),
    );
  }, [ancestorCacheKey, historyTurns, liveSession, mergedAncestorTurns, sourceKey, viewMode]);

  const liveTurnIdsKey = useMemo(
    () => liveSession?.turns.map((turn) => turn.turnId).join("|") ?? "",
    [liveSession],
  );
  const liveWorkingIndicatorTurnId = useMemo(
    () => liveWorkingIndicatorTurn(liveSession)?.turnId ?? "",
    [liveSession],
  );
  const displayItems = useMemo(
    () => cachedAcpRenderItems(acpViewModel, liveTurnIdsKey, liveWorkingIndicatorTurnId),
    [acpViewModel, liveTurnIdsKey, liveWorkingIndicatorTurnId],
  );
  const visibleDisplayItems = useMemo(() => {
    if (liveSession || historyRenderReady) return displayItems;
    if (displayItems.length <= INITIAL_HISTORY_RENDER_ITEMS) return displayItems;
    return displayItems.slice(-INITIAL_HISTORY_RENDER_ITEMS);
  }, [displayItems, historyRenderReady, liveSession]);
  const visibleDisplayItemKeys = useMemo(
    () => renderItemKeys(visibleDisplayItems),
    [visibleDisplayItems],
  );
  const liveActiveKey = useMemo(() => {
    if (!liveSession) return "";
    return liveSession.turns
      .filter((turn) => turn.status === "pending" || turn.status === "streaming" || turn.status === "cancelling")
      .map((turn) => turn.turnId)
      .join("|");
  }, [liveSession]);
  const sessionStateRuntimeSessionId = runtimeSessionId;
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
  const initialPositionMode = useMemo(() => {
    if (!available || !filePath || skipHistoryLoad) {
      return skipHistoryLoad ? "bottom" : null;
    }
    return hasMessageStreamScrollSnapshot(sourceKey) ? "restore" : "bottom";
  }, [available, filePath, skipHistoryLoad, sourceKey]);
  const keepInitialBottomLock = initialPositionMode === "bottom";

  const {
    bubbleRefs,
    chatContentRef,
    viewportRef,
    showScrollToBottom,
    positionReady,
    beginFollowingLiveStream,
    saveScrollSnapshot,
    scrollChatToBottom,
  } = useMessageStreamScrollController({
    sourceKey,
    available,
    filePath,
    skipHistoryLoad,
    loading: loading || ancestorsLoading,
    visibleDisplayItemCount: visibleDisplayItems.length,
    visibleDisplayItemKeys,
    liveActiveKey,
    liveCacheKey,
    initialPositionMode,
    keepInitialBottomLock,
  });

  useLayoutEffect(() => {
    if (!available || !filePath || skipHistoryLoad) {
      setLoading(false);
      setError(null);
      return;
    }
    const cached = historyCache.get(sourceKey);
    if (cached) {
      setHistoryTurns(cached.turns);
      setLoading(cached.messageCount !== messageCount);
      setError(null);
    } else {
      setHistoryTurns([]);
      setLoading(true);
      setError(null);
    }
    // Keep cached sessions fully rendered so the vertical scrollbar thumb
    // does not shrink after a delayed history expansion when switching back.
    setHistoryRenderReady(hasCachedHistory);
  }, [available, filePath, hasCachedHistory, messageCount, sourceKey, skipHistoryLoad]);

  useEffect(() => {
    if (liveSession || historyRenderReady) return;
    if (displayItems.length <= INITIAL_HISTORY_RENDER_ITEMS) {
      setHistoryRenderReady(true);
      return;
    }
    const timeout = window.setTimeout(() => {
      startTransition(() => setHistoryRenderReady(true));
    }, 80);
    return () => window.clearTimeout(timeout);
  }, [displayItems.length, historyRenderReady, liveSession]);

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
    const runtimeAgent = runtimeAgents.find((item) => item.agent === agent) ?? null;
    setComposerAgent(agent);
    setComposerModel(initialRuntimeModel(runtimeAgent));
    setComposerEffort(initialRuntimeEffort(runtimeAgent));
    setComposerPermissionMode(initialRuntimePermission(runtimeAgent));
    activeRuntimeTurnIdRef.current = null;
  }, [agent, sessionId]);

  useEffect(() => {
    if (agentModelOptions.some((option) => option.value === selectedAgentModelValue)) return;
    const current = runtimeAgents.find((item) => item.agent === composerAgent) ?? null;
    const next = current ?? runtimeAgents[0] ?? null;
    if (!next) return;
    setComposerAgent(next.agent);
    setComposerModel(initialRuntimeModel(next));
    setComposerEffort(initialRuntimeEffort(next));
    setComposerPermissionMode(initialRuntimePermission(next));
  }, [agentModelOptions, composerAgent, runtimeAgents, selectedAgentModelValue]);

  useEffect(() => {
    if (!selectedComposerAgent) return;
    if (
      composerPermissionMode &&
      selectedComposerAgent.permissionModes.some((option) => option.value === composerPermissionMode)
    ) {
      return;
    }
    setComposerPermissionMode(initialRuntimePermission(selectedComposerAgent));
  }, [
    composerPermissionMode,
    selectedComposerAgent?.agent,
    selectedComposerAgent?.permissionMode,
    selectedComposerAgent?.permissionModes,
  ]);

  const handleComposerAgentModelChange = useCallback(async (nextValue: string) => {
    const parsed = parseAgentModelSelectValue(nextValue);
    if (!parsed) return;
    const targetRuntimeAgent =
      runtimeAgents.find((runtimeAgent) => runtimeAgent.agent === parsed.agent) ?? null;
    if (!targetRuntimeAgent) return;
    setComposerAgent(parsed.agent);
    setComposerModel(parsed.model);
    setComposerEffort(initialRuntimeEffort(targetRuntimeAgent));
    setComposerPermissionMode(initialRuntimePermission(targetRuntimeAgent));
    try {
      await updateRuntimeAgentPreferences({ agent: parsed.agent, model: parsed.model });
    } catch (err) {
      setComposerError(String(err));
    }
  }, [runtimeAgents]);

  const handleComposerPermissionChange = useCallback(async (nextValue: string) => {
    if (!selectedComposerAgent) return;
    setComposerPermissionMode(nextValue);
    try {
      await updateRuntimeAgentPreferences({ agent: composerAgent, permissionMode: nextValue });
    } catch (err) {
      setComposerError(String(err));
    }
  }, [composerAgent, selectedComposerAgent]);

  const handleSendText = useCallback(async (
    rawText: string,
    clearComposer = false,
    inputAttachments: ComposerAttachment[] = [],
  ) => {
    const text = rawText.trim();
    if (!text || sending) return;
    const agentAttachments: AgentAttachment[] = await Promise.all(
      inputAttachments.map(async ({ path, mimeType, kind, previewDataUrl }) => {
        if (kind !== "image" || previewDataUrl) {
          return {
            path,
            mimeType,
            kind,
            previewDataUrl,
          };
        }
        try {
          return {
            path,
            mimeType,
            kind,
            previewDataUrl: await readLocalImageDataUrl(path),
          };
        } catch {
          return {
            path,
            mimeType,
            kind,
            previewDataUrl: null,
          };
        }
      }),
    );
    if (!workspacePath) {
      setComposerError("This session has no workspace path, so live chat cannot start yet.");
      return;
    }
    setSending(true);
    setComposerError(null);
    activeRuntimeTurnIdRef.current = null;
    const timestamp = Date.now();
    const targetAgent = composerAgent;
    const sameAgent = targetAgent === agent;
    console.info("[sessio-runtime:frontend:send]", {
      text,
      runtimeSessionId,
      targetAgent,
      workspacePath,
      sourceSessionId: sessionId,
    });
    if (sameAgent && !liveState.sessions[runtimeSessionId]) {
      dispatchLiveEvent({
        type: "ensure-session",
        session: pendingLiveSession({
          sessioRuntimeSessionId: runtimeSessionId,
          agent: targetAgent,
          workspacePath: workspacePath ?? "",
          capabilities: fallbackComposerCapabilities,
        }),
      });
    }
    if (sameAgent) {
      beginFollowingLiveStream();
      scrollChatToBottom();
    }
    try {
      const handle = sameAgent
        ? await ensureAgentRuntimeSession({
            agent: targetAgent,
            sessioRuntimeSessionId: runtimeSessionId,
            workspacePath,
            agentRuntimeSessionId: sessionId,
            sourceAgent: agent,
          })
        : await startAgentSession({
            agent: targetAgent,
            workspacePath,
            sourceAgent: agent,
            sourceSessionId: sessionId,
            options: runtimeSessionOptions(composerModel, composerPermissionMode, composerEffort),
          });
      if (sameAgent) {
        if (composerModel) {
          await setAgentSessionConfigOption(handle.sessioRuntimeSessionId, {
            configId: "model",
            value: composerModel,
          }).catch((err) => {
            console.warn("set model config failed", err);
          });
        }
        if (composerPermissionMode) {
          await setAgentSessionConfigOption(handle.sessioRuntimeSessionId, {
            configId: "mode",
            value: composerPermissionMode,
          }).catch((err) => {
            console.warn("set permission config failed", err);
          });
        }
        if (composerEffort) {
          await setAgentSessionConfigOption(handle.sessioRuntimeSessionId, {
            configId: runtimeEffortConfigId(targetAgent),
            value: composerEffort,
          }).catch((err) => {
            console.warn("set effort config failed", err);
          });
        }
      }
      const parentSnapshotTurns = sameAgent
        ? []
        : mergeHistoryWithLiveTurns(
            forkVisibleHistoryTurns(
              mergedAncestorTurns,
              historyTurns,
            ),
            liveSession?.turns ?? [],
          );
      if (!sameAgent) {
        dispatchSessionStartedFallback({
          dispatch: dispatchLiveEvent,
          handle,
          liveState,
          sequenceRef: fallbackRuntimeSequenceRef,
          timestamp,
        });
        dispatchLiveEvent({
          type: "ensure-session",
          session: pendingLiveSession({
            sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
            agent: handle.agent,
            workspacePath: handle.workspacePath,
            capabilities: handle.capabilities,
          }),
        });
        onPendingSession({
          sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
          agent: handle.agent,
          projectPath: workspacePath,
          projectName: workspacePath.split(/[/\\]/).filter(Boolean).pop() ?? workspacePath,
          prompt: text,
          timestamp,
          forkedFromAgent: agent,
          forkedFromId: sessionId,
          historySnapshots: [
            {
              ancestorAgent: agent,
              ancestorSessionId: sessionId,
              ancestorIndex: 0,
              turns: liveTurnsToSessionHistoryTurns(parentSnapshotTurns),
            },
          ],
        });
      }
      const inputAttachmentsWithContext = sameAgent
        ? agentAttachments
        : [
            ...agentAttachments,
            await crossContextAttachment({
              sourceAgent: agent,
              sourceSessionId: sessionId,
              sourceFilePath: filePath,
              turns: parentSnapshotTurns,
            }),
          ];
      const turn = await sendAgentInput(handle.sessioRuntimeSessionId, {
        text,
        attachments: inputAttachmentsWithContext,
      });
      activeRuntimeTurnIdRef.current = turn.turnId;
      if (clearComposer) {
        setComposerText("");
        clearAttachments();
      }
      window.requestAnimationFrame(() => composerRef.current?.focus());
    } catch (err) {
      const message = String(err);
      setComposerError(message);
    } finally {
      setSending(false);
    }
  }, [agent, beginFollowingLiveStream, clearAttachments, composerAgent, composerEffort, composerModel, composerPermissionMode, dispatchLiveEvent, fallbackComposerCapabilities, filePath, historyTurns, liveSession, liveState.lastSequence, liveState.sessions, mergedAncestorTurns, onPendingSession, runtimeSessionId, scrollChatToBottom, sending, sessionId, workspacePath]);

  const handleSend = useCallback(async () => {
    await handleSendText(composerText, true, attachments);
  }, [attachments, composerText, handleSendText]);

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
    <div className="flex-1 min-h-0 flex flex-col">
      <div className="relative flex flex-1 min-h-0 flex-col">
        <ScrollArea
          ref={viewportRef}
          className="flex-1 min-h-0"
          viewportClassName={
            "px-10 py-4 session-chat-scroll-viewport transition-opacity duration-75 " +
            (positionReady ? "opacity-100" : "opacity-0")
          }
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
          {!loading && !error && available && !skipHistoryLoad && visibleDisplayItems.length === 0 && (
            <div className="text-ink/40 text-body">{t("detail.no_messages")}</div>
          )}
          <div ref={chatContentRef} className="flex flex-col gap-2">
            <AcpSessionStatePanel
              state={acpViewModel.sessionState}
              sessioRuntimeSessionId={sessionStateRuntimeSessionId}
              debugAcpConfig={debugAcpConfig}
              onRunCommand={handleSendText}
            />
            {visibleDisplayItems.map((item, i) => (
              <div
                key={visibleDisplayItemKeys[i]}
                ref={(el) => {
                  bubbleRefs.current[i] = el;
                }}
                className={renderItemSide(item) === "user" ? "flex justify-end" : ""}
              >
                <AcpLiveItem
                  item={item}
                  sessioRuntimeSessionId={runtimeSessionId}
                  now={runtimeNow}
                  onPreviewImage={onPreviewImage}
                  onPreviewFile={onPreviewFile}
                  onFilePreviewError={onFilePreviewError}
                  onPermissionResponse={respondAgentPermission}
                />
              </div>
            ))}
          </div>
        </ScrollArea>
        <RoleNav
          sideKind="assistant"
          side="left"
          items={visibleDisplayItems}
          refs={bubbleRefs}
          viewportRef={viewportRef}
        />
        <RoleNav
          sideKind="user"
          side="right"
          items={visibleDisplayItems}
          refs={bubbleRefs}
          viewportRef={viewportRef}
        />
        {showScrollToBottom && (
          <button
            type="button"
            onClick={() => scrollChatToBottom()}
            className="absolute bottom-3 left-1/2 z-20 flex h-9 w-9 -translate-x-1/2 items-center justify-center rounded-full border border-ink/15 bg-surface-panel/95 text-ink/85 shadow-sm transition hover:border-ink/25 hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ink/20"
            aria-label="Scroll to bottom"
          >
            <ArrowDownToLine className="h-5 w-5" />
          </button>
        )}
      </div>
      <ChatComposer
        ref={composerRef}
        value={composerText}
        disabled={sending}
        active={Boolean(activeTurnId)}
        sending={sending}
        error={composerError}
        agentModelValue={selectedAgentModelValue}
        agentModelOptions={agentModelOptions}
        permissionMode={composerPermissionMode}
        permissionOptions={composerPermissionOptions}
        placeholder="Ask, Search or Chat..."
        attachments={attachments}
        supportsAttachments={supportsAttachments}
        supportsImageAttachments={supportsImageAttachments}
        supportsEmbeddedContext={supportsEmbeddedContext}
        onRemoveAttachment={removeAttachment}
        onPickAttachments={pickAttachments}
        onAgentModelChange={handleComposerAgentModelChange}
        onPermissionChange={handleComposerPermissionChange}
        onChange={setComposerText}
        onSend={handleSend}
        onCancel={handleCancelTurn}
      />
    </div>
  );
}

function pendingLiveSession(handle: {
  sessioRuntimeSessionId: string;
  agent: SessionInfo["agent"];
  workspacePath: string;
  capabilities: RuntimeCapabilitySet | null;
}): LiveRuntimeSession {
  return {
    sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
    agent: handle.agent,
    agentRuntimeSessionId: "pending",
    transport: "fake",
    workspacePath: handle.workspacePath,
    capabilities: handle.capabilities ?? {
      supportsCancel: true,
      supportsPermissions: true,
      supportsToolDeltas: true,
      supportsLoadSession: true,
      supportsResume: false,
      supportsFork: false,
      supportsImageAttachments: false,
      supportsAudioAttachments: false,
      supportsEmbeddedContext: false,
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
  sideKind,
  side,
  items,
  refs,
  viewportRef,
}: {
  sideKind: "assistant" | "user";
  side: "left" | "right";
  items: AcpRenderItem[];
  refs: React.RefObject<(HTMLDivElement | null)[]>;
  viewportRef: React.RefObject<HTMLDivElement | null>;
}) {
  const { t } = useI18n();
  const showTimerRef = useRef<number | undefined>(undefined);
  const jumpTimerRefs = useRef<number[]>([]);
  const roleIndices = useMemo(
    () =>
      items
        .map((item, i) => (renderItemSide(item) === sideKind ? i : -1))
        .filter((i) => i >= 0),
    [items, sideKind],
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
      for (const timer of jumpTimerRefs.current) {
        window.clearTimeout(timer);
      }
      jumpTimerRefs.current = [];
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
  const jumpToIndex = (idx: number) => {
    const vp = viewportRef.current;
    const el = refs.current[idx];
    if (!vp || !el) return;

    for (const timer of jumpTimerRefs.current) {
      window.clearTimeout(timer);
    }
    jumpTimerRefs.current = [];

    const align = () => {
      const nextEl = refs.current[idx];
      if (!nextEl) return;
      const vpRect = vp.getBoundingClientRect();
      const targetTop = nextEl.getBoundingClientRect().top - vpRect.top + vp.scrollTop;
      const maxTop = Math.max(0, vp.scrollHeight - vp.clientHeight);
      vp.scrollTop = Math.max(0, Math.min(targetTop, maxTop));
      activeRef.current = idx;
      setActiveIdx(idx);
    };

    align();
    window.requestAnimationFrame(() => {
      align();
      window.requestAnimationFrame(align);
    });
    for (const delay of [80, 180]) {
      jumpTimerRefs.current.push(window.setTimeout(align, delay));
    }
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
        const cleaned = previewTextForAcpItem(items[idx]);
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
              onClick={() => jumpToIndex(idx)}
              style={{ top: `${ratio * 100}%`, transform: "translateY(-50%)" }}
              className={
                "group absolute cursor-pointer p-1.5 transition-opacity duration-150 focus-visible:opacity-100 " +
                (navVisible ? "opacity-100 " : "pointer-events-none opacity-0 ") +
                (isLeft ? "left-1.5" : "right-1.5")
              }
              aria-label={t(
                sideKind === "assistant"
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
  agentModelValue: string;
  agentModelOptions: InlineMenuSelectOption[];
  permissionMode: string;
  permissionOptions: InlineMenuSelectOption[];
  placeholder: string;
  attachments: ComposerAttachment[];
  supportsAttachments: boolean;
  supportsImageAttachments: boolean;
  supportsEmbeddedContext: boolean;
  onRemoveAttachment: (path: string) => void;
  onPickAttachments: (kind: "images" | "files") => Promise<void>;
  onAgentModelChange: (value: string) => void;
  onPermissionChange: (permissionMode: string) => void;
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
    agentModelValue,
    agentModelOptions,
    permissionMode,
    permissionOptions,
    placeholder,
    attachments,
    supportsAttachments,
    supportsImageAttachments,
    supportsEmbeddedContext,
    onRemoveAttachment,
    onPickAttachments,
    onAgentModelChange,
    onPermissionChange,
    onChange,
    onSend,
    onCancel,
  },
  ref,
) {
  const { t } = useI18n();
  const canSend = value.trim().length > 0 && !disabled && !sending && !active;
  const canCancel = active && !disabled && !sending;
  const disabledTitle = disabled ? "Select a project-backed session to chat" : undefined;
  const innerRef = useRef<HTMLTextAreaElement | null>(null);
  const attachmentButtonRef = useRef<HTMLButtonElement>(null);
  const [attachmentMenuOpen, setAttachmentMenuOpen] = useState(false);
  const attachmentOptions = attachmentMenuOptions({
    supportsImageAttachments,
    supportsEmbeddedContext,
    imageLabel: t("new_chat.add_images"),
    fileLabel: t("new_chat.add_files"),
  });

  useEffect(() => {
    if (!supportsAttachments) setAttachmentMenuOpen(false);
  }, [supportsAttachments]);
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
    <div className="shrink-0 px-10 pb-4 bg-gradient-to-t from-surface-panel via-surface-panel to-surface-panel/80">
      <div className="w-full">
        {error && (
          <div className="mb-2 rounded-md border border-status-error/25 bg-status-error/10 px-3 py-2 text-body-sm text-status-error">
            {error}
          </div>
        )}
        <div
          className={
            "overflow-hidden rounded-2xl bg-ink/[0.055] shadow-[inset_0_0_0_1px_rgb(var(--color-ink)/0.08)] transition-shadow " +
            (error
              ? "shadow-[inset_0_0_0_1px_rgb(var(--color-status-error)/0.35)]"
              : "focus-within:shadow-[inset_0_0_0_1px_rgb(var(--color-ink)/0.20)]")
          }
          title={disabledTitle}
        >
          <ComposerAttachmentPreviewList
            attachments={attachments}
            onRemove={onRemoveAttachment}
          />
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
            className="chat-composer-textarea block w-full resize-none bg-transparent px-3.5 py-3.5 text-body leading-5 text-ink/88 placeholder:text-ink/38 outline-none disabled:cursor-not-allowed disabled:opacity-55"
          />
          <div className="flex h-12 items-center justify-between gap-3 px-3 pb-2">
            <div className="flex min-w-0 items-center gap-3">
              {supportsAttachments && (
                <Tooltip content={t("new_chat.add_context")} placement="top">
                  <button
                    ref={attachmentButtonRef}
                    type="button"
                    disabled={disabled}
                    onClick={() => setAttachmentMenuOpen((open) => !open)}
                    className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-ink/55 transition hover:bg-ink/8 hover:text-ink disabled:cursor-not-allowed disabled:opacity-45"
                    aria-label={t("new_chat.add_context")}
                    aria-expanded={attachmentMenuOpen}
                    aria-haspopup="menu"
                  >
                    <Plus className="h-5 w-5" />
                  </button>
                </Tooltip>
              )}
              <RuntimeMenuSelect
                value={permissionMode}
                options={permissionOptions}
                disabled={disabled}
                ariaLabel="Default permissions"
                onChange={onPermissionChange}
                menuPlacement="top"
              />
            </div>
            <div className="flex shrink-0 items-center gap-2.5">
              <RuntimeMenuSelect
                value={agentModelValue}
                options={agentModelOptions}
                disabled={disabled || sending || active}
                ariaLabel={t("new_chat.agent")}
                onChange={onAgentModelChange}
                menuPlacement="top"
              />
              <ChatComposerMenuButton icon={Mic} label={t("new_chat.voice")} disabled={disabled} />
              <button
                type="button"
                disabled={active ? !canCancel : !canSend}
                onClick={active ? onCancel : onSend}
                className="flex h-7 w-7 items-center justify-center rounded-full bg-ink/70 text-[rgb(var(--color-bg-panel))] transition hover:bg-ink disabled:cursor-not-allowed disabled:bg-ink/25 disabled:text-[rgb(var(--color-bg-panel)/0.7)]"
                aria-label={active ? "Stop" : sending ? t("new_chat.sending") : t("new_chat.send")}
              >
                {active ? <Square className="h-3.5 w-3.5 fill-current" /> : <ArrowUp className="h-5 w-5" />}
              </button>
            </div>
          </div>
        </div>
        {attachmentMenuOpen && attachmentButtonRef.current && (
          <ComposerAttachmentMenu
            anchor={attachmentButtonRef.current}
            options={attachmentOptions}
            onClose={() => setAttachmentMenuOpen(false)}
            onSelect={(key) => {
              void onPickAttachments(key);
            }}
          />
        )}
      </div>
    </div>
  );
});

function ChatComposerMenuButton({
  icon: Icon,
  label,
  disabled,
  text,
}: {
  icon: LucideIcon;
  label: string;
  disabled?: boolean;
  text?: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      className={
        "flex min-w-0 items-center gap-1.5 rounded-md py-1 text-body-sm text-ink/55 transition hover:bg-ink/8 hover:text-ink disabled:cursor-not-allowed disabled:opacity-45 " +
        (text ? "max-w-[220px] px-1.5" : "h-7 w-7 justify-center px-0")
      }
      aria-label={label}
    >
      <Icon className="h-4 w-4 shrink-0" />
      {text && <span className="truncate">{label}</span>}
      {text && <ChevronDown className="h-3.5 w-3.5 shrink-0" />}
    </button>
  );
}

function resizeTextareaToContent(el: HTMLTextAreaElement) {
  el.style.height = "auto";
  const lineHeight = parseFloat(getComputedStyle(el).lineHeight) || 20;
  const minHeight = lineHeight * 2;
  const maxHeight = lineHeight * 6;
  const nextHeight = Math.min(Math.max(el.scrollHeight, minHeight), maxHeight);
  el.style.height = `${nextHeight}px`;
  el.style.overflowY = el.scrollHeight > maxHeight ? "auto" : "hidden";
}

function AcpSessionStatePanel({
  state,
  sessioRuntimeSessionId,
  debugAcpConfig,
  onRunCommand,
}: {
  state: AcpSessionState;
  sessioRuntimeSessionId: string;
  debugAcpConfig: boolean;
  onRunCommand: (text: string) => Promise<void>;
}) {
  const hasPlan = Boolean(state.plan && state.plan.entries.length > 0);
  const hasCommands = debugAcpConfig && state.availableCommands.length > 0;
  const hasConfig = debugAcpConfig && state.configOptions.length > 0;
  const hasMode = debugAcpConfig && Boolean(state.currentModeId);
  const hasInfo = Boolean(state.sessionInfo?.title || hasMode);
  if (!hasPlan && !hasCommands && !hasConfig && !hasInfo) return null;
  return (
    <div className="mb-2 rounded-md border border-ink/[0.08] bg-ink/[0.025] px-3 py-2 text-body-sm">
      <div className="flex flex-wrap items-center gap-2">
        {state.sessionInfo?.title && (
          <span className="font-medium text-ink/80">{state.sessionInfo.title}</span>
        )}
        {hasMode && state.currentModeId && (
          <span className="rounded border border-ink/10 bg-bg-panel px-1.5 py-0.5 text-caption text-ink/55">
            Mode · {state.currentModeId}
          </span>
        )}
        {hasCommands && (
          <AcpCommandsMenu commands={state.availableCommands} onRunCommand={onRunCommand} />
        )}
        {debugAcpConfig && state.configOptions.map((option, index) => (
          <AcpConfigControl
            key={`${option.id || option.category || option.name}-${index}`}
            option={option}
            sessioRuntimeSessionId={sessioRuntimeSessionId}
          />
        ))}
      </div>
      {hasPlan && (
        <ol className="mt-2 space-y-1 border-t border-ink/[0.06] pt-2">
          {state.plan?.entries.map((entry, index) => (
            <li key={`${entry.content}-${index}`} className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-2">
              <span className={planStatusDotClass(entry.status)} />
              <span className="min-w-0 text-ink/70">{entry.content}</span>
              <span className="text-caption text-ink/35">{entry.priority}</span>
            </li>
          ))}
        </ol>
      )}
      {hasConfig && (
        <AcpConfigDebugPanel options={state.configOptions} />
      )}
    </div>
  );
}

function AcpCommandsMenu({
  commands,
  onRunCommand,
}: {
  commands: AcpAvailableCommand[];
  onRunCommand: (text: string) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [inputs, setInputs] = useState<Record<string, string>>({});
  const [pendingCommand, setPendingCommand] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const runCommand = (command: AcpAvailableCommand) => {
    if (pendingCommand) return;
    const extra = inputs[command.name]?.trim();
    const text = extra ? `/${command.name} ${extra}` : `/${command.name}`;
    setPendingCommand(command.name);
    setError(null);
    onRunCommand(text).then(() => {
      setOpen(false);
    }).catch((err) => {
      setError(String(err));
    }).finally(() => {
      setPendingCommand(null);
    });
  };
  return (
    <div className="relative">
      <button
        type="button"
        className="rounded border border-ink/10 bg-bg-panel px-1.5 py-0.5 text-caption text-ink/60 hover:text-ink/85"
        onClick={() => setOpen((value) => !value)}
      >
        Commands · {commands.length}
      </button>
      {open && (
        <div className="absolute left-0 top-full z-20 mt-1 w-72 rounded-md border border-ink/10 bg-bg-panel p-1 shadow-lg">
          {commands.map((command) => (
            <div key={command.name} className="rounded px-2 py-1.5">
              <div className="font-medium text-ink/75">{command.name}</div>
              {command.description && (
                <div className="text-caption text-ink/45">{command.description}</div>
              )}
              {command.input?.kind === "unstructured" && (
                <input
                  value={inputs[command.name] ?? ""}
                  onChange={(event) => {
                    setInputs((current) => ({
                      ...current,
                      [command.name]: event.target.value,
                    }));
                  }}
                  placeholder={command.input.hint ?? ""}
                  className="mt-1 w-full rounded border border-ink/10 bg-ink/[0.025] px-2 py-1 text-caption text-ink/70 outline-none focus:border-ink/25"
                />
              )}
              <div className="mt-1 flex items-center justify-between gap-2">
                {command.input?.kind === "unknown" ? (
                  <span className="text-caption text-ink/35">custom input</span>
                ) : <span />}
                <button
                  type="button"
                  disabled={Boolean(pendingCommand)}
                  onClick={() => runCommand(command)}
                  className="rounded border border-ink/10 bg-ink/[0.04] px-2 py-0.5 text-caption text-ink/60 hover:bg-ink/[0.07] hover:text-ink/80 disabled:cursor-not-allowed disabled:opacity-55"
                >
                  {pendingCommand === command.name ? "Running..." : "Run"}
                </button>
              </div>
            </div>
          ))}
          {error && <div className="px-2 py-1 text-caption text-status-error">{error}</div>}
        </div>
      )}
    </div>
  );
}

function AcpConfigControl({
  option,
  sessioRuntimeSessionId,
}: {
  option: AcpSessionConfigOption;
  sessioRuntimeSessionId: string;
}) {
  const [value, setValue] = useState(() => String(option.currentValue ?? ""));
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setValue(String(option.currentValue ?? ""));
  }, [option.currentValue]);
  const submitValue = (nextValue: string | boolean) => {
    if (!option.id || pending) return;
    setPending(true);
    setError(null);
    setAgentSessionConfigOption(sessioRuntimeSessionId, {
      configId: option.id,
      value: nextValue,
    }).catch((err) => {
      setError(String(err));
      setValue(String(option.currentValue ?? ""));
    }).finally(() => {
      setPending(false);
    });
  };
  if (option.type === "select") {
    const choices = [
      ...(option.options ?? []),
      ...(option.groups ?? []).flatMap((group) => group.options),
    ];
    return (
      <label className="flex items-center gap-1 text-caption text-ink/50">
        <span>{option.name}</span>
        <select
          value={value}
          disabled={pending}
          onChange={(event) => {
            const nextValue = event.target.value;
            setValue(nextValue);
            submitValue(nextValue);
          }}
          className="rounded border border-ink/10 bg-bg-panel px-1 py-0.5 text-caption text-ink/70"
          title={error ?? option.description ?? option.name}
        >
          {choices.map((choice) => (
            <option key={choice.value} value={choice.value}>
              {choice.name}
            </option>
          ))}
        </select>
      </label>
    );
  }
  if (option.type === "boolean") {
    return (
      <label className="flex items-center gap-1 text-caption text-ink/55">
        <input
          type="checkbox"
          checked={value === "true"}
          disabled={pending}
          onChange={(event) => {
            const nextValue = event.target.checked;
            setValue(String(nextValue));
            submitValue(nextValue);
          }}
          title={error ?? option.description ?? option.name}
        />
        <span>{option.name}</span>
      </label>
    );
  }
  return (
    <span className="rounded border border-ink/10 bg-bg-panel px-1.5 py-0.5 text-caption text-ink/50">
      {option.name}
    </span>
  );
}

function AcpConfigDebugPanel({
  options,
}: {
  options: AcpSessionConfigOption[];
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="mt-2 border-t border-ink/[0.06] pt-2">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="flex items-center gap-1 text-caption font-medium text-ink/50 hover:text-ink/75"
      >
        <ChevronRight className={"h-3.5 w-3.5 transition " + (open ? "rotate-90" : "")} />
        <span>Config · {options.length}</span>
      </button>
      {open && (
        <div className="mt-2 space-y-2">
          {options.map((option, index) => (
            <AcpConfigDebugOption
              key={`${option.id || option.name}-${index}`}
              option={option}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function AcpConfigDebugOption({
  option,
}: {
  option: AcpSessionConfigOption;
}) {
  const choices = [
    ...(option.options ?? []),
    ...(option.groups ?? []).flatMap((group) => group.options),
  ];
  return (
    <details className="rounded-md border border-ink/[0.08] bg-bg-panel/60 px-2 py-1.5 text-caption">
      <summary className="cursor-pointer text-ink/65">
        <span className="font-medium">{option.name}</span>
        {option.id && <span className="ml-1 text-ink/35">id={option.id}</span>}
        {option.type && <span className="ml-1 text-ink/35">type={option.type}</span>}
        {option.currentValue !== undefined && option.currentValue !== null && (
          <span className="ml-1 text-ink/35">current={String(option.currentValue)}</span>
        )}
      </summary>
      {option.description && (
        <div className="mt-1 text-ink/45">{option.description}</div>
      )}
      {choices.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1">
          {choices.map((choice) => (
            <span
              key={choice.value}
              className="rounded border border-ink/10 bg-ink/[0.035] px-1.5 py-0.5 text-ink/55"
              title={choice.description ?? choice.name}
            >
              {choice.value}
              {choice.name && choice.name !== choice.value ? ` · ${choice.name}` : ""}
            </span>
          ))}
        </div>
      )}
      <PlainTextContent text={JSON.stringify(option.raw, null, 2)} />
    </details>
  );
}

function planStatusDotClass(status: string): string {
  const base = "mt-1.5 h-1.5 w-1.5 rounded-full";
  if (status === "completed") return `${base} bg-[rgb(var(--color-emerald))]`;
  if (status === "in_progress") return `${base} bg-[rgb(var(--color-blue))]`;
  return `${base} bg-ink/25`;
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

function scrollBlockStartIntoView(el: HTMLElement | null) {
  if (!el) return;
  window.requestAnimationFrame(() => {
    window.requestAnimationFrame(() => {
      const scroller = findScroller(el);
      const rect = el.getBoundingClientRect();
      if (!scroller) {
        if (rect.top < 12) {
          el.scrollIntoView({ block: "start", behavior: "auto" });
        }
        return;
      }
      const scrollerRect = scroller.getBoundingClientRect();
      const delta = rect.top - scrollerRect.top - 12;
      if (delta < 0) {
        scroller.scrollTop += delta;
      }
    });
  });
}

function AcpLiveItem({
  item,
  sessioRuntimeSessionId,
  now,
  onPreviewImage,
  onPreviewFile,
  onFilePreviewError,
  onPermissionResponse,
}: {
  item: AcpRenderItem;
  sessioRuntimeSessionId: string;
  now: number;
  onPreviewImage: (image: MarkdownImage) => void;
  onPreviewFile: (file: FilePreview) => void;
  onFilePreviewError: (message: string) => void;
  onPermissionResponse: (
    sessioRuntimeSessionId: string,
    requestId: string,
    optionId: string,
  ) => Promise<void>;
}) {
  const { lang } = useI18n();
  if (item.kind === "turnStatus") {
    return <RuntimeStatusContent text={liveTurnStatusText(item.turn, now)} />;
  }
  if (item.kind === "workingIndicator") {
    return <LemniscateBloomIndicator />;
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
  if (item.block.kind === "sessionUpdate") {
    return (
      <AcpSessionUpdateView
        update={item.block}
        locale={localeTag(lang)}
      />
    );
  }
  return (
    <AcpContentBlockGroup
      block={item.block}
      timestamp={item.block.timestamp ?? item.turn.updatedAt}
      typewriterActive={
        isTypewriterTurn(item.turn) &&
        (item.block.kind === "assistant" || item.block.kind === "thought")
      }
      typewriterKey={`${item.turn.turnId}:${item.block.kind}`}
      messageFinished={isAcpMessageBlockFinished(item.turn, item.block)}
      onPreviewImage={onPreviewImage}
      onPreviewFile={onPreviewFile}
      onFilePreviewError={onFilePreviewError}
    />
  );
}

function AcpSessionUpdateView({
  update,
  locale,
}: {
  update: Extract<AcpRenderBlock, { kind: "sessionUpdate" }>;
  locale: string;
}) {
  const data = asRecord(update.data);
  const text = typeof data.text === "string" ? data.text : "";
  const timestamp =
    typeof data.timestamp === "number" ? data.timestamp : null;
  if (update.updateType === "file_edit") {
    return (
      <div className="text-body leading-relaxed break-words py-1">
        <FileEditContent text={text} />
      </div>
    );
  }
  if (update.updateType === "runtime_status") {
    return (
      <div className="py-1">
        <RuntimeStatusContent text={text} />
      </div>
    );
  }
  if (update.updateType === "turn_note") {
    return (
      <div className="flex items-center gap-2 py-1 text-body-sm italic text-ink/40">
        <span>{text}</span>
        {timestamp && (
          <span className="text-caption not-italic text-ink/30">
            {new Date(timestamp).toLocaleString(locale, {
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
  if (update.updateType === "plan") {
    return <PlanUpdateCard plan={update.data} />;
  }
  return (
    <PlainTextContent
      text={
        text ||
        `${update.updateType}\n${JSON.stringify(update.data, null, 2)}`
      }
    />
  );
}

function AcpContentBlockGroup({
  block,
  timestamp,
  typewriterActive = false,
  typewriterKey,
  messageFinished = true,
  onPreviewImage,
  onPreviewFile,
  onFilePreviewError,
}: {
  block: AcpRenderBlock;
  timestamp: number;
  typewriterActive?: boolean;
  typewriterKey?: string;
  messageFinished?: boolean;
  onPreviewImage: (image: MarkdownImage) => void;
  onPreviewFile: (file: FilePreview) => void;
  onFilePreviewError: (message: string) => void;
}) {
  if (block.kind !== "user" && block.kind !== "assistant" && block.kind !== "thought") {
    return null;
  }
  const isUser = block.kind === "user";
  const isThought = block.kind === "thought";
  const [thoughtExpanded, setThoughtExpanded] = useState(() => isThought && typewriterActive);
  const [messageExpanded, setMessageExpanded] = useState(() => !isThought && !isUser && typewriterActive);
  const [messageOverflowing, setMessageOverflowing] = useState(false);
  const messageGroupRef = useRef<HTMLDivElement>(null);
  const messageBodyRef = useRef<HTMLDivElement>(null);
  const label =
    isThought ? "Thinking" : block.kind === "assistant" ? "assistant" : "user";
  const userAttachmentBlocks = isUser ? block.blocks.filter(isUserAttachmentContentBlock) : [];
  const bodyBlocks = isUser
    ? block.blocks.filter((item) => !isUserAttachmentContentBlock(item))
    : block.blocks;
  const messageExpandButtonClass =
    "mt-2 flex items-center gap-1 border-t border-ink/[0.07] py-1.5 text-left text-body-sm text-ink/75 hover:bg-ink/[0.04] " +
    (isUser ? "-mx-4 w-[calc(100%+2rem)] px-4" : "w-full px-3");
  useLayoutEffect(() => {
    if (isThought || !messageFinished) {
      setMessageOverflowing(false);
      return;
    }
    const node = messageBodyRef.current;
    if (!node) return;
    const update = () => {
      const lineHeight = parseFloat(getComputedStyle(node).lineHeight) || 26;
      const collapsedHeight = lineHeight * MESSAGE_COLLAPSE_LINES;
      setMessageOverflowing(node.scrollHeight > collapsedHeight + 1);
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(node);
    return () => ro.disconnect();
  }, [bodyBlocks, isThought, messageExpanded, messageFinished]);
  return (
    <div
      ref={messageGroupRef}
      className={
        "text-body leading-relaxed break-words " +
        (isUser
          ? "w-fit max-w-[75%] rounded-lg border border-ink/[0.04] bg-ink/[0.06] px-4 pt-3 " +
            (messageOverflowing ? "pb-0" : "pb-3")
          : isThought
            ? "py-1.5 text-ink/55 text-body-sm"
            : "px-0 pt-1 text-ink/85 " + (messageOverflowing ? "pb-0" : "pb-1"))
      }
    >
      {isThought ? (
        <button
          type="button"
          className="mb-2 flex w-full items-center gap-2 text-left leading-none text-ink/55"
          onClick={() => setThoughtExpanded((value) => !value)}
          aria-expanded={thoughtExpanded}
        >
          <Brain className="h-3.5 w-3.5 shrink-0" />
          <span className="text-body-sm font-medium text-ink/75">
            {label}
          </span>
          <span className="text-ink/35">
            {thoughtExpanded ? (
              <ChevronDown className="h-3.5 w-3.5" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5" />
            )}
          </span>
          <span className="text-caption text-ink/30">
            {new Date(timestamp).toLocaleTimeString([], {
              hour: "2-digit",
              minute: "2-digit",
            })}
          </span>
        </button>
      ) : (
        <div className="mb-2 flex items-center gap-2 leading-none">
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
      )}
      <div className={isThought ? "ml-[1.375rem]" : ""}>
        {isThought && !thoughtExpanded ? null : (
          <div
            ref={isThought ? undefined : messageBodyRef}
            className={
              !isThought && !messageExpanded
                ? "message-body-clamp-20"
                : undefined
            }
          >
            {userAttachmentBlocks.length > 0 && (
              <AcpUserAttachmentStrip
                blocks={userAttachmentBlocks}
                onPreviewImage={onPreviewImage}
                onPreviewFile={onPreviewFile}
                onFilePreviewError={onFilePreviewError}
              />
            )}
            <AcpContentBlocks
              blocks={bodyBlocks}
              imageAlign={isUser ? "right" : undefined}
              typewriterActive={!isUser && typewriterActive}
              typewriterKey={typewriterKey}
              onPreviewImage={onPreviewImage}
            />
          </div>
        )}
      </div>
      {!isThought && messageFinished && messageOverflowing && (
        <button
          type="button"
          className={messageExpandButtonClass}
          onClick={() => {
            if (messageExpanded) {
              scrollBlockStartIntoView(messageGroupRef.current);
            }
            setMessageExpanded((value) => !value);
          }}
          aria-expanded={messageExpanded}
        >
          <span>{messageExpanded ? "Collapse" : "Expand"}</span>
          <ChevronDown className={"h-3.5 w-3.5 " + (messageExpanded ? "rotate-180" : "")} />
        </button>
      )}
    </div>
  );
}

function isUserAttachmentContentBlock(block: AcpContentBlock): boolean {
  return block.type === "image" || block.type === "resource" || block.type === "resource_link";
}

function resourceDisplayName(block: AcpContentBlock): string {
  if (block.type === "resource_link") {
    return block.title ?? block.name ?? basenameFromUri(block.uri) ?? "Resource";
  }
  if (block.type === "resource") {
    return block.name ?? basenameFromUri(block.uri ?? "") ?? "Embedded resource";
  }
  if (block.type === "image") {
    return basenameFromUri(block.uri ?? "") ?? block.mimeType ?? "Image";
  }
  return "Attachment";
}

function basenameFromUri(uri: string): string | null {
  if (!uri) return null;
  const decoded = uri.startsWith("file://") ? uri.slice("file://".length) : uri;
  const name = decoded.split(/[/\\]/).filter(Boolean).pop();
  return name || null;
}

function AcpAttachmentPill({
  block,
  onPreviewFile,
  onFilePreviewError,
}: {
  block: AcpContentBlock;
  onPreviewFile: (file: FilePreview) => void;
  onFilePreviewError: (message: string) => void;
}) {
  const label = resourceDisplayName(block);
  const handleClick = async () => {
    const inlineText = block.type === "resource" ? block.text : undefined;
    if (inlineText?.trim()) {
      onPreviewFile({ title: label, text: stripSessioUploadWrapper(inlineText) });
      return;
    }
    const path = filePathFromResourceBlock(block);
    if (!path) {
      onFilePreviewError("Cannot preview this file: no local file path is available.");
      return;
    }
    try {
      const text = await readLocalTextFile(path);
      onPreviewFile({ title: label, text });
    } catch (error) {
      onFilePreviewError(`Cannot preview ${label}: ${String(error)}`);
    }
  };
  return (
    <button
      type="button"
      onClick={() => {
        void handleClick();
      }}
      className="my-1 inline-flex max-w-[180px] items-center gap-1.5 rounded-full border border-ink/[0.085] bg-ink/[0.055] px-2.5 py-1 text-caption font-medium text-ink/72 shadow-sm transition hover:border-ink/[0.15] hover:bg-ink/[0.07] hover:text-ink focus:outline-none focus:ring-2 focus:ring-ink/15"
      title={label}
    >
      <FileText className="h-3.5 w-3.5 shrink-0 text-ink/45" />
      <span className="min-w-0 truncate">{compactFileLabel(label)}</span>
    </button>
  );
}

function compactFileLabel(label: string): string {
  const trimmed = label.trim();
  const maxChars = 28;
  if (trimmed.length <= maxChars) return trimmed;
  const dot = trimmed.lastIndexOf(".");
  const extension =
    dot > 0 && dot < trimmed.length - 1 && trimmed.length - dot <= 8
      ? trimmed.slice(dot)
      : "";
  const stem = extension ? trimmed.slice(0, -extension.length) : trimmed;
  const head = stem.slice(0, 14);
  const tailBudget = Math.max(4, maxChars - head.length - extension.length - 1);
  return `${head}...${stem.slice(-tailBudget)}${extension}`;
}

function filePathFromResourceBlock(block: AcpContentBlock): string | null {
  const uri =
    block.type === "resource" || block.type === "resource_link"
      ? block.uri
      : null;
  if (!uri) return null;
  if (uri.startsWith("file://")) return decodeURIComponent(uri.slice("file://".length));
  if (/^\/|^[A-Za-z]:[\\/]/.test(uri)) return uri;
  return null;
}

function AcpUserAttachmentStrip({
  blocks,
  onPreviewImage,
  onPreviewFile,
  onFilePreviewError,
}: {
  blocks: AcpContentBlock[];
  onPreviewImage: (image: MarkdownImage) => void;
  onPreviewFile: (file: FilePreview) => void;
  onFilePreviewError: (message: string) => void;
}) {
  const orderedBlocks = [
    ...blocks.filter((block) => block.type !== "image"),
    ...blocks.filter((block) => block.type === "image"),
  ];
  return (
    <div className="mb-2 flex flex-wrap items-end justify-end gap-2">
      {orderedBlocks.map((block, index) => (
        block.type === "image" ? (
          <AcpContentBlockView
            key={`image-${block.uri ?? block.data ?? index}`}
            block={block}
            imageAlign="right"
            coverImage
            onPreviewImage={onPreviewImage}
          />
        ) : (
          <AcpAttachmentPill
            key={`file-${resourceDisplayName(block)}-${index}`}
            block={block}
            onPreviewFile={onPreviewFile}
            onFilePreviewError={onFilePreviewError}
          />
        )
      ))}
    </div>
  );
}

function AcpContentBlocks({
  blocks,
  imageAlign,
  typewriterActive = false,
  typewriterKey,
  onPreviewImage,
}: {
  blocks: AcpContentBlock[];
  imageAlign?: "left" | "right";
  typewriterActive?: boolean;
  typewriterKey?: string;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  const visibleBlocks = useTypewriterContentBlocks(blocks, typewriterActive, typewriterKey);
  return (
    <div className="space-y-2">
      {visibleBlocks.map((block, index) => (
        <AcpContentBlockView
          key={index}
          block={block}
          imageAlign={imageAlign}
          onPreviewImage={onPreviewImage}
        />
      ))}
    </div>
  );
}

const TYPEWRITER_INITIAL_CHARS_PER_SECOND = 72;
const TYPEWRITER_MIN_CHARS_PER_SECOND = 36;
const TYPEWRITER_MAX_CHARS_PER_SECOND = 360;
const TYPEWRITER_DRAIN_CHARS_PER_SECOND = 220;
const TYPEWRITER_TICK_MS = 24;
const MESSAGE_COLLAPSE_LINES = 20;

function useTypewriterContentBlocks(
  blocks: AcpContentBlock[],
  active: boolean,
  typewriterKey: string | undefined,
): AcpContentBlock[] {
  const totalChars = useMemo(() => textContentLength(blocks), [blocks]);
  const streamKey = typewriterKey ?? "default";
  const [visibleChars, setVisibleChars] = useState(() => active ? 0 : totalChars);
  const visibleCharsRef = useRef(active ? 0 : totalChars);
  const stateRef = useRef({
    streamKey,
    targetChars: totalChars,
    lastTargetChars: totalChars,
    lastTargetAt: 0,
    charsPerSecond: TYPEWRITER_INITIAL_CHARS_PER_SECOND,
    wasActive: active,
  });

  useLayoutEffect(() => {
    const state = stateRef.current;
    const now = performance.now();
    const streamChanged = state.streamKey !== streamKey;
    if (streamChanged) {
      state.streamKey = streamKey;
      state.targetChars = totalChars;
      state.lastTargetChars = totalChars;
      state.lastTargetAt = now;
      state.charsPerSecond = TYPEWRITER_INITIAL_CHARS_PER_SECOND;
      state.wasActive = active;
      const initialVisible = active ? 0 : totalChars;
      visibleCharsRef.current = initialVisible;
      setVisibleChars(initialVisible);
      return;
    }

    if (!active && !state.wasActive) {
      state.targetChars = totalChars;
      state.lastTargetChars = totalChars;
      state.lastTargetAt = now;
      visibleCharsRef.current = totalChars;
      setVisibleChars(totalChars);
      return;
    }

    if (totalChars < state.targetChars) {
      state.targetChars = totalChars;
      state.lastTargetChars = totalChars;
      visibleCharsRef.current = Math.min(visibleCharsRef.current, totalChars);
      setVisibleChars((current) => Math.min(current, totalChars));
      return;
    }

    if (totalChars > state.targetChars) {
      const deltaChars = totalChars - state.targetChars;
      const elapsedMs = Math.max(48, now - state.lastTargetAt);
      const observedCharsPerSecond = deltaChars / (elapsedMs / 1000);
      const backlog = Math.max(0, totalChars - visibleCharsRef.current);
      const backlogBoost = backlog > 600 ? 1.9 : backlog > 260 ? 1.45 : backlog > 120 ? 1.2 : 1;
      state.charsPerSecond = clampNumber(
        observedCharsPerSecond * backlogBoost,
        TYPEWRITER_MIN_CHARS_PER_SECOND,
        TYPEWRITER_MAX_CHARS_PER_SECOND,
      );
      state.targetChars = totalChars;
      state.lastTargetChars = totalChars;
      state.lastTargetAt = now;
    } else {
      state.targetChars = totalChars;
    }

    if (!active) {
      state.charsPerSecond = Math.max(state.charsPerSecond, TYPEWRITER_DRAIN_CHARS_PER_SECOND);
    }
    state.wasActive = state.wasActive || active || visibleCharsRef.current < state.targetChars;
  }, [active, streamKey, totalChars]);

  const shouldAnimate = visibleChars < totalChars;
  useEffect(() => {
    if (!shouldAnimate) return;
    const timer = window.setInterval(() => {
      setVisibleChars((current) => {
        const target = stateRef.current.targetChars;
        if (current >= target) {
          visibleCharsRef.current = current;
          return current;
        }
        const step = Math.max(
          1,
          Math.ceil(stateRef.current.charsPerSecond * (TYPEWRITER_TICK_MS / 1000)),
        );
        const next = Math.min(target, current + step);
        visibleCharsRef.current = next;
        return next;
      });
    }, TYPEWRITER_TICK_MS);
    return () => window.clearInterval(timer);
  }, [shouldAnimate, streamKey, totalChars]);

  return useMemo(() => {
    if (!active && visibleChars >= totalChars) return blocks;
    return visibleContentBlocks(blocks, visibleChars);
  }, [active, blocks, totalChars, visibleChars]);
}

function textContentLength(blocks: AcpContentBlock[]): number {
  return blocks.reduce((total, block) => total + (block.type === "text" ? block.text.length : 0), 0);
}

function visibleContentBlocks(blocks: AcpContentBlock[], visibleChars: number): AcpContentBlock[] {
  const visible: AcpContentBlock[] = [];
  let consumedTextChars = 0;
  for (const block of blocks) {
    if (block.type !== "text") {
      if (visibleChars >= consumedTextChars) visible.push(block);
      continue;
    }
    const remaining = visibleChars - consumedTextChars;
    if (remaining <= 0) break;
    if (remaining >= block.text.length) {
      visible.push(block);
      consumedTextChars += block.text.length;
      continue;
    }
    visible.push({ ...block, text: block.text.slice(0, remaining) });
    break;
  }
  return visible;
}

function clampNumber(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, value));
}

function LemniscateBloomIndicator() {
  const groupRef = useRef<SVGGElement>(null);
  const pathRef = useRef<SVGPathElement>(null);
  const particlesRef = useRef<SVGCircleElement[]>([]);

  useEffect(() => {
    const group = groupRef.current;
    const path = pathRef.current;
    if (!group || !path) return;
    let frame = 0;
    const startedAt = performance.now();
    const render = (now: number) => {
      const time = now - startedAt;
      const progress = (time % LEMNISCATE_CONFIG.durationMs) / LEMNISCATE_CONFIG.durationMs;
      const detailScale = lemniscateDetailScale(time);
      path.setAttribute("d", lemniscatePath(detailScale));
      particlesRef.current.forEach((node, index) => {
        const particle = lemniscateParticle(index, progress, detailScale);
        node.setAttribute("cx", particle.x.toFixed(2));
        node.setAttribute("cy", particle.y.toFixed(2));
        node.setAttribute("r", particle.radius.toFixed(2));
        node.setAttribute("opacity", particle.opacity.toFixed(3));
      });
      frame = requestAnimationFrame(render);
    };
    frame = requestAnimationFrame(render);
    return () => cancelAnimationFrame(frame);
  }, []);

  return (
    <div className="flex justify-start text-ink/45" aria-label="Working">
      <svg
        className="h-4 w-8 overflow-visible"
        viewBox="18 34 64 32"
        fill="none"
        preserveAspectRatio="xMinYMid meet"
        aria-hidden="true"
      >
        <g ref={groupRef}>
          <path
            ref={pathRef}
            stroke="currentColor"
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={LEMNISCATE_STROKE_WIDTH}
            opacity="0.1"
          />
          {Array.from({ length: LEMNISCATE_CONFIG.particleCount }, (_, index) => (
            <circle
              key={index}
              ref={(node) => {
                if (node) particlesRef.current[index] = node;
              }}
              fill="currentColor"
            />
          ))}
        </g>
      </svg>
    </div>
  );
}

const LEMNISCATE_STROKE_WIDTH = 4.8;
const LEMNISCATE_CONFIG = {
  particleCount: 70,
  trailSpan: 0.4,
  durationMs: 2800,
  pulseDurationMs: 2600,
  lemniscateA: 20,
  lemniscateBoost: 7,
};

function lemniscatePoint(progress: number, detailScale: number): { x: number; y: number } {
  const t = normalizeUnitProgress(progress) * Math.PI * 2;
  const scale = LEMNISCATE_CONFIG.lemniscateA + detailScale * LEMNISCATE_CONFIG.lemniscateBoost;
  const denom = 1 + Math.sin(t) ** 2;
  return {
    x: 50 + (scale * Math.cos(t)) / denom,
    y: 50 + (scale * Math.sin(t) * Math.cos(t)) / denom,
  };
}

function lemniscateDetailScale(time: number): number {
  const pulseProgress = (time % LEMNISCATE_CONFIG.pulseDurationMs) / LEMNISCATE_CONFIG.pulseDurationMs;
  const pulseAngle = pulseProgress * Math.PI * 2;
  return 0.52 + ((Math.sin(pulseAngle + 0.55) + 1) / 2) * 0.48;
}

function lemniscatePath(detailScale: number, steps = 180): string {
  return Array.from({ length: steps + 1 }, (_, index) => {
    const point = lemniscatePoint(index / steps, detailScale);
    return `${index === 0 ? "M" : "L"} ${point.x.toFixed(2)} ${point.y.toFixed(2)}`;
  }).join(" ");
}

function lemniscateParticle(
  index: number,
  progress: number,
  detailScale: number,
): { x: number; y: number; radius: number; opacity: number } {
  const tailOffset = index / (LEMNISCATE_CONFIG_SAFE_PARTICLE_COUNT - 1);
  const point = lemniscatePoint(
    progress - tailOffset * LEMNISCATE_CONFIG.trailSpan,
    detailScale,
  );
  const fade = Math.pow(1 - tailOffset, 0.56);
  return {
    x: point.x,
    y: point.y,
    radius: 0.9 + fade * 2.7,
    opacity: 0.04 + fade * 0.96,
  };
}

const LEMNISCATE_CONFIG_SAFE_PARTICLE_COUNT = Math.max(2, LEMNISCATE_CONFIG.particleCount);

function normalizeUnitProgress(progress: number): number {
  return ((progress % 1) + 1) % 1;
}

function AcpContentBlockView({
  block,
  imageAlign,
  coverImage = false,
  onPreviewImage,
}: {
  block: AcpContentBlock;
  imageAlign?: "left" | "right";
  coverImage?: boolean;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  switch (block.type) {
    case "text":
      return (
        <div>
          <MarkdownContent
            text={block.text}
            onPreviewImage={onPreviewImage}
          />
          <AcpMetaBadges value={block} />
        </div>
      );
    case "image": {
      const mimeType = block.mimeType ?? "image";
      const src = block.uri || (block.data ? `data:${mimeType};base64,${block.data}` : "");
      return src ? (
        <div className={imageAlign === "right" ? "flex justify-end" : undefined}>
          <MarkdownImageButton
            image={{ alt: mimeType, src }}
            cover={coverImage}
            onPreviewImage={onPreviewImage}
          />
          <AcpMetaBadges value={block} />
        </div>
      ) : (
        <PlainTextContent text={JSON.stringify(block, null, 2)} />
      );
    }
    case "audio":
      return (
        <div className="rounded-md border border-ink/[0.08] bg-ink/[0.035] px-3 py-2 text-body-sm">
          <div className="font-medium text-ink/75">Audio</div>
          <div className="text-caption text-ink/45">{block.mimeType ?? "unknown"}</div>
          <AcpMetaBadges value={block} />
        </div>
      );
    case "resource_link":
      return (
        <div className="rounded-md border border-ink/[0.08] bg-ink/[0.035] px-3 py-2 text-body-sm">
          <div className="font-medium text-ink/75">{block.title ?? block.name ?? "Resource"}</div>
          {block.description && (
            <div className="text-caption text-ink/50">{block.description}</div>
          )}
          <div className="truncate font-mono text-caption text-ink/45">{block.uri}</div>
          {(block.mimeType || block.size !== undefined) && (
            <div className="text-caption text-ink/35">
              {[block.mimeType, block.size !== undefined ? `${block.size} bytes` : ""]
                .filter(Boolean)
                .join(" · ")}
            </div>
          )}
          <AcpMetaBadges value={block} />
        </div>
      );
    case "resource": {
      const uri = block.uri ?? "";
      const mimeType = block.mimeType ?? "";
      const text = block.text ?? "";
      const blob = block.blob ?? "";
      return (
        <div className="rounded-md border border-ink/[0.08] bg-ink/[0.035] px-3 py-2 text-body-sm">
          <div className="font-medium text-ink/75">
            {block.name ?? (uri || "Embedded resource")}
          </div>
          {mimeType && <div className="text-caption text-ink/45">{mimeType}</div>}
        {text ? (
          <div className="mt-2">
            <MarkdownContent
              text={text}
              onPreviewImage={onPreviewImage}
            />
          </div>
        ) : blob ? (
          <PlainTextContent text={`Embedded data: ${blob.length} base64 chars`} />
        ) : block.resource ? (
          <PlainTextContent text={JSON.stringify(block.resource, null, 2)} />
        ) : (
          <PlainTextContent text={JSON.stringify(block, null, 2)} />
        )}
          <AcpMetaBadges value={block} />
        </div>
      );
    }
    case "unknown":
      return <AcpUnknownCard title={`Content · ${block.originalType ?? "unknown"}`} value={block} />;
  }
}

function AcpToolCard({
  tool,
  onPreviewImage,
}: {
  tool: AcpToolCall;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  const displayTool = canonicalizeAcpTool(tool);
  if (isHiddenHistoryTool(displayTool)) return null;
  const taskEntries = parseTaskEntries(displayTool.rawInput);
  if (isPlanTool(displayTool)) {
    return (
        <TodoToolCard
          tool={displayTool}
          title={{ main: "Update Plan" }}
          iconName="TaskUpdate"
          todos={taskEntries}
          onPreviewImage={onPreviewImage}
        />
    );
  }
  if (isTodoTool(displayTool)) {
    return (
        <TodoToolCard
          tool={displayTool}
          title={{ main: todoToolTitle() }}
          iconName="TodoWrite"
          todos={taskEntries}
          onPreviewImage={onPreviewImage}
        />
    );
  }
  const detail = acpToolDisplayDetail(displayTool);
  const showToolPairs = !isFileToolWithoutPairs(displayTool.title);
  const input = showToolPairs ? detail.command : "";
  const output = showToolPairs ? acpToolOutputText(displayTool) : "";
  return (
    <ToolTimelineFrame title={detail.title} iconName={displayTool.title}>
      {(input || output) && (
        <div className="overflow-hidden rounded-md border border-card-border/[0.14] bg-bg-panel/65 text-body-sm">
          <ToolPairPanel
            input={input}
            output={output}
            outputContent={toolOutputContentBlocks(displayTool.rawOutput)}
            onPreviewImage={onPreviewImage}
          />
        </div>
      )}
    </ToolTimelineFrame>
  );
}

function acpToolInputText(tool: AcpToolCall): string {
  if (typeof tool.rawInput === "string") {
    return formatToolInputValue(tool.rawInput);
  }
  if (tool.rawInput !== null) return formatToolInputValue(tool.rawInput);
  return "";
}

function acpToolOutputText(tool: AcpToolCall): string {
  if (typeof tool.rawOutput === "string") return tool.rawOutput;
  if (toolOutputContentBlocks(tool.rawOutput).length > 0) return "";
  if (tool.rawOutput !== null) return JSON.stringify(tool.rawOutput, null, 2);
  return "";
}

function toolOutputContentBlocks(value: unknown): AcpContentBlock[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((item) => normalizeToolOutputContentBlock(item));
}

function normalizeToolOutputContentBlock(value: unknown): AcpContentBlock[] {
  const record = parseObjectLike(value);
  if (!record) return [];
  const nested = record.content;
  if (nested !== undefined && nested !== value) {
    const nestedBlocks = normalizeToolOutputContentBlock(nested);
    if (nestedBlocks.length > 0) return nestedBlocks;
  }
  const type = pickString(record.type) ?? pickString(record.kind);
  if (type === "text") {
    const text = pickString(record.text);
    return text ? [{ type: "text", text }] : [];
  }
  if (type === "image") {
    const uri = pickString(record.uri) ?? pickString(record.image_url) ?? pickString(record.url);
    const data = pickString(record.data);
    const mimeType =
      pickString(record.mimeType) ??
      pickString(record.mime_type) ??
      imageMimeTypeFromSrc(uri ?? "") ??
      "image/png";
    if (!uri && !data) return [];
    return [{
      type: "image",
      uri: uri ?? undefined,
      data: data ?? undefined,
      mimeType,
      meta: record.meta ?? null,
      annotations: record.annotations ?? null,
    }];
  }
  if (type === "input_image" || type === "image_url") {
    const src =
      pickString(record.image_url) ??
      pickString(record.url) ??
      pickString(record.uri) ??
      pickString(record.data);
    if (!src) return [];
    const mimeType =
      pickString(record.mimeType) ??
      pickString(record.mime_type) ??
      imageMimeTypeFromSrc(src) ??
      "image/png";
    return [{
      type: "image",
      uri: src.startsWith("data:") || isLikelyImageUrl(src) ? src : undefined,
      data: src.startsWith("data:") || isLikelyImageUrl(src) ? undefined : src,
      mimeType,
      meta: record.meta ?? null,
    }];
  }
  return [];
}

function imageMimeTypeFromSrc(src: string): string | null {
  const match = src.match(/^data:([^;]+);/i);
  return match?.[1]?.toLowerCase().startsWith("image/") ? match[1] : null;
}

function isLikelyImageUrl(src: string): boolean {
  return /^(https?:|asset:|blob:|file:\/\/|\/|[A-Za-z]:[\\/])/i.test(src);
}

function canonicalizeAcpTool(tool: AcpToolCall): AcpToolCall {
  const inlineTitle = splitInlineToolTitle(tool.title);
  const webActionDisplay = webActionToolDisplay(tool.rawInput);
  const display =
    webActionDisplay ??
    (inlineTitle
      ? {
          ...canonicalToolDisplay(inlineTitle.main, tool.rawInput ?? inlineTitle.detail, tool.kind),
          detail: inlineTitle.detail,
        }
      : canonicalToolDisplay(tool.title, tool.rawInput ?? "", tool.kind));
  const hidden = shouldHideTool(tool.title, tool.rawInput ?? "");
  if (display.main === tool.title && !display.detail && !hidden) return tool;
  const meta = parseObjectLike(tool.meta) ?? {};
  return {
    ...tool,
    title: toolDisplayName(display.main),
    meta: {
      ...meta,
      titleDetail: display.detail ?? pickString(meta.titleDetail) ?? undefined,
      hidden: hidden || meta.hidden === true,
    },
  };
}

function acpToolDisplayDetail(tool: AcpToolCall): { title: ToolTitleParts; command: string } {
  const input = parseObjectLike(tool.rawInput);
  if (!input) {
    const metaDetail = historyToolTitleDetail(tool);
    return {
      title: { main: tool.title, detail: metaDetail ?? undefined },
      command: acpToolInputText(tool),
    };
  }
  const title = acpToolTitle(tool, input);
  const command = pickToolInputDisplayText(input) ?? acpToolInputText(tool);
  return { title, command };
}

type ToolTitleParts = {
  main: string;
  detail?: string;
};

function acpToolTitle(tool: AcpToolCall, input: Record<string, unknown>): ToolTitleParts {
  const metaDetail = historyToolTitleDetail(tool);
  if (tool.title === "Read") {
    return metaDetail ? { main: tool.title, detail: metaDetail } : fileToolTitle(tool.title, input, true);
  }
  if (isFileMutationTool(tool.title)) {
    return metaDetail ? { main: tool.title, detail: metaDetail } : fileToolTitle(tool.title, input, false);
  }
  const description = pickString(input.description);
  return { main: tool.title, detail: metaDetail ?? description ?? undefined };
}

function fileToolTitle(title: string, input: Record<string, unknown>, includeRange: boolean): ToolTitleParts {
  const path = toolInputFilePath(input) ?? "";
  const basename = path ? basenameFromUri(path) ?? path : "";
  const range = includeRange ? readLineRange(input) : null;
  return { main: title, detail: [basename, range].filter(Boolean).join(" ") };
}

function isFileMutationTool(title: string): boolean {
  return title === "Write" || title === "Edit" || title === "MultiEdit" || title === "Delete" || title === "Move";
}

function isFileToolWithoutPairs(title: string): boolean {
  return title === "Read" || isFileMutationTool(title);
}

function historyToolTitleDetail(tool: AcpToolCall): string | null {
  const meta = parseObjectLike(tool.meta);
  return meta ? pickString(meta.titleDetail) : null;
}

function isHiddenHistoryTool(tool: AcpToolCall): boolean {
  const meta = parseObjectLike(tool.meta);
  return meta?.hidden === true;
}

function readLineRange(input: Record<string, unknown>): string | null {
  const offset = pickNumber(input.offset);
  const limit = pickNumber(input.limit);
  const startLine = pickNumber(input.start_line) ?? pickNumber(input.startLine);
  const endLine = pickNumber(input.end_line) ?? pickNumber(input.endLine);
  if (startLine !== null) {
    if (endLine !== null && endLine > startLine) return `(lines ${startLine}-${endLine})`;
    return `(line ${startLine})`;
  }
  if (offset === null) return null;
  if (limit === null || limit <= 1) return `(line ${offset})`;
  return `(lines ${offset}-${offset + limit - 1})`;
}

function pickString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function pickNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function parseObjectLike(value: unknown): Record<string, unknown> | null {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return value as Record<string, unknown>;
  }
  if (typeof value !== "string") return null;
  try {
    const parsed = JSON.parse(value) as unknown;
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : null;
  } catch {
    return null;
  }
}

function pickToolInputDisplayText(record: Record<string, unknown>): string | null {
  const webInput = webActionDisplayText(record);
  if (webInput) return webInput;
  const command = pickCommandText(record);
  if (command) return command;
  return formatToolInputValue(record);
}

function pickCommandText(record: Record<string, unknown>): string | null {
  const direct =
    pickString(record.command) ??
    pickString(record.cmd) ??
    pickString(record.input);
  if (direct) return formatToolInputValue(direct);
  for (const key of ["command", "cmd"]) {
    const value = record[key];
    if (Array.isArray(value)) {
      const parts = value
        .map((item) => pickString(item))
        .filter((item): item is string => Boolean(item));
      if (parts.length > 0) return parts.join(" ");
    }
  }
  return null;
}

function formatToolInputValue(value: unknown): string {
  const parsed = typeof value === "string" ? parseJsonInputString(value) : value;
  if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
    return formatObjectEntries(parsed as Record<string, unknown>);
  }
  if (parsed !== value) return JSON.stringify(parsed, null, 2);
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function parseJsonInputString(text: string): unknown {
  const trimmed = text.trim();
  if (!trimmed) return text;
  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    return text;
  }
}

function formatObjectEntries(record: Record<string, unknown>): string {
  return Object.entries(record)
    .map(([key, value]) => `${key}: ${formatObjectEntryValue(value)}`)
    .join("\n");
}

function formatObjectEntryValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (value === null || typeof value !== "object") return String(value);
  return JSON.stringify(value, null, 2);
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
    optionId: string,
  ) => Promise<void>;
}) {
  const [pendingChoice, setPendingChoice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const resolved = Boolean(permission.selectedOptionId || permission.cancelled);
  const options = permission.options.length > 0
    ? permission.options
    : [
        { optionId: "allow_once", name: "Allow once", kind: "allow_once", meta: null },
        { optionId: "reject_once", name: "Reject once", kind: "reject_once", meta: null },
      ];
  const detail = permissionDisplayDetail(permission);
  const respond = (optionId: string) => {
    if (resolved || pendingChoice) return;
    setPendingChoice(optionId);
    setError(null);
    onRespond(sessioRuntimeSessionId, permission.requestId, optionId).catch((err) => {
      setError(String(err));
      setPendingChoice(null);
    });
  };
  return (
    <ToolTimelineFrame
      title={{ main: "Permission", detail: detail.reason ?? undefined }}
      iconName="Permission"
    >
      <div className="space-y-2 text-body-sm">
        <div className="text-caption text-ink/45">
          {permissionStatusText(permission, pendingChoice)}
        </div>
        {detail.command && (
          <CodeScrollArea className="w-full">
            <pre className="whitespace-pre-wrap break-words font-mono text-caption leading-relaxed text-ink/75">
              <code>{detail.command}</code>
            </pre>
          </CodeScrollArea>
        )}
        {!resolved && (
          <div className="overflow-hidden rounded-md border border-status-warn/35 bg-status-warn/[0.06]">
            {options.map((option) => (
              <button
                key={option.optionId}
                type="button"
                disabled={Boolean(pendingChoice)}
                onClick={() => respond(option.optionId)}
                className={permissionOptionButtonClass(option.kind)}
              >
                {pendingChoice === option.optionId ? "Applying..." : option.name}
              </button>
            ))}
          </div>
        )}
        {error && <div className="text-caption text-status-error">{error}</div>}
      </div>
    </ToolTimelineFrame>
  );
}

function permissionOptionButtonClass(kind: string): string {
  void kind;
  return "block w-full border-b border-status-warn/25 px-3 py-2 text-left text-caption font-medium text-ink/70 transition last:border-b-0 hover:bg-status-warn/[0.08] hover:text-ink/85 disabled:cursor-not-allowed disabled:opacity-55";
}

function permissionStatusText(permission: AcpPermissionRequest, pendingChoice: string | null): string {
  if (pendingChoice) return "Applying permission decision";
  if (permission.cancelled) return "Cancelled";
  if (permission.selectedOptionId) return `Resolved · ${permission.selectedOptionId}`;
  return "Waiting for approval";
}

function permissionDisplayDetail(permission: AcpPermissionRequest): {
  reason: string | null;
  command: string | null;
} {
  const input = parseObjectLike(permission.input);
  const raw = parseObjectLike(permission.raw);
  const rawToolCall = parseObjectLike(permission.toolCall);
  const toolFields = parseObjectLike(rawToolCall?.fields) ?? rawToolCall;
  const reason =
    pickString(input?.reason) ??
    pickString(toolFields?.reason) ??
    pickString(raw?.reason) ??
    pickString(raw?.description) ??
    permission.toolName;
  const command =
    pickPermissionCommand(input) ??
    pickPermissionCommand(toolFields) ??
    pickPermissionCommand(raw);
  return { reason, command };
}

function pickPermissionCommand(record: Record<string, unknown> | null): string | null {
  if (!record) return null;
  const direct = pickCommandText(record);
  if (direct) return direct;
  const command = record.command;
  if (Array.isArray(command)) {
    const parts = command
      .map((item) => pickString(item))
      .filter((item): item is string => Boolean(item));
    if (parts.length > 0) return parts.join(" ");
  }
  const parsedCommand = record.parsedCommand;
  if (Array.isArray(parsedCommand)) {
    const parts = parsedCommand
      .map((item) => {
        const parsed = parseObjectLike(item);
        return parsed ? pickString(parsed.cmd) ?? pickString(parsed.command) : pickString(item);
      })
      .filter((item): item is string => Boolean(item));
    if (parts.length > 0) return parts.join("\n");
  }
  return null;
}

function AcpMetaBadges({ value }: { value: unknown }) {
  const record = asRecord(value);
  const meta = record.meta ?? record._meta;
  const annotations = record.annotations;
  const hasMeta = Boolean(meta);
  const hasAnnotations = Boolean(annotations);
  if (!hasMeta && !hasAnnotations) return null;
  return (
    <div className="mt-1 flex flex-wrap gap-1 text-caption">
      {hasAnnotations && (
        <details className="rounded border border-ink/10 px-1 py-0.5 text-ink/40">
          <summary className="cursor-pointer">
          annotations
          </summary>
          <PlainTextContent text={JSON.stringify(annotations, null, 2)} />
        </details>
      )}
      {hasMeta && (
        <details className="rounded border border-ink/10 px-1 py-0.5 text-ink/40">
          <summary className="cursor-pointer">
          _meta
          </summary>
          <PlainTextContent text={JSON.stringify(meta, null, 2)} />
        </details>
      )}
    </div>
  );
}

function AcpUnknownCard({ title, value }: { title: string; value: unknown }) {
  return (
    <details className="rounded-md border border-ink/[0.08] bg-ink/[0.035] px-3 py-2 text-body-sm">
      <summary className="cursor-pointer font-medium text-ink/65">{title}</summary>
      <div className="mt-2">
        <PlainTextContent text={JSON.stringify(value, null, 2)} />
      </div>
    </details>
  );
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

type AcpRenderItem =
  | { kind: "turnStatus"; turn: LiveTurn }
  | { kind: "workingIndicator"; turn: LiveTurn }
  | { kind: "block"; turn: LiveTurn; block: AcpRenderBlock }
  | { kind: "tool"; turn: LiveTurn; tool: AcpToolCall }
  | { kind: "permission"; turn: LiveTurn; permission: AcpPermissionRequest }
  | { kind: "error"; turn: LiveTurn; error: RuntimeError };

function acpViewModelToRenderItems(
  viewModel: AcpViewModel,
  liveTurnIds: Set<string>,
  workingIndicatorTurnId: string,
): AcpRenderItem[] {
  const items: AcpRenderItem[] = [];
  const latestLiveTurn = latestTurnWithIds(viewModel.turns, liveTurnIds);
  let lastUserIndex = -1;
  for (const turn of viewModel.turns) {
    const renderedTools = new Set<string>();
    const renderedPermissions = new Set<string>();
    turn.blocks.forEach((block) => {
      if (block.kind === "tool") {
        const originalTool = turn.tools.find((item) => item.toolId === block.toolId);
        if (!originalTool || renderedTools.has(originalTool.toolId)) return;
        renderedTools.add(originalTool.toolId);
        items.push({ kind: "tool", turn, tool: originalTool });
        return;
      }
      if (block.kind === "permission") {
        const permission = turn.permissions.find((item) => item.requestId === block.requestId);
        if (!permission || renderedPermissions.has(permission.requestId)) return;
        renderedPermissions.add(permission.requestId);
        items.push({ kind: "permission", turn, permission });
        return;
      }
      if (block.kind === "error") return;
      items.push({ kind: "block", turn, block });
      if (block.kind === "user") {
        lastUserIndex = items.length;
      }
    });
    if (turn.error) {
      items.push({ kind: "error", turn, error: turn.error });
    }
    if (turn.turnId === workingIndicatorTurnId) {
      items.push({ kind: "workingIndicator", turn });
    }
  }
  if (latestLiveTurn) {
    const insertAt = lastUserIndex >= 0 ? lastUserIndex : 0;
    items.splice(insertAt, 0, { kind: "turnStatus", turn: latestLiveTurn });
  }
  return items;
}

function latestTurnWithIds(turns: LiveTurn[], ids: Set<string>): LiveTurn | null {
  for (let index = turns.length - 1; index >= 0; index -= 1) {
    const turn = turns[index];
    if (ids.has(turn.turnId)) return turn;
  }
  return null;
}

function liveWorkingIndicatorTurn(liveSession: LiveRuntimeSession | null | undefined): LiveTurn | null {
  if (!liveSession) return null;
  for (let index = liveSession.turns.length - 1; index >= 0; index -= 1) {
    const turn = liveSession.turns[index];
    if (isTypewriterTurn(turn)) return turn;
  }
  return null;
}

function mergeHistoryAndLiveViewModels(
  historyViewModel: AcpViewModel,
  liveViewModel: AcpViewModel,
): AcpViewModel {
  if (historyViewModel.turns.length === 0) return liveViewModel;
  if (liveViewModel.turns.length === 0) return historyViewModel;
  return {
    ...liveViewModel,
    turns: mergeHistoryWithLiveTurns(historyViewModel.turns, liveViewModel.turns),
  };
}

async function crossContextAttachment({
  sourceAgent,
  sourceSessionId,
  sourceFilePath,
  turns,
}: {
  turns: LiveTurn[];
  sourceAgent: Agent;
  sourceSessionId: string;
  sourceFilePath: string;
}): Promise<AgentAttachment> {
  const content = buildCrossPromptFromTurns(turns, {
    sourceAgent,
    sourceSessionId,
    sourceFilePath,
  });
  const path = await writeCrossPrompt(
    sourceSessionId,
    content || "# Continued session from another agent\n",
  );
  const displayName = path.split(/[/\\]/).filter(Boolean).pop() || "sessio-cross-context.md";
  return {
    path,
    mimeType: "text/markdown",
    kind: "file",
    displayName,
  };
}

function cachedHistoryViewModel(
  sourceKey: string,
  viewMode: ViewMode,
  turns: LiveTurn[],
): AcpViewModel {
  const cacheKey = `${sourceKey}:${viewMode}:${turns.length}`;
  const cached = historyViewCache.get(cacheKey);
  if (cached?.turns === turns) return cached.viewModel;
  const viewModel = historyTurnsToAcpViewModel(filterHistoryTurnsForViewMode(turns, viewMode));
  historyViewCache.set(cacheKey, { sourceKey, viewMode, turns, viewModel });
  trimHistoryViewCache();
  return viewModel;
}

function normalizeSessionHistoryTurns(turns: SessionHistoryTurn[] | undefined): LiveTurn[] {
  if (!Array.isArray(turns)) return [];
  return turns as LiveTurn[];
}

function liveTurnsToSessionHistoryTurns(turns: LiveTurn[]): SessionHistoryTurn[] {
  return turns as SessionHistoryTurn[];
}

function filterHistoryTurnsForViewMode(turns: LiveTurn[], viewMode: ViewMode): LiveTurn[] {
  if (viewMode === "native") return turns;
  return turns
    .map((turn) => ({
      ...turn,
      blocks: turn.blocks.filter((block) =>
        block.kind === "user" || block.kind === "assistant" || block.kind === "thought",
      ),
      tools: [],
      permissions: [],
    }))
    .filter((turn) => turn.blocks.length > 0);
}

function cachedAcpRenderItems(
  viewModel: AcpViewModel,
  liveTurnIdsKey: string,
  workingIndicatorTurnId: string,
): AcpRenderItem[] {
  const cacheKey = `${liveTurnIdsKey}\u0002${workingIndicatorTurnId}`;
  const cachedByLiveTurns = renderItemsCache.get(viewModel);
  const cached = cachedByLiveTurns?.get(cacheKey);
  if (cached) return cached;
  const liveTurnIds = new Set(liveTurnIdsKey.split("|").filter(Boolean));
  const items = acpViewModelToRenderItems(viewModel, liveTurnIds, workingIndicatorTurnId);
  const next = cachedByLiveTurns ?? new Map<string, AcpRenderItem[]>();
  next.set(cacheKey, items);
  renderItemsCache.set(viewModel, next);
  return items;
}

function trimHistoryViewCache(): void {
  const maxEntries = 24;
  if (historyViewCache.size <= maxEntries) return;
  const overflow = historyViewCache.size - maxEntries;
  let removed = 0;
  for (const key of historyViewCache.keys()) {
    historyViewCache.delete(key);
    removed += 1;
    if (removed >= overflow) break;
  }
}

function liveTurnStatusText(turn: LiveTurn, now: number): string {
  const running =
    turn.status === "pending" ||
    turn.status === "streaming" ||
    turn.status === "cancelling";
  const elapsedMs = Math.max(0, (running ? now : turn.updatedAt) - turn.startedAt);
  const state = running ? "running" : turn.status === "completed" ? "completed" : "done";
  return `${state}|${formatDuration(elapsedMs)}`;
}

function renderItemKeys(items: AcpRenderItem[]): string[] {
  const blockCounts = new Map<string, number>();
  return items.map((item) => {
    if (item.kind !== "block") return renderItemKey(item);
    const count = blockCounts.get(item.turn.turnId) ?? 0;
    blockCounts.set(item.turn.turnId, count + 1);
    return `acp:${item.turn.turnId}:block:${count}`;
  });
}

function renderItemKey(item: AcpRenderItem): string {
  if (item.kind === "turnStatus") return `acp:${item.turn.turnId}:status`;
  if (item.kind === "workingIndicator") return `acp:${item.turn.turnId}:working`;
  if (item.kind === "block") return `acp:${item.turn.turnId}:block`;
  if (item.kind === "tool") return `acp:${item.turn.turnId}:tool:${item.tool.toolId}`;
  if (item.kind === "permission") return `acp:${item.turn.turnId}:permission:${item.permission.requestId}`;
  return `acp:${item.turn.turnId}:error`;
}

function renderItemSide(item: AcpRenderItem): "assistant" | "user" | "other" {
  if (item.kind !== "block") return "other";
  if (item.block.kind === "user") return "user";
  if (item.block.kind === "assistant") return "assistant";
  return "other";
}

function isTurnFinished(turn: LiveTurn): boolean {
  return turn.status === "completed" || turn.status === "failed" || turn.status === "cancelled";
}

function isTypewriterTurn(turn: LiveTurn): boolean {
  return turn.status === "pending" || turn.status === "streaming" || turn.status === "cancelling";
}

function isAcpMessageBlockFinished(turn: LiveTurn, block: AcpRenderBlock): boolean {
  if (block.kind !== "assistant" && block.kind !== "thought") return true;
  if (isTurnFinished(turn)) return true;
  const index = turn.blocks.indexOf(block);
  return index < 0 || index < turn.blocks.length - 1;
}

function previewTextForAcpItem(item: AcpRenderItem): string {
  if (
    item.kind !== "block" ||
    (
      item.block.kind !== "user" &&
      item.block.kind !== "assistant" &&
      item.block.kind !== "thought"
    )
  ) {
    return "";
  }
  const text = contentBlocksText(item.block.blocks);
  return item.block.kind === "user"
    ? stripInjectedContext(text)
    : stripImagePlaceholders(text);
}

function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.round(ms / 1000));
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}m ${seconds}s`;
}

function sessionIdFromRecord(input: Record<string, unknown>): string | null {
  const sessionId = input.session_id ?? input.sessionId;
  return typeof sessionId === "number" || typeof sessionId === "string"
    ? String(sessionId)
    : null;
}

function canonicalToolDisplay(name: string, body: unknown, kind?: string): ToolTitleParts {
  if (name === "apply_patch") {
    return { main: "Edit", detail: patchDisplayFile(patchInputText(body)) ?? undefined };
  }
  if (name === "write_stdin") {
    return { main: humanizeToolName(name), detail: writeStdinDisplayTarget(body) ?? undefined };
  }
  if (name === "web_search" || name === "WebSearch") return webSearchToolDisplay(body);
  if (isShellToolName(name)) {
    const cmd = toolInputCommand(body);
    if (!cmd) return { main: "Bash" };
    return commandToolDisplay(cmd);
  }
  if (isReadToolName(name)) {
    return { main: "Read", detail: fileToolDisplayDetail(body, true) ?? undefined };
  }
  if (isEditToolName(name)) {
    return { main: "Edit", detail: fileToolDisplayDetail(body, false) ?? undefined };
  }
  if (isGrepToolName(name)) {
    return { main: "Grep" };
  }
  if (name !== "exec_command" && name !== "shell_command") {
    const knownName = canonicalKnownToolName(name);
    if (knownName) return { main: knownName };
    const kindDisplay = canonicalToolKindDisplay(kind, body);
    if (kindDisplay) return kindDisplay;
    return { main: humanizeToolName(name) };
  }
  const cmd = toolInputCommand(body);
  if (!cmd) return { main: "Bash" };
  return commandToolDisplay(cmd);
}

function isShellToolName(name: string): boolean {
  return ["Shell", "Run Shell Command", "run_shell_command"].includes(name);
}

function isReadToolName(name: string): boolean {
  return ["ReadFile", "read_file"].includes(name);
}

function isEditToolName(name: string): boolean {
  return ["replace", "write_file", "WriteFile"].includes(name);
}

function isGrepToolName(name: string): boolean {
  return ["SearchText", "grep_search"].includes(name);
}

function splitInlineToolTitle(title: string): ToolTitleParts | null {
  const normalized = title.replace(/\s+/g, " ").trim();
  if (!normalized) return null;
  const match = normalized.match(/^(Read|List|LS|Edit|Write|MultiEdit|Search|Grep|Glob|Bash|WebFetch|WebSearch)\s+(.+)$/);
  if (!match) return null;
  const main = inlineToolMainName(match[1]);
  const detail = inlineToolDetail(match[2]);
  return detail ? { main, detail } : { main };
}

function inlineToolMainName(name: string): string {
  if (name === "LS") return "List";
  return name;
}

function inlineToolDetail(detail: string): string | undefined {
  const normalized = detail
    .split("|")
    .map((part) => part.trim())
    .filter((part) => part && !/^click(?:\s+to\s+copy)?$/i.test(part))
    .join(" ");
  if (!normalized) return undefined;
  return basenameFromUri(normalized) ?? normalized;
}

function toolInputCommand(body: unknown): string {
  const input = parseObjectLike(body);
  return input
    ? pickString(input.cmd) ?? pickString(input.command) ?? ""
    : typeof body === "string" ? body : "";
}

function fileToolDisplayDetail(body: unknown, includeRange: boolean): string | null {
  const input = parseObjectLike(body);
  if (!input) return null;
  const path = toolInputFilePath(input);
  const basename = path ? basenameFromUri(path) ?? path : "";
  const range = includeRange ? readLineRange(input) : null;
  const detail = [basename, range].filter(Boolean).join(" ");
  return detail || null;
}

function toolInputFilePath(input: Record<string, unknown>): string | null {
  return (
    pickString(input.file_path) ??
    pickString(input.filePath) ??
    pickString(input.path) ??
    pickString(input.absolute_path) ??
    pickString(input.absolutePath)
  );
}

function patchInputText(value: unknown): string {
  if (typeof value === "string") return value;
  const input = parseObjectLike(value);
  return input
    ? pickString(input.input) ?? pickString(input.patch) ?? pickString(input.text) ?? ""
    : "";
}

function shouldHideTool(name: string, body: unknown): boolean {
  const input = parseObjectLike(body);
  const action = input ? webActionRecord(input) : null;
  const actionType = action ? pickString(action.type) : null;
  if (actionType === "open_page") return !action || !firstWebActionUrl(action);
  if (name !== "web_search" && name !== "WebSearch") return false;
  return false;
}

function commandToolDisplay(command: string): ToolTitleParts {
  const first = firstShellCommandToken(command);
  if (["cat", "sed", "tail", "head", "nl"].includes(first)) {
    return { main: "Read", detail: commandDisplayFile(command) ?? undefined };
  }
  if (first === "rg" || first === "grep") return { main: "Grep" };
  if (first === "ls" || first === "find") return { main: "LS" };
  return { main: "Bash" };
}

function canonicalKnownToolName(name: string): string | null {
  const inlineTitle = splitInlineToolTitle(name);
  if (inlineTitle) return inlineTitle.main;
  switch (name) {
    case "Read":
    case "Write":
    case "Edit":
    case "MultiEdit":
    case "Delete":
    case "Move":
    case "Search":
    case "Grep":
    case "Glob":
    case "Bash":
    case "WebFetch":
    case "WebSearch":
    case "NotebookEdit":
    case "TodoWrite":
    case "ToolSearch":
    case "AskUserQuestion":
    case "TaskUpdate":
    case "Task":
    case "View Image":
      return name;
    case "LS":
    case "List":
      return "List";
    case "web_fetch":
    case "webfetch":
      return "WebFetch";
    case "web_search":
    case "websearch":
      return "WebSearch";
    case "read_file":
    case "ReadFile":
    case "read":
      return "Read";
    case "write":
      return "Write";
    case "replace":
    case "write_file":
    case "WriteFile":
    case "edit":
      return "Edit";
    case "multi_edit":
    case "MultiEditTool":
      return "MultiEdit";
    case "delete":
    case "remove":
    case "remove_file":
    case "delete_file":
    case "DeleteFile":
      return "Delete";
    case "move":
    case "move_file":
    case "MoveFile":
      return "Move";
    case "glob":
    case "find_files":
      return "Glob";
    case "list":
    case "ls":
    case "list_dir":
    case "list_directory":
    case "list_files":
      return "List";
    case "shell":
    case "bash":
    case "terminal":
      return "Bash";
    case "grep_search":
    case "SearchText":
      return "Grep";
    case "tool_search":
    case "toolsearch":
      return "ToolSearch";
    case "load_workspace_dependencies":
    case "install_workspace_dependencies":
      return "Read";
    case "automation_update":
      return "TaskUpdate";
    case "request_user_input":
      return "AskUserQuestion";
    case "read_thread_terminal":
      return "Bash";
    case "update_plan":
      return "TaskUpdate";
    case "task":
      return "Task";
    case "todo_write":
      return "TodoWrite";
    case "notebook_edit":
      return "NotebookEdit";
    case "view_image":
      return "View Image";
    default:
      return null;
  }
}

function canonicalToolKindDisplay(kind: string | undefined, body: unknown): ToolTitleParts | null {
  switch (kind) {
    case "read":
      return { main: "Read", detail: fileToolDisplayDetail(body, true) ?? undefined };
    case "edit":
      return { main: "Edit", detail: fileToolDisplayDetail(body, false) ?? undefined };
    case "delete":
      return { main: "Delete", detail: fileToolDisplayDetail(body, false) ?? undefined };
    case "move":
      return { main: "Move", detail: moveToolDisplayDetail(body) ?? fileToolDisplayDetail(body, false) ?? undefined };
    case "search":
      return { main: "Search" };
    case "execute": {
      const command = toolInputCommand(body);
      return command ? commandToolDisplay(command) : { main: "Bash" };
    }
    case "fetch":
      return webActionToolDisplay(body) ?? { main: "WebFetch" };
    case "think":
      return { main: "Think" };
    case "switch_mode":
      return { main: "Switch Mode" };
    default:
      return null;
  }
}

function moveToolDisplayDetail(body: unknown): string | null {
  const input = parseObjectLike(body);
  if (!input) return null;
  const source = toolInputFilePath(input);
  const target =
    pickString(input.new_path) ??
    pickString(input.newPath) ??
    pickString(input.destination) ??
    pickString(input.dest) ??
    pickString(input.target);
  const sourceLabel = source ? basenameFromUri(source) ?? source : "";
  const targetLabel = target ? basenameFromUri(target) ?? target : "";
  const detail = [sourceLabel, targetLabel].filter(Boolean).join(" -> ");
  return detail || null;
}

function humanizeToolName(name: string): string {
  const text = name.replace(/_/g, " ").trim();
  return text ? text.charAt(0).toUpperCase() + text.slice(1) : name;
}

function webSearchToolDisplay(body: unknown): ToolTitleParts {
  return webActionToolDisplay(body) ?? { main: "WebSearch" };
}

function webActionToolDisplay(body: unknown): ToolTitleParts | null {
  const input = parseObjectLike(body);
  const action = input ? webActionRecord(input) : null;
  const actionType = action ? pickString(action.type) : null;
  if (action && actionType === "open_page") {
    return { main: "WebFetch", detail: firstWebActionUrl(action) ?? undefined };
  }
  if (action && actionType === "search") {
    return { main: "WebSearch", detail: firstWebActionQuery(action) ?? undefined };
  }
  return null;
}

function webActionDisplayText(record: Record<string, unknown>): string | null {
  const action = webActionRecord(record);
  const actionType = pickString(action.type);
  if (actionType === "open_page") return firstWebActionUrl(action);
  if (actionType === "search") return webActionQueries(action).join("\n");
  return null;
}

function webActionRecord(record: Record<string, unknown>): Record<string, unknown> {
  const action = asRecord(record.action);
  return Object.keys(action).length > 0 ? action : record;
}

function firstWebActionUrl(action: Record<string, unknown>): string | null {
  return pickString(action.url) ?? webActionStringList(action, "urls")[0] ?? null;
}

function firstWebActionQuery(action: Record<string, unknown>): string | null {
  return webActionQueries(action)[0] ?? null;
}

function webActionQueries(action: Record<string, unknown>): string[] {
  return [
    ...webActionStringList(action, "queries"),
    ...webActionStringList(action, "query"),
  ];
}

function webActionStringList(record: Record<string, unknown>, key: string): string[] {
  const value = record[key];
  if (Array.isArray(value)) {
    return value
      .map((item) => pickString(item))
      .filter((item): item is string => Boolean(item));
  }
  const single = pickString(value);
  return single ? [single] : [];
}

function writeStdinDisplayTarget(body: unknown): string | null {
  const input = parseObjectLike(body);
  const sessionId = input ? sessionIdFromRecord(input) : null;
  if (sessionId) {
    return `session ${sessionId}`;
  }
  return null;
}

function commandDisplayFile(command: string): string | null {
  const tokens = shellTokens(command);
  for (let i = tokens.length - 1; i >= 0; i -= 1) {
    const token = stripShellTokenQuotes(tokens[i]);
    if (!token || token.startsWith("-") || /^[0-9,]+p$/.test(token)) continue;
    if (["cat", "sed", "tail", "head", "nl", "|"].includes(token)) continue;
    return basenameFromUri(token) ?? token;
  }
  return null;
}

function patchDisplayFile(patch: string): string | null {
  const match = patch.match(/^\*\*\* (?:Update|Add|Delete) File: (.+)$/m);
  const path = match?.[1]?.trim();
  return path ? basenameFromUri(path) ?? path : null;
}

function shellTokens(command: string): string[] {
  return command
    .trim()
    .split(/\s+/)
    .filter(Boolean);
}

function stripShellTokenQuotes(token: string): string {
  return token.replace(/^['"]|['"]$/g, "");
}

function firstShellCommandToken(command: string): string {
  const trimmed = command.trim();
  if (!trimmed) return "";
  const firstSegment = trimmed.split(/[|;&]/, 1)[0]?.trim() ?? trimmed;
  const tokens = firstSegment.split(/\s+/).filter(Boolean);
  const commandToken =
    tokens.find((token) => !/^[A-Za-z_][A-Za-z0-9_]*=/.test(token) && token !== "sudo") ?? "";
  return commandToken.split(/[/\\]/).pop() ?? commandToken;
}

function toolDisplayName(name: string): string {
  if (name === "web_search") return "Searching web";
  if (name === "LS") return "List";
  return name;
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
  const pendingBottomStickRef = useRef(false);
  const fileEditBlockRef = useRef<HTMLDivElement>(null);
  const detailRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const visibleEdits = expanded ? edits : edits.slice(0, 3);
  const hiddenCount = Math.max(0, edits.length - visibleEdits.length);
  const setExpanded = (nextExpanded: boolean) => {
    setExpandedState({ text, expanded: nextExpanded });
  };

  useEffect(() => {
    setOpenDetails(new Set());
    pendingScrollKeyRef.current = null;
    pendingBottomStickRef.current = false;
    detailRefs.current.clear();
  }, [text]);

  const toggleDetail = (key: string) => {
    setOpenDetails((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        const scroller = findScroller(fileEditBlockRef.current);
        pendingBottomStickRef.current = scroller instanceof HTMLDivElement
          ? isNearScrollBottom(scroller)
          : false;
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
    const stickToBottom = pendingBottomStickRef.current;
    pendingBottomStickRef.current = false;
    const align = () => {
      const scroller = findScroller(node);
      if (!scroller) {
        node.scrollIntoView({ block: "end", behavior: "auto" });
        return;
      }
      if (stickToBottom) {
        scroller.scrollTop = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
        return;
      }
      const nodeBottom = node.getBoundingClientRect().bottom;
      const scrollerBottom = scroller.getBoundingClientRect().bottom;
      const delta = nodeBottom - scrollerBottom + 12;
      if (delta > 0) {
        scroller.scrollTop += delta;
      }
    };
    window.requestAnimationFrame(() => {
      align();
      window.requestAnimationFrame(align);
    });
    const timers = [80, 180, 360].map((delay) => window.setTimeout(align, delay));
    return () => {
      timers.forEach((timer) => window.clearTimeout(timer));
    };
  }, [openDetails]);

  return (
    <div ref={fileEditBlockRef} className="overflow-hidden rounded-md bg-ink/[0.035]">
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
            const detailKey = `${i}:${edit.path || edit.displayPath || label}`;
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
              onClick={() => {
                scrollBlockStartIntoView(fileEditBlockRef.current);
                setExpanded(false);
              }}
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

interface MarkdownImage {
  alt: string;
  src: string;
}

function PlainTextContent({ text }: { text: string }) {
  if (!text.trim()) return null;
  return (
    <CodeScrollArea className="w-full">
      <pre className="whitespace-pre-wrap break-words font-mono text-caption leading-relaxed">
        <code>{text}</code>
      </pre>
    </CodeScrollArea>
  );
}

function CodeScrollArea({
  children,
  className = "",
  viewportClassName = "",
}: {
  children: ReactNode;
  className?: string;
  viewportClassName?: string;
}) {
  return (
    <ScrollArea
      className={"min-w-0 " + className}
      viewportClassName={"pb-2 " + viewportClassName}
      orientation="horizontal"
      persistScrollbars
    >
      {children}
    </ScrollArea>
  );
}

function RuntimeStatusContent({ text }: { text: string }) {
  const [state = "running", duration = "0s"] = text.split("|");
  const running = state === "running";
  const successful = state === "completed";
  return (
    <div className="flex items-center gap-2 text-body-sm text-ink/50">
      <span
        className={
          "h-1.5 w-1.5 shrink-0 rounded-full " +
          (running || successful ? "bg-[rgb(var(--color-emerald))]" : "bg-ink/30")
        }
      />
      <span>{running ? "Working for" : "Worked for"} {duration}</span>
    </div>
  );
}

function ToolPairRow({
  label,
  text,
  expanded,
  maxLines = 3,
  content = [],
  onPreviewImage,
}: {
  label: string;
  text: string;
  expanded: boolean;
  maxLines?: number;
  content?: AcpContentBlock[];
  onPreviewImage?: (image: MarkdownImage) => void;
}) {
  const shouldClampContent =
    !expanded && shouldClampToolPairContent(content, maxLines);
  const contentClassName = [
    text.trim() ? "mt-2 space-y-2" : "space-y-2",
    shouldClampContent ? `tool-pair-clamp-${maxLines}` : "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <div className="border-b border-card-border/[0.12] last:border-b-0">
      <div className="grid grid-cols-[2.25rem_minmax(0,1fr)] gap-2 px-3 py-2">
        <div className="font-mono text-[10px] leading-relaxed text-ink/35">
          {label}
        </div>
        <div className="min-w-0">
          {text.trim() && (
            expanded ? (
              <CodeScrollArea className="w-full">
                <pre className="min-w-0 whitespace-pre-wrap break-words font-mono text-caption leading-relaxed text-ink/75">
                  <code>{text}</code>
                </pre>
              </CodeScrollArea>
            ) : (
              <pre
                className={
                  "min-w-0 whitespace-pre-wrap break-words font-mono text-caption leading-relaxed text-ink/75 " +
                  `tool-pair-clamp-${maxLines}`
                }
              >
                <code>{text}</code>
              </pre>
            )
          )}
          {content.length > 0 && onPreviewImage && (
            <div className={contentClassName}>
              {content.map((block, index) => (
                <AcpContentBlockView
                  key={`tool-output-content-${index}`}
                  block={block}
                  coverImage
                  onPreviewImage={onPreviewImage}
                />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ToolPairPanel({
  input,
  output,
  outputContent = [],
  onPreviewImage,
}: {
  input: string;
  output: string;
  outputContent?: AcpContentBlock[];
  onPreviewImage?: (image: MarkdownImage) => void;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const stateKey = `${input}\u0000${output}\u0000${outputContent.length}`;
  const [expandedState, setExpandedState] = useState(() => ({
    key: stateKey,
    expanded: false,
  }));
  const expanded =
    expandedState.key === stateKey ? expandedState.expanded : false;
  const canExpand =
    shouldClampToolPairText(input, 3) ||
    shouldClampToolPairText(output, 3) ||
    shouldClampToolPairContent(outputContent, 3);
  return (
    <div ref={panelRef}>
      {input && (
        <ToolPairRow
          label="IN"
          text={input}
          expanded={expanded}
          maxLines={3}
        />
      )}
      {(output || outputContent.length > 0) && (
        <ToolPairRow
          label="OUT"
          text={output}
          expanded={expanded}
          maxLines={3}
          content={outputContent}
          onPreviewImage={onPreviewImage}
        />
      )}
      {canExpand && (
        <button
          type="button"
          className="flex w-full items-center gap-1 px-3 py-1.5 text-left text-body-sm text-ink/75 hover:bg-ink/[0.04]"
          onClick={() => {
            if (expanded) {
              scrollBlockStartIntoView(panelRef.current);
            }
            setExpandedState({ key: stateKey, expanded: !expanded });
          }}
        >
          <span>{expanded ? "Collapse" : "Expand"}</span>
          <ChevronDown className={"h-3.5 w-3.5 " + (expanded ? "rotate-180" : "")} />
        </button>
      )}
    </div>
  );
}

function shouldClampToolPairText(text: string, maxLines: number): boolean {
  const estimatedVisualLines = text
    .split(/\r?\n/)
    .reduce((count, line) => count + Math.max(1, Math.ceil(line.length / 72)), 0);
  return estimatedVisualLines > maxLines;
}

function shouldClampToolPairContent(
  content: AcpContentBlock[],
  maxLines: number,
): boolean {
  if (content.length === 0) return false;
  const text = contentTextForClamp(content);
  return (
    (text.trim() ? shouldClampToolPairText(text, maxLines) : false) ||
    content.length > 1
  );
}

function contentTextForClamp(content: AcpContentBlock[]): string {
  return content
    .map((block) => {
      if (block.type === "text") return block.text;
      if (block.type === "resource") return block.text ?? "";
      return "";
    })
    .filter(Boolean)
    .join("\n");
}

type TodoEntry = {
  content: string;
  activeForm?: string;
  status?: string;
};

function TodoToolCard({
  tool,
  title,
  iconName,
  todos,
  onPreviewImage,
}: {
  tool: AcpToolCall;
  title: ToolTitleParts;
  iconName: string;
  todos: TodoEntry[];
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  if (todos.length === 0) {
    return (
        <AcpToolCardFallback
          tool={tool}
          title={title}
          iconName={iconName}
          onPreviewImage={onPreviewImage}
        />
      );
  }
  return <TaskListFrame title={title} iconName={iconName} todos={todos} />;
}

function TaskListFrame({
  title,
  iconName,
  todos,
}: {
  title: ToolTitleParts;
  iconName: string;
  todos: TodoEntry[];
}) {
  return (
    <ToolTimelineFrame title={title} iconName={iconName}>
      <div className="space-y-1 text-body-sm">
        {todos.map((todo, index) => (
          <div key={`${todo.content}-${index}`} className="flex items-start gap-2 text-body-sm text-ink/80">
            <span className="mt-0.5 flex w-4 shrink-0 justify-center text-ink/45">
              <TodoStatusIcon status={todo.status} />
            </span>
            <span className={todo.status === "completed" ? "text-ink/45 line-through" : ""}>
              {todo.content}
            </span>
          </div>
        ))}
      </div>
    </ToolTimelineFrame>
  );
}

function PlanUpdateCard({
  plan,
}: {
  plan: unknown;
}) {
  const todos = parseTaskEntries(plan);
  return (
    <TaskListFrame
      title={{ main: "Update Plan" }}
      iconName="TaskUpdate"
      todos={todos}
    />
  );
}

function AcpToolCardFallback({
  tool,
  title,
  iconName,
  onPreviewImage,
}: {
  tool: AcpToolCall;
  title: ToolTitleParts;
  iconName: string;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  const input = acpToolInputText(tool);
  const output = acpToolOutputText(tool);
  return (
    <ToolTimelineFrame title={title} iconName={iconName}>
      <div className="overflow-hidden rounded-md border border-card-border/[0.14] bg-bg-panel/65 text-body-sm">
        <ToolPairPanel
          input={input}
          output={output}
          outputContent={toolOutputContentBlocks(tool.rawOutput)}
          onPreviewImage={onPreviewImage}
        />
      </div>
    </ToolTimelineFrame>
  );
}

function ToolTimelineFrame({
  title,
  iconName,
  children,
}: {
  title: ToolTitleParts;
  iconName: string;
  children: ReactNode;
}) {
  return (
    <div className="text-body-sm">
      <div className="mb-2 flex items-center gap-2">
        <span className="flex w-4 shrink-0 justify-center">
          <ToolTitleIcon name={iconName} />
        </span>
        <span className="min-w-0 truncate text-ink/70">
          <span className="font-medium text-ink/75">{title.main}</span>
          {title.detail && (
            <span className="ml-1 font-normal text-ink/50">{title.detail}</span>
          )}
        </span>
      </div>
      <div className="ml-6">
        {children}
      </div>
    </div>
  );
}

function ToolTitleIcon({ name }: { name: string }) {
  const className = "h-3.5 w-3.5";
  switch (name) {
    case "TodoWrite":
      return <ListTodo className={className} aria-label="Todo" />;
    case "Read":
      return <BookOpen className={className} aria-label="Read" />;
    case "Write":
    case "Edit":
    case "MultiEdit":
      return <Pen className={className} aria-label="Edit" />;
    case "Delete":
      return <Trash2 className={className} aria-label="Delete" />;
    case "Move":
      return <MoveRight className={className} aria-label="Move" />;
    case "Bash":
      return <SquareTerminal className={className} aria-label="Terminal" />;
    case "Search":
    case "Grep":
      return <Search className={className} aria-label="Search" />;
    case "Glob":
      return <FileSearch className={className} aria-label="Find files" />;
    case "LS":
    case "List":
      return <FolderOpen className={className} aria-label="List files" />;
    case "WebFetch":
    case "WebSearch":
      return <Globe className={className} aria-label="Web" />;
    case "NotebookEdit":
      return <Code2 className={className} aria-label="Notebook edit" />;
    case "ToolSearch":
      return <SearchCheck className={className} aria-label="Tool search" />;
    case "AskUserQuestion":
      return <MessageCircleQuestionMark className={className} aria-label="Ask user" />;
    case "TaskUpdate":
      return <ListChecks className={className} aria-label="Task update" />;
    case "Permission":
      return <UserKey className={className} aria-label="Permission" />;
    case "View Image":
      return <ImageIcon className={className} aria-label="View image" />;
    case "Task":
      return <ClipboardList className={className} aria-label="Task" />;
    case "Think":
      return <Brain className={className} aria-label="Think" />;
    case "Switch Mode":
      return <ClipboardList className={className} aria-label="Switch mode" />;
    default:
      return <Wrench className={className} aria-label={name || "Tool"} />;
  }
}

function parseTaskEntries(value: unknown): TodoEntry[] {
  const entries = value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>).entries
    : null;
  return parseTaskListEntries(entries, ["content"]);
}

function parseTaskListEntries(value: unknown, contentKeys: string[]): TodoEntry[] {
  const todos = value;
  if (!Array.isArray(todos)) return [];
  return todos.flatMap((item) => {
    if (!item || typeof item !== "object" || Array.isArray(item)) return [];
    const record = item as Record<string, unknown>;
    const content = contentKeys
      .map((key) => pickString(record[key]))
      .find((text): text is string => Boolean(text));
    if (!content) return [];
    return [{
      content,
      activeForm: pickString(record.activeForm) ?? undefined,
      status: pickString(record.status) ?? undefined,
    }];
  });
}

function TodoStatusIcon({ status }: { status?: string }) {
  if (status === "completed") {
    return <CheckSquare className="h-3.5 w-3.5" aria-label="Completed" />;
  }
  if (status === "in_progress") {
    return <LoaderCircle className="h-3.5 w-3.5" aria-label="In progress" />;
  }
  return <Square className="h-3.5 w-3.5" aria-label="Pending" />;
}

function isTodoTool(tool: AcpToolCall): boolean {
  const meta = tool.meta;
  if (meta && typeof meta === "object" && !Array.isArray(meta)) {
    const role = (meta as Record<string, unknown>).role;
    if (role === "todo") return true;
  }
  return tool.kind === "todo" || tool.title === "TodoWrite";
}

function isPlanTool(tool: AcpToolCall): boolean {
  return tool.title === "TaskUpdate" || tool.title === "update_plan";
}

function todoToolTitle(): string {
  return "Update Todos";
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
  const components = useMemo(
    () => createMarkdownComponents(onPreviewImage),
    [onPreviewImage],
  );
  return (
    <div className="markdown-content">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkBreaks, remarkMath]}
        rehypePlugins={[rehypeRaw, [rehypeSanitize, markdownSanitizeSchema], rehypeKatex]}
        components={components}
        urlTransform={markdownUrlTransform}
      >
        {safeText}
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
      <CodeScrollArea
        className="my-2 w-full rounded-md border border-card-border/[0.16] bg-bg-panel-alt"
        viewportClassName="px-3 pt-2"
      >
        <pre className="text-caption leading-relaxed">
          {children}
        </pre>
      </CodeScrollArea>
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
    img: ({ src, alt }) => {
      const image = { src: src ?? "", alt: alt ?? "image" };
      if (!isRenderableMarkdownImageSrc(image.src)) {
        return <MarkdownImageFallback image={image} />;
      }
      return (
        <MarkdownImageButton
          image={image}
          onPreviewImage={onPreviewImage}
        />
      );
    },
  };
}

function MarkdownImageButton({
  image,
  cover = false,
  onPreviewImage,
}: {
  image: MarkdownImage;
  cover?: boolean;
  onPreviewImage: (image: MarkdownImage) => void;
}) {
  const resolvedSrc = useResolvedImageSrc(image.src);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    setFailed(false);
  }, [resolvedSrc]);
  const previewImage = useMemo(
    () => ({ ...image, src: resolvedSrc }),
    [image, resolvedSrc],
  );
  if (failed) {
    return <MarkdownImageFallback image={image} />;
  }
  return (
    <button
      type="button"
      onClick={() => onPreviewImage(previewImage)}
      className="my-1 block overflow-hidden rounded-md border border-card-border/[0.16] bg-bg-panel-alt hover:border-card-border/25 focus:outline-none focus:ring-2 focus:ring-ink/20 transition"
      title={image.alt}
    >
      <img
        src={resolvedSrc}
        alt={image.alt}
        className={"h-28 w-36 " + (cover ? "object-cover" : "object-contain")}
        loading="lazy"
        onError={() => setFailed(true)}
      />
    </button>
  );
}

function MarkdownImageFallback({ image }: { image: MarkdownImage }) {
  return (
    <code className="rounded bg-ink/[0.08] px-1 py-0.5 font-mono text-[0.92em] text-ink">
      {`![${image.alt}](${image.src})`}
    </code>
  );
}

function ImagePreviewOverlay({
  image,
  onClose,
}: {
  image: MarkdownImage;
  onClose: () => void;
}) {
  const TOP_DRAG_SAFE_PX = 48;
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
      className="fixed inset-x-0 bottom-0 z-50 flex items-center justify-center bg-black/70 p-6"
      style={{ top: TOP_DRAG_SAFE_PX }}
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

function FilePreviewOverlay({
  file,
  onClose,
}: {
  file: FilePreview;
  onClose: () => void;
}) {
  const TOP_DRAG_SAFE_PX = 48;
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div
      className="fixed inset-x-0 bottom-0 z-50 flex items-center justify-center bg-black/60 p-6"
      style={{ top: TOP_DRAG_SAFE_PX }}
      onClick={onClose}
      role="dialog"
      aria-modal="true"
    >
      <div
        className="flex max-h-[calc(100vh-48px)] w-full max-w-3xl flex-col overflow-hidden rounded-xl border border-black/10 bg-white shadow-[0_30px_90px_rgba(0,0,0,0.35)]"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between gap-3 border-b border-black/8 bg-white px-4 py-3">
          <div className="min-w-0">
            <div className="truncate text-body-sm font-semibold text-black/85">{file.title}</div>
            <div className="text-caption text-black/45">File preview</div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-md px-2 py-1 text-caption font-medium text-black/55 transition hover:bg-black/6 hover:text-black"
          >
            Close
          </button>
        </div>
        <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words bg-[#f7f7f4] px-4 py-3 font-mono text-caption leading-relaxed text-black/80">
          <code>{file.text}</code>
        </pre>
      </div>
    </div>
  );
}

function FilePreviewNotice({
  message,
  onClose,
}: {
  message: string;
  onClose: () => void;
}) {
  useEffect(() => {
    const timer = window.setTimeout(onClose, 2600);
    return () => window.clearTimeout(timer);
  }, [onClose]);

  return (
    <div className="fixed bottom-6 left-1/2 z-50 max-w-md -translate-x-1/2 rounded-lg border border-ink/10 bg-bg-panel px-3 py-2 text-body-sm text-ink/72 shadow-xl">
      {message}
    </div>
  );
}

function isRenderableMarkdownImageSrc(src: string): boolean {
  const value = src.trim().replace(/^<|>$/g, "");
  if (/^(https?:|data:image\/|asset:|blob:)/i.test(value)) return true;
  if (/^file:\/\//i.test(value)) return /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(value);
  if (/^\/|^[A-Za-z]:[\\/]/.test(value)) return /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(value);
  return false;
}

function markdownUrlTransform(url: string): string {
  const raw = url.trim().replace(/^<|>$/g, "");
  const assetPath = localAssetImagePath(raw);
  if (assetPath) return convertFileSrc(assetPath);
  if (/^(https?:|mailto:|data:|asset:|blob:)/i.test(raw)) return raw;
  if (/^file:\/\//i.test(raw)) return convertFileSrc(decodeFileUri(raw));
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
  const assetPath = localAssetImagePath(src);
  if (assetPath) return convertFileSrc(assetPath);
  if (/^(https?:|data:|asset:|blob:)/i.test(src)) return src;
  if (/^file:\/\//i.test(src)) return convertFileSrc(decodeFileUri(src));
  if (/^\/|^[A-Za-z]:[\\/]/.test(src)) return convertFileSrc(src);
  return src;
}

function localImagePath(rawSrc: string): string | null {
  const src = rawSrc.trim().replace(/^<|>$/g, "");
  const assetPath = localAssetImagePath(src);
  if (assetPath) return assetPath;
  if (/^file:\/\//i.test(src)) return decodeFileUri(src);
  if (/^\/|^[A-Za-z]:[\\/]/.test(src)) return src;
  return null;
}

function localAssetImagePath(src: string): string | null {
  const match = src.match(/^asset:\/\/localhost\/(.+)$/i);
  if (!match) return null;
  try {
    return decodeURIComponent(match[1]);
  } catch {
    return match[1];
  }
}

function decodeFileUri(uri: string): string {
  try {
    return decodeURIComponent(uri.replace(/^file:\/\//i, ""));
  } catch {
    return uri.replace(/^file:\/\//i, "");
  }
}

function safeHref(rawHref: string): string | null {
  const href = rawHref.trim().replace(/^<|>$/g, "");
  if (/^(https?:|mailto:)/i.test(href)) return href;
  return null;
}
