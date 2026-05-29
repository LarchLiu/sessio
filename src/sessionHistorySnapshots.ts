import type { Agent, SessionHistorySnapshotGroup } from "./api";

const snapshotCache = new Map<string, SessionHistorySnapshotGroup[]>();

function snapshotKey(agent: Agent, sessionId: string): string {
  return `${agent}:${sessionId}`;
}

export function getCachedSessionHistorySnapshots(
  agent: Agent,
  sessionId: string,
): SessionHistorySnapshotGroup[] | null {
  return snapshotCache.get(snapshotKey(agent, sessionId)) ?? null;
}

export function setCachedSessionHistorySnapshots(
  agent: Agent,
  sessionId: string,
  groups: SessionHistorySnapshotGroup[],
): void {
  snapshotCache.set(snapshotKey(agent, sessionId), groups);
}
