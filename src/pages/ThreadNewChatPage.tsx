import { useEffect, useMemo, useState } from "react";
import { Folder, Workflow } from "lucide-react";
import type {
  RuntimeAgentMetadata,
  RuntimeAgentSelection,
  SetRuntimeAgentSelectionRequest,
  StageInfo,
  ThreadInfo,
  ThreadReplayInfo,
} from "../api";
import { getThreadReplay, getThreadWorkState, listThreads } from "../api";
import ChatComposer, { NewChatMenuButton, ScrambledProjectName } from "../components/ChatComposer";
import InlineMenuSelect, { type InlineMenuSelectOption } from "../components/InlineMenuSelect";
import { HashIcon } from "../components/IconifyIcon";
import { RuntimeMenuSelect } from "../components/RuntimeMenuSelect";
import { useChatComposer } from "../hooks/useChatComposer";
import { useI18n } from "../i18n";
import type { PendingNewChatSession, ProjectGroup } from "../navigation";
import type { LiveRuntimeAction, LiveRuntimeState } from "../runtimeChat";
import { buildThreadWorkSnapshot, renderThreadWorkContext } from "../threadSnapshot";
import { collectThreadHistorySnapshots, withThreadChatSessions } from "../threadWorkContext";
import { collectThreadChatSessions } from "../threadChats";

