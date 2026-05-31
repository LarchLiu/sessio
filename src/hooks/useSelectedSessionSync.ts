import { useEffect } from "react";
import type { Dispatch, SetStateAction } from "react";
import type { Agent, ProjectInfo, SessionInfo } from "../api";
import type { DetailMode } from "../navigation";
import {
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
  setSelectedThread: Dispatch<SetStateAction<{ projectId: string; threadId: string } | null>>;
  setExpandedProjects: Dispatch<SetStateAction<Set<string>>>;
  setPendingSelectSession: Dispatch<SetStateAction<{
    agent: Agent;
    sessionId: string;
  } | null>>;
}) {
  useEffect(() => {
    if (!selected) return;
    const next =
      availableSessions.find((s) => sessionKey(s) === sessionKey(selected)) ??
      availableSessions.find((s) => sessionIdentityKey(s) === sessionIdentityKey(selected));
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
    const next = availableSessions.find(
      (session) =>
        session.agent === pendingSelectSession.agent &&
        session.id === pendingSelectSession.sessionId,
    );
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
