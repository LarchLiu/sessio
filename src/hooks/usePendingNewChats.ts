import { useEffect, useRef } from "react";
import {
  createPendingSession,
  linkKanbanItemSession,
  linkPlanTaskSession,
  linkStageSession,
  linkThreadSession,
  saveSessionHistorySnapshots,
  saveThreadWorkSnapshot,
  updatePlanTaskStatus,
  updateKanbanItemStatus,
  type Agent,
  type KanbanItem,
  type SessionInfo,
} from "../api";
import { mergePendingSession } from "../appUtils";
import type { DetailMode, PendingNewChatSession } from "../navigation";
import type { LiveRuntimeState } from "../runtimeChat";
import { setCachedSessionHistorySnapshots } from "../sessionHistorySnapshots";

export function usePendingNewChats({
  pendingNewChats,
  liveSessions,
  setRuntimeSessionAliases,
  setSessions,
  setSelectedProject,
  setSelectedThread,
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
  setSelectedThread: React.Dispatch<React.SetStateAction<{ projectId: string; threadId: string; goal: string } | null>>;
  setSelected: React.Dispatch<React.SetStateAction<SessionInfo | null>>;
  setDetailMode: React.Dispatch<React.SetStateAction<DetailMode>>;
  setPendingSelectSession: React.Dispatch<React.SetStateAction<{
    agent: Agent;
    sessionId: string;
    detailMode?: DetailMode;
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
        renameTitle: null,
        title: pending.prompt,
        firstUserMessage: pending.prompt,
        filePath: "",
        fileSize: 0,
        partial: true,
        available: true,
        archived: false,
        subagents: [],
      };
      if (pending.historySnapshots && pending.historySnapshots.length > 0) {
        setCachedSessionHistorySnapshots(
          pending.agent,
          agentSessionId,
          pending.historySnapshots,
        );
      }
      setRuntimeSessionAliases((prev) => ({
        ...prev,
        [`${pending.agent}:${agentSessionId}`]: pending.sessioRuntimeSessionId,
      }));
      setSessions((prev) => mergePendingSession(prev, pendingSession));
      const autoSelect = shouldAutoSelectPendingSession(pending);
      if (autoSelect) {
        const detailMode = pending.origin === "thread_chat" ? "threadChat" : "chat";
        setSelected(pendingSession);
        setSelectedThread(null);
        setDetailMode(detailMode);
        setPendingSelectSession({ agent: pending.agent, sessionId: agentSessionId, detailMode });
      }
      if (autoSelect) {
        setPendingNewChats((prev) => {
          const next = { ...prev };
          delete next[pending.sessioRuntimeSessionId];
          return next;
        });
      }

      createPendingSession(pendingSession)
        .then(async () => {
          if (pending.historySnapshots && pending.historySnapshots.length > 0) {
            saveSessionHistorySnapshots(
              pending.agent,
              agentSessionId,
              pending.historySnapshots,
            ).catch((err) => console.warn("save history snapshots failed", err));
          }
          if (pending.workSnapshot) {
            saveThreadWorkSnapshot(
              pending.agent,
              agentSessionId,
              pending.workSnapshot.threadId,
              pending.workSnapshot.stageId,
              pending.workSnapshot.snapshot,
            ).catch((err) => console.warn("save work snapshot failed", err));
          }
          if (pending.threadLink) {
            if (pending.threadLink.stageId) {
              await linkStageSession(pending.threadLink.stageId, pending.agent, agentSessionId);
            } else {
              await linkThreadSession(pending.threadLink.threadId, pending.agent, agentSessionId);
            }
          }
          if (pending.planTaskLink) {
            await linkPlanTaskSession({
              taskId: pending.planTaskLink.taskId,
              agent: pending.agent,
              sessionId: agentSessionId,
              role: pending.planTaskLink.role,
            });
            await updatePlanTaskStatus(pending.planTaskLink.taskId, { status: "running" });
            setPendingNewChats((prev) => {
              const current = prev[pending.sessioRuntimeSessionId];
              if (!current?.planTaskLink) return prev;
              if (current.planTaskLink.runtimeStarted) return prev;
              return {
                ...prev,
                [pending.sessioRuntimeSessionId]: {
                  ...current,
                  planTaskLink: {
                    ...current.planTaskLink,
                    runtimeStarted: true,
                  },
                },
              };
            });
          }
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
    setSelectedThread,
    setSessions,
  ]);
}

export function shouldAutoSelectPendingSession(pending: PendingNewChatSession): boolean {
  return !pending.suppressAutoSelect;
}
