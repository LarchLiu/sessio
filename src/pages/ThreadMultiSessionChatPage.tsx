import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import HashIcon from "@iconify-react/mynaui/hash";
import {
  AlertCircle,
  ArrowLeft,
  Bot,
  Clock,
  ExternalLink,
  GitBranch,
  ListChecks,
  LoaderCircle,
  MessagesSquare,
  Sparkles,
  Square,
  RefreshCw,
} from "lucide-react";
import type {
  Agent,
  AstraEvent,
  AstraHandle,
  AstraRunStatus,
  PlanRoundInfo,
  PlanTaskInfo,
  PlanTaskStatus,
  ProjectInfo,
  RuntimeAgentMetadata,
  RuntimeAgentSelection,
  SessionInfo,
  SetRuntimeAgentSelectionRequest,
  StageInfo,
  ThreadInfo,
  ThreadKind,
  ThreadReplayInfo,
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
  updatePlanTaskStatus,
} from "../api";
import ChatComposer, { NewChatMenuButton } from "../components/ChatComposer";
import { AgentGlyph } from "../components/AgentIcon";
import { LiveSessionStatusBadge } from "../components/AcpTranscriptPanel";
import ScrollArea from "../components/ScrollArea";
import {
  contentBlocksText,
  mergeHistoryWithLiveTurns,
  stripImagePlaceholders,
} from "../historyMerge";
import { useChatComposer } from "../hooks/useChatComposer";
import { localeTag, useI18n } from "../i18n";
import type { PendingNewChatSession } from "../navigation";
import type { AcpRenderBlock, LiveRuntimeAction, LiveRuntimeState, LiveTurn } from "../runtimeChat";
import { sessionDisplayTitle } from "../appUtils";
import {
  buildThreadSessionLanes,
  replaySourceKey,
  replaySourceTitle,
  shortSessionId,
  type ThreadSessionLane,
  type ThreadSessionLaneStatus,
} from "../threadReplayView";
import { buildThreadWorkSnapshot, renderThreadWorkContext } from "../threadSnapshot";
import { collectThreadChatSessions } from "../threadChats";
import { collectThreadHistorySnapshots, withThreadChatSessions } from "../threadWorkContext";
import { projectStageLabel, stageStatusVisual } from "../utils/stageDisplay";
import { MarkdownContent, type MarkdownImage } from "./ChatPage";

