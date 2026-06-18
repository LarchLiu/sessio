import { useEffect, useState, type Dispatch, type SetStateAction } from "react";
import type { Agent, AssistantInfo, ProjectInfo, RuntimeAgentMetadata, RuntimeAgentSelection, SetRuntimeAgentSelectionRequest, SessionInfo } from "../api";
import type { ActiveMessageMeta } from "../pages/ChatPage";
import type { ChatFilesSubview } from "./ChatFilesView";
import ChatPage from "../pages/ChatPage";
import NewChatPage from "../pages/NewChatPage";
import { ProjectWorkbenchPage } from "../pages/ProjectPage";
import ThreadPage from "../pages/ThreadPage";
import ThreadChatPage from "../pages/ThreadChatPage";
import ThreadMultiSessionChatPage from "../pages/ThreadMultiSessionChatPage";
import { projectFilterKey, type Filter } from "../appUtils";
import type { ChatView, DetailMode, PendingNewChatSession, ViewMode, ProjectGroup } from "../navigation";
import type {
  LiveRuntimeAction,
  LiveRuntimeState,
} from "../runtimeChat";
import { listAssistants } from "../api";

export default function AppMain({
  activeProject,
  selectedThreadId,
  selected,
  selectedSessionProject,
  detailRoute,
  viewMode,
  chatView,
  filesSubview,
  onFilesSubviewChange,
  projectFilesReloadKey,
  selectedProjectFileRequest,
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
  chatView: ChatView;
  filesSubview: ChatFilesSubview;
  onFilesSubviewChange: (subview: ChatFilesSubview) => void;
  projectFilesReloadKey: number;
  selectedProjectFileRequest?: {
    path: string;
    requestId: number;
  } | null;
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
  const [projectAssistants, setProjectAssistants] = useState<Record<string, AssistantInfo[]>>({});
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
    onSelectThreadChatSession: (session: SessionInfo) => {
      setSelectedProject(null);
      setSelectedThread(null);
      setSelected(session);
      setDetailMode("threadChat");
    },
    onError,
  });

  useEffect(() => {
    const projectIds = Array.from(new Set([
      activeProject?.id,
      selectedSessionProject?.id,
    ].filter((value): value is string => Boolean(value))));
    if (projectIds.length === 0) return;
    let cancelled = false;
    Promise.all(
      projectIds.map(async (projectId) => [projectId, await listAssistants(projectId)] as const),
    )
      .then((entries) => {
        if (cancelled) return;
        setProjectAssistants((current) => {
          const next = { ...current };
          for (const [projectId, assistants] of entries) next[projectId] = assistants;
          return next;
        });
      })
      .catch((err) => {
        if (!cancelled) onError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [activeProject?.id, onError, selectedSessionProject?.id]);

  if (activeProject) {
    return (
      <div className="flex min-h-0 flex-1 overflow-hidden">
        {selectedThreadId && detailRoute === "threadMultiSessionChat" ? (
          <ThreadMultiSessionChatPage
            project={activeProject}
            assistants={projectAssistants[activeProject.id] ?? []}
            threadId={selectedThreadId}
            liveState={liveState}
            runtimeAgents={runtimeAgents}
            lastRuntimeAgentSelection={lastRuntimeAgentSelection}
            rememberRuntimeAgentSelection={rememberRuntimeAgentSelection}
            runtimeSessionAliases={runtimeSessionAliases}
            pendingNewChats={pendingNewChats}
            dispatchLiveEvent={dispatchLiveEvent}
            onPendingSession={addPendingSession}
            onError={onError}
          />
        ) : selectedThreadId ? (
          <ThreadPage
            project={activeProject}
            threadId={selectedThreadId}
            onSelectSession={projectWorkbenchProps(activeProject).onSelectThreadChatSession}
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
        onProjectChange={setNewChatProjectKey}
        onPendingSession={addPendingSession}
        onThreadCreated={(projectGroup, thread) => {
          setSelected(null);
          setSelectedProject(null);
          setSelectedThread({
            projectId: projectGroup.project.id,
            threadId: thread.id,
            goal: thread.goal,
          });
          setNewChatProjectKey(projectGroup.key);
          setDetailMode("threadMultiSessionChat");
          setFilter({
            kind: "project",
            key: projectFilterKey(projectGroup.project),
            label: projectGroup.label,
          });
          void refreshSessions();
        }}
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
        {detailRoute !== "threadChat" && (
          <ChatPage
            session={selected}
            assistants={selectedSessionProject ? (projectAssistants[selectedSessionProject.id] ?? []) : []}
            viewMode={viewMode}
            chatView={chatView}
            filesSubview={filesSubview}
            onFilesSubviewChange={onFilesSubviewChange}
            projectFilesReloadKey={projectFilesReloadKey}
            selectedProjectFileRequest={selectedProjectFileRequest}
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
        )}
      </div>
      <div
        className={
          "absolute inset-0 " +
          (detailRoute === "threadChat" ? "visible" : "invisible pointer-events-none")
        }
        aria-hidden={detailRoute !== "threadChat"}
      >
        {detailRoute === "threadChat" && (
          <ThreadChatPage
            session={selected}
            assistants={selectedSessionProject ? (projectAssistants[selectedSessionProject.id] ?? []) : []}
            viewMode={viewMode}
            projectFilesReloadKey={projectFilesReloadKey}
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
        )}
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
