import { useCallback, useEffect, useRef, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import type { Agent, SessionInfo } from "../api";
import type { ActiveMessageMeta } from "../pages/ChatPage";
import {
  addUnreadKeys,
  deleteUnreadKeys,
  intersectsSet,
  messageCountKey,
  sessionIdentityKey,
  sessionUnreadKeys,
} from "../appUtils";

export function useUnreadSessions({
  sessions,
  selected,
  runtimeSessionAliases,
  setSessions,
  setSelected,
  setActiveMessageMeta,
}: {
  sessions: SessionInfo[];
  selected: SessionInfo | null;
  runtimeSessionAliases: Record<string, string>;
  setSessions: Dispatch<SetStateAction<SessionInfo[]>>;
  setSelected: Dispatch<SetStateAction<SessionInfo | null>>;
  setActiveMessageMeta: Dispatch<SetStateAction<ActiveMessageMeta | null>>;
}) {
  const [unreadSessionIds, setUnreadSessionIds] = useState<Set<string>>(
    () => new Set(),
  );
  const messageCountBySourceRef = useRef<Map<string, number>>(new Map());
  const runtimeSessionAliasesRef = useRef<Record<string, string>>({});
  const selectedUnreadKeysRef = useRef<Set<string>>(new Set());
  const sessionsLoadedRef = useRef(false);

  useEffect(() => {
    runtimeSessionAliasesRef.current = runtimeSessionAliases;
    selectedUnreadKeysRef.current = new Set(
      selected ? sessionUnreadKeys(selected, runtimeSessionAliases) : [],
    );
  }, [runtimeSessionAliases, selected]);

  useEffect(() => {
    const previous = messageCountBySourceRef.current;
    const next = new Map<string, number>();
    const changedSessions = new Map<string, SessionInfo>();
    for (const session of sessions) {
      const mainKey = messageCountKey(session.agent, session.filePath, session.id);
      next.set(mainKey, session.messageCount);
      const previousMainCount = previous.get(mainKey);
      if (
        sessionsLoadedRef.current &&
        previousMainCount !== undefined &&
        session.messageCount > previousMainCount
      ) {
        changedSessions.set(sessionIdentityKey(session), session);
      }
      for (const subagent of session.subagents) {
        const subKey = messageCountKey(session.agent, subagent.filePath, session.id);
        next.set(subKey, subagent.messageCount);
        const previousSubCount = previous.get(subKey);
        if (
          sessionsLoadedRef.current &&
          previousSubCount !== undefined &&
          subagent.messageCount > previousSubCount
        ) {
          changedSessions.set(sessionIdentityKey(session), session);
        }
      }
    }
    messageCountBySourceRef.current = next;
    sessionsLoadedRef.current = true;
    if (changedSessions.size > 0) {
      setUnreadSessionIds((prev) => {
        let nextUnread = prev;
        for (const session of changedSessions.values()) {
          const keys = sessionUnreadKeys(session, runtimeSessionAliasesRef.current);
          if (intersectsSet(keys, selectedUnreadKeysRef.current)) continue;
          nextUnread = addUnreadKeys(nextUnread, keys);
        }
        return nextUnread;
      });
    }
  }, [sessions]);

  useEffect(() => {
    const selectedKeys = selected
      ? sessionUnreadKeys(selected, runtimeSessionAliases)
      : [];
    if (!selected) return;
    setUnreadSessionIds((prev) => {
      return deleteUnreadKeys(prev, selectedKeys);
    });
  }, [runtimeSessionAliases, selected]);

  const handleMessageCount = useCallback((
    agent: Agent,
    filePath: string,
    sessionId: string,
    count: number,
  ) => {
    const countKey = messageCountKey(agent, filePath, sessionId);
    if (messageCountBySourceRef.current.get(countKey) === count) return false;
    messageCountBySourceRef.current.set(countKey, count);

    const patchSession = (session: SessionInfo): SessionInfo => {
      if (
        session.agent === agent &&
        session.id === sessionId &&
        session.filePath === filePath
      ) {
        return { ...session, messageCount: count };
      }
      let changed = false;
      const subagents = session.subagents.map((sub) => {
        if (
          session.agent !== agent ||
          session.id !== sessionId ||
          sub.filePath !== filePath
        ) {
          return sub;
        }
        changed = true;
        return { ...sub, messageCount: count };
      });
      return changed ? { ...session, subagents } : session;
    };

    setSessions((prev) => prev.map(patchSession));
    setSelected((prev) => (prev ? patchSession(prev) : prev));
    setActiveMessageMeta((prev) =>
      prev && prev.filePath === filePath ? { ...prev, count } : prev,
    );
    return true;
  }, [setActiveMessageMeta, setSelected, setSessions]);

  return {
    unreadSessionIds,
    setUnreadSessionIds,
    handleMessageCount,
  };
}
