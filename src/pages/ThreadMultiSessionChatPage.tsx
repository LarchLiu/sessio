import { useCallback, useEffect, useMemo, useState } from "react";
import HashIcon from "@iconify-react/mynaui/hash";
import {
  AlertCircle,
  ArrowLeft,
  Clock,
  ExternalLink,
  LoaderCircle,
  MessagesSquare,
  RefreshCw,
} from "lucide-react";
import type {
  Agent,
  ProjectInfo,
  SessionInfo,
  ThreadInfo,
  ThreadKind,
  ThreadReplayInfo,
  ThreadReplaySessionInfo,
  ThreadReplaySessionSourceInfo,
  ThreadWorkState,
} from "../api";
import { AGENT_LABEL, getThreadReplay, getThreadWorkState } from "../api";
import { AgentGlyph } from "../components/AgentIcon";
import { LiveSessionStatusBadge } from "../components/AcpTranscriptPanel";
import ScrollArea from "../components/ScrollArea";
import { localeTag, useI18n } from "../i18n";
import type { PendingNewChatSession } from "../navigation";
import type { LiveRuntimeState } from "../runtimeChat";
import { sessionDisplayTitle } from "../appUtils";
import { projectStageLabel, stageStatusVisual } from "../utils/stageDisplay";

type ThreadSessionLaneStatus =
  | "history"
  | "live"
  | "pending"
  | "missing"
  | "failed";

interface ThreadSessionLane {
  laneId: string;
  agent: Agent;
  sessionId: string;
  sessioRuntimeSessionId: string | null;
  session: SessionInfo | null;
  sources: ThreadReplaySessionSourceInfo[];
  groupKey: string;
  groupLabel: string;
  status: ThreadSessionLaneStatus;
  liveSession: LiveRuntimeState["sessions"][string] | null;
}

