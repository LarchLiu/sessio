import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import type { SessionInfo } from "../api";
import type { LiveRuntimeAction, LiveRuntimeTurnSnapshotEvent } from "../runtimeChat";
import { normalizeAgentRuntimeEvent, normalizeRuntimeTurnSnapshot } from "../runtimeChat";
import {
  addUnreadKeys,
  intersectsSet,
  runtimeEventUnreadKeys,
  sessionUnreadKeys,
} from "../appUtils";

export function useRuntimeEventSubscription({
  selected,
  runtimeSessionAliases,
  dispatchLiveRuntimeEvent,
  setUnreadSessionIds,
  setError,
}: {
  selected: SessionInfo | null;
  runtimeSessionAliases: Record<string, string>;
  dispatchLiveRuntimeEvent: React.Dispatch<LiveRuntimeAction>;
  setUnreadSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>;
  setError: (error: string | null) => void;
}) {
  const runtimeSessionAliasesRef = useRef<Record<string, string>>({});
  const selectedUnreadKeysRef = useRef<Set<string>>(new Set());
  const pendingSnapshotsRef = useRef<Map<string, LiveRuntimeTurnSnapshotEvent>>(new Map());
  const snapshotFlushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    runtimeSessionAliasesRef.current = runtimeSessionAliases;
    selectedUnreadKeysRef.current = new Set(
      selected ? sessionUnreadKeys(selected, runtimeSessionAliases) : [],
    );
  }, [runtimeSessionAliases, selected]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let unlistenSnapshot: (() => void) | null = null;
    const flushSnapshots = () => {
      if (snapshotFlushTimerRef.current) {
        clearTimeout(snapshotFlushTimerRef.current);
        snapshotFlushTimerRef.current = null;
      }
      if (pendingSnapshotsRef.current.size === 0) return;
      const snapshots = Array.from(pendingSnapshotsRef.current.values())
        .sort((a, b) => a.sequence - b.sequence);
      pendingSnapshotsRef.current.clear();
      for (const snapshot of snapshots) {
        dispatchLiveRuntimeEvent({ type: "runtime-turn-snapshot", event: snapshot });
      }
    };
    const queueSnapshot = (snapshot: LiveRuntimeTurnSnapshotEvent) => {
      pendingSnapshotsRef.current.set(snapshot.session.sessioRuntimeSessionId, snapshot);
      if (shouldFlushSnapshotImmediately(snapshot)) {
        flushSnapshots();
        return;
      }
      if (snapshotFlushTimerRef.current) return;
      snapshotFlushTimerRef.current = setTimeout(flushSnapshots, 160);
    };
    listen<unknown>("agent-runtime-event", (event) => {
      if (cancelled) return;
      const payload = normalizeAgentRuntimeEvent(event.payload);
      const unreadKeys = runtimeEventUnreadKeys(
        payload,
        runtimeSessionAliasesRef.current,
      );
      if (
        !intersectsSet(unreadKeys, selectedUnreadKeysRef.current) &&
        payload.kind !== "sessionEnded"
      ) {
        setUnreadSessionIds((prev) => {
          return addUnreadKeys(prev, unreadKeys);
        });
      }
      if (shouldDispatchRuntimeEvent(payload.kind)) {
        dispatchLiveRuntimeEvent({ type: "runtime-event", event: payload });
      }
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((err) => setError(String(err)));
    listen<unknown>("agent-runtime-turn-snapshot", (event) => {
      if (cancelled) return;
      const payload = normalizeRuntimeTurnSnapshot(event.payload);
      queueSnapshot(payload);
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlistenSnapshot = fn;
      })
      .catch((err) => setError(String(err)));
    return () => {
      cancelled = true;
      if (snapshotFlushTimerRef.current) {
        clearTimeout(snapshotFlushTimerRef.current);
        snapshotFlushTimerRef.current = null;
      }
      pendingSnapshotsRef.current.clear();
      unlisten?.();
      unlistenSnapshot?.();
    };
  }, [
    dispatchLiveRuntimeEvent,
    setError,
    setUnreadSessionIds,
  ]);
}

function shouldDispatchRuntimeEvent(kind: string): boolean {
  return kind === "sessionStarted" || kind === "sessionEnded";
}

function shouldFlushSnapshotImmediately(snapshot: LiveRuntimeTurnSnapshotEvent): boolean {
  return snapshot.session.turns.some((turn) =>
    turn.status === "completed" || turn.status === "failed" || turn.status === "cancelled"
  );
}