const THREAD_REFRESH_ASTRA_EVENTS = new Set(["delegated", "stage_update_result", "task_dispatch"]);

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
  onBackToOverview,
  onSelectSession,
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
  onBackToOverview: () => void;
  onSelectSession: (session: SessionInfo) => void;
  onPendingSession: (session: PendingNewChatSession) => void;
  onError: (error: string | null) => void;
}) {
  const { t, lang } = useI18n();
  const [thread, setThread] = useState<ThreadWorkState | null>(null);
  const [replay, setReplay] = useState<ThreadReplayInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [stageTaskMode, setStageTaskMode] = useState(false);
  const [stageTaskBusy, setStageTaskBusy] = useState(false);
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

  const refresh = async () => {
    setRefreshing(true);
    setLoadError(null);
    try {
      const { nextThread, nextReplay } = await load();
      setThread(nextThread);
      setReplay(nextReplay);
    } catch (err) {
      const message = String(err);
      setLoadError(message);
      onError(message);
    } finally {
      setRefreshing(false);
    }
  };

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
  const liveCount = lanes.filter((lane) => lane.status === "live").length;
  const pendingCount = lanes.filter((lane) => lane.status === "pending").length;
  const threadChatSessions = useMemo(
    () => thread ? collectThreadChatSessions(thread, replay) : [],
    [replay, thread],
  );
  const canRunStageTask =
    Boolean(thread && thread.kind === "workflow" && activeStage && composer.selectedAgent);

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
    const stageId = thread.kind === "workflow" ? activeStage?.id ?? null : null;
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
      <div className="shrink-0 border-b border-ink/5 bg-surface-panel-alt px-5 py-3">
        <div className="flex min-w-0 flex-wrap items-center justify-between gap-3">
          <div className="flex min-w-0 items-center gap-3">
            <button
              type="button"
              onClick={onBackToOverview}
              title={t("thread.back_to_overview")}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded border border-ink/12 bg-surface-panel text-ink/50 transition hover:bg-ink/[0.05] hover:text-ink/80"
            >
              <ArrowLeft className="h-4 w-4" />
            </button>
            <div className="min-w-0">
              <div className="flex min-w-0 flex-wrap items-center gap-2">
                <MessagesSquare className="h-4 w-4 shrink-0 text-[rgb(var(--color-emerald)/0.85)]" />
                <h1 className="min-w-0 truncate text-body font-medium text-ink/85">
                  {thread?.goal ?? t("thread.multi_session_chat")}
                </h1>
                {thread && (
                  <span className="rounded bg-ink/[0.06] px-1.5 py-0.5 text-meta font-medium text-ink/45">
                    {t(`thread.kind.${thread.kind}`)}
                  </span>
                )}
              </div>
              <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2 text-caption text-ink/38">
                <span className="truncate">{project.name}</span>
                {thread && (
                  <>
                    <span>{t("thread.sessions")}: {lanes.length}</span>
                    <span>{t("thread.live_lanes")}: {liveCount}</span>
                    {pendingCount > 0 && <span>{t("thread.pending_lanes")}: {pendingCount}</span>}
                    <span>{t("meta.updated")}: {formatDate(thread.updatedAt, lang) ?? "-"}</span>
                  </>
                )}
              </div>
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              onClick={() => void refresh()}
              disabled={refreshing}
              title={t("thread.refresh")}
              className="flex h-8 w-8 items-center justify-center rounded border border-ink/12 bg-surface-panel text-ink/50 transition hover:bg-ink/[0.05] hover:text-ink/80 disabled:opacity-45"
            >
              {refreshing ? (
                <LoaderCircle className="h-4 w-4 animate-spin" />
              ) : (
                <RefreshCw className="h-4 w-4" />
              )}
            </button>
          </div>
        </div>
      </div>

      <ScrollArea className="min-h-0 flex-1" viewportClassName="px-5 py-4">
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
          <div className="grid gap-4">
            <section className="grid grid-cols-[repeat(auto-fit,minmax(180px,1fr))] gap-3">
              <ThreadSummaryStat label={t("stage.project_stages")} value={String(sortedStages.length)} />
              <ThreadSummaryStat label={t("assistant.title")} value={String(threadAssistantCount(thread))} />
              <ThreadSummaryStat label={t("thread.sessions")} value={String(lanes.length)} />
              <ThreadSummaryStat
                label={t("thread.current_stage")}
                value={activeStage ? projectStageLabel(activeStage, t) : "-"}
              />
            </section>

            {thread.description && (
              <p className="max-w-[860px] whitespace-pre-wrap text-body-sm leading-relaxed text-ink/55">
                {thread.description}
              </p>
            )}

            <ThreadOrchestrationPanel
              thread={thread}
              stages={sortedStages}
              onError={onError}
              onReload={refresh}
            />

            {sortedStages.length > 0 && (
              <section className="grid grid-cols-[repeat(auto-fit,minmax(180px,1fr))] gap-2">
                {sortedStages.map((stage) => {
                  const visual = stageStatusVisual(stage.status);
                  const Icon = visual.icon;
                  return (
                    <div
                      key={stage.id}
                      className="min-w-0 rounded-md border border-card-border/[0.10] bg-card px-2.5 py-2"
                    >
                      <div className="flex min-w-0 items-center gap-2">
                        <Icon className={"h-4 w-4 shrink-0 " + visual.textClass} />
                        <span className="min-w-0 flex-1 truncate text-body-sm font-medium text-ink/75">
                          {projectStageLabel(stage, t)}
                        </span>
                        <span className={"rounded px-1.5 py-0.5 text-meta font-medium " + visual.textClass}>
                          {t(`stage.status.${stage.status}`)}
                        </span>
                      </div>
                    </div>
                  );
                })}
              </section>
            )}

            {lanes.length === 0 ? (
              <ThreadMultiSessionEmpty
                icon={<MessagesSquare className="h-5 w-5 text-ink/35" />}
                title={t("thread.multi_session_empty")}
                detail={replay ? t("thread.no_chats") : t("thread.history_pending")}
              />
            ) : (
              <section className="grid gap-3 lg:grid-cols-[repeat(auto-fit,minmax(360px,1fr))]">
                {lanes.map((lane) => (
                  <ThreadSessionLaneCard
                    key={lane.laneId}
                    lane={lane}
                    threadKind={thread.kind}
                    now={Date.now()}
                    onSelectSession={onSelectSession}
                  />
                ))}
              </section>
            )}
          </div>
        )}
      </ScrollArea>
      <div className="shrink-0 border-t border-ink/[0.08] bg-surface-panel px-5 py-3">
        <div className="mx-auto flex w-full max-w-[760px] justify-center">
          <ChatComposer
            composer={composer}
            title={null}
            canSend={
              Boolean(thread) &&
              composer.canSendWithWorkspace(project.path) &&
              (!stageTaskMode || canRunStageTask) &&
              !stageTaskBusy
            }
            onSend={() => void handleSend()}
            bottomRow={
              <div className="flex h-10 min-w-0 items-center gap-2 px-3 text-body-sm text-ink/55">
                <span className="min-w-0 truncate rounded-md px-1.5 py-1 text-ink/55">
                  {thread?.goal ?? t("thread.multi_session_chat")}
                </span>
                {thread?.kind === "workflow" && (
                  <button
                    type="button"
                    onClick={() => setStageTaskMode((value) => !value)}
                    disabled={!activeStage || !composer.selectedAgent || stageTaskBusy}
                    className={
                      "flex shrink-0 items-center gap-1.5 rounded-md px-1.5 py-1 transition disabled:opacity-40 " +
                      (stageTaskMode
                        ? "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]"
                        : "text-ink/55 hover:bg-ink/8 hover:text-ink")
                    }
                    title={t("thread.run_stage_task")}
                  >
                    {stageTaskBusy ? (
                      <LoaderCircle className="h-4 w-4 animate-spin" />
                    ) : (
                      <ListChecks className="h-4 w-4" />
                    )}
                    <span className="truncate">{t("thread.stage_task_mode")}</span>
                  </button>
                )}
                <NewChatMenuButton icon={GitBranch} label="thread" text />
              </div>
            }
          />
        </div>
      </div>
    </div>
  );
}

