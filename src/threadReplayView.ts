import type {
  Agent,
  AstraHandle,
  PlanRoundInfo,
  PlanTaskInfo,
  ThreadInfo,
  ThreadKind,
  ThreadReplayInfo,
  ThreadReplaySessionInfo,
  ThreadReplaySessionSourceInfo,
} from "./api";
import { AGENT_LABEL } from "./api";
import type { PendingNewChatSession } from "./navigation";
import type { LiveRuntimeState } from "./runtimeChat";
import { isThreadPlannerLiveSession, stringMeta } from "./runtimeChat";
import { isAstraActive } from "./threadAstraView";

type TFn = (key: string, vars?: Record<string, string | number>) => string;

export type ThreadSessionLaneStatus =
  | "history"
  | "live"
  | "pending"
  | "missing"
  | "failed";

export interface ThreadSessionLane {
  laneId: string;
  agent: Agent;
  sessionId: string;
  sessioRuntimeSessionId: string | null;
  session: ThreadReplaySessionInfo["session"];
  sources: ThreadReplaySessionSourceInfo[];
  groupKey: string;
  groupLabel: string;
  status: ThreadSessionLaneStatus;
  liveSession: LiveRuntimeState["sessions"][string] | null;
}

export type ReplaySessionGroup = {
  key: string;
  label: string;
  agent: Agent | null;
  sessions: ThreadReplaySessionInfo[];
};

export type ThreadTimelineRow = {
  key: string;
  kind: "orchestration" | "sessions";
  time: number;
  order: number;
  round: PlanRoundInfo | null;
  run: AstraHandle | null;
  lanes: ThreadSessionLane[];
  debatePair: boolean;
};

type ThreadTimelineGroup = {
  key: string;
  time: number;
  order: number;
  rows: ThreadTimelineRow[];
};

export function buildThreadSessionLanes({
  thread,
  replay,
  liveState,
  runtimeSessionAliases,
  pendingNewChats,
  t,
}: {
  thread: Pick<ThreadInfo, "id" | "kind"> | null;
  replay: ThreadReplayInfo | null;
  liveState: LiveRuntimeState;
  runtimeSessionAliases: Record<string, string>;
  pendingNewChats: Record<string, PendingNewChatSession>;
  t: TFn;
}): ThreadSessionLane[] {
  const lanes: ThreadSessionLane[] = [];
  for (const replaySession of replay?.sessions ?? []) {
    const group = replayGroupForSession(replay?.kind ?? thread?.kind ?? "teamwork", replaySession, t);
    const aliasKey = `${replaySession.agent}:${replaySession.sessionId}`;
    const runtimeSessionId = runtimeSessionAliases[aliasKey]
      ?? (liveState.sessions[replaySession.sessionId] ? replaySession.sessionId : null);
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

  const existingRuntimeIds = new Set(
    lanes.flatMap((lane) =>
      lane.sessioRuntimeSessionId ? [lane.sessioRuntimeSessionId] : [],
    ),
  );
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
      sources: pendingReplaySources(pending),
      groupKey: "pending",
      groupLabel: t("thread.pending_lane"),
      status: liveSession ? liveSessionStatus(liveSession) : "pending",
      liveSession,
    });
    existingRuntimeIds.add(pending.sessioRuntimeSessionId);
  }

  for (const liveSession of Object.values(liveState.sessions)) {
    if (!isThreadPlannerLiveSession(liveSession, thread?.id)) continue;
    if (existingRuntimeIds.has(liveSession.sessioRuntimeSessionId)) continue;
    const sessionId =
      liveSession.agentRuntimeSessionId && liveSession.agentRuntimeSessionId !== "pending"
        ? liveSession.agentRuntimeSessionId
        : liveSession.sessioRuntimeSessionId;
    const astraRunId = stringMeta(liveSession.metadata, "astraRunId");
    lanes.push({
      laneId: `${liveSession.agent}:${sessionId}:planner-live:${liveSession.sessioRuntimeSessionId}`,
      agent: liveSession.agent,
      sessionId,
      sessioRuntimeSessionId: liveSession.sessioRuntimeSessionId,
      session: null,
      sources: [{
        kind: "astra_internal",
        threadId: thread?.id ?? null,
        stageId: null,
        planRoundId: null,
        planTaskId: null,
        astraRunId,
        role: "planner",
        label: "Astra planner",
        stageSnapshotJson: null,
        assistantSnapshotJson: null,
        agentSnapshotJson: null,
        createdAt: liveSessionTimelineTime(liveSession) || null,
      }],
      groupKey: `astra:${astraRunId ?? sessionId}`,
      groupLabel: "Astra planner",
      status: liveSessionStatus(liveSession),
      liveSession,
    });
    existingRuntimeIds.add(liveSession.sessioRuntimeSessionId);
  }

  return lanes.sort(compareLanes);
}

