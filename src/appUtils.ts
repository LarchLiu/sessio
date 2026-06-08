import type {
  Agent,
  AgentRuntimeEvent,
  ProjectInfo,
  SessionInfo,
  SessionScope,
} from "./api";

export type Filter =
  | SessionScope
  | { kind: "project"; key: string; label: string };

export type ProjectSelection = { kind: "project"; projectId: string } | null;

export function scopeForFilter(filter: Filter): SessionScope {
  if (filter.kind === "project") return { kind: "project", key: filter.key };
  return filter;
}

export function projectKey(s: SessionInfo): string {
  return s.projectPath ?? `__unknown__:${s.agent}`;
}

export function projectFilterKey(project: ProjectInfo): string {
  return project.path;
}

export function matchesScope(scope: SessionScope, session: SessionInfo): boolean {
  if (scope.kind === "all") return true;
  if (scope.kind === "agent") return session.agent === scope.agent;
  return projectKey(session) === scope.key;
}

export function sessionKey(s: SessionInfo): string {
  return `${s.agent}:${s.filePath}:${s.id}`;
}

export function sessionIdentityKey(s: SessionInfo): string {
  return `${s.agent}:${s.id}`;
}

export function sessionIdentity(agent: Agent, sessionId: string): string {
  return `${agent}:${sessionId}`;
}

export function sessionDisplayTitle(session: SessionInfo): string | null {
  return session.renameTitle ?? session.title ?? session.firstUserMessage ?? null;
}

export function isRealSessionFilePath(filePath: string | null | undefined): boolean {
  const trimmed = filePath?.trim() ?? "";
  return trimmed !== "" && !trimmed.startsWith("astra://");
}

export function isPersistedSession(
  session: Pick<SessionInfo, "available" | "filePath"> | null | undefined,
): boolean {
  return Boolean(session?.available && isRealSessionFilePath(session.filePath));
}

export function sessionUnreadKeys(
  session: SessionInfo,
  runtimeSessionAliases: Record<string, string>,
): string[] {
  const keys = new Set<string>([session.id, sessionIdentityKey(session)]);
  const runtimeSessionId = runtimeSessionAliases[sessionIdentityKey(session)];
  if (runtimeSessionId) keys.add(runtimeSessionId);
  return Array.from(keys);
}

export function runtimeEventUnreadKeys(
  event: AgentRuntimeEvent,
  runtimeSessionAliases: Record<string, string>,
): string[] {
  const keys = new Set<string>([event.sessioRuntimeSessionId]);
  if (event.kind === "sessionStarted") {
    keys.add(event.agentRuntimeSessionId);
    keys.add(sessionIdentity(event.agent, event.agentRuntimeSessionId));
    return Array.from(keys);
  }
  for (const [identity, runtimeSessionId] of Object.entries(runtimeSessionAliases)) {
    if (runtimeSessionId !== event.sessioRuntimeSessionId) continue;
    keys.add(identity);
    const sessionId = identity.slice(identity.indexOf(":") + 1);
    if (sessionId) keys.add(sessionId);
    break;
  }
  return Array.from(keys);
}

type RuntimeSessionAliasSource = {
  agent: Agent;
  agentRuntimeSessionId: string;
  sessioRuntimeSessionId: string;
};

export function mergeRuntimeSessionAliases<T extends RuntimeSessionAliasSource>(
  aliases: Record<string, string>,
  liveSessions: Record<string, T>,
): Record<string, string> {
  let next = aliases;
  for (const liveSession of Object.values(liveSessions)) {
    const agentSessionId = liveSession.agentRuntimeSessionId.trim();
    const liveRuntimeSessionId = liveSession.sessioRuntimeSessionId.trim();
    if (!agentSessionId || !liveRuntimeSessionId) continue;
    if (!isPersistableAgentSessionId(agentSessionId)) continue;

    const identity = sessionIdentity(liveSession.agent, agentSessionId);
    if (next[identity] === liveRuntimeSessionId) continue;
    if (next === aliases) next = { ...aliases };
    next[identity] = liveRuntimeSessionId;
  }
  return next;
}

