import type {
  Agent,
  ThreadInfo,
  ThreadKind,
  ThreadReplayInfo,
  ThreadReplaySessionInfo,
  ThreadReplaySessionSourceInfo,
} from "./api";
import { AGENT_LABEL } from "./api";
import type { PendingNewChatSession } from "./navigation";
import type { LiveRuntimeState } from "./runtimeChat";

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
      sources: [],
      groupKey: "pending",
      groupLabel: t("thread.pending_lane"),
      status: liveSession ? liveSessionStatus(liveSession) : "pending",
      liveSession,
    });
  }

  return lanes.sort(compareLanes);
}

export function groupReplaySessionsByThreadKind(
  replay: ThreadReplayInfo,
  t: TFn,
): ReplaySessionGroup[] {
  const groups = new Map<string, ReplaySessionGroup>();
  for (const session of replay.sessions) {
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
  if (kind === "workflow") {
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
  const hasFailure = liveSession.turns.some((turn) => turn.error);
  if (hasFailure) return "failed";
  return "live";
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