function isDeterministicOrchestratorReplaySession(session: ThreadReplaySessionInfo): boolean {
  return session.sessionId.startsWith("deterministic-orchestrator-")
    && session.sources.some((source) => source.kind === "astra_internal");
}

function pendingReplaySources(pending: PendingNewChatSession): ThreadReplaySessionSourceInfo[] {
  const threadId = pending.threadLink?.threadId ?? pending.workSnapshot?.threadId ?? null;
  if (!threadId) return [];
  const stageId = pending.threadLink?.stageId ?? pending.workSnapshot?.stageId ?? null;
  if (pending.planTaskLink) {
    return [{
      kind: "plan_task",
      threadId,
      stageId,
      planRoundId: null,
      planTaskId: pending.planTaskLink.taskId,
      astraRunId: null,
      role: pending.planTaskLink.role,
      label: null,
      stageSnapshotJson: null,
      assistantSnapshotJson: null,
      agentSnapshotJson: null,
      createdAt: pending.timestamp,
    }];
  }
  return [{
    kind: stageId ? "stage" : "thread",
    threadId,
    stageId,
    planRoundId: null,
    planTaskId: null,
    astraRunId: null,
    role: null,
    label: pending.prompt,
    stageSnapshotJson: null,
    assistantSnapshotJson: null,
    agentSnapshotJson: null,
    createdAt: pending.timestamp,
  }];
}

