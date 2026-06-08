import type { Agent, SessionInfo, ThreadInfo, ThreadReplayInfo } from "./api";
import {
  betterSessionCandidate,
  sessionIdentity,
  sessionIdentityKey,
} from "./appUtils";

export function collectThreadChatSessions(
  thread: ThreadInfo,
  replay: ThreadReplayInfo | null,
): SessionInfo[] {
  const byKey = new Map<string, SessionInfo>();
  const addSession = (session: SessionInfo | null | undefined) => {
    if (!session) return;
    const key = sessionIdentityKey(session);
    const current = byKey.get(key);
    if (!current || betterSessionCandidate(session, current)) byKey.set(key, session);
  };

  for (const session of thread.sessions) addSession(session);
  for (const stage of thread.stages) {
    for (const session of stage.sessions) addSession(session);
  }
  for (const replaySession of replay?.sessions ?? []) addSession(replaySession.session);

  return Array.from(byKey.values()).sort(compareSessionTime);
}

export function threadChatSessionIdentityKeys(
  thread: ThreadInfo,
  replay: ThreadReplayInfo | null,
): Set<string> {
  const keys = new Set<string>();
  for (const session of collectThreadChatSessions(thread, replay)) {
    keys.add(sessionIdentityKey(session));
  }
  for (const replaySession of replay?.sessions ?? []) {
    keys.add(threadReplaySessionIdentityKey(replaySession.agent, replaySession.sessionId));
  }
  return keys;
}

export function threadChatEntryTime(thread: ThreadInfo, replay: ThreadReplayInfo | null): number {
  let latest = Math.max(thread.updatedAt ?? 0, thread.createdAt ?? 0);
  for (const session of collectThreadChatSessions(thread, replay)) {
    latest = Math.max(latest, sessionTime(session));
  }
  for (const stage of thread.stages) {
    latest = Math.max(latest, stage.updatedAt ?? 0, stage.createdAt ?? 0);
  }
  for (const session of replay?.sessions ?? []) {
    latest = Math.max(latest, replaySessionTime(session));
  }
  return latest;
}

export function compareSessionTime(a: SessionInfo, b: SessionInfo): number {
  return sessionTime(b) - sessionTime(a);
}

export function sessionTime(session: SessionInfo): number {
  return session.updatedAt ?? session.startedAt ?? 0;
}

function replaySessionTime(session: ThreadReplayInfo["sessions"][number]): number {
  return Math.max(
    session.lastSeenAt ?? 0,
    session.firstSeenAt ?? 0,
    session.session ? sessionTime(session.session) : 0,
    ...session.sources.map((source) => source.createdAt ?? 0),
  );
}

function threadReplaySessionIdentityKey(agent: Agent, sessionId: string): string {
  return sessionIdentity(agent, sessionId);
}
