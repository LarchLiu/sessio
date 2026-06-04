import { useEffect, useMemo, useState } from "react";
import HashIcon from "@iconify-react/mynaui/hash";
import { Bot, Folder, GitBranch, LoaderCircle } from "lucide-react";
import type {
  ProjectInfo,
  RuntimeAgentMetadata,
  RuntimeAgentSelection,
  SessionHistorySnapshotGroup,
  SessionHistoryTurn,
  SessionInfo,
  SetRuntimeAgentSelectionRequest,
  StageInfo,
  ThreadInfo,
  ThreadWorkSnapshot,
  ThreadWorkSnapshotSessionRef,
} from "../api";
import { getSessionHistory, getThreadWorkState, listThreads } from "../api";
import ChatComposer, { NewChatMenuButton, ScrambledProjectName } from "../components/ChatComposer";
import InlineMenuSelect, { type InlineMenuSelectOption } from "../components/InlineMenuSelect";
import { RuntimeMenuSelect } from "../components/RuntimeMenuSelect";
import ScrollArea from "../components/ScrollArea";
import SessionHistoryReadonly from "../components/SessionHistoryReadonly";
import { useChatComposer } from "../hooks/useChatComposer";
import { useI18n } from "../i18n";
import type { PendingNewChatSession, ProjectGroup } from "../navigation";
import type { LiveRuntimeAction, LiveRuntimeState } from "../runtimeChat";
import { sessionDisplayTitle } from "../appUtils";
import { buildThreadWorkSnapshot, renderThreadWorkContext } from "../threadSnapshot";
import { projectStageIcon, projectStageLabel, stageStatusVisual } from "../utils/stageDisplay";