export function buildThreadTimelineRows(
  lanes: ThreadSessionLane[],
  planRounds: PlanRoundInfo[],
  astraRuns: AstraHandle[],
  threadKind: ThreadKind,
): ThreadTimelineRow[] {
  const groups: ThreadTimelineGroup[] = [];
  const consumed = new Set<string>();
  const runById = new Map(astraRuns.map((run) => [run.runId, run]));
  const taskById = new Map<string, PlanTaskInfo>();
  for (const round of planRounds) {
    for (const task of round.tasks) taskById.set(task.id, task);
  }

  const plannerQueues = new Map<string, ThreadSessionLane[]>();
  const staticPlannerQueues = new Map<string, ThreadSessionLane[]>();
  for (const lane of lanes) {
    const source = plannerSourceForLane(lane);
    if (!source) continue;
    const runKey = source.astraRunId ?? `lane:${lane.laneId}`;
    if (!plannerLaneHasTranscript(lane)) {
      const queue = staticPlannerQueues.get(runKey) ?? [];
      queue.push(lane);
      staticPlannerQueues.set(runKey, queue);
      continue;
    }
    const queue = plannerQueues.get(runKey) ?? [];
    queue.push(lane);
    plannerQueues.set(runKey, queue);
  }
  for (const queue of [...plannerQueues.values(), ...staticPlannerQueues.values()]) {
    queue.sort(compareTimelineLanesAsc);
  }

  const sortedRounds = planRounds.slice().sort(comparePlanRoundsAsc);
  for (const round of sortedRounds) {
    const run = round.astraRunId ? runById.get(round.astraRunId) ?? null : null;
    const groupRows: ThreadTimelineRow[] = [];
    const plannerLanes = round.astraRunId
      ? (plannerQueues.get(round.astraRunId)?.splice(0, 1) ?? [])
      : [];
    for (const lane of plannerLanes) consumed.add(lane.laneId);
    const staticPlannerLane = round.astraRunId
      ? staticPlannerQueues.get(round.astraRunId)?.shift() ?? null
      : null;
    if (staticPlannerLane) consumed.add(staticPlannerLane.laneId);
    groupRows.push({
      key: `orchestration:${round.id}`,
      kind: "orchestration",
      time: round.createdAt,
      order: 0,
      round,
      run,
      lanes: plannerLanes,
      debatePair: false,
    });

    const taskLanes = lanes
      .filter((lane) => {
        if (consumed.has(lane.laneId)) return false;
        const source = planTaskSourceForLane(lane);
        if (!source) return false;
        return source.planRoundId === round.id
          || (source.planTaskId ? round.tasks.some((task) => task.id === source.planTaskId) : false);
      })
      .sort((a, b) => compareTaskLanes(a, b, taskById));

    for (const lane of taskLanes) consumed.add(lane.laneId);
    if (threadKind === "debate") {
      chunkLanes(taskLanes, 2).forEach((pair, index) => {
        groupRows.push({
          key: `debate-tasks:${round.id}:${index}`,
          kind: "sessions",
          time: firstLaneTime(pair) || round.updatedAt,
          order: 1 + index,
          round,
          run,
          lanes: pair,
          debatePair: true,
        });
      });
    } else {
      taskLanes.forEach((lane, index) => {
        groupRows.push({
          key: `task:${round.id}:${lane.laneId}`,
          kind: "sessions",
          time: laneTimelineTime(lane) || round.updatedAt,
          order: 1 + index,
          round,
          run,
          lanes: [lane],
          debatePair: false,
        });
      });
    }

    groups.push({
      key: `round:${round.id}`,
      time: round.createdAt,
      order: 1000 + round.roundIndex,
      rows: groupRows,
    });
  }

  for (const [runKey, queue] of staticPlannerQueues.entries()) {
    for (const lane of queue) {
      if (consumed.has(lane.laneId)) continue;
      const run = runById.get(runKey) ?? null;
      consumed.add(lane.laneId);
      const time = laneTimelineTime(lane) || run?.updatedAt || run?.createdAt || 0;
      groups.push({
        key: `orchestration-static:${lane.laneId}`,
        time,
        order: 500,
        rows: [{
          key: `orchestration-static:${lane.laneId}`,
          kind: "orchestration",
          time,
          order: 0,
          round: null,
          run,
          lanes: [],
          debatePair: false,
        }],
      });
    }
  }

  for (const [runKey, queue] of plannerQueues.entries()) {
    for (const lane of queue) {
      if (consumed.has(lane.laneId)) continue;
      const run = runById.get(runKey) ?? null;
      consumed.add(lane.laneId);
      const time = laneTimelineTime(lane) || run?.updatedAt || run?.createdAt || 0;
      groups.push({
        key: `orchestration-live:${lane.laneId}`,
        time,
        order: 500,
        rows: [{
          key: `orchestration-live:${lane.laneId}`,
          kind: "orchestration",
          time,
          order: 0,
          round: null,
          run,
          lanes: [lane],
          debatePair: false,
        }],
      });
    }
  }

  for (const run of astraRuns) {
    if (planRounds.some((round) => round.astraRunId === run.runId)) continue;
    const livePlannerAlreadyShown = groups.some((group) =>
      group.rows.some((row) => row.run?.runId === run.runId && row.kind === "orchestration"),
    );
    if (livePlannerAlreadyShown || !isAstraActive(run.status)) continue;
    groups.push({
      key: `orchestration-run:${run.runId}`,
      time: run.createdAt,
      order: 400,
      rows: [{
        key: `orchestration-run:${run.runId}`,
        kind: "orchestration",
        time: run.createdAt,
        order: 0,
        round: null,
        run,
        lanes: [],
        debatePair: false,
      }],
    });
  }

  lanes
    .filter((lane) => !consumed.has(lane.laneId))
    .sort(compareTimelineLanesAsc)
    .forEach((lane, index) => {
      const time = laneTimelineTime(lane);
      groups.push({
        key: `session:${lane.laneId}`,
        time,
        order: 10000 + index,
        rows: [{
          key: `session:${lane.laneId}`,
          kind: "sessions",
          time,
          order: 50 + index,
          round: null,
          run: null,
          lanes: [lane],
          debatePair: false,
        }],
      });
    });

  return groups
    .sort(compareTimelineGroups)
    .flatMap((group) => group.rows);
}

