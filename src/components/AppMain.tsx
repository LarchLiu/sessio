import type { Dispatch, SetStateAction } from "react";
import type { Agent, ProjectInfo, RuntimeAgentMetadata, RuntimeAgentSelection, SetRuntimeAgentSelectionRequest, SessionInfo } from "../api";
import type { ActiveMessageMeta } from "../pages/ChatPage";
import ChatPage from "../pages/ChatPage";
import NewChatPage from "../pages/NewChatPage";
import { ProjectWorkbenchPage } from "../pages/ProjectPage";
import { projectFilterKey, type Filter } from "../appUtils";
import type { PendingNewChatSession, ViewMode, ProjectGroup } from "../navigation";
import type {
  LiveRuntimeAction,
  LiveRuntimeState,
} from "../runtimeChat";

export default function AppMain({
  error,
  activeProject,
  selected,
  selectedSessionProject,
  detailRoute,
  viewMode,
  liveState,
  runtimeAgents,
  lastRuntimeAgentSelection,
  rememberRuntimeAgentSelection,
  debugAcpConfig,
  runtimeSessionAliases,
  selectedAncestorSessions,
  newChatProjectKey,
  projectGroups,
  availableSessions,
  dispatchLiveEvent,
  setProjects,
  setFilter,
  setSelectedProject,
  setSelected,
  setDetailMode,
  setPendingNewChats,
  refreshSessions,
  onMessageCount,
  onActiveMessageMeta,
  onError,
}: {
  error: string | null;
  activeProject: ProjectInfo | null;
  selected: SessionInfo | null;
  selectedSessionProject: ProjectInfo | null;
  detailRoute: "chat" | "project";
  viewMode: ViewMode;
  liveState: LiveRuntimeState;
  runtimeAgents: RuntimeAgentMetadata[];
  lastRuntimeAgentSelection: RuntimeAgentSelection | null;
  rememberRuntimeAgentSelection: (selection: SetRuntimeAgentSelectionRequest) => Promise<void>;
  debugAcpConfig: boolean;
  runtimeSessionAliases: Record<string, string>;
  selectedAncestorSessions: SessionInfo[];
  newChatProjectKey: string | null;
  projectGroups: ProjectGroup[];
  availableSessions: SessionInfo[];
  dispatchLiveEvent: Dispatch<LiveRuntimeAction>;
  setProjects: Dispatch<SetStateAction<ProjectInfo[]>>;
  setFilter: Dispatch<SetStateAction<Filter>>;
  setSelectedProject: Dispatch<SetStateAction<{ kind: "project"; projectId: string } | null>>;
  setSelected: Dispatch<SetStateAction<SessionInfo | null>>;
  setDetailMode: Dispatch<SetStateAction<"chat" | "project">>;
  setPendingNewChats: Dispatch<SetStateAction<Record<string, PendingNewChatSession>>>;
  refreshSessions: () => Promise<void>;
  onMessageCount: (
    agent: Agent,
    filePath: string,
    sessionId: string,
    count: number,
  ) => boolean;
  onActiveMessageMeta: (meta: ActiveMessageMeta) => void;
  onError: (error: string | null) => void;
}) {
  const addPendingSession = (pending: PendingNewChatSession) => {
    setPendingNewChats((prev) => ({
      ...prev,
      [pending.sessioRuntimeSessionId]: pending,
    }));
  };

  const projectWorkbenchProps = (project: ProjectInfo) => ({
    project,
    sessions: availableSessions.filter((session) => session.projectPath === project.path),
    runtimeAgents,
    debugAcpConfig,
    liveState,
    dispatchLiveEvent,
    onProjectUpdated: (updatedProject: ProjectInfo) => {
      setProjects((prev) => prev.map((item) => (item.id === updatedProject.id ? updatedProject : item)));
      setFilter({ kind: "project", key: projectFilterKey(updatedProject), label: updatedProject.name });
    },
    onProjectArchived: (projectId: string) => {
      setProjects((prev) => prev.filter((item) => item.id !== projectId));
      setSelectedProject(null);
      setFilter({ kind: "all" });
      void refreshSessions();
    },
    onSelectSession: (session: SessionInfo) => {
      setSelectedProject(null);
      setSelected(session);
      setDetailMode("chat");
    },
    onPendingSession: addPendingSession,
    onChatStarted: () => {
      setSelectedProject(null);
      setDetailMode("chat");
    },
    onError,
  });

  if (error) {
    return (
      <div className="m-5 p-3 rounded bg-status-error/10 text-status-error text-body-sm">
        {error}
      </div>
    );
  }

  if (activeProject) {
    return (
      <div className="flex min-h-0 flex-1 overflow-hidden">
        <ProjectWorkbenchPage {...projectWorkbenchProps(activeProject)} />
      </div>
    );
  }

  if (!selected) {
    return (
      <NewChatPage
        projects={projectGroups}
        initialProjectKey={newChatProjectKey}
        runtimeAgents={runtimeAgents}
        lastRuntimeAgentSelection={lastRuntimeAgentSelection}
        rememberRuntimeAgentSelection={rememberRuntimeAgentSelection}
        liveState={liveState}
        dispatchLiveEvent={dispatchLiveEvent}
        onError={onError}
        onPendingSession={addPendingSession}
      />
    );
  }

  return (
    <div className="relative flex-1 min-h-0">
      <div
        className={
          "absolute inset-0 " +
          (detailRoute === "chat" ? "visible" : "invisible pointer-events-none")
        }
        aria-hidden={detailRoute !== "chat"}
      >
        <ChatPage
          session={selected}
          viewMode={viewMode}
          liveState={liveState}
          runtimeAgents={runtimeAgents}
          rememberRuntimeAgentSelection={rememberRuntimeAgentSelection}
          debugAcpConfig={debugAcpConfig}
          runtimeSessionAliases={runtimeSessionAliases}
          ancestorSessions={selectedAncestorSessions}
          dispatchLiveEvent={dispatchLiveEvent}
          onPendingSession={addPendingSession}
          onMessageCount={onMessageCount}
          onActiveMessageMeta={onActiveMessageMeta}
        />
      </div>
      <div
        className={
          "absolute inset-0 " +
          (detailRoute === "project" ? "visible" : "invisible pointer-events-none")
        }
        aria-hidden={detailRoute !== "project"}
      >
        {selectedSessionProject ? (
          <ProjectWorkbenchPage {...projectWorkbenchProps(selectedSessionProject)} />
        ) : (
          <div className="flex h-full min-h-0 items-center justify-center bg-surface-panel px-6 text-body-sm text-ink/45">
            No project is linked to this session.
          </div>
        )}
      </div>
    </div>
  );
}