function ThreadSessionLaneCard({
  lane,
  threadKind,
  now,
  onSelectSession,
}: {
  lane: ThreadSessionLane;
  threadKind: ThreadKind;
  now: number;
  onSelectSession: (session: SessionInfo) => void;
}) {
  const { t } = useI18n();
  return (
    <article className="flex min-h-[260px] min-w-0 flex-col overflow-hidden rounded-lg border border-card-border/[0.12] bg-card">
      <header className="shrink-0 border-b border-card-border/[0.10] px-3 py-2.5">
        <div className="flex min-w-0 items-start justify-between gap-2">
          <div className="min-w-0">
            <div className="flex min-w-0 items-center gap-2">
              <AgentGlyph agent={lane.agent} className="h-4 w-4 shrink-0" />
              <h2 className="min-w-0 truncate text-body-sm font-medium text-ink/78">
                {lane.session
                  ? sessionDisplayTitle(lane.session) ?? t("list.no_user_message")
                  : shortSessionId(lane.sessionId)}
              </h2>
            </div>
            <div className="mt-1 flex min-w-0 flex-wrap items-center gap-1.5">
              <span className="rounded bg-ink/[0.06] px-1.5 py-0.5 text-meta text-ink/42">
                {AGENT_LABEL[lane.agent]}
              </span>
              <span className="rounded bg-ink/[0.06] px-1.5 py-0.5 text-meta text-ink/42">
                {lane.groupLabel}
              </span>
              <LaneStatusBadge status={lane.status} />
              <LiveSessionStatusBadge liveSession={lane.liveSession} now={now} />
              {threadKind === "brainstorm" && (
                <span className="rounded bg-sky-500/[0.08] px-1.5 py-0.5 text-meta text-sky-500">
                  {t("thread.shared_board")}
                </span>
              )}
              {threadKind === "debate" && (
                <span className="rounded bg-amber-500/[0.08] px-1.5 py-0.5 text-meta text-amber-500">
                  {t("thread.isolated_lane")}
                </span>
              )}
            </div>
          </div>
          {lane.session && (
            <button
              type="button"
              onClick={() => onSelectSession(lane.session!)}
              title={t("thread.open_full_chat")}
              className="flex h-8 shrink-0 items-center gap-1.5 rounded border border-ink/12 bg-surface-panel px-2 text-caption font-medium text-ink/55 transition hover:bg-ink/[0.05] hover:text-ink/82"
            >
              <ExternalLink className="h-3.5 w-3.5" />
              <span>{t("thread.open_detail_page")}</span>
            </button>
          )}
        </div>
        <div className="mt-2 flex min-w-0 flex-wrap gap-1">
          {lane.sources.length === 0 ? (
            <span className="rounded border border-dashed border-card-border/[0.12] px-1.5 py-0.5 text-meta text-ink/32">
              {t("thread.pending_lane")}
            </span>
          ) : (
            lane.sources.map((source) => (
              <span
                key={replaySourceKey(source)}
                title={replaySourceTitle(source)}
                className="max-w-full truncate rounded bg-ink/[0.055] px-1.5 py-0.5 text-meta text-ink/38"
              >
                {source.label ?? t(`thread.replay_source.${source.kind}`)}
              </span>
            ))
          )}
        </div>
      </header>
      <div className="flex min-h-0 flex-1 flex-col px-3 py-3">
        <ThreadSessionLanePreview lane={lane} />
        <div className="mt-2 flex min-w-0 flex-wrap items-center gap-2 text-meta text-ink/32">
          {lane.session && (
            <span>{t("list.msgs", { count: lane.session.messageCount })}</span>
          )}
          <span className="min-w-0 truncate">{lane.sessionId}</span>
          {lane.sessioRuntimeSessionId && (
            <span className="min-w-0 truncate">{lane.sessioRuntimeSessionId}</span>
          )}
        </div>
      </div>
    </article>
  );
}