export default function ThreadMultiSessionChatPage({
  project,
  threadId,
  liveState,
  runtimeSessionAliases,
  pendingNewChats,
  onBackToOverview,
  onSelectSession,
  onError,
}: {
  project: ProjectInfo;
  threadId: string;
  liveState: LiveRuntimeState;
  runtimeSessionAliases: Record<string, string>;
  pendingNewChats: Record<string, PendingNewChatSession>;
  onBackToOverview: () => void;
  onSelectSession: (session: SessionInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t, lang } = useI18n();
  const [thread, setThread] = useState<ThreadWorkState | null>(null);
  const [replay, setReplay] = useState<ThreadReplayInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

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
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded border border-ink/12 bg-surface-panel text-ink/45 transition hover:bg-ink/[0.05] hover:text-ink/78"
            >
              <ExternalLink className="h-4 w-4" />
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
        <div className="flex min-h-0 flex-1 items-center justify-center rounded-md border border-dashed border-card-border/[0.12] bg-card-panel px-3 py-8 text-center">
          <div className="max-w-[320px]">
            <Clock className="mx-auto h-5 w-5 text-ink/28" />
            <div className="mt-2 text-body-sm font-medium text-ink/55">
              {lane.status === "live"
                ? t("thread.live_lane_ready")
                : lane.status === "pending"
                  ? t("thread.pending_lane_ready")
                  : lane.status === "missing"
                    ? t("thread.missing_lane_ready")
                    : t("thread.history_lane_ready")}
            </div>
            <div className="mt-1 text-caption leading-relaxed text-ink/35">
              {t("thread.transcript_renderer_pending")}
            </div>
          </div>
        </div>
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

function buildThreadSessionLanes({
  thread,
  replay,
  liveState,
  runtimeSessionAliases,
  pendingNewChats,
  t,
}: {
  thread: ThreadInfo | null;
  replay: ThreadReplayInfo | null;
  liveState: LiveRuntimeState;
  runtimeSessionAliases: Record<string, string>;
  pendingNewChats: Record<string, PendingNewChatSession>;
  t: (key: string, vars?: Record<string, string | number>) => string;
}): ThreadSessionLane[] {
  const lanes: ThreadSessionLane[] = [];
  for (const replaySession of replay?.sessions ?? []) {
    const group = replayGroupForSession(replay?.kind ?? thread?.kind ?? "teamwork", replaySession, t);
    const aliasKey = `${replaySession.agent}:${replaySession.sessionId}`;
    const runtimeSessionId = runtimeSessionAliases[aliasKey] ?? null;
    const liveSession = runtimeSessionId ? liveState.sessions[runtimeSessionId] : null;
    lanes.push({
      laneId: `${replaySession.agent}:${replaySession.sessionId}:${group.key}`,
      agent: replaySession.agent,
      sessionId: replaySession.sessionId,
      sessioRuntimeSessionId: runtimeSessionId,
      session: replaySession.session,
      sources: replaySession.sources,
      groupKey: group.key,
      groupLabel: group.label,
      status: liveSession
        ? liveSessionStatus(liveSession)
        : replaySession.session
          ? "history"
          : "missing",
      liveSession,
    });
  }

  const existingRuntimeIds = new Set(lanes.flatMap((lane) => lane.sessioRuntimeSessionId ? [lane.sessioRuntimeSessionId] : []));
  for (const pending of Object.values(pendingNewChats)) {
    if (pending.threadLink?.threadId !== thread?.id && pending.workSnapshot?.threadId !== thread?.id) {
      continue;
    }
    if (existingRuntimeIds.has(pending.sessioRuntimeSessionId)) continue;
    const liveSession = liveState.sessions[pending.sessioRuntimeSessionId] ?? null;
    const agentSessionId =
      liveSession?.agentRuntimeSessionId && liveSession.agentRuntimeSessionId !== "pending"
        ? liveSession.agentRuntimeSessionId
        : pending.sessioRuntimeSessionId;
    lanes.push({
      laneId: `${pending.agent}:${agentSessionId}:pending:${pending.sessioRuntimeSessionId}`,
      agent: pending.agent,
      sessionId: agentSessionId,
      sessioRuntimeSessionId: pending.sessioRuntimeSessionId,
      session: null,
      sources: [],
      groupKey: "pending",
      groupLabel: t("thread.pending_lane"),
      status: liveSession ? liveSessionStatus(liveSession) : "pending",
      liveSession,
    });
  }

  return lanes.sort(compareLanes);
}

function liveSessionStatus(liveSession: LiveRuntimeState["sessions"][string]): ThreadSessionLaneStatus {
  if (liveSession.ended) return "history";
  const hasFailure = liveSession.turns.some((turn) => turn.error);
  if (hasFailure) return "failed";
  return "live";
}

function replayGroupForSession(
  kind: ThreadKind,
  session: ThreadReplaySessionInfo,
  t: (key: string, vars?: Record<string, string | number>) => string,
): { key: string; label: string } {
  if (kind === "workflow") {
    const stageSource = session.sources.find((source) => source.kind === "stage" || source.stageId);
    if (stageSource) {
      const label = stageSource.label ?? stageSource.stageId ?? t("thread.replay_source.stage");
      return { key: `stage:${stageSource.stageId ?? label}`, label };
    }
  }
  if (kind === "debate") {
    const roundSource = session.sources.find((source) => source.planRoundId || source.kind === "plan_task");
    if (roundSource) {
      const round = roundSource.planRoundId ? shortSessionId(roundSource.planRoundId) : t("thread.replay_source.plan_task");
      const lane = debateLaneLabel(roundSource)
        ?? (roundSource.planTaskId ? shortSessionId(roundSource.planTaskId) : roundSource.label)
        ?? AGENT_LABEL[session.agent];
      return {
        key: `debate:${roundSource.planRoundId ?? "round"}:${roundSource.planTaskId ?? lane}`,
        label: t("thread.replay_group.round_lane", { round, lane }),
      };
    }
  }
  if (kind === "teamwork" || kind === "brainstorm") {
    const roundSource = session.sources.find((source) => source.planRoundId || source.kind === "plan_task");
    if (roundSource) {
      const value = roundSource.planRoundId
        ? shortSessionId(roundSource.planRoundId)
        : roundSource.label ?? roundSource.planTaskId ?? t("thread.replay_source.plan_task");
      return {
        key: `round:${roundSource.planRoundId ?? roundSource.planTaskId ?? value}`,
        label: t("thread.replay_group.round", { value }),
      };
    }
  }
  const source = session.sources[0] ?? null;
  if (source?.kind === "thread") return { key: `thread:${session.agent}`, label: t("thread.replay_group.thread") };
  if (source?.kind === "astra_internal") {
    return {
      key: `astra:${source.astraRunId ?? session.agent}`,
      label: source.label ?? t("thread.replay_source.astra_internal"),
    };
  }
  return { key: `agent:${session.agent}`, label: AGENT_LABEL[session.agent] };
}

function debateLaneLabel(source: ThreadReplaySessionSourceInfo): string | null {
  const label = source.label?.trim();
  if (!label) return null;
  return label.replace(/\s+debate\s+(lane|cross-check)$/i, "").trim() || label;
}

function compareLanes(a: ThreadSessionLane, b: ThreadSessionLane): number {
  const liveRank = laneStatusRank(a.status) - laneStatusRank(b.status);
  if (liveRank !== 0) return liveRank;
  const timeA = a.session?.updatedAt ?? a.session?.startedAt ?? latestSourceTime(a.sources);
  const timeB = b.session?.updatedAt ?? b.session?.startedAt ?? latestSourceTime(b.sources);
  return timeB - timeA || a.groupLabel.localeCompare(b.groupLabel) || a.sessionId.localeCompare(b.sessionId);
}

function laneStatusRank(status: ThreadSessionLaneStatus): number {
  switch (status) {
    case "live":
      return 0;
    case "pending":
      return 1;
    case "history":
      return 2;
    case "missing":
      return 3;
    case "failed":
      return 4;
  }
}

function latestSourceTime(sources: ThreadReplaySessionSourceInfo[]): number {
  return sources.reduce((latest, source) => Math.max(latest, source.createdAt ?? 0), 0);
}

function threadAssistantCount(thread: ThreadInfo): number {
  if (thread.kind !== "workflow") return thread.assistants.length;
  return new Set(thread.stages.flatMap((stage) => stage.assistants.map((assistant) => assistant.assistantId))).size;
}

function replaySourceKey(source: ThreadReplaySessionSourceInfo): string {
  return [
    source.kind,
    source.stageId,
    source.planRoundId,
    source.planTaskId,
    source.astraRunId,
    source.role,
    source.createdAt,
  ].filter(Boolean).join(":");
}

function replaySourceTitle(source: ThreadReplaySessionSourceInfo): string {
  return [
    source.label,
    source.kind,
    source.role,
    source.planTaskId,
    source.planRoundId,
    source.stageId,
    source.astraRunId,
  ].filter(Boolean).join("\n");
}

function shortSessionId(value: string): string {
  return value.length <= 12 ? value : `${value.slice(0, 6)}...${value.slice(-4)}`;
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