export default function ThreadNewChatPage({
  projects,
  initialProjectKey,
  snapshotContext,
  runtimeAgents,
  lastRuntimeAgentSelection,
  rememberRuntimeAgentSelection,
  liveState,
  dispatchLiveEvent,
  onError,
  onPendingSession,
}: {
  projects: ProjectGroup[];
  initialProjectKey: string | null;
  snapshotContext: { thread: ThreadInfo; stage: StageInfo | null };
  runtimeAgents: RuntimeAgentMetadata[];
  lastRuntimeAgentSelection: RuntimeAgentSelection | null;
  rememberRuntimeAgentSelection: (selection: SetRuntimeAgentSelectionRequest) => Promise<void>;
  liveState: LiveRuntimeState;
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onError: (error: string | null) => void;
  onPendingSession: (session: PendingNewChatSession) => void;
}) {
  const { t } = useI18n();
  const [projectKeyValue, setProjectKeyValue] = useState(
    () =>
      projects.find((p) => p.project.id === snapshotContext.thread.projectId)?.key ??
      initialProjectKey ??
      projects[0]?.key ??
      "",
  );
  const projectGroup = projects.find((p) => p.key === projectKeyValue) ?? projects[0] ?? null;
  const project = projectGroup?.project ?? null;
  const workspacePath = projectGroup?.path ?? null;
  const [threads, setThreads] = useState<ThreadInfo[]>(
    () => project?.id === snapshotContext.thread.projectId ? [snapshotContext.thread] : [],
  );
  const [threadId, setThreadId] = useState(snapshotContext.thread.id);
  const [threadReplay, setThreadReplay] = useState<ThreadReplayInfo | null>(null);
  const [reloadToken, setReloadToken] = useState(0);
  const composer = useChatComposer({
    runtimeAgents,
    lastRuntimeAgentSelection,
    rememberRuntimeAgentSelection,
    liveState,
    dispatchLiveEvent,
    onError,
    onPendingSession,
  });
  const selectedThread = threads.find((thread) => thread.id === threadId) ?? threads[0] ?? null;
  const sortedStages = useMemo(
    () => (selectedThread?.stages ?? []).slice().sort((a, b) => a.order - b.order),
    [selectedThread?.stages],
  );
  const replayForSelectedThread = threadReplay?.threadId === selectedThread?.id ? threadReplay : null;
  const threadChatSessions = useMemo(
    () => selectedThread ? collectThreadChatSessions(selectedThread, replayForSelectedThread) : [],
    [replayForSelectedThread, selectedThread],
  );
  const snapshotStageId =
    selectedThread?.id === snapshotContext.thread.id
      ? snapshotContext.stage?.id ?? null
      : null;
  const focusedStage = snapshotStageId
    ? sortedStages.find((stage) => stage.id === snapshotStageId) ?? null
    : selectedThread?.stageId
      ? sortedStages.find((stage) => stage.id === selectedThread.stageId) ?? null
      : null;
  const linkStageId = snapshotStageId ? focusedStage?.id ?? null : null;
  const threadOptions: InlineMenuSelectOption[] = threads.map((thread) => ({
    value: thread.id,
    label: thread.goal,
    icon: <HashIcon className="h-4 w-4 text-ink/55" />,
  }));

  useEffect(() => {
    if (projectKeyValue && projects.some((p) => p.key === projectKeyValue)) return;
    setProjectKeyValue(projects[0]?.key ?? "");
  }, [projectKeyValue, projects]);

  useEffect(() => {
    let cancelled = false;
    const fallbackThread =
      project?.id === snapshotContext.thread.projectId ? snapshotContext.thread : null;
    setThreads(fallbackThread ? [fallbackThread] : []);
    setThreadReplay(null);
    if (!project?.id) return;
    listThreads(project.id)
      .then((rows) => {
        if (cancelled) return;
        setThreads(rows);
        const preferred =
          rows.find((thread) => thread.id === snapshotContext.thread.id) ??
          rows[0] ??
          null;
        setThreadId((current) => rows.some((thread) => thread.id === current) ? current : preferred?.id ?? "");
      })
      .catch((err) => {
        if (!cancelled) onError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [onError, project?.id, reloadToken, snapshotContext.thread]);

  useEffect(() => {
    if (!threadId) return;
    let cancelled = false;
    Promise.all([
      getThreadWorkState(threadId),
      getThreadReplay(threadId).catch((err) => {
        onError(String(err));
        return null;
      }),
    ])
      .then(([thread, replay]) => {
        if (cancelled) return;
        setThreads((prev) => {
          if (prev.some((item) => item.id === thread.id)) {
            return prev.map((item) => item.id === thread.id ? thread : item);
          }
          return [...prev, thread];
        });
        setThreadReplay(replay);
      })
      .catch((err) => {
        if (!cancelled) onError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [onError, threadId, reloadToken]);

  const handleSend = async () => {
    const prompt = composer.text.trim();
    if (!prompt) return;
    if (!workspacePath || !projectGroup) {
      composer.setComposerError(t("new_chat.no_project"));
      return;
    }
    if (!selectedThread) {
      composer.setComposerError(t("thread.not_found"));
      return;
    }
    const timestamp = Date.now();
    try {
      const workSnapshot = withThreadChatSessions(
        buildThreadWorkSnapshot(selectedThread, focusedStage, timestamp),
        threadChatSessions,
      );
      const { snapshot: snapshotWithSources, historySnapshots } = await collectThreadHistorySnapshots(workSnapshot);
      const sent = await composer.runStartSession(prompt, {
        workspacePath,
        projectName: projectGroup.label,
        extraContext: renderThreadWorkContext(snapshotWithSources, composer.selectedAgent),
        pendingSession: {
          origin: "thread_chat",
          historySnapshots,
          workSnapshot: {
            threadId: selectedThread.id,
            stageId: linkStageId,
            snapshot: snapshotWithSources,
          },
          threadLink: {
            threadId: selectedThread.id,
            stageId: linkStageId,
          },
        },
      });
      if (sent) setReloadToken((value) => value + 1);
    } catch (err) {
      const message = String(err);
      composer.setComposerError(message);
      onError(message);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface-panel">
      <div className="flex min-h-0 flex-1 items-center justify-center px-6 pb-16">
        <div className="w-full max-w-[730px]">
          <ChatComposer
            composer={composer}
            title={<>Work on <ScrambledProjectName name={selectedThread?.goal || projectGroup?.label || "thread"} /></>}
            canSend={Boolean(selectedThread) && composer.canSendWithWorkspace(workspacePath)}
            onSend={() => void handleSend()}
            variant="chat"
            bottomRow={
              <div className="flex h-10 items-center gap-2 px-3 text-body-sm text-ink/55">
                <RuntimeMenuSelect
                  ariaLabel={t("new_chat.project")}
                  value={projectKeyValue}
                  onChange={setProjectKeyValue}
                  disabled={projects.length === 0}
                  options={projects.map((p) => ({
                    value: p.key,
                    label: p.label,
                    icon: <Folder className="h-4 w-4 text-ink/55" />,
                  }))}
                />
                <div className="flex min-w-0 max-w-[320px] items-center rounded-md text-ink/55 transition hover:bg-ink/8 hover:text-ink">
                  <InlineMenuSelect
                    value={threadId}
                    options={threadOptions}
                    onChange={setThreadId}
                    menuAlign="trigger"
                    placeholder={t("thread.title")}
                    ariaLabel={t("thread.title")}
                    className="h-7 max-w-[320px] border-r-0 px-1.5 py-1 text-ink/60 hover:text-ink"
                    menuClassName="bg-surface-panel"
                    minMenuWidth={260}
                    emptyContent={t("thread.empty")}
                  />
                </div>
                <NewChatMenuButton icon={Workflow} label="main" text />
              </div>
            }
          />
        </div>
      </div>
    </div>
  );
}
