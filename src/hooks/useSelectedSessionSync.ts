import { useEffect } from "react";
import type { Dispatch, SetStateAction } from "react";
import type { Agent, ProjectInfo, SessionInfo } from "../api";
import type { DetailMode } from "../navigation";
import {
  betterSessionCandidate,
  projectFilterKey,
  sessionIdentityKey,
  sessionKey,
  type Filter,
  type ProjectSelection,
} from "../appUtils";

export function useSelectedSessionSync({
  availableSessions,
  selected,
  pendingSelectSession,
  projects,
  setSelected,
  setDetailMode,
  setFilter,
  setSelectedProject,
  setSelectedThread,
  setExpandedProjects,
  setPendingSelectSession,
}: {
  availableSessions: SessionInfo[];
  selected: SessionInfo | null;
  pendingSelectSession: { agent: Agent; sessionId: string } | null;
  projects: ProjectInfo[];
  setSelected: Dispatch<SetStateAction<SessionInfo | null>>;
  setDetailMode: Dispatch<SetStateAction<DetailMode>>;
  setFilter: Dispatch<SetStateAction<Filter>>;
  setSelectedProject: Dispatch<SetStateAction<ProjectSelection>>;
  setSelectedThread: Dispatch<SetStateAction<{ projectId: string; threadId: string; goal: string } | null>>;
  setExpandedProjects: Dispatch<SetStateAction<Set<string>>>;
  setPendingSelectSession: Dispatch<SetStateAction<{
    agent: Agent;
    sessionId: string;
  } | null>>;
}) {
  useEffect(() => {
    if (!selected) return;
    const exact = availableSessions.find((s) => sessionKey(s) === sessionKey(selected));
    const sameIdentity = availableSessions.filter(
      (s) => sessionIdentityKey(s) === sessionIdentityKey(selected),
    );
    const next = sameIdentity.reduce<SessionInfo | null>((best, session) => {
      if (!best) return session;
      return betterSessionCandidate(session, best) ? session : best;
    }, exact ?? null);
    if (!next) {
      setSelected(null);
      return;
    }
    if (next !== selected) {
      setSelected(next);
    }
  }, [availableSessions, selected, setSelected]);

  useEffect(() => {
    if (!pendingSelectSession) return;
    const matches = availableSessions.filter(
      (session) =>
        session.agent === pendingSelectSession.agent &&
        session.id === pendingSelectSession.sessionId,
    );
    const next = matches.reduce<SessionInfo | null>((best, session) => {
      if (!best) return session;
      return betterSessionCandidate(session, best) ? session : best;
    }, null);
    if (!next) return;
    setSelected(next);
    setSelectedThread(null);
    setDetailMode("chat");
    const project = projects.find((item) => item.path === next.projectPath);
    if (project) {
      setSelectedProject(null);
      setFilter({ kind: "project", key: projectFilterKey(project), label: project.name });
    }
    setExpandedProjects((prev) => {
      const expanded = new Set(prev);
      if (project) expanded.add(project.id);
      return expanded;
    });
    setPendingSelectSession(null);
  }, [
    availableSessions,
    pendingSelectSession,
    projects,
    setDetailMode,
    setExpandedProjects,
    setFilter,
    setPendingSelectSession,
    setSelected,
    setSelectedProject,
    setSelectedThread,
  ]);
}