function LaneStatusBadge({ status }: { status: ThreadSessionLaneStatus }) {
  const { t } = useI18n();
  const klass =
    status === "live"
      ? "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]"
      : status === "pending"
        ? "bg-sky-500/[0.10] text-sky-500"
        : status === "missing" || status === "failed"
          ? "bg-red-500/[0.10] text-red-500"
          : "bg-ink/[0.06] text-ink/45";
  return (
    <span className={"rounded px-1.5 py-0.5 text-meta font-medium " + klass}>
      {t(`thread.lane_status.${status}`)}
    </span>
  );
}

function ThreadSummaryStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-lg border border-card-border/[0.12] bg-card px-3 py-2.5">
      <div className="text-caption uppercase tracking-normal text-ink/35">{label}</div>
      <div className="mt-1 truncate text-body font-medium text-ink/80">{value}</div>
    </div>
  );
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

function ThreadOrchestrationPanel({
  thread,
  stages,
  onError,
  onReload,
}: {
  thread: ThreadInfo;
  stages: StageInfo[];
  onError: (error: string | null) => void;
  onReload: () => Promise<void>;
}) {
  const { t } = useI18n();
  const [runs, setRuns] = useState<AstraHandle[]>([]);
  const [planRounds, setPlanRounds] = useState<PlanRoundInfo[]>([]);
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState<"start" | "cancel" | null>(null);
  const activeRun = runs.find((run) => isAstraActive(run.status)) ?? runs[0] ?? null;
  const canStartAstra = thread.kind === "teamwork" || thread.kind === "brainstorm" || thread.kind === "debate";
  const orderedPlanRounds = useMemo(
    () => planRounds.slice().sort((a, b) => b.roundIndex - a.roundIndex || b.createdAt - a.createdAt),
    [planRounds],
  );

  const reloadAstraState = useCallback(() => {
    return Promise.all([listAstraRuns(thread.id), listPlanRounds(thread.id)])
      .then(([nextRuns, nextPlanRounds]) => {
        setRuns(nextRuns);
        setPlanRounds(nextPlanRounds);
      })
      .catch((err) => onError(String(err)));
  }, [onError, thread.id]);

  useEffect(() => {
    void reloadAstraState();
  }, [reloadAstraState]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<AstraEvent>("astra-run-event", (event) => {
      if (event.payload.threadId !== thread.id) return;
      const eventType = event.payload.eventType;
      void reloadAstraState();
      if (THREAD_REFRESH_ASTRA_EVENTS.has(eventType)) {
        void onReload();
      }
    }).then((fn) => {
      unlisten = fn;
    }).catch((err) => onError(String(err)));
    return () => {
      unlisten?.();
    };
  }, [onError, onReload, reloadAstraState, thread.id]);

  const start = async () => {
    if (!canStartAstra) return;
    setBusy("start");
    try {
      const run = await createAstraRun(thread.id, prompt.trim() || null);
      setRuns((prev) => upsertRun(prev, run));
      setPrompt("");
      await reloadAstraState();
    } catch (err) {
      onError(String(err));
    } finally {
      setBusy(null);
    }
  };

  const cancel = async () => {
    if (!activeRun) return;
    setBusy("cancel");
    try {
      const run = await cancelAstraRun(activeRun.runId);
      setRuns((prev) => upsertRun(prev, run));
      await reloadAstraState();
    } catch (err) {
      onError(String(err));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="rounded-lg border border-card-border/[0.12] bg-card px-3 py-2.5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-2 text-body-sm font-medium text-ink/78">
            <Sparkles className="h-4 w-4 text-[rgb(var(--color-emerald)/0.85)]" />
            <span>Astra</span>
            {activeRun && (
              <span className={"rounded px-1.5 py-0.5 text-meta font-medium " + astraStatusClass(activeRun.status)}>
                {formatPlanStatus(activeRun.status)}
              </span>
            )}
            {thread.kind === "workflow" && (
              <span className="rounded bg-ink/[0.06] px-1.5 py-0.5 text-meta text-ink/42">
                {t("thread.workflow_manual_tasks")}
              </span>
            )}
          </div>
          <div className="mt-1 max-w-[760px] truncate text-caption text-ink/38">
            {activeRun ? activeRun.runId : canStartAstra ? t("astra.idle") : t("astra.unsupported.workflow")}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {activeRun && isAstraActive(activeRun.status) && (
            <button
              type="button"
              disabled={busy !== null}
              onClick={() => void cancel()}
              title={t("astra.cancel")}
              className="flex h-8 w-8 items-center justify-center rounded border border-ink/15 bg-surface-panel text-ink/45 hover:bg-red-500/[0.08] hover:text-red-500 disabled:opacity-40"
            >
              {busy === "cancel" ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <Square className="h-3.5 w-3.5" />}
            </button>
          )}
          {canStartAstra && (
            <button
              type="button"
              disabled={busy !== null || Boolean(activeRun && isAstraActive(activeRun.status))}
              onClick={() => void start()}
              title={t("astra.start")}
              className="flex h-8 items-center gap-1.5 rounded border border-ink/15 bg-surface-panel px-2 text-caption text-ink/55 hover:bg-ink/[0.05] hover:text-ink/80 disabled:opacity-40"
            >
              {busy === "start" ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <Bot className="h-3.5 w-3.5" />}
              {t("astra.start")}
            </button>
          )}
        </div>
      </div>

      <div className="mt-2 grid gap-2">
        {canStartAstra && (
          <textarea
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            rows={2}
            placeholder={t("astra.prompt_placeholder")}
            className="min-w-0 resize-none rounded-md border border-input-border/[0.16] bg-input px-3 py-2 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30"
          />
        )}
        {orderedPlanRounds.length > 0 && (
          <div className="grid gap-1.5">
            {orderedPlanRounds.slice(0, 3).map((round) => (
              <ThreadPlanRoundSummary key={round.id} round={round} stages={stages} thread={thread} />
            ))}
          </div>
        )}
        {activeRun?.error && (
          <div className="rounded-md border border-status-error/20 bg-status-error/10 px-2.5 py-2 text-caption text-status-error">
            {activeRun.error}
          </div>
        )}
      </div>
    </section>
  );
}