function isPersistableAgentSessionId(sessionId: string): boolean {
  return sessionId !== "pending" && !sessionId.startsWith("fake-agent-session");
}

export function intersectsSet(keys: Iterable<string>, lookup: Set<string>): boolean {
  for (const key of keys) {
    if (lookup.has(key)) return true;
  }
  return false;
}

export function addUnreadKeys(prev: Set<string>, keys: Iterable<string>): Set<string> {
  let next = prev;
  for (const key of keys) {
    if (next.has(key)) continue;
    if (next === prev) next = new Set(prev);
    next.add(key);
  }
  return next;
}

export function deleteUnreadKeys(prev: Set<string>, keys: Iterable<string>): Set<string> {
  let next = prev;
  for (const key of keys) {
    if (!next.has(key)) continue;
    if (next === prev) next = new Set(prev);
    next.delete(key);
  }
  return next;
}

export function ancestorSessionsFor(session: SessionInfo, sessions: SessionInfo[]): SessionInfo[] {
  const byIdentity = new Map<string, SessionInfo>();
  for (const item of sessions) {
    const key = sessionIdentityKey(item);
    const current = byIdentity.get(key);
    if (!current || betterSessionCandidate(item, current)) {
      byIdentity.set(key, item);
    }
  }
  const chain: SessionInfo[] = [];
  const seen = new Set<string>([sessionIdentityKey(session)]);
  let cursor: SessionInfo | undefined = session;

  for (let depth = 0; depth < 32; depth += 1) {
    const parentId = cursor?.forkedFromId;
    const parentAgent = cursor?.forkedFromAgent ?? (parentId ? cursor?.agent : null);
    if (!parentId || !parentAgent) break;

    const key = sessionIdentity(parentAgent, parentId);
    if (seen.has(key)) break;
    seen.add(key);

    const parent = byIdentity.get(key);
    if (!parent) break;
    chain.push(parent);
    cursor = parent;
  }

  return chain.reverse();
}

export function betterSessionCandidate(candidate: SessionInfo, current: SessionInfo): boolean {
  if (candidate.available !== current.available) return candidate.available;
  if (candidate.partial !== current.partial) return !candidate.partial;
  const candidateRealPath = isRealSessionFilePath(candidate.filePath);
  const currentRealPath = isRealSessionFilePath(current.filePath);
  if (candidateRealPath !== currentRealPath) return candidateRealPath;
  if (Boolean(candidate.filePath) !== Boolean(current.filePath)) return Boolean(candidate.filePath);
  return (candidate.updatedAt ?? candidate.startedAt ?? 0) > (current.updatedAt ?? current.startedAt ?? 0);
}

export function mergePendingSession(sessions: SessionInfo[], pending: SessionInfo): SessionInfo[] {
  const index = sessions.findIndex((session) => sessionIdentityKey(session) === sessionIdentityKey(pending));
  if (index < 0) return [pending, ...sessions];

  const existing = sessions[index];
  const merged: SessionInfo = {
    ...pending,
    ...existing,
    forkedFromAgent: existing.forkedFromAgent ?? pending.forkedFromAgent ?? null,
    forkedFromId: existing.forkedFromId ?? pending.forkedFromId ?? null,
    renameTitle: existing.renameTitle ?? pending.renameTitle ?? null,
    filePath: existing.filePath || pending.filePath,
    fileSize: existing.filePath ? existing.fileSize : pending.fileSize,
    partial: existing.filePath ? existing.partial : pending.partial,
    messageCount: Math.max(existing.messageCount, pending.messageCount),
    subagents: existing.subagents.length > 0 ? existing.subagents : pending.subagents,
  };
  const next = sessions.slice();
  next[index] = merged;
  return next;
}

export function messageCountKey(agent: Agent, filePath: string, sessionId: string): string {
  return `${agent}:${sessionId}:${filePath}`;
}

// Orphan main session that only exists to carry subagents (Claude cleaned
// the main jsonl, no index entry either). Don't count it as a "real" session
// but still show it in the list so subagents stay reachable.
export function isSubagentOnly(s: SessionInfo): boolean {
  return s.archived && s.messageCount === 0 && s.subagents.length > 0;
}
