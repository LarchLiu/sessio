import { useEffect, useMemo, useState } from "react";
import { getSessionAncestors, type SessionInfo } from "../api";
import { ancestorSessionsFor, sessionIdentityKey } from "../appUtils";

export function useSessionAncestors(
  selected: SessionInfo | null,
  sessions: SessionInfo[],
  selectedIdentityKey: string | null,
): SessionInfo[] {
  const fallbackAncestorSessions = useMemo(
    () => (selected ? ancestorSessionsFor(selected, sessions) : []),
    [sessions, selected],
  );
  const [dbAncestorSessions, setDbAncestorSessions] = useState<SessionInfo[]>([]);
  const [dbAncestorSourceKey, setDbAncestorSourceKey] = useState<string | null>(null);

  useEffect(() => {
    if (!selected) {
      setDbAncestorSessions([]);
      setDbAncestorSourceKey(null);
      return;
    }
    const sourceKey = sessionIdentityKey(selected);
    let cancelled = false;
    getSessionAncestors(selected.agent, selected.id)
      .then((ancestors) => {
        if (cancelled) return;
        setDbAncestorSessions(ancestors);
        setDbAncestorSourceKey(sourceKey);
      })
      .catch((err) => {
        if (cancelled) return;
        console.warn("load session ancestors failed", err);
        setDbAncestorSessions([]);
        setDbAncestorSourceKey(null);
      });
    return () => {
      cancelled = true;
    };
  }, [selected]);

  return selected && dbAncestorSourceKey === selectedIdentityKey
    ? dbAncestorSessions
    : fallbackAncestorSessions;
}
