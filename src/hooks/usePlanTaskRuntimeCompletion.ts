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
      const completion = planTaskRuntimeCompletionForPending(pending, liveSessions);
      if (!completion || completedRef.current.has(completion.taskId)) continue;
      completedRef.current.add(completion.taskId);
      updatePlanTaskStatus(completion.taskId, completion.patch)
        .catch((err) => {
          completedRef.current.delete(completion.taskId);
          setError(String(err));
        });
    }
  }, [liveSessions, pendingNewChats, setError]);
}

export function planTaskRuntimeCompletionForPending(
  pending: PendingNewChatSession,
  liveSessions: LiveRuntimeState["sessions"],
): {
  taskId: string;
  patch: Parameters<typeof updatePlanTaskStatus>[1];
} | null {
  const link = pending.planTaskLink;
  if (!link?.taskId || !link.runtimeStarted) return null;
  const liveSession = liveSessions[pending.sessioRuntimeSessionId];
  if (!liveSession?.ended) return null;
  return {
    taskId: link.taskId,
    patch: terminalPatchForLiveSession(liveSession),
  };
}

export function terminalPatchForLiveSession(
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