export function groupReplaySessionsByThreadKind(
  replay: ThreadReplayInfo,
  t: TFn,
): ReplaySessionGroup[] {
  const groups = new Map<string, ReplaySessionGroup>();
  for (const session of replay.sessions) {
    if (isDeterministicOrchestratorReplaySession(session)) continue;
    const seed = replayGroupForSession(replay.kind, session, t);
    const group = groups.get(seed.key) ?? { ...seed, sessions: [] };
    group.sessions.push(session);
    groups.set(seed.key, group);
  }
  return Array.from(groups.values())
    .map((group) => ({
      ...group,
      sessions: group.sessions.slice().sort(compareReplaySessionTime),
    }))
    .sort(compareReplayGroups);
}

export function compareReplaySessionTime(
  a: ThreadReplaySessionInfo,
  b: ThreadReplaySessionInfo,
): number {
  return replaySessionTime(b) - replaySessionTime(a);
}

export function replayGroupForSession(
  kind: ThreadKind,
  session: ThreadReplaySessionInfo,
  t: TFn,
): Omit<ReplaySessionGroup, "sessions"> {
  if (kind === "process") {
    const stageSource = session.sources.find((source) => source.kind === "stage" || source.stageId);
    if (stageSource) {
      const label = stageSource.label ?? stageSource.stageId ?? t("thread.replay_source.stage");
      return {
        key: `stage:${stageSource.stageId ?? label}`,
        label,
        agent: null,
      };
    }
  }
  if (kind === "debate") {
    const roundSource = session.sources.find((source) => source.planRoundId || source.kind === "plan_task");
    if (roundSource) {
      const round = roundSource.planRoundId
        ? shortSessionId(roundSource.planRoundId)
        : t("thread.replay_source.plan_task");
      const lane = debateLaneLabel(roundSource)
        ?? (roundSource.planTaskId ? shortSessionId(roundSource.planTaskId) : roundSource.label)
        ?? AGENT_LABEL[session.agent];
      return {
        key: `debate:${roundSource.planRoundId ?? "round"}:${roundSource.planTaskId ?? lane}`,
        label: t("thread.replay_group.round_lane", { round, lane }),
        agent: session.agent,
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
        agent: null,
      };
    }
  }
  return fallbackReplayGroupKey(session, t);
}