function ThreadPlanRoundSummary({
  round,
  stages,
  thread,
}: {
  round: PlanRoundInfo;
  stages: StageInfo[];
  thread: ThreadInfo;
}) {
  const { t } = useI18n();
  return (
    <div className="rounded-md border border-card-border/[0.10] bg-card-panel px-2.5 py-2">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <span className="text-caption font-medium text-ink/65">
          {t("astra.round", { index: round.roundIndex + 1 })}
        </span>
        <span className={"rounded px-1.5 py-0.5 text-meta font-medium " + planRoundStatusClass(round.status)}>
          {formatPlanStatus(round.status)}
        </span>
        <span className="rounded bg-ink/[0.06] px-1.5 py-0.5 text-meta text-ink/42">
          {t(`astra.mode.${round.mode}`)}
        </span>
        <span className="rounded bg-ink/[0.06] px-1.5 py-0.5 text-meta text-ink/42">
          {round.tasks.length} {t("thread.tasks")}
        </span>
      </div>
      {round.tasks.length > 0 && (
        <div className="mt-1.5 grid gap-1">
          {round.tasks.slice(0, 4).map((task) => (
            <ThreadPlanTaskSummary
              key={task.id}
              task={task}
              stage={stages.find((stage) => stage.id === task.threadStageId) ?? null}
              assistantName={thread.assistants.find((assistant) => assistant.assistantId === task.assistantId)?.name ?? null}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function ThreadPlanTaskSummary({
  task,
  stage,
  assistantName,
}: {
  task: PlanTaskInfo;
  stage: StageInfo | null;
  assistantName: string | null;
}) {
  const { t } = useI18n();
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1.5 rounded bg-card px-2 py-1 text-caption text-ink/50">
      <AgentGlyph agent={task.targetAgent} className="h-3.5 w-3.5 shrink-0" />
      <span className="min-w-0 max-w-[320px] truncate font-medium text-ink/68">{task.title}</span>
      <span className={"rounded px-1 py-0.5 text-meta font-medium " + astraTaskStatusClass(task.status)}>
        {formatPlanStatus(task.status)}
      </span>
      {assistantName && <span className="truncate text-ink/35">{assistantName}</span>}
      {stage && <span className="truncate text-ink/35">{projectStageLabel(stage, t)}</span>}
      {task.sessions.length > 0 && (
        <span className="rounded bg-ink/[0.045] px-1 py-0.5 text-meta text-ink/35">
          {t("astra.task_sessions", { count: task.sessions.length })}
        </span>
      )}
    </div>
  );
}

type LanePreviewItem = {
  key: string;
  label: string;
  text: string;
  markdown: boolean;
  tone: "normal" | "muted" | "danger";
  timestamp: number | null;
};

function ThreadSessionLanePreview({ lane }: { lane: ThreadSessionLane }) {
  const { t } = useI18n();
  const [historyTurns, setHistoryTurns] = useState<LiveTurn[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const previewScrollRef = useRef<HTMLDivElement>(null);
  const onPreviewImage = useCallback((_image: MarkdownImage) => undefined, []);

  const sessionFilePath = lane.session?.filePath ?? "";
  const sessionAvailable = lane.session?.available ?? false;
  const sessionMessageCount = lane.session?.messageCount ?? 0;
  useEffect(() => {
    if (!lane.session || !sessionFilePath || !sessionAvailable) {
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
    lane.session,
    lane.sessionId,
    sessionAvailable,
    sessionFilePath,
    sessionMessageCount,
  ]);

  const mergedTurns = useMemo(
    () => mergeHistoryWithLiveTurns(historyTurns, lane.liveSession?.turns ?? []),
    [historyTurns, lane.liveSession?.turns],
  );
  const preview = useMemo(
    () => latestLanePreviewItem(mergedTurns, t),
    [mergedTurns, t],
  );
  const emptyText = lanePreviewEmptyText({
    lane,
    loading,
    loadError,
    t,
  });
  const visiblePreview = preview ?? (
    loadError
      ? {
        key: `error:${loadError}`,
        label: t("thread.preview_error"),
        text: loadError,
        markdown: false,
        tone: "danger" as const,
        timestamp: null,
      }
      : null
  );

  useEffect(() => {
    const viewport = previewScrollRef.current;
    if (!viewport) return;
    viewport.scrollTop = viewport.scrollHeight;
  }, [visiblePreview?.key, visiblePreview?.text]);

  return (
    <div className="min-h-0 overflow-hidden rounded-md border border-card-border/[0.12] bg-card-panel">
      <div className="flex h-8 items-center justify-between gap-2 border-b border-card-border/[0.10] px-2.5">
        <div className="min-w-0 truncate text-caption font-medium text-ink/48">
          {visiblePreview?.label ?? t("thread.preview_latest")}
        </div>
        {loading ? (
          <LoaderCircle className="h-3.5 w-3.5 shrink-0 animate-spin text-ink/32" />
        ) : visiblePreview?.timestamp ? (
          <span className="shrink-0 text-meta text-ink/28">
            {formatPreviewTime(visiblePreview.timestamp)}
          </span>
        ) : null}
      </div>
      <ScrollArea
        ref={previewScrollRef}
        className="h-40 min-h-0"
        viewportClassName="px-3 py-2.5"
        persistScrollbars
      >
        {visiblePreview ? (
          visiblePreview.markdown ? (
            <div className={lanePreviewTextClass(visiblePreview.tone)}>
              <MarkdownContent text={visiblePreview.text} onPreviewImage={onPreviewImage} />
            </div>
          ) : (
            <pre className={lanePreviewPlainTextClass(visiblePreview.tone)}>
              {visiblePreview.text}
            </pre>
          )
        ) : (
          <div className="flex h-full min-h-[136px] items-center justify-center px-2 text-center">
            <div className="max-w-[300px]">
              <Clock className="mx-auto h-5 w-5 text-ink/24" />
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
      </ScrollArea>
      {loadError && preview && (
        <div className="border-t border-status-error/15 px-2.5 py-1.5 text-meta text-status-error/80">
          {loadError}
        </div>
      )}
    </div>
  );
}

function normalizeSessionHistoryTurns(turns: unknown[] | undefined): LiveTurn[] {
  return Array.isArray(turns) ? (turns as LiveTurn[]) : [];
}

function latestLanePreviewItem(
  turns: LiveTurn[],
  t: (key: string, vars?: Record<string, string | number>) => string,
): LanePreviewItem | null {
  for (let turnIndex = turns.length - 1; turnIndex >= 0; turnIndex -= 1) {
    const turn = turns[turnIndex];
    for (let blockIndex = turn.blocks.length - 1; blockIndex >= 0; blockIndex -= 1) {
      const block = turn.blocks[blockIndex];
      if (block.kind !== "assistant") continue;
      const item = previewItemForBlock(block, turn, blockIndex, t, false);
      if (item) return item;
    }
  }

  for (let turnIndex = turns.length - 1; turnIndex >= 0; turnIndex -= 1) {
    const turn = turns[turnIndex];
    if (turn.error) {
      return {
        key: `${turn.turnId}:turn-error`,
        label: t("thread.preview_error"),
        text: turn.error.message,
        markdown: false,
        tone: "danger",
        timestamp: turn.updatedAt,
      };
    }
    for (let blockIndex = turn.blocks.length - 1; blockIndex >= 0; blockIndex -= 1) {
      const block = turn.blocks[blockIndex];
      if (block.kind === "assistant" || block.kind === "user") continue;
      const item = previewItemForBlock(block, turn, blockIndex, t, false);
      if (item) return item;
    }
  }

  for (let turnIndex = turns.length - 1; turnIndex >= 0; turnIndex -= 1) {
    const turn = turns[turnIndex];
    for (let blockIndex = turn.blocks.length - 1; blockIndex >= 0; blockIndex -= 1) {
      const block = turn.blocks[blockIndex];
      if (block.kind !== "user") continue;
      const item = previewItemForBlock(block, turn, blockIndex, t, true);
      if (item) return item;
    }
  }
  return null;
}

function previewItemForBlock(
  block: AcpRenderBlock,
  turn: LiveTurn,
  blockIndex: number,
  t: (key: string, vars?: Record<string, string | number>) => string,
  includeUser: boolean,
): LanePreviewItem | null {
  if (block.kind === "assistant" || block.kind === "thought" || block.kind === "user") {
    if (block.kind === "user" && !includeUser) return null;
    const text = stripImagePlaceholders(contentBlocksText(block.blocks)).trim();
    if (!text) return null;
    return {
      key: `${turn.turnId}:${block.kind}:${blockIndex}:${text.length}`,
      label:
        block.kind === "assistant"
          ? t("thread.preview_latest_result")
          : block.kind === "thought"
            ? t("thread.preview_thought")
            : t("thread.preview_last_user_message"),
      text,
      markdown: true,
      tone: block.kind === "thought" ? "muted" : "normal",
      timestamp: block.timestamp ?? turn.updatedAt,
    };
  }
  if (block.kind === "error") {
    return {
      key: `${turn.turnId}:error:${blockIndex}`,
      label: t("thread.preview_error"),
      text: block.error.message,
      markdown: false,
      tone: "danger",
      timestamp: block.timestamp ?? turn.updatedAt,
    };
  }
  if (block.kind === "sessionUpdate") {
    const text = sessionUpdatePreviewText(block);
    if (!text) return null;
    return {
      key: `${turn.turnId}:session-update:${blockIndex}:${text.length}`,
      label: block.updateType === "file_edit"
        ? t("thread.preview_file_edit")
        : t("thread.preview_latest_update"),
      text,
      markdown: false,
      tone: "muted",
      timestamp: block.timestamp ?? turn.updatedAt,
    };
  }
  if (block.kind === "tool") {
    const tool = turn.tools.find((item) => item.toolId === block.toolId);
    return {
      key: `${turn.turnId}:tool:${block.toolId}:${tool?.updatedAt ?? blockIndex}`,
      label: t("thread.preview_tool"),
      text: [tool?.title ?? block.toolId, tool?.status].filter(Boolean).join(" / "),
      markdown: false,
      tone: "muted",
      timestamp: block.timestamp ?? tool?.updatedAt ?? turn.updatedAt,
    };
  }
  if (block.kind === "permission") {
    const permission = turn.permissions.find((item) => item.requestId === block.requestId);
    return {
      key: `${turn.turnId}:permission:${block.requestId}:${permission?.selectedOptionId ?? ""}`,
      label: t("thread.preview_permission"),
      text: permission?.toolName ?? block.requestId,
      markdown: false,
      tone: "muted",
      timestamp: block.timestamp ?? turn.updatedAt,
    };
  }
  return null;
}

function sessionUpdatePreviewText(
  block: Extract<AcpRenderBlock, { kind: "sessionUpdate" }>,
): string {
  if (block.updateType === "file_edit") {
    const summary = fileEditPreviewText(block.data);
    if (summary) return summary;
  }
  const record = asRecord(block.data);
  const text = typeof record?.text === "string" ? record.text.trim() : "";
  if (text) return text;
  return stablePreviewText(block.data, 900);
}

function fileEditPreviewText(value: unknown): string | null {
  const parsed = parseMaybeJsonObject(value);
  const nested = typeof parsed?.text === "string" ? parseMaybeJsonObject(parsed.text) : null;
  const summary = nested ?? parsed;
  const edits = Array.isArray(summary?.edits)
    ? summary.edits.filter((item): item is Record<string, unknown> => Boolean(item) && typeof item === "object" && !Array.isArray(item))
    : [];
  if (edits.length === 0) return null;
  const additions = numberField(summary, "additions") ?? sumEditField(edits, "additions");
  const deletions = numberField(summary, "deletions") ?? sumEditField(edits, "deletions");
  const files = numberField(summary, "files") ?? edits.length;
  const paths = edits
    .map((edit) => stringField(edit, "displayPath") ?? stringField(edit, "path"))
    .filter((path): path is string => Boolean(path))
    .slice(0, 5);
  const more = Math.max(0, files - paths.length);
  return [
    `Edited ${files} ${files === 1 ? "file" : "files"} (+${additions} -${deletions})`,
    paths.length > 0 ? paths.join("\n") : null,
    more > 0 ? `+${more} more` : null,
  ].filter(Boolean).join("\n");
}

function lanePreviewEmptyText({
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
  if (lane.session && (!lane.session.available || !lane.session.filePath)) {
    return {
      title: t("thread.preview_history_unavailable"),
      detail: t("thread.preview_open_detail_for_full"),
    };
  }
  return {
    title: t("thread.preview_empty"),
    detail: t("thread.preview_waiting"),
  };
}

function lanePreviewTextClass(tone: LanePreviewItem["tone"]): string {
  return "text-body-sm leading-relaxed break-words " + (
    tone === "danger"
      ? "text-status-error"
      : tone === "muted"
        ? "text-ink/55"
        : "text-ink/72"
  );
}

function lanePreviewPlainTextClass(tone: LanePreviewItem["tone"]): string {
  return "whitespace-pre-wrap break-words text-body-sm leading-relaxed " + (
    tone === "danger"
      ? "text-status-error"
      : tone === "muted"
        ? "text-ink/55"
        : "text-ink/72"
  );
}

function formatPreviewTime(timestamp: number): string {
  const ms = timestamp > 10_000_000_000 ? timestamp : timestamp * 1000;
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(ms));
}

function parseMaybeJsonObject(value: unknown): Record<string, unknown> | null {
  let parsed = value;
  if (typeof parsed === "string") {
    try {
      parsed = JSON.parse(parsed) as unknown;
    } catch {
      return null;
    }
  }
  return asRecord(parsed);
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function stringField(value: Record<string, unknown>, key: string): string | null {
  const field = value[key];
  return typeof field === "string" && field.trim() ? field.trim() : null;
}

function numberField(value: Record<string, unknown> | null, key: string): number | null {
  const field = value?.[key];
  return typeof field === "number" && Number.isFinite(field) ? field : null;
}

function sumEditField(edits: Record<string, unknown>[], key: string): number {
  return edits.reduce((sum, edit) => sum + (numberField(edit, key) ?? 0), 0);
}

function stablePreviewText(value: unknown, maxLength: number): string {
  let text: string;
  try {
    text = typeof value === "string" ? value : JSON.stringify(value, null, 2) ?? "";
  } catch {
    text = String(value);
  }
  return text.length > maxLength ? `${text.slice(0, maxLength - 3)}...` : text;
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

function isAstraActive(status: AstraRunStatus): boolean {
  return (
    status === "planning"
    || status === "thinking"
    || status === "awaiting_approval"
    || status === "dispatching"
    || status === "running"
  );
}

function upsertRun(runs: AstraHandle[], run: AstraHandle): AstraHandle[] {
  const next = runs.some((item) => item.runId === run.runId)
    ? runs.map((item) => item.runId === run.runId ? run : item)
    : [run, ...runs];
  return next.slice().sort((a, b) => b.updatedAt - a.updatedAt);
}

function astraStatusClass(status: AstraRunStatus): string {
  switch (status) {
    case "awaiting_approval":
      return "bg-sky-500/[0.10] text-sky-500";
    case "thinking":
      return "bg-violet-500/[0.10] text-violet-500";
    case "dispatching":
    case "running":
      return "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]";
    case "errored":
      return "bg-red-500/[0.10] text-red-500";
    case "cancelled":
    case "interrupted":
      return "bg-ink/[0.08] text-ink/45";
    case "completed":
      return "bg-[rgb(var(--color-emerald)/0.12)] text-[rgb(var(--color-emerald)/0.95)]";
    case "planning":
    default:
      return "bg-amber-500/[0.10] text-amber-500";
  }
}

function astraTaskStatusClass(status: PlanTaskStatus): string {
  switch (status) {
    case "running":
      return "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]";
    case "completed":
      return "bg-[rgb(var(--color-emerald)/0.12)] text-[rgb(var(--color-emerald)/0.95)]";
    case "failed":
    case "errored":
      return "bg-red-500/[0.10] text-red-500";
    case "cancelled":
      return "bg-ink/[0.08] text-ink/45";
    case "planned":
    default:
      return "bg-ink/[0.06] text-ink/45";
  }
}

function planRoundStatusClass(status: PlanRoundInfo["status"]): string {
  switch (status) {
    case "running":
      return "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]";
    case "completed":
      return "bg-[rgb(var(--color-emerald)/0.12)] text-[rgb(var(--color-emerald)/0.95)]";
    case "errored":
      return "bg-red-500/[0.10] text-red-500";
    case "cancelled":
      return "bg-ink/[0.08] text-ink/45";
    case "planned":
    default:
      return "bg-amber-500/[0.10] text-amber-500";
  }
}

function formatPlanStatus(status: string): string {
  return status.replace(/_/g, " ");
}

function threadAssistantCount(thread: ThreadInfo): number {
  if (thread.kind !== "workflow") return thread.assistants.length;
  return new Set(thread.stages.flatMap((stage) => stage.assistants.map((assistant) => assistant.assistantId))).size;
}

function formatDate(timestamp: number | null | undefined, lang: "en" | "zh"): string | null {
  if (!timestamp) return null;
  return new Intl.DateTimeFormat(localeTag(lang), {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(timestamp * 1000));
}
