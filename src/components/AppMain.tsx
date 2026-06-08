import { useState, type Dispatch, type SetStateAction } from "react";
import type { Agent, ProjectInfo, RuntimeAgentMetadata, RuntimeAgentSelection, SetRuntimeAgentSelectionRequest, SessionInfo, StageInfo, ThreadInfo } from "../api";
import type { ActiveMessageMeta } from "../pages/ChatPage";
import ChatPage from "../pages/ChatPage";
import NewChatPage from "../pages/NewChatPage";
import { ProjectWorkbenchPage } from "../pages/ProjectPage";
import ThreadPage from "../pages/ThreadPage";
import ThreadChatPage from "../pages/ThreadChatPage";
import ThreadMultiSessionChatPage from "../pages/ThreadMultiSessionChatPage";
import { projectFilterKey, type Filter } from "../appUtils";
import type { DetailMode, PendingNewChatSession, ViewMode, ProjectGroup } from "../navigation";
import type {
  LiveRuntimeAction,
  LiveRuntimeState,
} from "../runtimeChat";

export default function AppMain({
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
  pendingNewChats,
  setNewChatProjectKey,
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
  activeProject: ProjectInfo | null;
  selectedThreadId: string | null;
  selected: SessionInfo | null;
  selectedSessionProject: ProjectInfo | null;
  detailRoute: DetailMode;
  viewMode: ViewMode;
  liveState: LiveRuntimeState;
  runtimeAgents: RuntimeAgentMetadata[];
  lastRuntimeAgentSelection: RuntimeAgentSelection | null;
  rememberRuntimeAgentSelection: (selection: SetRuntimeAgentSelectionRequest) => Promise<void>;
  debugAcpConfig: boolean;
  runtimeSessionAliases: Record<string, string>;
  selectedAncestorSessions: SessionInfo[];
  newChatProjectKey: string | null;
  pendingNewChats: Record<string, PendingNewChatSession>;
  setNewChatProjectKey: Dispatch<SetStateAction<string | null>>;
  projectGroups: ProjectGroup[];
  availableSessions: SessionInfo[];
  dispatchLiveEvent: Dispatch<LiveRuntimeAction>;
  setProjects: Dispatch<SetStateAction<ProjectInfo[]>>;
  setFilter: Dispatch<SetStateAction<Filter>>;
  setSelectedProject: Dispatch<SetStateAction<{ kind: "project"; projectId: string } | null>>;
  setSelectedThread: Dispatch<SetStateAction<{ projectId: string; threadId: string; goal: string } | null>>;
  setSelected: Dispatch<SetStateAction<SessionInfo | null>>;
  setDetailMode: Dispatch<SetStateAction<DetailMode>>;
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
  const [newChatSnapshot, setNewChatSnapshot] = useState<{
    thread: ThreadInfo;
    stage: StageInfo | null;
  } | null>(null);

  const addPendingSession = (pending: PendingNewChatSession) => {
    setNewChatSnapshot(null);
    setPendingNewChats((prev) => ({
      ...prev,
      [pending.sessioRuntimeSessionId]: pending,
    }));
  };

  const openNewChatForStage = (thread: ThreadInfo, stage: StageInfo | null) => {
    setNewChatSnapshot({ thread, stage });
    setSelectedProject(null);
    setSelectedThread(null);
    setSelected(null);
    setDetailMode("chat");
  };

  const projectWorkbenchProps = (project: ProjectInfo) => ({
    project,
    sessions: availableSessions.filter((session) => session.projectPath === project.path),
    runtimeAgents,
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
    onNewThreadChat: (thread: ThreadInfo) => {
      const projectGroup = projectGroups.find((group) => group.project.id === project.id);
      setNewChatSnapshot({ thread, stage: null });
      setSelectedProject(null);
      setSelectedThread(null);
      setSelected(null);
      setDetailMode("chat");
      setNewChatProjectKey(projectGroup?.key ?? projectFilterKey(project));
      setFilter({ kind: "project", key: projectFilterKey(project), label: project.name });
    },
    onError,
  });

  if (activeProject) {
    return (
      <div className="flex min-h-0 flex-1 overflow-hidden">
        {selectedThreadId && detailRoute === "threadMultiSessionChat" ? (
          <ThreadMultiSessionChatPage
            project={activeProject}
            threadId={selectedThreadId}
            liveState={liveState}
            runtimeSessionAliases={runtimeSessionAliases}
            pendingNewChats={pendingNewChats}
            onBackToOverview={() => setDetailMode("project")}
            onSelectSession={projectWorkbenchProps(activeProject).onSelectSession}
            onError={onError}
          />
        ) : selectedThreadId ? (
          <ThreadPage
            project={activeProject}
            threadId={selectedThreadId}
            onSelectSession={projectWorkbenchProps(activeProject).onSelectSession}
            onNewStageChat={openNewChatForStage}
            onOpenMultiSessionChat={() => setDetailMode("threadMultiSessionChat")}
            onError={onError}
          />
        ) : (
          <ProjectWorkbenchPage {...projectWorkbenchProps(activeProject)} />
        )}
      </div>
    );
  }

  if (!selected) {
    if (newChatSnapshot) {
      return (
        <ThreadChatPage
          projects={projectGroups}
          initialProjectKey={newChatProjectKey}
          snapshotContext={newChatSnapshot}
          runtimeAgents={runtimeAgents}
          lastRuntimeAgentSelection={lastRuntimeAgentSelection}
          rememberRuntimeAgentSelection={rememberRuntimeAgentSelection}
          liveState={liveState}
          dispatchLiveEvent={dispatchLiveEvent}
          onError={onError}
          onPendingSession={addPendingSession}
          onSelectSession={(session) => {
            setNewChatSnapshot(null);
            setSelectedProject(null);
            setSelectedThread(null);
            setSelected(session);
            setDetailMode("chat");
          }}
        />
      );
    }
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
