import type { Dispatch, SetStateAction, ReactNode } from "react";
import type { Agent, ProjectInfo, RuntimeAgentMetadata, RuntimeAgentSelection, SetRuntimeAgentSelectionRequest, SessionInfo } from "../api";
import type { ActiveMessageMeta } from "../pages/ChatPage";
import ChatPage from "../pages/ChatPage";
import NewChatPage from "../pages/NewChatPage";
import { ProjectWorkbenchPage } from "../pages/ProjectPage";
import ThreadPage from "../pages/ThreadPage";
import { type Filter } from "../appUtils";
import type { PendingNewChatSession, ViewMode, ProjectGroup } from "../navigation";
import type {
  LiveRuntimeAction,
  LiveRuntimeState,
} from "../runtimeChat";
import ToastStack from "./ToastStack";

export default function AppMain({
  error,
  activeProject,
  selectedThreadId,
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
  setSelectedThread,
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
  selectedThreadId: string | null;
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
  setSelectedThread: Dispatch<SetStateAction<{ projectId: string; threadId: string; goal: string } | null>>;
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
    onProjectArchived: (projectId: string) => {
      setProjects((prev) => prev.filter((item) => item.id !== projectId));
      setSelectedProject(null);
      setSelectedThread(null);
      setFilter({ kind: "all" });
      void refreshSessions();
    },
    onSelectSession: (session: SessionInfo) => {
      setSelectedProject(null);
      setSelectedThread(null);
      setSelected(session);
      setDetailMode("chat");
    },
    onPendingSession: addPendingSession,
    onChatStarted: () => {
      setSelectedProject(null);
      setSelectedThread(null);
      setDetailMode("chat");
    },
    onError,
  });

  const withErrorToast = (content: ReactNode) => (
    <div className="relative flex min-h-0 flex-1 overflow-hidden">
      {content}
      <ToastStack message={error} onMessageConsumed={() => onError(null)} />
    </div>
  );

  if (activeProject) {
    return (
      withErrorToast(
        <div className="flex min-h-0 flex-1 overflow-hidden">
          {selectedThreadId ? (
            <ThreadPage
              project={activeProject}
              threadId={selectedThreadId}
              onSelectSession={projectWorkbenchProps(activeProject).onSelectSession}
              onError={onError}
            />
          ) : (
            <ProjectWorkbenchPage {...projectWorkbenchProps(activeProject)} />
          )}
        </div>,
      )
    );
  }

  if (!selected) {
    return withErrorToast(
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
      />,
    );
  }

  return withErrorToast(
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
    </div>,
  );
}
