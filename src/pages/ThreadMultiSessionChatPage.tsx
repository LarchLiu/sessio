import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  AlertCircle,
  ArrowDownToLine,
  ListChecks,
  LoaderCircle,
  MessagesSquare,
  Sparkles,
  Square,
  Swords,
} from "lucide-react";
import type {
  Agent,
  AstraEvent,
  AstraHandle,
  PlanRoundInfo,
  PlanTaskInfo,
  ProjectInfo,
  RuntimeAgentMetadata,
  RuntimeAgentSelection,
  SetRuntimeAgentSelectionRequest,
  StageInfo,
  ThreadInfo,
  ThreadReplayInfo,
  ThreadReplaySessionSourceInfo,
  ThreadWorkState,
} from "../api";
import {
  AGENT_LABEL,
  cancelAstraRun,
  createAstraRun,
  createPlanRound,
  getSessionHistory,
  getThreadReplay,
  getThreadWorkState,
  listAstraRuns,
  listPlanRounds,
  respondAgentPermission,
  updatePlanTaskStatus,
} from "../api";
import ChatComposer from "../components/ChatComposer";
import { AgentGlyph } from "../components/AgentIcon";
import AssistantBotIcon from "../components/AssistantBotIcon";
import ScrollArea from "../components/ScrollArea";
import Tooltip from "../components/Tooltip";
import { HashIcon } from "../components/IconifyIcon";
import { useChatComposer } from "../hooks/useChatComposer";
import { useI18n } from "../i18n";
import type { PendingNewChatSession } from "../navigation";
import {
  historyTurnsToAcpViewModel,
  liveSessionToAcpViewModel,
  type AcpRenderBlock,
  type AcpViewModel,
  type LiveRuntimeAction,
  type LiveRuntimeState,
  type LiveTurn,
} from "../runtimeChat";
import { isPersistedSession, sessionDisplayTitle } from "../appUtils";
import {
  buildThreadTimelineRows,
  buildThreadSessionLanes,
  shortSessionId,
  type ThreadSessionLane,
  type ThreadTimelineRow,
} from "../threadReplayView";
import { buildThreadWorkSnapshot, renderThreadWorkContext } from "../threadSnapshot";
import { collectThreadChatSessions } from "../threadChats";
import { collectThreadHistorySnapshots, withThreadChatSessions } from "../threadWorkContext";
import {
  astraStatusClass,
  formatAstraStatus,
  isAstraActive,
  upsertAstraRun,
} from "../threadAstraView";
import { projectStageIcon, projectStageLabel } from "../utils/stageDisplay";
import {
  AcpRenderItems,
  acpViewModelToRenderItems,
  liveWorkingIndicatorTurn,
  renderItemKeys,
  type AcpRenderItem,
  type MarkdownImage,
} from "./ChatPage";

const THREAD_REFRESH_ASTRA_EVENTS = new Set(["delegated", "stage_update_result", "task_dispatch"]);
const THREAD_CONTEXT_NAV_SETTLE_MS = 140;