export default function ThreadChatPage({
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
  onSelectSession,
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
  onSelectSession: (session: SessionInfo) => void;
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
  const [threads, setThreads] = useState<ThreadInfo[]>([snapshotContext.thread]);
  const [threadId, setThreadId] = useState(snapshotContext.thread.id);
  const [historyTarget, setHistoryTarget] = useState<SessionInfo | null>(null);
  const [historyTurns, setHistoryTurns] = useState<SessionHistoryTurn[] | null>(null);
  const [historyLoading, setHistoryLoading] = useState(false);
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
    setThreads([]);
    setHistoryTarget(null);
    setHistoryTurns(null);
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
    if (!historyTarget) return;
    if (!historyTarget.filePath) {
      setHistoryTurns(null);
      return;
    }
    let cancelled = false;
    setHistoryLoading(true);
    getSessionHistory(historyTarget.agent, historyTarget.filePath, historyTarget.id)
      .then((result) => {
        if (!cancelled) setHistoryTurns(result.turns);
      })
      .catch((err) => {
        if (!cancelled) onError(String(err));
      })
      .finally(() => {
        if (!cancelled) setHistoryLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [historyTarget, onError]);

  useEffect(() => {
    if (!threadId) return;
    let cancelled = false;
    getThreadWorkState(threadId)
      .then((thread) => {
        if (cancelled) return;
        setThreads((prev) => {
          if (prev.some((item) => item.id === thread.id)) {
            return prev.map((item) => item.id === thread.id ? thread : item);
          }
          return [...prev, thread];
        });
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
    const workSnapshot = buildThreadWorkSnapshot(selectedThread, focusedStage, timestamp);
    const { snapshot: snapshotWithSources, historySnapshots } = await collectThreadHistorySnapshots(workSnapshot);
    const sent = await composer.runStartSession(prompt, {
      workspacePath,
      projectName: projectGroup.label,
      extraContext: renderThreadWorkContext(snapshotWithSources, composer.selectedAgent),
      pendingSession: {
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
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface-panel">
      <ScrollArea className="min-h-0 flex-1" viewportClassName="px-6 py-5">
        <div className="mx-auto grid w-full max-w-[980px] gap-4">
          {selectedThread ? (
            <ThreadWorkOverview
              project={project}
              thread={selectedThread}
              stages={sortedStages}
              currentStage={focusedStage}
              onSelectSession={(session) => {
                setHistoryTarget(session);
                if (!session.filePath) setHistoryTurns(null);
              }}
              onOpenFullSession={onSelectSession}
            />
          ) : (
            <section className="rounded-lg border border-dashed border-card-border/[0.12] bg-card px-3 py-10 text-center text-body-sm text-ink/35">
              {t("thread.empty")}
            </section>
          )}
          {historyTarget && (
            <section className="rounded-lg border border-card-border/[0.12] bg-card p-3">
              <div className="mb-3 flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="truncate text-body-sm font-medium text-ink/75">
                    {sessionDisplayTitle(historyTarget) ?? t("list.no_user_message")}
                  </div>
                  <div className="text-caption text-ink/35">
                    {historyTarget.agent}:{historyTarget.id}
                  </div>
                </div>
                <button
                  type="button"
                  onClick={() => onSelectSession(historyTarget)}
                  className="shrink-0 rounded border border-ink/15 bg-surface-panel px-2 py-1 text-caption text-ink/55 hover:bg-ink/[0.05] hover:text-ink/80"
                >
                  {t("thread.open_full_chat")}
                </button>
              </div>
              {!historyTarget.filePath ? (
                <div className="rounded-md border border-dashed border-card-border/[0.12] px-3 py-4 text-body-sm text-ink/35">
                  {t("thread.history_pending")}
                </div>
              ) : historyLoading ? (
                <div className="flex items-center gap-2 py-6 text-body-sm text-ink/40">
                  <LoaderCircle className="h-4 w-4 animate-spin" />
                  {t("memory_search.searching")}
                </div>
              ) : (
                <SessionHistoryReadonly turns={historyTurns ?? []} />
              )}
            </section>
          )}
        </div>
      </ScrollArea>

      <div className="border-t border-ink/[0.08] bg-surface-panel px-6 py-4">
        <div className="mx-auto flex w-full max-w-[730px] justify-center">
          <ChatComposer
            composer={composer}
            title={<>Work on <ScrambledProjectName name={selectedThread?.goal || projectGroup?.label || "thread"} /></>}
            canSend={Boolean(selectedThread) && composer.canSendWithWorkspace(workspacePath)}
            onSend={() => void handleSend()}
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
                <NewChatMenuButton icon={GitBranch} label="main" text />
              </div>
            }
          />
        </div>
      </div>
    </div>
  );
}

function ThreadWorkOverview({
  project,
  thread,
  stages,
  currentStage,
  onSelectSession,
  onOpenFullSession,
}: {
  project: ProjectInfo | null;
  thread: ThreadInfo;
  stages: StageInfo[];
  currentStage: StageInfo | null;
  onSelectSession: (session: SessionInfo) => void;
  onOpenFullSession: (session: SessionInfo) => void;
}) {
  const { t } = useI18n();
  const completed = stages.filter((stage) => stage.status === "completed" || stage.status === "skipped").length;
  const blocked = stages.filter((stage) => stage.status === "blocked").length;
  const openIssues = stages.reduce(
    (total, stage) => total + stage.issues.filter((issue) => issue.status === "open").length,
    0,
  );
  return (
    <section className="rounded-lg border border-card-border/[0.12] bg-card p-3">
      <div className="mb-3 min-w-0">
        <div className="text-caption uppercase text-ink/35">{project?.name ?? t("project.workbench")}</div>
        <h2 className="truncate text-body font-medium text-ink/85">{thread.goal}</h2>
        {thread.description && (
          <p className="mt-1 max-w-[760px] whitespace-pre-wrap text-body-sm leading-relaxed text-ink/50">
            {thread.description}
          </p>
        )}
      </div>
      <div className="mb-3 grid grid-cols-[repeat(auto-fit,minmax(150px,1fr))] gap-2">
        <OverviewStat label={t("stage.project_stages")} value={`${completed}/${stages.length}`} />
        <OverviewStat label={t("stage.status.blocked")} value={String(blocked)} />
        <OverviewStat label={t("issue.status.open")} value={String(openIssues)} />
        {currentStage && (
          <OverviewStat label={t("thread.current_stage")} value={projectStageLabel(currentStage, t)} />
        )}
      </div>
      <div className="grid gap-2">
        {stages.map((stage) => {
          const visual = stageStatusVisual(stage.status);
          const Icon = visual.icon;
          const openStageIssues = stage.issues.filter((issue) => issue.status === "open");
          return (
            <div
              key={stage.id}
              className={
                "rounded-md border px-2.5 py-2 " +
                (stage.id === currentStage?.id
                  ? "border-[rgb(var(--color-emerald)/0.35)] bg-[rgb(var(--color-emerald)/0.06)]"
                  : "border-card-border/[0.10] bg-card-panel")
              }
            >
              <div className="flex min-w-0 flex-wrap items-center gap-2">
                <span className={"flex h-6 w-6 shrink-0 items-center justify-center rounded-full border " + visual.markerClass}>
                  <Icon className="h-3.5 w-3.5" />
                </span>
                {projectStageIcon(stage, "h-4 w-4 shrink-0 text-ink/45")}
                <div className="min-w-0 flex-1 truncate text-body-sm font-medium text-ink/75">
                  {projectStageLabel(stage, t)}
                </div>
                <span className={"text-caption font-medium " + visual.textClass}>
                  {t(`stage.status.${stage.status}`)}
                </span>
                <span className="rounded bg-ink/[0.06] px-1.5 py-0.5 text-caption text-ink/40">
                  {openStageIssues.length} {t("issue.status.open")}
                </span>
                <span className="rounded bg-ink/[0.06] px-1.5 py-0.5 text-caption text-ink/40">
                  {stage.sessions.length} {t("thread.sessions")}
                </span>
              </div>
              {openStageIssues.length > 0 && (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {openStageIssues.map((issue) => (
                    <span key={issue.id} className="rounded border border-card-border/[0.12] bg-card px-1.5 py-0.5 text-caption text-ink/55">
                      [{issue.severity}] {issue.title}
                    </span>
                  ))}
                </div>
              )}
              {stage.sessions.length > 0 && (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {stage.sessions.slice().sort(compareSessionTime).map((session) => (
                    <button
                      key={`${session.agent}:${session.id}`}
                      type="button"
                      onClick={() => onSelectSession(session)}
                      onDoubleClick={() => onOpenFullSession(session)}
                      className="flex max-w-[260px] items-center gap-1.5 rounded border border-card-border/[0.10] bg-card px-2 py-1 text-caption text-ink/55 hover:bg-card-hover hover:text-ink/80"
                    >
                      <Bot className="h-3.5 w-3.5 shrink-0 text-ink/35" />
                      <span className="truncate">{sessionDisplayTitle(session) ?? session.id}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}

function OverviewStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border border-card-border/[0.10] bg-card-panel px-2.5 py-2">
      <div className="text-caption uppercase text-ink/35">{label}</div>
      <div className="mt-1 truncate text-body-sm font-medium text-ink/75">{value}</div>
    </div>
  );
}

function compareSessionTime(a: SessionInfo, b: SessionInfo): number {
  const left = a.updatedAt ?? a.startedAt ?? 0;
  const right = b.updatedAt ?? b.startedAt ?? 0;
  return right - left;
}

async function collectThreadHistorySnapshots(snapshot: ThreadWorkSnapshot): Promise<{
  snapshot: ThreadWorkSnapshot;
  historySnapshots: SessionHistorySnapshotGroup[];
}> {
  const sessionRefs = dedupeSnapshotSessionRefs(snapshot.detailRefs?.sessionRefs ?? []);
  const loadedRefs: ThreadWorkSnapshotSessionRef[] = [];
  const historySnapshots: SessionHistorySnapshotGroup[] = [];
  for (const ref of sessionRefs) {
    const filePath = ref.filePath ?? "";
    if (!filePath) {
      loadedRefs.push(ref);
      continue;
    }
    try {
      const result = await getSessionHistory(ref.agent, filePath, ref.sessionId);
      const ancestorIndex = historySnapshots.length;
      historySnapshots.push({
        ancestorAgent: ref.agent,
        ancestorSessionId: ref.sessionId,
        ancestorIndex,
        turns: result.turns.slice(-12),
      });
      loadedRefs.push({ ...ref, ancestorIndex });
    } catch {
      loadedRefs.push(ref);
    }
  }

  const byKey = new Map(loadedRefs.map((ref) => [`${ref.agent}:${ref.sessionId}`, ref]));
  const stages = snapshot.stages.map((stage) => ({
    ...stage,
    sessionRefs: stage.sessionRefs.map((ref) => byKey.get(`${ref.agent}:${ref.sessionId}`) ?? ref),
  }));
  const threadSessionRefs = (snapshot.threadSessionRefs ?? []).map(
    (ref) => byKey.get(`${ref.agent}:${ref.sessionId}`) ?? ref,
  );
  const sourceRefs = dedupeSnapshotSessionRefs([...threadSessionRefs, ...stages.flatMap((stage) => stage.sessionRefs)]);
  return {
    snapshot: {
      ...snapshot,
      stages,
      threadSessionRefs,
      relatedContext: {
        sessionExcerptRefs: sourceRefs,
      },
      detailRefs: {
        threadId: snapshot.threadId,
        focusedStageId: snapshot.focusedStageId,
        stageIds: stages.map((stage) => stage.threadStageId),
        issueIds: stages.flatMap((stage) => (stage.issues ?? []).map((issue) => issue.id)),
        sessionRefs: sourceRefs,
      },
    },
    historySnapshots,
  };
}

function dedupeSnapshotSessionRefs(refs: ThreadWorkSnapshotSessionRef[]): ThreadWorkSnapshotSessionRef[] {
  const seen = new Set<string>();
  const result: ThreadWorkSnapshotSessionRef[] = [];
  for (const ref of refs) {
    const key = `${ref.agent}:${ref.sessionId}`;
    if (seen.has(key)) continue;
    seen.add(key);
    result.push(ref);
  }
  return result;
}
