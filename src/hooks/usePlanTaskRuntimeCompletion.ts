import { useEffect, useRef } from "react";
import { updatePlanTaskStatus } from "../api";
import type { PendingNewChatSession } from "../navigation";
import type { LiveRuntimeSession, LiveRuntimeState } from "../runtimeChat";

export function usePlanTaskRuntimeCompletion({
  pendingNewChats,
  liveSessions,
  setError,
}: {
  pendingNewChats: Record<string, PendingNewChatSession>;
  liveSessions: LiveRuntimeState["sessions"];
  setError: (error: string | null) => void;
}) {
  const completedRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    for (const pending of Object.values(pendingNewChats)) {
      const taskId = pending.planTaskLink?.taskId;
      if (!taskId || completedRef.current.has(taskId)) continue;
      if (!pending.planTaskLink?.runtimeStarted) continue;
      const liveSession = liveSessions[pending.sessioRuntimeSessionId];
      if (!liveSession?.ended) continue;
      completedRef.current.add(taskId);
      const patch = terminalPatchForLiveSession(liveSession);
      updatePlanTaskStatus(taskId, patch)
        .catch((err) => {
          completedRef.current.delete(taskId);
          setError(String(err));
        });
    }
  }, [liveSessions, pendingNewChats, setError]);
}

function terminalPatchForLiveSession(
  session: LiveRuntimeSession,
): Parameters<typeof updatePlanTaskStatus>[1] {
  const failedTurn = session.turns.find((turn) => turn.error || turn.status === "failed");
  if (failedTurn) {
    return {
      status: "failed",
      error: failedTurn.error?.message ?? "Runtime turn failed",
    };
  }
  if (session.turns.some((turn) => turn.status === "cancelled")) {
    return { status: "cancelled" };
  }
  return {
    status: "completed",
    resultSummary: latestAssistantText(session) ?? null,
  };
}

function latestAssistantText(session: LiveRuntimeSession): string | null {
  for (let turnIndex = session.turns.length - 1; turnIndex >= 0; turnIndex -= 1) {
    const turn = session.turns[turnIndex];
    for (let blockIndex = turn.blocks.length - 1; blockIndex >= 0; blockIndex -= 1) {
      const block = turn.blocks[blockIndex];
      if (block.kind !== "assistant") continue;
      const text = block.blocks
        .map((content) => content.type === "text" ? content.text : "")
        .join("\n")
        .trim();
      if (text) return text.slice(0, 2000);
    }
  }
  return null;
}