export default function ThreadMultiSessionChatPage({
  project,
  threadId,
  liveState,
  runtimeAgents,
  lastRuntimeAgentSelection,
  rememberRuntimeAgentSelection,
  runtimeSessionAliases,
  pendingNewChats,
  dispatchLiveEvent,
  onPendingSession,
  onError,
}: {
  project: ProjectInfo;
  threadId: string;
  liveState: LiveRuntimeState;
  runtimeAgents: RuntimeAgentMetadata[];
  lastRuntimeAgentSelection: RuntimeAgentSelection | null;
  rememberRuntimeAgentSelection: (selection: SetRuntimeAgentSelectionRequest) => Promise<void>;
  runtimeSessionAliases: Record<string, string>;
  pendingNewChats: Record<string, PendingNewChatSession>;
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onPendingSession: (session: PendingNewChatSession) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const viewportRef = useRef<HTMLDivElement>(null);
  const timelineContentRef = useRef<HTMLDivElement>(null);
  const laneRefs = useRef<Record<string, HTMLElement | null>>({});
  const followTimelineBottomRef = useRef(true);
  const [thread, setThread] = useState<ThreadWorkState | null>(null);
  const [replay, setReplay] = useState<ThreadReplayInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [showScrollToBottom, setShowScrollToBottom] = useState(false);
  const [stageTaskMode, setStageTaskMode] = useState(false);
  const [stageTaskBusy, setStageTaskBusy] = useState(false);
  const [astraBusy, setAstraBusy] = useState<"start" | "cancel" | null>(null);
  const [astraRuns, setAstraRuns] = useState<AstraHandle[]>([]);
  const [planRounds, setPlanRounds] = useState<PlanRoundInfo[]>([]);
  const composer = useChatComposer({
    runtimeAgents,
    lastRuntimeAgentSelection,
    rememberRuntimeAgentSelection,
    liveState,
    dispatchLiveEvent,
    onError,
    onPendingSession,
  });

  const load = useCallback(async () => {
    const nextThread = await getThreadWorkState(threadId);
    const nextReplay = await getThreadReplay(threadId);
    return { nextThread, nextReplay };
  }, [threadId]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    load()
      .then(({ nextThread, nextReplay }) => {
        if (cancelled) return;
        setThread(nextThread);
        setReplay(nextReplay);
      })
      .catch((err) => {
        if (cancelled) return;
        const message = String(err);
        setLoadError(message);
        onError(message);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [load, onError]);

  const refresh = useCallback(async () => {
    setLoadError(null);
    try {
      const { nextThread, nextReplay } = await load();
      setThread(nextThread);
      setReplay(nextReplay);
    } catch (err) {
      const message = String(err);
      setLoadError(message);
      onError(message);
    }
  }, [load, onError]);

  const reloadAstraState = useCallback(async () => {
    try {
      const [nextRuns, nextPlanRounds] = await Promise.all([
        listAstraRuns(threadId),
        listPlanRounds(threadId),
      ]);
      setAstraRuns(nextRuns);
      setPlanRounds(nextPlanRounds);
    } catch (err) {
      onError(String(err));
    }
  }, [onError, threadId]);

  useEffect(() => {
    void reloadAstraState();
  }, [reloadAstraState]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<AstraEvent>("astra-run-event", (event) => {
      if (event.payload.threadId !== threadId) return;
      void reloadAstraState();
      if (THREAD_REFRESH_ASTRA_EVENTS.has(event.payload.eventType)) {
        void refresh();
      }
    }).then((fn) => {
      unlisten = fn;
    }).catch((err) => onError(String(err)));
    return () => {
      unlisten?.();
    };
  }, [onError, refresh, reloadAstraState, threadId]);

  const lanes = useMemo(
    () =>
      buildThreadSessionLanes({
        thread,
        replay,
        liveState,
        runtimeSessionAliases,
        pendingNewChats,
        t,
      }),
    [liveState, pendingNewChats, replay, runtimeSessionAliases, t, thread],
  );
  const sortedStages = useMemo(
    () => (thread?.stages ?? []).slice().sort((a, b) => a.order - b.order),
    [thread?.stages],
  );
  const activeStage = thread?.stageId
    ? sortedStages.find((stage) => stage.id === thread.stageId) ?? null
    : null;
  const timelineRows = useMemo(
    () => buildThreadTimelineRows(lanes, planRounds, astraRuns, thread?.kind ?? "teamwork"),
    [astraRuns, lanes, planRounds, thread?.kind],
  );
  const timelineScrollKey = useMemo(
    () => timelineRows
      .map((row) => row.lanes
        .map((lane) => {
          const latestLiveTurn = lane.liveSession?.turns.at(-1);
          return [
            row.key,
            lane.laneId,
            lane.status,
            lane.session?.messageCount ?? 0,
            lane.liveSession?.turns.length ?? 0,
            latestLiveTurn?.updatedAt ?? latestLiveTurn?.startedAt ?? 0,
          ].join(":");
        })
        .join(","))
      .join("|"),
    [timelineRows],
  );
  const timelineNavItems = useMemo(
    () => thread ? threadTimelineNavItems(timelineRows, thread, t) : [],
    [thread, timelineRows, t],
  );
  const showTimelineNav = !loading && Boolean(thread) && timelineRows.length > 0;
  const threadChatSessions = useMemo(
    () => thread ? collectThreadChatSessions(thread, replay) : [],
    [replay, thread],
  );
  const canRunStageTask =
    Boolean(thread && thread.kind === "process" && activeStage && composer.selectedAgent);
  const activeAstraRun = useMemo(
    () => astraRuns.find((run) => isAstraActive(run.status)) ?? null,
    [astraRuns],
  );
  const canStartAstra =
    Boolean(thread && (
      thread.kind === "teamwork"
      || thread.kind === "process"
      || thread.kind === "brainstorm"
      || thread.kind === "debate"
    ));

  const updateScrollToBottomButton = useCallback((vp: HTMLDivElement | null = viewportRef.current) => {
    if (!vp) return;
    const bottomDistance = Math.max(0, vp.scrollHeight - vp.clientHeight - vp.scrollTop);
    const visible = bottomDistance > 16;
    setShowScrollToBottom(visible);
    if (!visible) followTimelineBottomRef.current = true;
  }, []);

  const scrollTimelineToBottom = useCallback((behavior: ScrollBehavior = "auto") => {
    followTimelineBottomRef.current = true;
    const scroll = () => {
      const vp = viewportRef.current;
      if (!vp) return;
      vp.scrollTo({
        top: Math.max(0, vp.scrollHeight - vp.clientHeight),
        behavior,
      });
      setShowScrollToBottom(false);
    };
    scroll();
    window.requestAnimationFrame(() => {
      scroll();
      window.requestAnimationFrame(scroll);
    });
    window.setTimeout(scroll, 80);
  }, []);

  const handleTimelineScroll = useCallback((vp: HTMLDivElement) => {
    const bottomDistance = Math.max(0, vp.scrollHeight - vp.clientHeight - vp.scrollTop);
    const awayFromBottom = bottomDistance > 16;
    followTimelineBottomRef.current = !awayFromBottom;
    setShowScrollToBottom(awayFromBottom);
  }, []);

  useEffect(() => {
    followTimelineBottomRef.current = true;
    setShowScrollToBottom(false);
  }, [threadId]);

  useLayoutEffect(() => {
    if (loading || !thread) return;
    if (followTimelineBottomRef.current) {
      scrollTimelineToBottom();
    } else {
      updateScrollToBottomButton();
    }
  }, [loading, scrollTimelineToBottom, thread, timelineScrollKey, updateScrollToBottomButton]);

  useLayoutEffect(() => {
    const vp = viewportRef.current;
    const content = timelineContentRef.current;
    if (!vp || !content) return;
    let frame: number | null = null;
    const schedule = () => {
      if (frame !== null) return;
      frame = window.requestAnimationFrame(() => {
        frame = null;
        if (followTimelineBottomRef.current) {
          scrollTimelineToBottom();
        } else {
          updateScrollToBottomButton(vp);
        }
      });
    };
    const ro = new ResizeObserver(schedule);
    ro.observe(vp);
    ro.observe(content);
    schedule();
    return () => {
      if (frame !== null) window.cancelAnimationFrame(frame);
      ro.disconnect();
    };
  }, [scrollTimelineToBottom, updateScrollToBottomButton, timelineRows.length]);

  const handleStartAstra = async () => {
    if (!thread || !canStartAstra || activeAstraRun) return;
    setAstraBusy("start");
    try {
      const run = await createAstraRun(thread.id, composer.text.trim() || null);
      setAstraRuns((prev) => upsertAstraRun(prev, run));
      composer.setText("");
      await reloadAstraState();
      await refresh();
    } catch (err) {
      onError(String(err));
      composer.setComposerError(String(err));
    } finally {
      setAstraBusy(null);
    }
  };

  const handleCancelAstra = async () => {
    if (!activeAstraRun) return;
    setAstraBusy("cancel");
    try {
      const run = await cancelAstraRun(activeAstraRun.runId);
      setAstraRuns((prev) => upsertAstraRun(prev, run));
      await reloadAstraState();
      await refresh();
    } catch (err) {
      onError(String(err));
      composer.setComposerError(String(err));
    } finally {
      setAstraBusy(null);
    }
  };

  const handleSend = async () => {
    const prompt = composer.text.trim();
    if (!prompt) return;
    if (!thread) {
      composer.setComposerError(t("thread.not_found"));
      return;
    }
    if (!project.path) {
      composer.setComposerError(t("new_chat.no_project"));
      return;
    }
    const timestamp = Date.now();
    const baseSnapshot = withThreadChatSessions(
      buildThreadWorkSnapshot(thread, activeStage, timestamp),
      threadChatSessions,
    );
    const { snapshot: snapshotWithSources, historySnapshots } = await collectThreadHistorySnapshots(baseSnapshot);
    const stageId = thread.kind === "process" ? activeStage?.id ?? null : null;
    if (stageTaskMode && !stageId) {
      composer.setComposerError(t("thread.stage_task_requires_stage"));
      return;
    }
    if (stageTaskMode && !composer.selectedAgent) {
      composer.setComposerError(t("thread.stage_task_requires_agent"));
      return;
    }
    setStageTaskBusy(stageTaskMode);
    let manualTask: PlanTaskInfo | null = null;
    let pendingRuntimeCreated = false;
    try {
      manualTask = stageTaskMode && stageId
        ? await createManualStagePlanTask({
          thread,
          stage: activeStage!,
          targetAgent: composer.selectedAgent!,
          prompt,
          runtimeAgent: runtimeAgents.find((agent) => agent.agent === composer.selectedAgent) ?? null,
        })
        : null;
      const sent = await composer.runStartSession(prompt, {
        workspacePath: project.path,
        projectName: project.name,
        extraContext: renderThreadWorkContext(snapshotWithSources, composer.selectedAgent),
        pendingSession: {
          suppressAutoSelect: true,
          origin: "thread_multi_session",
          historySnapshots,
          workSnapshot: {
            threadId: thread.id,
            stageId,
            snapshot: snapshotWithSources,
          },
          threadLink: {
            threadId: thread.id,
            stageId,
          },
          ...(manualTask ? { planTaskLink: { taskId: manualTask.id, role: "runtime" } } : {}),
        },
        onPendingCreated: () => {
          pendingRuntimeCreated = true;
        },
      });
      if (!sent && manualTask) {
        await updatePlanTaskStatus(manualTask.id, {
          status: "failed",
          error: "Runtime session did not start",
        });
      }
      if (sent) void refresh();
    } catch (err) {
      if (manualTask && !pendingRuntimeCreated) {
        updatePlanTaskStatus(manualTask.id, {
          status: "failed",
          error: String(err),
        }).catch((statusErr) => onError(String(statusErr)));
      }
      composer.setComposerError(String(err));
      onError(String(err));
    } finally {
      setStageTaskBusy(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col overflow-hidden bg-surface-panel">
      <div className="relative flex min-h-0 flex-1 flex-col">
        <ScrollArea
          ref={viewportRef}
          className="min-h-0 flex-1"
          viewportClassName="px-14 py-4 session-chat-scroll-viewport"
          onScroll={handleTimelineScroll}
        >
          {loading ? (
            <div className="flex items-center justify-center gap-2 py-16 text-body-sm text-ink/45">
              <LoaderCircle className="h-4 w-4 animate-spin" />
              {t("memory_search.searching")}
            </div>
          ) : loadError && !thread ? (
            <ThreadMultiSessionEmpty
              icon={<AlertCircle className="h-5 w-5 text-status-error" />}
              title={t("thread.multi_session_load_failed")}
              detail={loadError}
            />
          ) : !thread ? (
            <ThreadMultiSessionEmpty
              icon={<HashIcon className="h-5 w-5 text-ink/35" />}
              title={t("thread.not_found")}
            />
          ) : (
            <div ref={timelineContentRef} className="relative grid min-h-full gap-3">
              {timelineRows.length === 0 ? (
                <ThreadMultiSessionEmpty
                  icon={<MessagesSquare className="h-5 w-5 text-ink/35" />}
                  title={t("thread.multi_session_empty")}
                  detail={replay ? t("thread.no_chats") : t("thread.history_pending")}
                />
              ) : (
                <ThreadTimeline
                  rows={timelineRows}
                  thread={thread}
                  laneRefs={laneRefs}
                  now={Date.now()}
                />
              )}
            </div>
          )}
        </ScrollArea>
        {showTimelineNav && (
          <ThreadContextNav
            items={timelineNavItems}
            laneRefs={laneRefs}
            viewportRef={viewportRef}
          />
        )}
        {showScrollToBottom && (
          <button
            type="button"
            onClick={() => scrollTimelineToBottom("smooth")}
            className="absolute bottom-3 left-1/2 z-20 flex h-9 w-9 -translate-x-1/2 items-center justify-center rounded-full border border-ink/15 bg-surface-panel/95 text-ink/85 shadow-sm transition hover:border-ink/25 hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ink/20"
            aria-label="Scroll to bottom"
          >
            <ArrowDownToLine className="h-5 w-5" />
          </button>
        )}
      </div>
      <div className="shrink-0 bg-gradient-to-t from-surface-panel via-surface-panel to-surface-panel/80 px-14 pb-4">
        <div className="w-full">
          <ChatComposer
            composer={composer}
            title={null}
            variant="chat"
            canSend={
              Boolean(thread) &&
              composer.canSendWithWorkspace(project.path) &&
              (!stageTaskMode || canRunStageTask) &&
              !stageTaskBusy
            }
            onSend={() => void handleSend()}
            modeActions={
              <>
                {thread?.kind === "process" && (
                  <Tooltip content={t("thread.stage_task_mode")} placement="top">
                    <button
                      type="button"
                      onClick={() => setStageTaskMode((value) => !value)}
                      disabled={!activeStage || !composer.selectedAgent || stageTaskBusy}
                      className={
                        "flex h-7 w-7 shrink-0 items-center justify-center rounded-full transition disabled:opacity-40 " +
                        (stageTaskMode
                          ? "bg-[rgb(var(--color-emerald)/0.12)] text-[rgb(var(--color-emerald)/0.95)]"
                          : "text-ink/55 hover:bg-ink/8 hover:text-ink")
                      }
                      aria-label={t("thread.stage_task_mode")}
                    >
                      {stageTaskBusy ? (
                        <LoaderCircle className="h-4 w-4 animate-spin" />
                      ) : (
                        <ListChecks className="h-4 w-4" />
                      )}
                    </button>
                  </Tooltip>
                )}
              </>
            }
            sendActions={
              activeAstraRun ? (
                <Tooltip content={`${formatAstraStatus(activeAstraRun.status)} · ${t("astra.cancel")}`} placement="top">
                  <button
                    type="button"
                    onClick={() => void handleCancelAstra()}
                    disabled={astraBusy !== null}
                    className={
                      "relative flex h-7 w-7 shrink-0 items-center justify-center overflow-hidden rounded-full transition before:pointer-events-none before:absolute before:inset-y-[-25%] before:left-[-55%] before:w-3 before:rotate-12 before:bg-white/65 before:opacity-0 before:blur-[1px] before:transition-all before:duration-500 hover:before:left-[130%] hover:before:opacity-80 disabled:opacity-40 " +
                      astraStatusClass(activeAstraRun.status) +
                      " hover:bg-red-500/[0.08] hover:text-red-500"
                    }
                    aria-label={t("astra.cancel")}
                  >
                    {astraBusy === "cancel" ? (
                      <LoaderCircle className="relative z-10 h-4 w-4 animate-spin" />
                    ) : (
                      <Square className="relative z-10 h-4 w-4" />
                    )}
                  </button>
                </Tooltip>
              ) : canStartAstra ? (
                <Tooltip content={t("astra.start")} placement="top">
                  <button
                    type="button"
                    onClick={() => void handleStartAstra()}
                    disabled={astraBusy !== null || !thread}
                    className="astra-send-button relative flex h-7 w-7 shrink-0 items-center justify-center overflow-hidden rounded-full"
                    aria-label={t("astra.start")}
                  >
                    {astraBusy === "start" ? (
                      <LoaderCircle className="relative z-10 h-4 w-4 animate-spin" />
                    ) : (
                      <Sparkles className="relative z-10 h-4 w-4" />
                    )}
                  </button>
                </Tooltip>
              ) : null
            }
          />
        </div>
      </div>
    </div>
  );
}

function ThreadSessionLaneCard({
  lane,
  thread,
  laneRefs,
  now,
  content,
  plannerRun,
}: {
  lane: ThreadSessionLane;
  thread: ThreadWorkState;
  laneRefs: React.RefObject<Record<string, HTMLElement | null>>;
  now: number;
  content?: ReactNode;
  plannerRun?: AstraHandle | null;
}) {
  const { t } = useI18n();
  const meta = laneDisplayMeta(lane, thread, t);
  return (
    <section
      ref={(el) => {
        laneRefs.current[lane.laneId] = el;
      }}
      className="min-w-0 scroll-mt-4"
    >
      <header
        data-thread-lane-header="true"
        className="mb-1.5 flex min-w-0 items-center gap-2 px-1"
      >
        {thread.kind === "debate" && (
          <AgentGlyph agent={lane.agent} className="h-3.5 w-3.5 shrink-0" />
        )}
        <h2 className="min-w-0 truncate text-caption font-medium text-ink/48">
          {meta.title}
        </h2>
      </header>
      {content ?? <ThreadSessionLatestMessage lane={lane} now={now} plannerRun={plannerRun} />}
    </section>
  );
}

type ThreadContextNavItem = {
  key: string;
  laneId: string;
  context: LaneDisplayContext;
};

function threadTimelineNavItems(
  rows: ThreadTimelineRow[],
  thread: ThreadWorkState,
  t: (key: string, vars?: Record<string, string | number>) => string,
): ThreadContextNavItem[] {
  const seen = new Set<string>();
  const items: ThreadContextNavItem[] = [];
  for (const row of rows) {
    if (row.kind === "orchestration") {
      const visibleStaticPlanner = row.round
        ? Boolean(planRoundSummaryText(row.round, row.run))
        : false;
      if (row.lanes.length > 0 || visibleStaticPlanner) {
        row.lanes.forEach((lane) => seen.add(lane.laneId));
        items.push({
          key: `planner:${row.key}`,
          laneId: row.lanes[0]?.laneId ?? row.key,
          context: {
            kind: "planner",
            label: "Astra planner",
            title: readableTooltipText(row.round?.summary) ?? null,
          },
        });
      }
      continue;
    }
    if (row.debatePair && row.lanes.length > 0) {
      row.lanes.forEach((lane) => seen.add(lane.laneId));
      const participants = row.lanes.map((lane) => battleParticipantFromLane(lane, thread, t));
      items.push({
        key: `battle:${row.key}`,
        laneId: row.lanes[0].laneId,
        context: {
          kind: "battle",
          label: participants.map((participant) => participant.label).join(" vs ") || "Battle",
          title: null,
          participants,
        },
      });
      continue;
    }
    for (const lane of row.lanes) {
      if (seen.has(lane.laneId)) continue;
      seen.add(lane.laneId);
      items.push({
        key: lane.laneId,
        laneId: lane.laneId,
        context: laneDisplayMeta(lane, thread, t).context,
      });
    }
  }
  return items;
}

function ThreadContextNav({
  items,
  laneRefs,
  viewportRef,
}: {
  items: ThreadContextNavItem[];
  laneRefs: React.RefObject<Record<string, HTMLElement | null>>;
  viewportRef: React.RefObject<HTMLDivElement | null>;
}) {
  const itemSignature = useMemo(
    () => items.map((item) => `${item.key}:${item.laneId}`).join("|"),
    [items],
  );
  const itemsRef = useRef(items);
  const [positions, setPositions] = useState<Map<string, number>>(new Map());
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const [positionsReady, setPositionsReady] = useState(false);
  const positionsReadyRef = useRef(false);
  const activeKeyRef = useRef<string | null>(null);
  const settleTimerRef = useRef<number | null>(null);
  itemsRef.current = items;

  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp || itemSignature.length === 0) {
      const empty = new Map<string, number>();
      setPositions(empty);
      setActiveKey(null);
      setPositionsReady(false);
      positionsReadyRef.current = false;
      activeKeyRef.current = null;
      return;
    }

    let frame: number | null = null;
    const itemAnchor = (item: ThreadContextNavItem) => {
      const el = laneRefs.current[item.laneId];
      return el?.querySelector<HTMLElement>("[data-thread-lane-header='true']") ?? el;
    };
    const clearSettleTimer = () => {
      if (settleTimerRef.current === null) return;
      window.clearTimeout(settleTimerRef.current);
      settleTimerRef.current = null;
    };
    const setReady = (ready: boolean) => {
      positionsReadyRef.current = ready;
      setPositionsReady(ready);
    };
    const setMeasuredPositions = (next: Map<string, number>) => {
      setPositions(next);
    };
    const measurePositions = (): Map<string, number> | null => {
      const currentItems = itemsRef.current;
      if (currentItems.length === 0) return null;
      const contentHeight = Math.max(1, vp.scrollHeight);
      const vpRect = vp.getBoundingClientRect();
      const next = new Map<string, number>();
      for (const item of currentItems) {
        const anchor = itemAnchor(item);
        if (!anchor) continue;
        const rect = anchor.getBoundingClientRect();
        const contentCenter = rect.top - vpRect.top + vp.scrollTop + rect.height / 2;
        next.set(item.key, Math.max(0, Math.min(1, contentCenter / contentHeight)));
      }
      return next.size === currentItems.length ? next : null;
    };
    const computeActive = () => {
      const currentItems = itemsRef.current;
      if (currentItems.length === 0) return;
      const vpRect = vp.getBoundingClientRect();
      const enter = vpRect.top + vpRect.height * 0.25;
      const exit = vpRect.top + vpRect.height * 0.75;
      const atBottom = vp.scrollTop + vp.clientHeight >= vp.scrollHeight - 1;
      const atTop = vp.scrollTop <= 0;

      let active = activeKeyRef.current;
      let activeIndex = active ? currentItems.findIndex((item) => item.key === active) : -1;
      if (activeIndex < 0) {
        activeIndex = 0;
        for (let index = 0; index < currentItems.length; index += 1) {
          const anchor = itemAnchor(currentItems[index]);
          if (!anchor) continue;
          if (anchor.getBoundingClientRect().top <= enter) activeIndex = index;
          else break;
        }
        active = currentItems[activeIndex]?.key ?? null;
      } else {
        for (let index = activeIndex + 1; index < currentItems.length; index += 1) {
          const anchor = itemAnchor(currentItems[index]);
          if (!anchor) continue;
          if (anchor.getBoundingClientRect().top <= enter) {
            active = currentItems[index].key;
            activeIndex = index;
          } else {
            break;
          }
        }
        while (activeIndex > 0) {
          const activeItem = currentItems[activeIndex];
          const anchor = itemAnchor(activeItem);
          if (!anchor || anchor.getBoundingClientRect().top <= exit) break;
          activeIndex -= 1;
          active = currentItems[activeIndex].key;
        }
      }

      if (atBottom) active = currentItems[currentItems.length - 1]?.key ?? active;
      if (atTop) active = currentItems[0]?.key ?? active;
      activeKeyRef.current = active;
      setActiveKey(active);
    };
    const scheduleMeasure = () => {
      clearSettleTimer();
      settleTimerRef.current = window.setTimeout(() => {
        settleTimerRef.current = null;
        const next = measurePositions();
        if (!next) {
          if (!positionsReadyRef.current) setReady(false);
          return;
        }
        setMeasuredPositions(next);
        setReady(true);
        computeActive();
      }, THREAD_CONTEXT_NAV_SETTLE_MS);
    };
    const commitInitialMeasure = () => {
      if (frame !== null) return;
      frame = window.requestAnimationFrame(() => {
        frame = null;
        const next = measurePositions();
        if (next) setMeasuredPositions(next);
        computeActive();
        scheduleMeasure();
      });
    };

    const empty = new Map<string, number>();
    setPositions(empty);
    setReady(false);
    activeKeyRef.current = null;
    commitInitialMeasure();
    vp.addEventListener("scroll", computeActive, { passive: true });
    const ro = new ResizeObserver(scheduleMeasure);
    ro.observe(vp);
    for (const child of Array.from(vp.children)) ro.observe(child);
    for (const item of items) {
      const el = laneRefs.current[item.laneId];
      if (el) ro.observe(el);
    }
    return () => {
      vp.removeEventListener("scroll", computeActive);
      if (frame !== null) window.cancelAnimationFrame(frame);
      clearSettleTimer();
      ro.disconnect();
    };
  }, [itemSignature, laneRefs, viewportRef]);

  if (items.length === 0) return null;
  const canRenderPositions = positionsReady && positions.size === items.length;

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

  const jumpToLane = (item: ThreadContextNavItem) => {
    const vp = viewportRef.current;
    const el = laneRefs.current[item.laneId];
    if (!vp || !el) return;
    activeKeyRef.current = item.key;
    setActiveKey(item.key);
    const vpRect = vp.getBoundingClientRect();
    const targetTop = el.getBoundingClientRect().top - vpRect.top + vp.scrollTop - 8;
    const maxTop = Math.max(0, vp.scrollHeight - vp.clientHeight);
    vp.scrollTo({
      top: Math.max(0, Math.min(targetTop, maxTop)),
      behavior: "smooth",
    });
  };

  return (
    <div
      onWheel={handleWheel}
      className="absolute top-2 bottom-2 left-0 z-10 w-10"
    >
      {canRenderPositions && items.map((item) => {
        const ratio = positions.get(item.key);
        if (ratio === undefined) return null;
        return (
          <Tooltip
            key={item.key}
            content={<ThreadContextTooltip context={item.context} />}
            placement="right"
            offset={10}
            delayMs={120}
          >
            <button
              type="button"
              onClick={() => jumpToLane(item)}
              aria-label={item.context.label}
              className={
                "group pointer-events-auto absolute left-2 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded-full transition-[background-color,color,transform] duration-150 focus-visible:bg-ink/[0.08] focus-visible:text-ink " +
                (item.key === activeKey
                  ? "scale-110 bg-ink/[0.08] text-ink shadow-sm "
                  : "scale-100 text-ink/24 hover:bg-ink/[0.06] hover:text-ink/72")
              }
              style={{ top: `${ratio * 100}%` }}
            >
              <span
                className={
                  "flex h-3.5 w-3.5 items-center justify-center transition-opacity duration-150 " +
                  (item.key === activeKey
                    ? "opacity-100"
                    : "opacity-40 group-hover:opacity-75 group-focus-visible:opacity-100")
                }
              >
                <ThreadContextIcon context={item.context} className="h-3.5 w-3.5" />
              </span>
            </button>
          </Tooltip>
        );
      })}
    </div>
  );
}

function ThreadContextIcon({
  context,
  className,
}: {
  context: LaneDisplayContext;
  className: string;
}) {
  const icon =
    context.kind === "stage"
      ? (context.stage
        ? projectStageIcon(context.stage, className)
        : <ListChecks className={className} />)
      : context.kind === "assistant"
        ? <AssistantBotIcon color={context.color} className={className} />
        : context.kind === "planner"
          ? <Sparkles className={className} />
          : context.kind === "battle"
            ? <Swords className={className} />
        : <AgentGlyph agent={context.agent} className={className} />;
  return icon;
}

function ThreadContextTooltip({ context }: { context: LaneDisplayContext }) {
  if (context.kind === "battle") {
    const details = context.participants
      .map((participant) => tooltipDetailText(participant.title))
      .filter((detail): detail is string => Boolean(detail));
    return (
      <div className="max-w-[300px]">
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-caption font-medium text-tooltip-fg">
          {context.participants.map((participant, index) => (
            <div key={`${participant.label}:${index}`} className="flex min-w-0 items-center gap-1.5">
              {index > 0 && <span className="text-tooltip-fg/45">vs</span>}
              <AgentGlyph agent={participant.agent} className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{participant.label}</span>
            </div>
          ))}
        </div>
        {details.length > 0 && (
          <div className="mt-1 grid gap-0.5">
            {details.map((detail, index) => (
              <div key={`${detail}:${index}`} className="truncate text-meta text-tooltip-fg/62">
                {detail}
              </div>
            ))}
          </div>
        )}
      </div>
    );
  }
  const detail = tooltipDetailText(context.title);
  return (
    <div className="max-w-[260px]">
      <div className="text-caption font-medium text-tooltip-fg">{context.label}</div>
      {detail && (
        <div className="mt-0.5 line-clamp-2 text-meta text-tooltip-fg/70">
          {detail}
        </div>
      )}
    </div>
  );
}

function tooltipDetailText(value: string | null): string | null {
  const trimmed = readableTooltipText(value);
  if (!trimmed) return null;
  return trimmed.length > 160 ? `${trimmed.slice(0, 157)}...` : trimmed;
}

function readableTooltipText(value: string | null | undefined): string | null {
  if (!value) return null;
  const trimmed = value.replace(/\s+/g, " ").trim();
  if (!trimmed || isJsonText(trimmed)) return null;
  return trimmed;
}

function isJsonText(value: string): boolean {
  const first = value[0];
  const last = value[value.length - 1];
  if (!((first === "{" && last === "}") || (first === "[" && last === "]"))) {
    return false;
  }
  try {
    const parsed = JSON.parse(value);
    return parsed !== null && typeof parsed === "object";
  } catch {
    return false;
  }
}

function ThreadMultiSessionEmpty({
  icon,
  title,
  detail,
}: {
  icon: React.ReactNode;
  title: string;
  detail?: string;
}) {
  return (
    <div className="flex min-h-[320px] items-center justify-center rounded-lg border border-dashed border-ink/15 bg-card/[0.35] px-6 text-center">
      <div className="max-w-[520px]">
        <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-full bg-ink/[0.04]">
          {icon}
        </div>
        <div className="mt-3 text-body-sm font-medium text-ink/60">{title}</div>
        {detail && (
          <div className="mt-1 text-caption leading-relaxed text-ink/38">{detail}</div>
        )}
      </div>
    </div>
  );
}

function ThreadTimeline({
  rows,
  thread,
  laneRefs,
  now,
}: {
  rows: ThreadTimelineRow[];
  thread: ThreadWorkState;
  laneRefs: React.RefObject<Record<string, HTMLElement | null>>;
  now: number;
}) {
  return (
    <section className="grid gap-4">
      {rows.map((row) => {
        if (row.kind === "orchestration") {
          const summaryText = row.round ? planRoundSummaryText(row.round, row.run) : null;
          if (row.lanes.length === 0) {
            if (!row.round || !summaryText) return null;
            return (
              <ThreadTimelineStaticCard
                key={row.key}
                laneId={row.key}
                laneRefs={laneRefs}
                title="Astra planner"
              >
                <ThreadPlanRoundSummaryMessage
                  round={row.round}
                  text={summaryText}
                  now={now}
                />
              </ThreadTimelineStaticCard>
            );
          }
          return (
            <div key={row.key} className="grid gap-5">
              {row.lanes.map((lane) => (
                <ThreadSessionLaneCard
                  key={lane.laneId}
                  lane={lane}
                  thread={thread}
                  laneRefs={laneRefs}
                  now={now}
                  plannerRun={row.run}
                  content={row.round && summaryText ? (
                    <ThreadPlanRoundSummaryMessage
                      round={row.round}
                      text={summaryText}
                      now={now}
                    />
                  ) : undefined}
                />
              ))}
            </div>
          );
        }
        return (
          <div
            key={row.key}
            className={row.debatePair ? "grid gap-5 md:grid-cols-2" : "grid gap-4"}
          >
            {row.lanes.map((lane) => (
              <ThreadSessionLaneCard
                key={lane.laneId}
                lane={lane}
                thread={thread}
                laneRefs={laneRefs}
                now={now}
              />
            ))}
          </div>
        );
      })}
    </section>
  );
}

function ThreadTimelineStaticCard({
  laneId,
  laneRefs,
  title,
  children,
}: {
  laneId: string;
  laneRefs: React.RefObject<Record<string, HTMLElement | null>>;
  title: string;
  children: ReactNode;
}) {
  return (
    <section
      ref={(el) => {
        laneRefs.current[laneId] = el;
      }}
      className="min-w-0 scroll-mt-4"
    >
      <header
        data-thread-lane-header="true"
        className="mb-1.5 flex min-w-0 items-center gap-2 px-1"
      >
        <h2 className="min-w-0 truncate text-caption font-medium text-ink/48">
          {title}
        </h2>
      </header>
      {children}
    </section>
  );
}

function ThreadPlanRoundSummaryMessage({
  round,
  text,
  now,
}: {
  round: PlanRoundInfo;
  text: string;
  now: number;
}) {
  return (
    <ThreadPlannerSummaryMessage
      id={round.id}
      timestamp={round.updatedAt || round.createdAt}
      text={text}
      now={now}
    />
  );
}

function ThreadPlannerSummaryMessage({
  id,
  timestamp,
  text,
  now,
}: {
  id: string;
  timestamp: number;
  text: string;
  now: number;
}) {
  const bubbleRefs = useRef<(HTMLDivElement | null)[]>([]);
  const onPreviewImage = useCallback((_image: MarkdownImage) => undefined, []);
  const onPreviewFile = useCallback(() => undefined, []);
  const onFilePreviewError = useCallback(() => undefined, []);
  const onPermissionResponse = useCallback(() => Promise.resolve(), []);
  const block = useMemo<AcpRenderBlock>(() => ({
    kind: "assistant",
    blocks: [{ type: "text", text }],
    raw: { source: "thread_planner_summary", id },
    timestamp,
  }), [id, text, timestamp]);
  const turn = useMemo<LiveTurn>(() => ({
    turnId: `thread-planner-summary:${id}:${timestamp}`,
    status: "completed",
    blocks: [block],
    tools: [],
    permissions: [],
    protocolMessages: [],
    stopReason: null,
    error: null,
    startedAt: timestamp,
    updatedAt: timestamp,
  }), [block, id, timestamp]);
  const items = useMemo<AcpRenderItem[]>(() => [{ kind: "block", turn, block }], [block, turn]);
  const itemKeys = useMemo(() => renderItemKeys(items), [items]);

  return (
    <div className="grid min-w-0 gap-2">
      <AcpRenderItems
        items={items}
        itemKeys={itemKeys}
        bubbleRefs={bubbleRefs}
        sessioRuntimeSessionId=""
        now={now}
        defaultMessageExpanded={false}
        onPreviewImage={onPreviewImage}
        onPreviewFile={onPreviewFile}
        onFilePreviewError={onFilePreviewError}
        onPermissionResponse={onPermissionResponse}
      />
    </div>
  );
}

function ThreadSessionLatestMessage({
  lane,
  now,
  plannerRun,
}: {
  lane: ThreadSessionLane;
  now: number;
  plannerRun?: AstraHandle | null;
}) {
  const { t } = useI18n();
  const [historyTurns, setHistoryTurns] = useState<LiveTurn[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const bubbleRefs = useRef<(HTMLDivElement | null)[]>([]);
  const onPreviewImage = useCallback((_image: MarkdownImage) => undefined, []);
  const onPreviewFile = useCallback(() => undefined, []);
  const onFilePreviewError = useCallback(() => undefined, []);

  const sessionFilePath = lane.session?.filePath ?? "";
  const sessionPersisted = isPersistedSession(lane.session);
  const sessionMessageCount = lane.session?.messageCount ?? 0;
  useEffect(() => {
    if (!lane.session || !sessionPersisted) {
      setHistoryTurns([]);
      setLoading(false);
      setLoadError(null);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    getSessionHistory(lane.agent, sessionFilePath, lane.sessionId)
      .then((result) => {
        if (cancelled) return;
        setHistoryTurns(normalizeSessionHistoryTurns(result.turns));
      })
      .catch((err) => {
        if (cancelled) return;
        setHistoryTurns([]);
        setLoadError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [
    lane.agent,
    lane.sessionId,
    sessionFilePath,
    sessionPersisted,
    sessionMessageCount,
  ]);

  const hasLiveTurns = Boolean(lane.liveSession?.turns.length);
  const viewModel = useMemo<AcpViewModel>(
    () => hasLiveTurns && lane.liveSession
      ? liveSessionToAcpViewModel(lane.liveSession)
      : historyTurnsToAcpViewModel(historyTurns),
    [hasLiveTurns, historyTurns, lane.liveSession],
  );
  const liveTurnIds = useMemo(
    () => new Set(hasLiveTurns ? lane.liveSession?.turns.map((turn) => turn.turnId) ?? [] : []),
    [hasLiveTurns, lane.liveSession?.turns],
  );
  const workingIndicatorTurnId = hasLiveTurns
    ? liveWorkingIndicatorTurn(lane.liveSession)?.turnId ?? ""
    : "";
  const visibleItems = useMemo(
    () => latestLaneRenderItems(
      acpViewModelToRenderItems(viewModel, liveTurnIds, workingIndicatorTurnId),
    ),
    [liveTurnIds, viewModel, workingIndicatorTurnId],
  );
  const plannerSummary = useMemo(
    () => isPlannerLane(lane) ? plannerSummaryFromTurns(viewModel.turns, plannerRun ?? null) : null,
    [lane, plannerRun, viewModel.turns],
  );
  const itemKeys = useMemo(() => renderItemKeys(visibleItems), [visibleItems]);
  const permissionSessionId = lane.sessioRuntimeSessionId ?? "";
  const handlePermissionResponse = useCallback(
    (sessioRuntimeSessionId: string, requestId: string, optionId: string) => {
      if (!lane.sessioRuntimeSessionId) {
        return Promise.reject(new Error("Permission can only be handled while the session is live."));
      }
      return respondAgentPermission(sessioRuntimeSessionId, requestId, optionId);
    },
    [lane.sessioRuntimeSessionId],
  );

  const emptyText = laneMessageEmptyText({ lane, loading, loadError, t });

  return (
    <div className="min-w-0">
      {plannerSummary ? (
        <ThreadPlannerSummaryMessage
          id={lane.laneId}
          timestamp={plannerSummary.timestamp}
          text={plannerSummary.text}
          now={now}
        />
      ) : visibleItems.length > 0 ? (
        <div className="grid min-w-0 gap-2">
          <AcpRenderItems
            items={visibleItems}
            itemKeys={itemKeys}
            bubbleRefs={bubbleRefs}
            sessioRuntimeSessionId={permissionSessionId}
            now={now}
            defaultMessageExpanded={false}
            onPreviewImage={onPreviewImage}
            onPreviewFile={onPreviewFile}
            onFilePreviewError={onFilePreviewError}
            onPermissionResponse={handlePermissionResponse}
          />
        </div>
      ) : (
        <div className="flex min-h-[108px] items-center justify-center px-2 text-center">
          <div className="max-w-[300px]">
            {loading ? (
              <LoaderCircle className="mx-auto h-5 w-5 animate-spin text-ink/24" />
            ) : (
              <MessagesSquare className="mx-auto h-5 w-5 text-ink/24" />
            )}
            <div className="mt-2 text-body-sm font-medium text-ink/50">
              {emptyText.title}
            </div>
            {emptyText.detail && (
              <div className="mt-1 text-caption leading-relaxed text-ink/32">
                {emptyText.detail}
              </div>
            )}
          </div>
        </div>
      )}
      {loadError && visibleItems.length > 0 && (
        <div className="mt-1.5 px-1 text-meta text-status-error/80">
          {loadError}
        </div>
      )}
    </div>
  );
}

function normalizeSessionHistoryTurns(turns: unknown[] | undefined): LiveTurn[] {
  return Array.isArray(turns) ? (turns as LiveTurn[]) : [];
}

function latestLaneRenderItems(items: AcpRenderItem[]): AcpRenderItem[] {
  const pendingPermission = items
    .slice()
    .reverse()
    .find((item) =>
      item.kind === "permission" &&
      !item.permission.selectedOptionId &&
      !item.permission.cancelled,
    );
  if (pendingPermission) return [pendingPermission];
  const latestMessageIndex = findLastRenderItemIndex(items, isLanePreviewRenderItem);
  if (latestMessageIndex < 0) return items.slice(-1);
  const latestMessage = items[latestMessageIndex];
  if (isFileEditRenderItem(latestMessage)) {
    const previousMessageIndex = findLastRenderItemIndex(
      items,
      (item, index) =>
        index < latestMessageIndex &&
        isLanePreviewRenderItem(item) &&
        !isFileEditRenderItem(item),
    );
    return previousMessageIndex >= 0
      ? [items[previousMessageIndex], latestMessage]
      : [latestMessage];
  }
  return [latestMessage];
}

function isLanePreviewRenderItem(item: AcpRenderItem): boolean {
  if (item.kind === "turnStatus" || item.kind === "workingIndicator") return false;
  if (item.kind !== "block") return true;
  if (item.block.kind === "user") return !isDelegatedTaskPromptBlock(item.block);
  return item.block.kind === "assistant" ||
    item.block.kind === "thought" ||
    isFileEditRenderItem(item);
}

function isDelegatedTaskPromptBlock(block: AcpRenderBlock): boolean {
  if (block.kind !== "user") return false;
  const text = acpTextBlockText(block).trimStart();
  return text.startsWith("# Sessio plan task") ||
    text.includes("You are working on a delegated Astra plan task.");
}

function acpTextBlockText(block: AcpRenderBlock): string {
  if (block.kind !== "user" && block.kind !== "assistant" && block.kind !== "thought") {
    return "";
  }
  return block.blocks
    .map((item) => item.type === "text" ? item.text : "")
    .filter(Boolean)
    .join("\n");
}

function findLastRenderItemIndex(
  items: AcpRenderItem[],
  predicate: (item: AcpRenderItem, index: number) => boolean,
): number {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    if (predicate(items[index], index)) return index;
  }
  return -1;
}

function isFileEditRenderItem(item: AcpRenderItem): boolean {
  return (
    item.kind === "block" &&
    item.block.kind === "sessionUpdate" &&
    item.block.updateType === "file_edit"
  );
}

type LaneDisplayContext =
  | {
      kind: "stage";
      label: string;
      title: string | null;
      stage: Pick<StageInfo, "kind" | "icon"> | null;
    }
  | {
      kind: "assistant";
      label: string;
      title: string | null;
      color: string | null;
    }
  | {
      kind: "planner";
      label: string;
      title: string | null;
    }
  | {
      kind: "battle";
      label: string;
      title: string | null;
      participants: BattleParticipant[];
    }
  | {
      kind: "agent";
      label: string;
      title: string | null;
      agent: Agent;
    };

type LaneDisplayMeta = {
  title: string;
  context: LaneDisplayContext;
};

type BattleParticipant = {
  label: string;
  title: string | null;
  agent: Agent;
};

function battleParticipantFromLane(
  lane: ThreadSessionLane,
  thread: ThreadWorkState,
  t: (key: string, vars?: Record<string, string | number>) => string,
): BattleParticipant {
  const context = laneDisplayMeta(lane, thread, t).context;
  return {
    label: context.label,
    title: context.title,
    agent: lane.agent,
  };
}

function laneDisplayMeta(
  lane: ThreadSessionLane,
  thread: ThreadWorkState,
  t: (key: string, vars?: Record<string, string | number>) => string,
): LaneDisplayMeta {
  const source = preferredLaneSource(lane);
  const sourceLabel = plannerDisplayLabel(source);
  if (source?.kind === "astra_internal") {
    return {
      title: sourceLabel || lane.groupLabel || "Astra planner",
      context: {
        kind: "planner",
        label: sourceLabel || "Astra planner",
        title: readableTooltipText(source?.label) ?? null,
      },
    };
  }
  const stageSnapshot = parseJsonObject(source?.stageSnapshotJson ?? null);
  const assistantSnapshot = parseJsonObject(source?.assistantSnapshotJson ?? null);
  const agentSnapshot = parseJsonObject(source?.agentSnapshotJson ?? null);
  const participantSnapshot = objectField(agentSnapshot, "participant");
  const participantLabel = participantSnapshotLabel(participantSnapshot);
  const fallbackStage =
    (source?.stageId ? thread.stages.find((stage) => stage.id === source.stageId) : null)
    ?? (thread.stageId ? thread.stages.find((stage) => stage.id === thread.stageId) : null)
    ?? null;
  const fallbackAssistant = participantSnapshot
    ? null
    : (
        assistantFromSnapshot(assistantSnapshot)
        ?? assistantFromThread(thread, source, lane.agent)
        ?? null
      );
  const agentInfo = objectField(agentSnapshot, "agentInfo");
  const snapshotAgent = stringField(agentSnapshot, "agent");
  const snapshotAgentLabel =
    stringField(agentInfo, "displayName")
    ?? stringField(agentInfo, "name")
    ?? (snapshotAgent && snapshotAgent in AGENT_LABEL ? AGENT_LABEL[snapshotAgent as Agent] : snapshotAgent);
  const stageLabel =
    stringField(stageSnapshot, "name")
    ?? stringField(stageSnapshot, "stageId")
    ?? stringField(stageSnapshot, "id")
    ?? (fallbackStage ? projectStageLabel(fallbackStage, t) : null);
  const stageIcon = snapshotStageIcon(stageSnapshot) ?? fallbackStage;
  const agentLabel = participantLabel ?? snapshotAgentLabel ?? AGENT_LABEL[lane.agent];
  const title =
    sourceLabel
    || (lane.session ? sessionDisplayTitle(lane.session) ?? null : null)
    || fallbackAssistant?.name
    || lane.groupLabel
    || shortSessionId(lane.sessionId);
  const assistantLabel =
    fallbackAssistant?.name
    ?? source?.label?.trim()
    ?? lane.groupLabel;
  const agentDetail = participantTooltipDetail(participantSnapshot) ?? agentTooltipDetail(agentSnapshot);
  const context: LaneDisplayContext = stageLabel
    ? {
        kind: "stage",
        label: stageLabel,
        title: stageTooltipDetail(stageSnapshot, fallbackStage),
        stage: stageIcon ? {
          kind: stageIcon.kind ?? null,
          icon: stageIcon.icon ?? null,
        } : null,
      }
    : fallbackAssistant
      ? {
          kind: "assistant",
          label: assistantLabel,
          color: fallbackAssistant.color ?? null,
          title: assistantTooltipDetail(assistantSnapshot, fallbackAssistant, source),
        }
      : thread.kind === "debate"
        ? {
            kind: "battle",
            label: agentLabel,
            title: agentDetail,
            participants: [{
              label: agentLabel,
              title: agentDetail,
              agent: lane.agent,
            }],
          }
      : {
          kind: "agent",
          label: agentLabel,
          title: agentDetail,
          agent: lane.agent,
        };

  return {
    title,
    context,
  };
}

function planRoundSummaryText(round: PlanRoundInfo, run: AstraHandle | null): string | null {
  const lines: string[] = [];
  const summary = round.summary?.trim();
  if (summary) lines.push(summary);
  const tasks = round.tasks
    .map(planTaskSummaryText)
    .filter((value): value is string => Boolean(value));
  if (tasks.length > 0) {
    if (lines.length > 0) lines.push("");
    lines.push(tasks.join("\n\n"));
  } else {
    const reason = astraRunReasonText(run);
    if (reason) {
      if (lines.length > 0) lines.push("");
      lines.push(reason);
    }
  }
  return lines.join("\n").trim() || null;
}

function planTaskSummaryText(task: PlanTaskInfo): string | null {
  const title = task.title.trim();
  const prompt = task.prompt.trim();
  if (!title && !prompt) return null;
  if (!prompt) return `- ${title}`;
  if (!title) return `- ${indentPlanTaskPrompt(prompt)}`;
  return `- **${title}**\n  ${indentPlanTaskPrompt(prompt)}`;
}

function indentPlanTaskPrompt(prompt: string): string {
  return prompt.replace(/\n+/g, "\n").replace(/\n/g, "\n  ");
}

function isPlannerLane(lane: ThreadSessionLane): boolean {
  return lane.sources.some((source) =>
    source.role === "planner" ||
    (source.kind === "astra_internal" && Boolean(source.astraRunId)),
  );
}

function plannerSummaryFromTurns(
  turns: LiveTurn[],
  run: AstraHandle | null,
): { text: string; timestamp: number } | null {
  for (let turnIndex = turns.length - 1; turnIndex >= 0; turnIndex -= 1) {
    const turn = turns[turnIndex];
    for (let blockIndex = turn.blocks.length - 1; blockIndex >= 0; blockIndex -= 1) {
      const output = parsePlannerOutputText(plannerBlockText(turn.blocks[blockIndex]));
      if (!output) continue;
      const text = plannerOutputSummaryText(output, astraRunReasonText(run));
      if (!text) continue;
      return {
        text,
        timestamp:
          turn.blocks[blockIndex].timestamp ??
          turn.updatedAt ??
          turn.startedAt ??
          run?.updatedAt ??
          run?.createdAt ??
          Date.now(),
      };
    }
  }
  return null;
}

type PlannerOutput = {
  summary: string | null;
  reason: string | null;
  tasks: Array<{ title: string | null; prompt: string | null }>;
};

function plannerOutputSummaryText(output: PlannerOutput, fallbackReason: string | null): string | null {
  const lines: string[] = [];
  if (output.summary) lines.push(output.summary);
  const tasks = output.tasks
    .map((task) => planTaskTitlePromptText(task.title, task.prompt))
    .filter((value): value is string => Boolean(value));
  if (tasks.length > 0) {
    if (lines.length > 0) lines.push("");
    lines.push(tasks.join("\n\n"));
  } else {
    const reason = output.reason ?? fallbackReason;
    if (reason) {
      if (lines.length > 0) lines.push("");
      lines.push(reason);
    }
  }
  return lines.join("\n").trim() || null;
}

function planTaskTitlePromptText(
  titleValue: string | null | undefined,
  promptValue: string | null | undefined,
): string | null {
  const title = titleValue?.trim() ?? "";
  const prompt = promptValue?.trim() ?? "";
  if (!title && !prompt) return null;
  if (!prompt) return `- ${title}`;
  if (!title) return `- ${indentPlanTaskPrompt(prompt)}`;
  return `- **${title}**\n  ${indentPlanTaskPrompt(prompt)}`;
}

function astraRunReasonText(run: AstraHandle | null): string | null {
  return readableTooltipText(run?.terminalReason)
    ?? readableTooltipText(run?.lastErrorMessage)
    ?? readableTooltipText(run?.error);
}

function plannerBlockText(block: AcpRenderBlock): string {
  if (block.kind === "assistant" || block.kind === "thought" || block.kind === "user") {
    return block.blocks
      .map((item) => item.type === "text" ? item.text : "")
      .filter(Boolean)
      .join("\n")
      .trim();
  }
  if (block.kind === "sessionUpdate") {
    const data = block.data && typeof block.data === "object" && !Array.isArray(block.data)
      ? block.data as Record<string, unknown>
      : null;
    for (const key of ["text", "content", "output", "message", "summary"]) {
      const value = stringField(data, key);
      if (value) return value;
    }
  }
  return "";
}

function parsePlannerOutputText(text: string): PlannerOutput | null {
  for (const candidate of plannerOutputCandidates(text)) {
    const json = parsePlannerJsonOutput(candidate);
    if (json) return json;
    const fields = parsePlannerFieldOutput(candidate);
    if (fields) return fields;
  }
  return null;
}

function plannerOutputCandidates(text: string): string[] {
  const trimmed = text.trim();
  if (!trimmed) return [];
  const candidates = [trimmed];
  for (const match of trimmed.matchAll(/```(?:json|ya?ml)?\s*([\s\S]*?)```/gi)) {
    const fenced = match[1]?.trim();
    if (fenced) candidates.push(fenced);
  }
  const start = trimmed.indexOf("{");
  const end = trimmed.lastIndexOf("}");
  if (start >= 0 && end > start) candidates.push(trimmed.slice(start, end + 1));
  return Array.from(new Set(candidates));
}

function parsePlannerJsonOutput(text: string): PlannerOutput | null {
  try {
    const parsed = JSON.parse(text);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
    return plannerOutputFromRecord(parsed as Record<string, unknown>);
  } catch {
    return null;
  }
}

function plannerOutputFromRecord(record: Record<string, unknown>): PlannerOutput | null {
  const tasks = Array.isArray(record.tasks)
    ? record.tasks.map((item) => {
      const task = item && typeof item === "object" && !Array.isArray(item)
        ? item as Record<string, unknown>
        : null;
      return {
        title: stringField(task, "title"),
        prompt: stringField(task, "prompt"),
      };
    })
    : [];
  const summary = stringField(record, "summary");
  const reason = stringField(record, "reason");
  return summary || reason || tasks.length > 0 ? { summary, reason, tasks } : null;
}

function parsePlannerFieldOutput(text: string): PlannerOutput | null {
  const summary = plannerFieldValue(text, "summary");
  const reason = plannerFieldValue(text, "reason");
  const tasks = plannerFieldTasks(text);
  if (!summary && !reason && tasks.length === 0 && !/^\s*tasks\s*:/im.test(text)) return null;
  return { summary, reason, tasks };
}

function plannerFieldValue(text: string, key: string): string | null {
  const match = text.match(new RegExp(`^\\s*${key}\\s*:\\s*(.+?)\\s*$`, "im"));
  const value = match?.[1]?.trim();
  if (!value || value === "null") return null;
  return unquotePlannerValue(value);
}

function plannerFieldTasks(text: string): PlannerOutput["tasks"] {
  const lines = text.split(/\r?\n/);
  const start = lines.findIndex((line) => /^\s*tasks\s*:/i.test(line));
  if (start < 0) return [];
  const inline = lines[start].replace(/^\s*tasks\s*:\s*/i, "").trim();
  if (inline && inline !== "[]") {
    const parsed = parsePlannerJsonOutput(`{"tasks":${inline}}`);
    if (parsed) return parsed.tasks;
  }
  const tasks: PlannerOutput["tasks"] = [];
  let current: PlannerOutput["tasks"][number] | null = null;
  for (const line of lines.slice(start + 1)) {
    if (/^\S[\w-]*\s*:/i.test(line)) break;
    const itemMatch = line.match(/^\s*-\s*(.*)$/);
    if (itemMatch) {
      current = { title: null, prompt: null };
      tasks.push(current);
      readPlannerTaskField(itemMatch[1], current);
      continue;
    }
    if (current) readPlannerTaskField(line, current);
  }
  return tasks;
}

function readPlannerTaskField(line: string, task: PlannerOutput["tasks"][number]): void {
  const match = line.match(/^\s*(title|prompt)\s*:\s*(.*?)\s*$/i);
  if (!match) return;
  const key = match[1].toLowerCase();
  const value = unquotePlannerValue(match[2]);
  if (key === "title") task.title = value;
  if (key === "prompt") task.prompt = value;
}

function unquotePlannerValue(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed || trimmed === "null") return null;
  return trimmed.replace(/^["']|["']$/g, "").trim() || null;
}

function plannerDisplayLabel(source: ThreadReplaySessionSourceInfo | null): string | null {
  const label = source?.label?.trim();
  if (!label) return null;
  if (source?.role !== "planner") return label;
  return label.replace(/\s*[:：].*$/, "").trim() || label;
}

function stageTooltipDetail(
  snapshot: Record<string, unknown> | null,
  fallbackStage: StageInfo | null,
): string | null {
  return (
    firstReadableSnapshotField(snapshot, ["summary", "description", "outcome"])
    ?? readableTooltipText(fallbackStage?.summary)
    ?? readableTooltipText(fallbackStage?.description)
  );
}

function assistantTooltipDetail(
  snapshot: Record<string, unknown> | null,
  fallbackAssistant: { systemPrompt?: string | null } | null,
  source: ThreadReplaySessionSourceInfo | null,
): string | null {
  return (
    firstReadableSnapshotField(snapshot, [
      "summary",
      "description",
      "intro",
      "profile",
      "systemPrompt",
      "system_prompt",
    ])
    ?? readableTooltipText(fallbackAssistant?.systemPrompt)
    ?? readableTooltipText(source?.role)
  );
}

function agentTooltipDetail(snapshot: Record<string, unknown> | null): string | null {
  const agentInfo = objectField(snapshot, "agentInfo");
  const detail =
    firstReadableSnapshotField(agentInfo, ["description", "summary"])
    ?? [
      stringField(agentInfo, "model"),
      stringField(agentInfo, "mode"),
      stringField(agentInfo, "effort"),
    ].filter((item): item is string => Boolean(item)).join(" / ");
  return readableTooltipText(detail);
}

function participantSnapshotLabel(participant: Record<string, unknown> | null): string | null {
  if (!participant) return null;
  const agent = stringField(participant, "agent");
  return agent && agent in AGENT_LABEL ? AGENT_LABEL[agent as Agent] : agent;
}

function participantTooltipDetail(participant: Record<string, unknown> | null): string | null {
  if (!participant) return null;
  const detail = [
    stringField(participant, "model"),
    stringField(participant, "effort"),
    stringField(participant, "permissionMode"),
  ].filter((item): item is string => Boolean(item)).join(" / ");
  return readableTooltipText(detail);
}

function firstReadableSnapshotField(
  snapshot: Record<string, unknown> | null,
  fields: string[],
): string | null {
  for (const field of fields) {
    const value = readableTooltipText(stringField(snapshot, field));
    if (value) return value;
  }
  return null;
}

function preferredLaneSource(lane: ThreadSessionLane): ThreadReplaySessionSourceInfo | null {
  return (
    lane.sources.find((source) => source.kind === "plan_task")
    ?? lane.sources.find((source) => source.kind === "stage")
    ?? lane.sources.find((source) => source.kind === "astra_internal")
    ?? lane.sources.find((source) => source.kind === "thread")
    ?? lane.sources[0]
    ?? null
  );
}

function assistantFromSnapshot(snapshot: Record<string, unknown> | null): {
  name: string;
  color: string | null;
  agent: { id: Agent | string; model: string | null };
  systemPrompt?: string | null;
} | null {
  const name =
    stringField(snapshot, "name")
    ?? stringField(snapshot, "assistantId")
    ?? stringField(snapshot, "id");
  if (!name) return null;
  const agent = objectField(snapshot, "agent");
  return {
    name,
    color: stringField(snapshot, "color"),
    agent: {
      id: stringField(agent, "id") ?? "",
      model: stringField(agent, "model"),
    },
    systemPrompt: stringField(snapshot, "systemPrompt"),
  };
}

function assistantFromThread(
  thread: ThreadWorkState,
  source: ThreadReplaySessionSourceInfo | null,
  agent: Agent,
) {
  const direct =
    source?.planTaskId || source?.stageId
      ? null
      : thread.assistants.find((assistant) => assistant.agent.id === agent) ?? null;
  if (direct) return direct;
  if (source?.stageId) {
    const stage = thread.stages.find((item) => item.id === source.stageId) ?? null;
    const stageAssistant = stage?.assistants.find((assistant) => assistant.agent.id === agent) ?? null;
    if (stageAssistant) return stageAssistant;
  }
  return thread.assistants.find((assistant) => assistant.agent.id === agent) ?? null;
}

function snapshotStageIcon(snapshot: Record<string, unknown> | null): Pick<StageInfo, "kind" | "icon"> | null {
  if (!snapshot) return null;
  return {
    kind: stageKindField(snapshot, "kind"),
    icon: stringField(snapshot, "icon"),
  };
}

function stageKindField(record: Record<string, unknown> | null, key: string): StageInfo["kind"] {
  const value = stringField(record, key);
  if (
    value === "research" ||
    value === "plan" ||
    value === "develop" ||
    value === "build" ||
    value === "writing" ||
    value === "editing" ||
    value === "review" ||
    value === "proofreading" ||
    value === "screenplay" ||
    value === "storyboard" ||
    value === "design" ||
    value === "production" ||
    value === "human" ||
    value === "done"
  ) {
    return value;
  }
  return null;
}

function parseJsonObject(value: string | null): Record<string, unknown> | null {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : null;
  } catch {
    return null;
  }
}

function objectField(value: Record<string, unknown> | null, key: string): Record<string, unknown> | null {
  const field = value?.[key];
  return field && typeof field === "object" && !Array.isArray(field)
    ? field as Record<string, unknown>
    : null;
}

function stringField(value: Record<string, unknown> | null, key: string): string | null {
  const field = value?.[key];
  return typeof field === "string" && field.trim() ? field.trim() : null;
}

function laneMessageEmptyText({
  lane,
  loading,
  loadError,
  t,
}: {
  lane: ThreadSessionLane;
  loading: boolean;
  loadError: string | null;
  t: (key: string, vars?: Record<string, string | number>) => string;
}): { title: string; detail: string | null } {
  if (loading) {
    return { title: t("thread.preview_loading"), detail: null };
  }
  if (loadError) {
    return { title: t("thread.preview_error"), detail: loadError };
  }
  if (lane.status === "missing") {
    return {
      title: t("thread.missing_lane_ready"),
      detail: t("thread.preview_reference_only"),
    };
  }
  if (!lane.session && !lane.liveSession) {
    return {
      title: t("thread.pending_lane_ready"),
      detail: t("thread.preview_waiting"),
    };
  }
  if (lane.session && !isPersistedSession(lane.session)) {
    return {
      title: t("thread.preview_history_unavailable"),
      detail: t("thread.preview_waiting"),
    };
  }
  return {
    title: t("thread.preview_empty"),
    detail: t("thread.preview_waiting"),
  };
}

async function createManualStagePlanTask({
  thread,
  stage,
  targetAgent,
  prompt,
  runtimeAgent,
}: {
  thread: ThreadInfo;
  stage: StageInfo;
  targetAgent: Agent;
  prompt: string;
  runtimeAgent: RuntimeAgentMetadata | null;
}): Promise<PlanTaskInfo> {
  const stageAssistant =
    stage.assistants.find((assistant) => assistant.agent.id === targetAgent)
    ?? stage.assistants[0]
    ?? null;
  const threadAssistant =
    stageAssistant
      ? thread.assistants.find((assistant) => assistant.assistantId === stageAssistant.assistantId) ?? null
      : null;
  const assistantSnapshot = stageAssistant ?? threadAssistant;
  const title = manualStageTaskTitle(stage, prompt);
  const round = await createPlanRound({
    threadId: thread.id,
    mode: "parallel",
    source: "manual",
    status: "planned",
    summary: title,
    tasks: [
      {
        threadStageId: stage.id,
        assistantId: stageAssistant?.assistantId ?? threadAssistant?.assistantId ?? null,
        targetAgent,
        stageSnapshotJson: JSON.stringify(stage),
        assistantSnapshotJson: assistantSnapshot ? JSON.stringify(assistantSnapshot) : null,
        agentSnapshotJson: JSON.stringify({
          agent: targetAgent,
          agentInfo: runtimeAgent,
        }),
        title,
        prompt,
        expectedOutput: null,
        risk: "medium",
        sortOrder: 0,
        status: "planned",
      },
    ],
  });
  const task = round.tasks[0];
  if (!task) {
    throw new Error("Manual plan round did not create a task");
  }
  return task;
}

function manualStageTaskTitle(stage: StageInfo, prompt: string): string {
  const stageName = stage.name?.trim() || stage.kind || "Stage";
  const compactPrompt = prompt.replace(/\s+/g, " ").trim();
  const shortPrompt =
    compactPrompt.length > 72 ? `${compactPrompt.slice(0, 69)}...` : compactPrompt;
  return `${stageName}: ${shortPrompt || "Manual task"}`;
}