export function replaySourceKey(source: ThreadReplaySessionSourceInfo): string {
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

export function replaySourceTitle(source: ThreadReplaySessionSourceInfo): string {
  return [
    source.label,
    source.kind,
    source.role,
    source.planTaskId,
    source.planRoundId,
    source.stageId,
    source.astraRunId,
    ...replaySourceSnapshotTitles(source),
  ].filter(Boolean).join("\n");
}

export function shortSessionId(sessionId: string): string {
  const trimmed = sessionId.trim();
  if (trimmed.length <= 18) return trimmed;
  return `${trimmed.slice(0, 8)}...${trimmed.slice(-6)}`;
}

function liveSessionStatus(liveSession: LiveRuntimeState["sessions"][string]): ThreadSessionLaneStatus {
  if (liveSession.ended) return "history";
  const hasActiveTurn = liveSession.turns.some((turn) =>
    turn.status === "pending" ||
    turn.status === "streaming" ||
    turn.status === "cancelling"
  );
  const hasFailure = liveSession.turns.some((turn) =>
    turn.error ||
    turn.status === "failed" ||
    turn.status === "cancelled"
  );
  if (hasFailure) return "failed";
  if (!hasActiveTurn && liveSession.turns.length > 0) return "history";
  return "live";
}

function liveSessionTimelineTime(liveSession: LiveRuntimeState["sessions"][string]): number {
  return liveSession.turns.reduce((latest, turn) => {
    return Math.max(latest, turn.updatedAt ?? 0, turn.startedAt ?? 0);
  }, 0);
}

function comparePlanRoundsAsc(a: PlanRoundInfo, b: PlanRoundInfo): number {
  return a.roundIndex - b.roundIndex || a.createdAt - b.createdAt || a.id.localeCompare(b.id);
}

function plannerSourceForLane(lane: ThreadSessionLane) {
  return lane.sources.find((source) => source.kind === "astra_internal") ?? null;
}

function plannerLaneHasTranscript(lane: ThreadSessionLane): boolean {
  return Boolean(lane.liveSession || lane.session);
}

function planTaskSourceForLane(lane: ThreadSessionLane) {
  return lane.sources.find((source) => source.kind === "plan_task") ?? null;
}

function compareTaskLanes(
  a: ThreadSessionLane,
  b: ThreadSessionLane,
  taskById: Map<string, PlanTaskInfo>,
): number {
  const sourceA = planTaskSourceForLane(a);
  const sourceB = planTaskSourceForLane(b);
  const taskA = sourceA?.planTaskId ? taskById.get(sourceA.planTaskId) : null;
  const taskB = sourceB?.planTaskId ? taskById.get(sourceB.planTaskId) : null;
  return (taskA?.sortOrder ?? 0) - (taskB?.sortOrder ?? 0)
    || (sourceA?.createdAt ?? laneTimelineTime(a)) - (sourceB?.createdAt ?? laneTimelineTime(b))
    || compareTimelineLanesAsc(a, b);
}

function chunkLanes(lanes: ThreadSessionLane[], size: number): ThreadSessionLane[][] {
  const out: ThreadSessionLane[][] = [];
  for (let index = 0; index < lanes.length; index += size) {
    out.push(lanes.slice(index, index + size));
  }
  return out;
}

function firstLaneTime(lanes: ThreadSessionLane[]): number {
  return lanes.reduce((time, lane) => {
    const laneTime = laneTimelineTime(lane);
    if (!laneTime) return time;
    return time === 0 ? laneTime : Math.min(time, laneTime);
  }, 0);
}

function laneTimelineTime(lane: ThreadSessionLane): number {
  const sourceTimes = lane.sources
    .map((source) => source.createdAt ?? 0)
    .filter((time) => time > 0);
  if (sourceTimes.length > 0) return Math.min(...sourceTimes);
  const liveTurnTimes = (lane.liveSession?.turns ?? [])
    .flatMap((turn) => [turn.startedAt, turn.updatedAt])
    .filter((time) => time > 0);
  if (liveTurnTimes.length > 0) return Math.min(...liveTurnTimes);
  return lane.session?.startedAt ?? lane.session?.updatedAt ?? 0;
}

function compareTimelineLanesAsc(a: ThreadSessionLane, b: ThreadSessionLane): number {
  return timelineSortTime(laneTimelineTime(a)) - timelineSortTime(laneTimelineTime(b))
    || timelineSortTime(laneActivityTime(a)) - timelineSortTime(laneActivityTime(b))
    || a.groupLabel.localeCompare(b.groupLabel)
    || a.sessionId.localeCompare(b.sessionId)
    || a.laneId.localeCompare(b.laneId);
}

function laneActivityTime(lane: ThreadSessionLane): number {
  const liveTurnTimes = (lane.liveSession?.turns ?? [])
    .flatMap((turn) => [turn.startedAt, turn.updatedAt])
    .filter((time) => time > 0);
  if (liveTurnTimes.length > 0) return Math.max(...liveTurnTimes);
  return lane.session?.updatedAt ?? lane.session?.startedAt ?? latestSourceTime(lane.sources);
}

function compareTimelineGroups(a: ThreadTimelineGroup, b: ThreadTimelineGroup): number {
  return timelineSortTime(a.time) - timelineSortTime(b.time)
    || a.order - b.order
    || a.key.localeCompare(b.key);
}

function timelineSortTime(time: number): number {
  return time > 0 ? time : Number.MAX_SAFE_INTEGER;
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

function replaySessionTime(session: ThreadReplaySessionInfo): number {
  return session.lastSeenAt ?? session.firstSeenAt ?? session.session?.updatedAt ?? session.session?.startedAt ?? 0;
}

function fallbackReplayGroupKey(
  session: ThreadReplaySessionInfo,
  t: TFn,
): Omit<ReplaySessionGroup, "sessions"> {
  const source = session.sources[0] ?? null;
  if (source?.kind === "thread") {
    return {
      key: `thread:${session.agent}`,
      label: t("thread.replay_group.thread"),
      agent: session.agent,
    };
  }
  if (source?.kind === "astra_internal") {
    return {
      key: `astra:${source.astraRunId ?? session.agent}`,
      label: source.label ?? t("thread.replay_source.astra_internal"),
      agent: null,
    };
  }
  return {
    key: `agent:${session.agent}`,
    label: AGENT_LABEL[session.agent],
    agent: session.agent,
  };
}

function compareReplayGroups(a: ReplaySessionGroup, b: ReplaySessionGroup): number {
  const latestA = latestReplayGroupTime(a);
  const latestB = latestReplayGroupTime(b);
  return latestB - latestA || a.label.localeCompare(b.label);
}

function latestReplayGroupTime(group: ReplaySessionGroup): number {
  return group.sessions.reduce((latest, session) => Math.max(latest, replaySessionTime(session)), 0);
}

function debateLaneLabel(source: ThreadReplaySessionSourceInfo): string | null {
  const label = source.label?.trim();
  if (!label) return null;
  return label
    .replace(/\s+debate\s+(lane|cross-check)$/i, "")
    .trim() || label;
}

function replaySourceSnapshotTitles(source: ThreadReplaySessionSourceInfo): string[] {
  const titles: string[] = [];
  const stage = parseJsonObject(source.stageSnapshotJson);
  const stageName = stringField(stage, "name") ?? stringField(stage, "stageId") ?? stringField(stage, "id");
  if (stageName) titles.push(`Stage snapshot: ${stageName}`);

  const assistant = parseJsonObject(source.assistantSnapshotJson);
  const assistantName = stringField(assistant, "name") ?? stringField(assistant, "assistantId") ?? stringField(assistant, "id");
  if (assistantName) {
    const agentInfo = objectField(assistant, "agent");
    const model = stringField(agentInfo, "model");
    titles.push(`Assistant snapshot: ${model ? `${assistantName} / ${model}` : assistantName}`);
  }

  const agent = parseJsonObject(source.agentSnapshotJson);
  const participant = objectField(agent, "participant");
  const participantLabel = participantSnapshotLabel(participant);
  if (participantLabel) {
    titles.push(`Participant snapshot: ${participantLabel}`);
  }
  const agentInfo = objectField(agent, "agentInfo");
  const agentLabel = stringField(agentInfo, "displayName")
    ?? stringField(agentInfo, "name")
    ?? stringField(agent, "agent");
  if (agentLabel) {
    const model = stringField(agentInfo, "model");
    titles.push(`Agent snapshot: ${model ? `${agentLabel} / ${model}` : agentLabel}`);
  }
  return titles;
}

function participantSnapshotLabel(participant: Record<string, unknown> | null): string | null {
  if (!participant) return null;
  const agent = stringField(participant, "agent");
  const agentLabel = agent && agent in AGENT_LABEL ? AGENT_LABEL[agent as Agent] : agent;
  const model = stringField(participant, "model");
  const effort = stringField(participant, "effort");
  const permissionMode = stringField(participant, "permissionMode");
  return [agentLabel, model, effort, permissionMode]
    .filter((item): item is string => Boolean(item))
    .join(" / ") || null;
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
  return typeof field === "string" && field.trim() ? field : null;
}
