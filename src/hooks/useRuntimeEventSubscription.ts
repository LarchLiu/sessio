import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import type { SessionInfo } from "../api";
import type { LiveRuntimeAction } from "../runtimeChat";
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
    listen<unknown>("agent-runtime-event", (event) => {
      if (cancelled) return;
      const payload = normalizeAgentRuntimeEvent(event.payload);
      console.info("[sessio-runtime:frontend:event]", payload);
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
      dispatchLiveRuntimeEvent({ type: "runtime-event", event: payload });
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((err) => setError(String(err)));
    listen<unknown>("agent-runtime-turn-snapshot", (event) => {
      if (cancelled) return;
      const payload = normalizeRuntimeTurnSnapshot(event.payload);
      console.info("[sessio-runtime:frontend:turn-snapshot]", payload);
      dispatchLiveRuntimeEvent({ type: "runtime-turn-snapshot", event: payload });
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlistenSnapshot = fn;
      })
      .catch((err) => setError(String(err)));
    return () => {
      cancelled = true;
      unlisten?.();
      unlistenSnapshot?.();
    };
  }, [
    dispatchLiveRuntimeEvent,
    setError,
    setUnreadSessionIds,
  ]);
}
