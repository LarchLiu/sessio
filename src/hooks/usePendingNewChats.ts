import { useEffect, useRef } from "react";
import {
  createPendingSession,
  linkKanbanItemSession,
  updateKanbanItemStatus,
  type Agent,
  type KanbanItem,
  type SessionInfo,
} from "../api";
import { mergePendingSession } from "../appUtils";
import type { DetailMode, PendingNewChatSession } from "../navigation";
import type { LiveRuntimeState } from "../runtimeChat";

export function usePendingNewChats({
  pendingNewChats,
  liveSessions,
  setRuntimeSessionAliases,
  setSessions,
  setSelectedProject,
  setSelected,
  setDetailMode,
  setPendingSelectSession,
  setPendingNewChats,
  setError,
}: {
  pendingNewChats: Record<string, PendingNewChatSession>;
  liveSessions: LiveRuntimeState["sessions"];
  setRuntimeSessionAliases: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  setSessions: React.Dispatch<React.SetStateAction<SessionInfo[]>>;
  setSelectedProject: React.Dispatch<React.SetStateAction<{ kind: "project"; projectId: string } | null>>;
  setSelected: React.Dispatch<React.SetStateAction<SessionInfo | null>>;
  setDetailMode: React.Dispatch<React.SetStateAction<DetailMode>>;
  setPendingSelectSession: React.Dispatch<React.SetStateAction<{
    agent: Agent;
    sessionId: string;
  } | null>>;
  setPendingNewChats: React.Dispatch<React.SetStateAction<Record<string, PendingNewChatSession>>>;
  setError: (error: string | null) => void;
}) {
  const writesRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    for (const pending of Object.values(pendingNewChats)) {
      const liveSession = liveSessions[pending.sessioRuntimeSessionId];
      if (!liveSession) continue;
      const agentSessionId = liveSession.agentRuntimeSessionId;
      if (
        !agentSessionId ||
        agentSessionId === "pending" ||
        agentSessionId.startsWith("fake-agent-session")
      ) {
        continue;
      }
      if (writesRef.current.has(pending.sessioRuntimeSessionId)) continue;

      writesRef.current.add(pending.sessioRuntimeSessionId);
      // Flow: persist pending session, link kanban if needed, then select the chat view.
      const pendingSession: SessionInfo = {
        id: agentSessionId,
        agent: pending.agent,
        forkedFromAgent: pending.forkedFromAgent ?? null,
        forkedFromId: pending.forkedFromId ?? null,
        projectPath: pending.projectPath,
        projectName: pending.projectName,
        startedAt: pending.timestamp,
        updatedAt: pending.timestamp,
        messageCount: 0,
        title: pending.prompt,
        firstUserMessage: pending.prompt,
        filePath: "",
        fileSize: 0,
        partial: true,
        available: true,
        archived: false,
        subagents: [],
      };
      createPendingSession(pendingSession)
        .then(async () => {
          let linkedKanbanItem: KanbanItem | null = null;
          if (pending.kanbanItemId) {
            linkedKanbanItem = await linkKanbanItemSession(
              pending.kanbanItemId,
              pending.agent,
              agentSessionId,
            );
            if (pending.kanbanItemStatus === "todo") {
              linkedKanbanItem = await updateKanbanItemStatus(
                pending.kanbanItemId,
                "in_progress",
              );
            }
          }
          setRuntimeSessionAliases((prev) => ({
            ...prev,
            [`${pending.agent}:${agentSessionId}`]: pending.sessioRuntimeSessionId,
          }));
          setSessions((prev) => mergePendingSession(prev, pendingSession));
          if (linkedKanbanItem) {
            setSelectedProject((current) => (current ? { ...current } : current));
          }
          setSelected(pendingSession);
          setDetailMode("chat");
          setPendingSelectSession({ agent: pending.agent, sessionId: agentSessionId });
          setPendingNewChats((prev) => {
            const next = { ...prev };
            delete next[pending.sessioRuntimeSessionId];
            return next;
          });
        })
        .catch((err) => {
          writesRef.current.delete(pending.sessioRuntimeSessionId);
          setError(String(err));
        });
    }
  }, [
    liveSessions,
    pendingNewChats,
    setDetailMode,
    setError,
    setPendingNewChats,
    setPendingSelectSession,
    setRuntimeSessionAliases,
    setSelected,
    setSelectedProject,
    setSessions,
  ]);
}
