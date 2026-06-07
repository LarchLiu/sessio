import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AlertCircle, Bot, LoaderCircle, MessageSquarePlus, Plus, Sparkles, Square, Trash2 } from "lucide-react";
import HashIcon from "@iconify-react/mynaui/hash";
import type { Agent, AstraEvent, AstraHandle, AstraRunStatus, IssueSeverity, IssueStatus, PlanRoundInfo, PlanTaskInfo, PlanTaskSessionInfo, PlanTaskStatus, ProjectInfo, SessionInfo, StageInfo, StageStatus, ThreadInfo, ThreadReplayInfo, ThreadReplaySessionInfo, ThreadReplaySessionSourceInfo } from "../api";
import {
  AGENT_LABEL,
  cancelAstraRun,
  createAstraRun,
  createThreadStageIssue,
  deleteThreadStageIssue,
  getThreadReplay,
  listAstraRuns,
  listPlanRounds,
  listThreads,
  updateThreadStageIssue,
  updateThreadStageState,
} from "../api";
import { AgentGlyph } from "../components/AgentIcon";
import AssistantBotIcon from "../components/AssistantBotIcon";
import ScrollArea from "../components/ScrollArea";
import { localeTag, useI18n } from "../i18n";
import { sessionDisplayTitle, sessionIdentityKey } from "../appUtils";
import { projectStageIcon, projectStageLabel, STAGE_STATUS_ORDER, stageStatusVisual } from "../utils/stageDisplay";

const THREAD_REFRESH_ASTRA_EVENTS = new Set(["delegated", "stage_update_result"]);

export default function ThreadPage({
  project,
  threadId,
  onSelectSession,
  onNewStageChat,
  onError,
}: {
  project: ProjectInfo;
  threadId: string;
  onSelectSession: (session: SessionInfo) => void;
  onNewStageChat: (thread: ThreadInfo, stage: StageInfo | null) => void;
  onError: (error: string | null) => void;
}) {
  const { t, lang } = useI18n();
  const [threads, setThreads] = useState<ThreadInfo[]>([]);
  const [replay, setReplay] = useState<ThreadReplayInfo | null>(null);
  const [loading, setLoading] = useState(true);

  const loadThreadData = useCallback(async () => {
    const rows = await listThreads(project.id);
    const nextReplay = rows.some((row) => row.id === threadId)
      ? await getThreadReplay(threadId)
      : null;
    return { rows, nextReplay };
  }, [project.id, threadId]);

  const reload = useCallback(() => {
    return loadThreadData()
      .then(({ rows, nextReplay }) => {
        setThreads(rows);
        setReplay(nextReplay);
      })
      .catch((err) => onError(String(err)));
  }, [loadThreadData, onError]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    loadThreadData()
      .then(({ rows, nextReplay }) => {
        if (!cancelled) {
          setThreads(rows);
          setReplay(nextReplay);
        }
      })
      .catch((err) => {
        if (!cancelled) onError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [loadThreadData, onError]);

  const thread = threads.find((row) => row.id === threadId) ?? null;
  const sortedStages = useMemo(
    () => (thread?.stages ?? []).slice().sort((a, b) => a.order - b.order),
    [thread?.stages],
  );
  return (
    <div className="flex h-full min-h-0 flex-1 flex-col overflow-hidden bg-surface-panel">
      <ScrollArea className="min-h-0 flex-1" viewportClassName="px-6 py-5">
        {loading ? (
          <div className="flex items-center justify-center gap-2 py-16 text-body-sm text-ink/45">
            <LoaderCircle className="h-4 w-4 animate-spin" />
            {t("memory_search.searching")}
          </div>
        ) : !thread ? (
          <div className="rounded-lg border border-dashed border-ink/15 py-16 text-center text-body-sm text-ink/40">
            {t("thread.not_found")}
          </div>
        ) : (
          <div className="grid gap-5">
            <div className="grid grid-cols-[repeat(auto-fit,minmax(180px,1fr))] gap-3">
              <ThreadStat label={t("stage.project_stages")} value={String(sortedStages.length)} />
              <ThreadStat label={t("assistant.title")} value={String(threadAssistantCount(thread, sortedStages))} />
              <ThreadStat label={t("thread.sessions")} value={String(replay?.sessions.length ?? thread.sessions.length)} />
              <ThreadStat label={t("meta.updated")} value={formatDate(thread.updatedAt, lang) ?? "-"} />
            </div>

            {thread.description && (
              <p className="max-w-[820px] whitespace-pre-wrap text-body-sm leading-relaxed text-ink/55">
                {thread.description}
              </p>
            )}

            <ThreadAstraPanel
              thread={thread}
              stages={sortedStages}
              onError={onError}
              onReload={reload}
            />

            {replay && replay.sessions.length > 0 && (
              <ThreadReplaySessions replay={replay} onSelectSession={onSelectSession} />
            )}

            {sortedStages.length === 0 && thread.kind !== "workflow" ? (
              thread.assistants.length > 0 ? (
                <div className="grid gap-2">
                  {thread.assistants.map((assistant) => {
                    const runtimeAgent = agentFromId(assistant.agent.id);
                    return (
                      <AssistantSessionLane
                        key={assistant.assistantId}
                        label={assistant.name}
                        agent={runtimeAgent}
                        agentLabel={runtimeAgent ? undefined : assistant.agent.name}
                        assistantColor={assistant.color}
                        sessions={[]}
                        onSelectSession={onSelectSession}
                      />
                    );
                  })}
                </div>
              ) : (
                <div className="rounded-lg border border-dashed border-ink/15 py-16 text-center text-body-sm text-ink/40">
                  {t("thread.no_assistants")}
                </div>
              )
            ) : sortedStages.length === 0 ? (
              <div className="rounded-lg border border-dashed border-ink/15 py-16 text-center text-body-sm text-ink/40">
                {t("stage.empty")}
              </div>
            ) : (
              <div className="grid gap-0">
                {sortedStages.map((stage, index) => (
                  <ThreadStageStep
                    key={stage.id}
                    stage={stage}
                    previousStatus={index > 0 ? sortedStages[index - 1].status : null}
                    first={index === 0}
                    last={index === sortedStages.length - 1}
                    onSelectSession={onSelectSession}
                    onNewChat={() => onNewStageChat(thread, stage)}
                    onError={onError}
                    reload={reload}
                    onStatusChange={async (status) => {
                      try {
                        await updateThreadStageState(stage.id, { status });
                        await reload();
                      } catch (err) {
                        onError(String(err));
                      }
                    }}
                  />
                ))}
              </div>
            )}
          </div>
        )}
      </ScrollArea>
    </div>
  );
}

function ThreadAstraPanel({
  thread,
  stages,
  onError,
  onReload,
}: {
  thread: ThreadInfo;
  stages: StageInfo[];
  onError: (error: string | null) => void;
  onReload: () => Promise<void>;
}) {
  const { t } = useI18n();
  const [runs, setRuns] = useState<AstraHandle[]>([]);
  const [planRounds, setPlanRounds] = useState<PlanRoundInfo[]>([]);
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState<"start" | "cancel" | null>(null);
  const activeRun = runs.find((run) => isAstraActive(run.status)) ?? runs[0] ?? null;
  const canStartAstra = thread.kind === "teamwork" || thread.kind === "brainstorm" || thread.kind === "debate";
  const astraBoundary = canStartAstra ? null : t(`astra.unsupported.${thread.kind}`);
  const orderedPlanRounds = useMemo(
    () => planRounds.slice().sort((a, b) => b.roundIndex - a.roundIndex || b.createdAt - a.createdAt),
    [planRounds],
  );

  const reloadAstraState = useCallback(() => {
    return Promise.all([listAstraRuns(thread.id), listPlanRounds(thread.id)])
      .then(([nextRuns, nextPlanRounds]) => {
        setRuns(nextRuns);
        setPlanRounds(nextPlanRounds);
      })
      .catch((err) => onError(String(err)));
  }, [onError, thread.id]);

  useEffect(() => {
    void reloadAstraState();
  }, [reloadAstraState]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<AstraEvent>("astra-run-event", (event) => {
      if (event.payload.threadId !== thread.id) return;
      const eventType = event.payload.eventType;
      void reloadAstraState();
      if (THREAD_REFRESH_ASTRA_EVENTS.has(eventType)) {
        void onReload();
      }
    }).then((fn) => {
      unlisten = fn;
    }).catch((err) => onError(String(err)));
    return () => {
      unlisten?.();
    };
  }, [onError, onReload, reloadAstraState, thread.id]);

  const start = async () => {
    if (!canStartAstra) return;
    setBusy("start");
    try {
      const run = await createAstraRun(thread.id, prompt.trim() || null);
      setRuns((prev) => upsertRun(prev, run));
      setPrompt("");
      await reloadAstraState();
    } catch (err) {
      onError(String(err));
    } finally {
      setBusy(null);
    }
  };

  const cancel = async () => {
    if (!activeRun) return;
    setBusy("cancel");
    try {
      const run = await cancelAstraRun(activeRun.runId);
      setRuns((prev) => upsertRun(prev, run));
      await reloadAstraState();
    } catch (err) {
      onError(String(err));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="rounded-lg border border-card-border/[0.12] bg-card p-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2 text-body-sm font-medium text-ink/78">
            <Sparkles className="h-4 w-4 text-[rgb(var(--color-emerald)/0.85)]" />
            Astra
            {activeRun && (
              <span className={"rounded px-1.5 py-0.5 text-meta font-medium " + astraStatusClass(activeRun.status)}>
                {formatAstraStatus(activeRun.status)}
              </span>
            )}
          </div>
          <div className="mt-1 max-w-[760px] text-caption leading-relaxed text-ink/40">
            {activeRun ? activeRun.runId : astraBoundary ?? t("astra.idle")}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {activeRun && isAstraActive(activeRun.status) && (
            <button
              type="button"
              disabled={busy !== null}
              onClick={() => void cancel()}
              title={t("astra.cancel")}
              className="flex h-8 w-8 items-center justify-center rounded border border-ink/15 bg-surface-panel text-ink/45 hover:bg-red-500/[0.08] hover:text-red-500 disabled:opacity-40"
            >
              {busy === "cancel" ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <Square className="h-3.5 w-3.5" />}
            </button>
          )}
          {canStartAstra && (
            <button
              type="button"
              disabled={busy !== null || Boolean(activeRun && isAstraActive(activeRun.status))}
              onClick={() => void start()}
              title={t("astra.start")}
              className="flex h-8 items-center gap-1.5 rounded border border-ink/15 bg-surface-panel px-2 text-caption text-ink/55 hover:bg-ink/[0.05] hover:text-ink/80 disabled:opacity-40"
            >
              {busy === "start" ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <Bot className="h-3.5 w-3.5" />}
              {t("astra.start")}
            </button>
          )}
        </div>
      </div>

      <div className="mt-3 grid gap-2">
        {canStartAstra ? (
          <textarea
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            rows={2}
            placeholder={t("astra.prompt_placeholder")}
            className="min-w-0 resize-none rounded-md border border-input-border/[0.16] bg-input px-3 py-2 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30"
          />
        ) : (
          <div className="flex items-start gap-2 rounded-md border border-dashed border-card-border/[0.14] px-3 py-2 text-caption leading-relaxed text-ink/38">
            <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-ink/32" />
            <span>{astraBoundary}</span>
          </div>
        )}

        {orderedPlanRounds.length > 0 && (
          <div className="grid gap-2">
            {orderedPlanRounds.map((round) => (
              <AstraPlanRoundCard
                key={round.id}
                round={round}
                thread={thread}
                stages={stages}
              />
            ))}
          </div>
        )}

        {activeRun && (
          <AstraRunDiagnostics run={activeRun} />
        )}
        {activeRun?.error && (
          <div
            title={activeRun.error}
            className="flex min-w-0 items-start gap-1.5 rounded-md border border-status-error/20 bg-status-error/10 px-2.5 py-2 text-caption leading-relaxed text-status-error"
          >
            <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span className="min-w-0 break-words">{activeRun.error}</span>
          </div>
        )}
      </div>
    </section>
  );
}

function AstraPlanRoundCard({
  round,
  thread,
  stages,
}: {
  round: PlanRoundInfo;
  thread: ThreadInfo;
  stages: StageInfo[];
}) {
  const { t } = useI18n();
  return (
    <div className="rounded-md border border-card-border/[0.10] bg-card-panel px-2.5 py-2">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <span className="text-body-sm font-medium text-ink/78">
          {t("astra.round", { index: round.roundIndex + 1 })}
        </span>
        <span className={"rounded px-1.5 py-0.5 text-meta font-medium " + planRoundStatusClass(round.status)}>
          {formatPlanStatus(round.status)}
        </span>
        <span className="rounded bg-ink/[0.06] px-1.5 py-0.5 text-meta text-ink/45">
          {t(`astra.mode.${round.mode}`)}
        </span>
        {round.astraRunId && (
          <span className="min-w-0 truncate text-meta text-ink/35">
            {round.astraRunId}
          </span>
        )}
      </div>
      {round.summary && (
        <div className="mt-1 line-clamp-2 text-caption leading-relaxed text-ink/45">
          {round.summary}
        </div>
      )}
      {round.tasks.length > 0 && (
        <div className="mt-2 grid gap-1.5">
          {round.tasks.map((task) => (
            <AstraPlanTaskRow
              key={task.id}
              task={task}
              assistantName={thread.assistants.find((assistant) => assistant.assistantId === task.assistantId)?.name ?? null}
              stage={stages.find((stage) => stage.id === task.threadStageId) ?? null}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function AstraPlanTaskRow({
  task,
  assistantName,
  stage,
}: {
  task: PlanTaskInfo;
  assistantName: string | null;
  stage: StageInfo | null;
}) {
  const { t } = useI18n();
  const detail = task.error ?? task.resultSummary ?? task.expectedOutput ?? null;
  const snapshots = planTaskSnapshotLabels(task, t);
  return (
    <div className="rounded-md border border-card-border/[0.10] bg-card px-2 py-1.5">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <span className="min-w-0 truncate text-body-sm font-medium text-ink/78">{task.title}</span>
        <AgentGlyph agent={task.targetAgent} className="h-3.5 w-3.5 shrink-0" />
        <span className={"rounded px-1.5 py-0.5 text-meta font-medium " + astraRiskClass(task.risk)}>
          {t(`astra.risk.${task.risk}`)}
        </span>
        <span className={"rounded px-1.5 py-0.5 text-meta font-medium " + astraTaskStatusClass(task.status)}>
          {formatPlanStatus(task.status)}
        </span>
        {assistantName && (
          <span className="truncate text-meta text-ink/35">
            {assistantName}
          </span>
        )}
        {stage && (
          <span className="truncate text-meta text-ink/35">
            {projectStageLabel(stage, t)}
          </span>
        )}
      </div>
      {task.sessions.length > 0 && (
        <AstraPlanTaskSessions sessions={task.sessions} />
      )}
      {detail && (
        <div className={"mt-1 line-clamp-2 text-caption leading-relaxed " + (task.error ? "text-status-error" : "text-ink/45")}>
          {detail}
        </div>
      )}
      {snapshots.length > 0 && (
        <div className="mt-1.5 flex min-w-0 flex-wrap gap-1">
          {snapshots.map((snapshot) => (
            <span
              key={`${snapshot.kind}:${snapshot.label}`}
              title={snapshot.title}
              className="max-w-full truncate rounded bg-ink/[0.045] px-1.5 py-0.5 text-meta text-ink/40"
            >
              {snapshot.label}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

function AstraPlanTaskSessions({ sessions }: { sessions: PlanTaskSessionInfo[] }) {
  const { t } = useI18n();
  const visible = sessions.slice(0, 3);
  const hiddenCount = sessions.length - visible.length;
  return (
    <div className="mt-1.5 flex min-w-0 flex-wrap gap-1">
      {visible.map((session) => (
        <span
          key={`${session.agent}:${session.sessionId}:${session.role}`}
          title={`${t(`astra.session_role.${session.role}`)}\n${AGENT_LABEL[session.agent]}\n${session.sessionId}`}
          className="inline-flex max-w-full items-center gap-1 rounded bg-ink/[0.045] px-1.5 py-0.5 text-meta text-ink/40"
        >
          <AgentGlyph agent={session.agent} className="h-3 w-3 shrink-0" />
          <span className="shrink-0">{t(`astra.session_role.${session.role}`)}</span>
          <span className="min-w-0 truncate text-ink/30">{shortSessionId(session.sessionId)}</span>
        </span>
      ))}
      {hiddenCount > 0 && (
        <span className="rounded bg-ink/[0.045] px-1.5 py-0.5 text-meta text-ink/35">
          {t("astra.task_sessions_more", { count: hiddenCount })}
        </span>
      )}
    </div>
  );
}

function AstraRunDiagnostics({ run }: { run: AstraHandle }) {
  const { t } = useI18n();
  const diagnostics = run.runDiagnostics.slice(-3).reverse().map((diagnostic, index) => {
    return describeAstraDiagnostic(diagnostic, index);
  });
  const hasSummary = Boolean(run.terminalReason || run.lastErrorCode || run.lastErrorMessage || diagnostics.length > 0);
  if (!hasSummary) return null;

  return (
    <div className="rounded-md border border-dashed border-card-border/[0.12] px-2.5 py-2">
      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        <span className="text-caption font-medium text-ink/55">{t("astra.diagnostics")}</span>
        {run.terminalReason && (
          <span title={run.terminalReason} className="max-w-full truncate rounded bg-ink/[0.06] px-1.5 py-0.5 text-meta text-ink/45">
            {t("astra.terminal_reason", { value: run.terminalReason })}
          </span>
        )}
        {run.lastErrorCode && (
          <span title={run.lastErrorCode} className="max-w-full truncate rounded bg-red-500/[0.10] px-1.5 py-0.5 text-meta text-red-500">
            {t("astra.error_code", { value: run.lastErrorCode })}
          </span>
        )}
      </div>
      {run.lastErrorMessage && (
        <div className="mt-1 line-clamp-2 text-caption leading-relaxed text-status-error">
          {run.lastErrorMessage}
        </div>
      )}
      {diagnostics.length > 0 && (
        <div className="mt-1.5 grid gap-1">
          {diagnostics.map((diagnostic) => (
            <div key={diagnostic.key} title={diagnostic.raw} className="min-w-0 rounded bg-ink/[0.035] px-2 py-1 text-caption text-ink/42">
              <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                <span className="truncate font-medium text-ink/55">{diagnostic.label}</span>
                {diagnostic.code && (
                  <span className="truncate rounded bg-ink/[0.06] px-1 py-0.5 text-meta text-ink/38">
                    {diagnostic.code}
                  </span>
                )}
              </div>
              {diagnostic.detail && (
                <div className="mt-0.5 line-clamp-2 leading-relaxed">{diagnostic.detail}</div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ThreadStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-card-border/[0.12] bg-card px-3 py-2.5">
      <div className="text-caption uppercase tracking-normal text-ink/35">{label}</div>
      <div className="mt-1 truncate text-body font-medium text-ink/80">{value}</div>
    </div>
  );
}

function ThreadStageStep({
  stage,
  previousStatus,
  first,
  last,
  onSelectSession,
  onNewChat,
  onError,
  reload,
  onStatusChange,
}: {
  stage: StageInfo;
  previousStatus: StageStatus | null;
  first: boolean;
  last: boolean;
  onSelectSession: (session: SessionInfo) => void;
  onNewChat: () => void;
  onError: (error: string | null) => void;
  reload: () => Promise<void>;
  onStatusChange: (status: StageStatus) => void;
}) {
  const { t } = useI18n();
  const visual = stageStatusVisual(stage.status);
  const Icon = visual.icon;
  const previousComplete = previousStatus === "completed";
  const nextComplete = stage.status === "completed";
  const completeLineClass = "bg-[rgb(var(--color-emerald)/0.75)]";
  const handleIssueError = useCallback(
    (err: unknown) => onError(String(err)),
    [onError],
  );
  return (
    <section className="grid grid-cols-[32px_minmax(0,1fr)] gap-3">
      <div className="relative flex justify-center">
        {!first && (
          <div className={"absolute top-0 h-5 w-px " + (previousComplete ? completeLineClass : "bg-ink/[0.12]")} />
        )}
        <span className={"relative z-10 mt-5 flex h-8 w-8 items-center justify-center rounded-full border " + visual.markerClass}>
          <Icon className="h-4 w-4" />
        </span>
        {!last && (
          <div className={"absolute bottom-0 top-12 w-px " + (nextComplete ? completeLineClass : "bg-ink/[0.12]")} />
        )}
      </div>

      <div className="pb-5 pt-3">
        <div className="rounded-lg border border-card-border/[0.12] bg-card p-3">
          <div className="flex flex-wrap items-start justify-between gap-2">
            <div className="min-w-0">
              <h2 className="flex min-w-0 items-center gap-2 text-body font-medium text-ink/85">
                {projectStageIcon(stage, "h-4 w-4 shrink-0 text-ink/45")}
                <span className="truncate">{projectStageLabel(stage, t)}</span>
              </h2>
              {stage.description && (
                <p className="mt-1 max-w-[720px] whitespace-pre-wrap text-body-sm leading-relaxed text-ink/50">
                  {stage.description}
                </p>
              )}
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <button
                type="button"
                onClick={onNewChat}
                title={t("stage.new_chat")}
                className="flex items-center gap-1 rounded border border-ink/15 bg-surface-panel px-1.5 py-0.5 text-meta text-ink/55 hover:bg-ink/[0.05] hover:text-ink/80"
              >
                <MessageSquarePlus className="h-3.5 w-3.5" />
                {t("stage.new_chat")}
              </button>
              <select
                value={stage.status}
                onChange={(event) => onStatusChange(event.target.value as StageStatus)}
                className={"rounded border border-ink/15 bg-surface-panel px-1.5 py-0.5 text-meta font-medium " + visual.textClass}
              >
                {STAGE_STATUS_ORDER.map((status) => (
                  <option key={status} value={status}>
                    {t(`stage.status.${status}`)}
                  </option>
                ))}
              </select>
              <span className="rounded bg-ink/[0.08] px-1.5 py-0.5 text-meta text-ink/40">
                {stage.sessions.length}
              </span>
            </div>
          </div>

          <div className="mt-3 grid gap-2">
            {stage.assistants.length === 0 ? (
              <AssistantSessionLane
                label={t("assistant.empty")}
                agent={null}
                sessions={stage.sessions}
                onSelectSession={onSelectSession}
              />
            ) : (
              stage.assistants.map((assistant) => (
                <AssistantSessionLane
                  key={`${stage.id}:${assistant.assistantId}`}
                  label={assistant.name}
                  agent={knownAgent(assistant.agent.id)}
                  agentLabel={assistant.agent.name}
                  assistantColor={assistant.color}
                  sessions={stage.sessions.filter((session) => session.agent === assistant.agent.id)}
                  onSelectSession={onSelectSession}
                />
              ))
            )}
          </div>

          <StageIssueSection
            stage={stage}
            reload={reload}
            onError={handleIssueError}
          />
        </div>
      </div>
    </section>
  );
}

const ISSUE_STATUS_ORDER: IssueStatus[] = ["open", "resolved", "dismissed"];
const ISSUE_SEVERITY_ORDER: IssueSeverity[] = ["low", "medium", "high", "critical"];

function StageIssueSection({
  stage,
  reload,
  onError,
}: {
  stage: StageInfo;
  reload: () => Promise<void>;
  onError: (error: unknown) => void;
}) {
  const { t } = useI18n();
  const [newTitle, setNewTitle] = useState("");
  const [newSeverity, setNewSeverity] = useState<IssueSeverity>("medium");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  const addDisabled = adding || newTitle.trim().length === 0;

  const handleAdd = async () => {
    const title = newTitle.trim();
    if (!title) return;
    setAdding(true);
    try {
      await createThreadStageIssue(stage.id, title, newSeverity);
      setNewTitle("");
      setNewSeverity("medium");
      await reload();
    } catch (err) {
      onError(err);
    } finally {
      setAdding(false);
    }
  };

  const handleUpdate = async (
    issueId: string,
    patch: { status?: IssueStatus; severity?: IssueSeverity },
  ) => {
    setBusyId(issueId);
    try {
      await updateThreadStageIssue(issueId, patch);
      await reload();
    } catch (err) {
      onError(err);
    } finally {
      setBusyId(null);
    }
  };

  const handleDelete = async (issueId: string) => {
    setBusyId(issueId);
    try {
      await deleteThreadStageIssue(issueId);
      await reload();
    } catch (err) {
      onError(err);
    } finally {
      setBusyId(null);
    }
  };

  return (
    <div className="mt-3 rounded-md border border-card-border/[0.10] bg-card-panel px-2.5 py-2">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="text-body-sm font-medium text-ink/75">{t("issue.title")}</div>
        <span className="rounded bg-ink/[0.06] px-1.5 py-0.5 text-meta text-ink/35">
          {stage.issues.length}
        </span>
      </div>

      {stage.issues.length === 0 ? (
        <div className="rounded border border-dashed border-card-border/[0.10] px-2 py-2 text-caption text-ink/35">
          {t("issue.empty")}
        </div>
      ) : (
        <div className="grid gap-1.5">
          {stage.issues.map((issue) => {
            const disabled = busyId === issue.id;
            return (
              <div
                key={issue.id}
                className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] gap-2 rounded-md border border-card-border/[0.10] bg-card px-2 py-1.5"
              >
                <div className="min-w-0">
                  <div className="flex min-w-0 items-center gap-2">
                    <span className={"h-2 w-2 shrink-0 rounded-full " + issueSeverityDotClass(issue.severity)} />
                    <div className="min-w-0 truncate text-body-sm text-ink/75">{issue.title}</div>
                  </div>
                  {issue.description && (
                    <div className="mt-1 whitespace-pre-wrap text-caption leading-relaxed text-ink/40">
                      {issue.description}
                    </div>
                  )}
                </div>
                <div className="flex shrink-0 flex-wrap items-center justify-end gap-1.5">
                  <select
                    value={issue.status}
                    disabled={disabled}
                    onChange={(event) => handleUpdate(issue.id, { status: event.target.value as IssueStatus })}
                    className="rounded border border-ink/15 bg-surface-panel px-1.5 py-0.5 text-meta text-ink/55 disabled:opacity-50"
                  >
                    {ISSUE_STATUS_ORDER.map((status) => (
                      <option key={status} value={status}>
                        {t(`issue.status.${status}`)}
                      </option>
                    ))}
                  </select>
                  <select
                    value={issue.severity}
                    disabled={disabled}
                    onChange={(event) => handleUpdate(issue.id, { severity: event.target.value as IssueSeverity })}
                    className={"rounded border border-ink/15 bg-surface-panel px-1.5 py-0.5 text-meta font-medium disabled:opacity-50 " + issueSeverityTextClass(issue.severity)}
                  >
                    {ISSUE_SEVERITY_ORDER.map((severity) => (
                      <option key={severity} value={severity}>
                        {t(`issue.severity.${severity}`)}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    disabled={disabled}
                    onClick={() => handleDelete(issue.id)}
                    title={t("issue.delete")}
                    className="flex h-6 w-6 items-center justify-center rounded border border-ink/10 bg-surface-panel text-ink/40 hover:bg-red-500/[0.08] hover:text-red-500 disabled:opacity-50"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      <div className="mt-2 grid grid-cols-[minmax(0,1fr)_auto_auto] gap-1.5">
        <input
          value={newTitle}
          onChange={(event) => setNewTitle(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void handleAdd();
            }
          }}
          placeholder={t("issue.new_placeholder")}
          className="min-w-0 rounded border border-ink/15 bg-surface-panel px-2 py-1 text-body-sm text-ink/75 outline-none placeholder:text-ink/30 focus:border-[rgb(var(--color-emerald)/0.45)]"
        />
        <select
          value={newSeverity}
          disabled={adding}
          onChange={(event) => setNewSeverity(event.target.value as IssueSeverity)}
          className={"rounded border border-ink/15 bg-surface-panel px-1.5 py-1 text-meta font-medium disabled:opacity-50 " + issueSeverityTextClass(newSeverity)}
        >
          {ISSUE_SEVERITY_ORDER.map((severity) => (
            <option key={severity} value={severity}>
              {t(`issue.severity.${severity}`)}
            </option>
          ))}
        </select>
        <button
          type="button"
          disabled={addDisabled}
          onClick={() => void handleAdd()}
          title={t("issue.add")}
          className="flex h-8 w-8 items-center justify-center rounded border border-ink/15 bg-surface-panel text-ink/50 hover:bg-ink/[0.05] hover:text-ink/80 disabled:opacity-40"
        >
          {adding ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <Plus className="h-3.5 w-3.5" />}
        </button>
      </div>
    </div>
  );
}

function ThreadReplaySessions({
  replay,
  onSelectSession,
}: {
  replay: ThreadReplayInfo;
  onSelectSession: (session: SessionInfo) => void;
}) {
  const { t } = useI18n();
  const groups = groupReplaySessionsByThreadKind(replay, t);
  return (
    <section className="rounded-lg border border-card-border/[0.12] bg-card p-3">
      <div className="mb-3 flex items-center gap-2 text-body-sm font-medium text-ink/75">
        <HashIcon className="h-4 w-4 text-ink/40" />
        {t("thread.replay_sessions")}
      </div>
      <div className="grid gap-2">
        {groups.map(({ key, label, agent, sessions: groupSessions }) => (
          <ThreadReplaySessionLane
            key={key}
            label={label}
            agent={agent}
            sessions={groupSessions}
            onSelectSession={onSelectSession}
          />
        ))}
      </div>
    </section>
  );
}

function ThreadReplaySessionLane({
  label,
  agent,
  sessions,
  onSelectSession,
}: {
  label: string;
  agent: Agent | null;
  sessions: ThreadReplaySessionInfo[];
  onSelectSession: (session: SessionInfo) => void;
}) {
  const { t, lang } = useI18n();
  return (
    <div className="rounded-md border border-card-border/[0.10] bg-card-panel px-2.5 py-2">
      <div className="flex min-w-0 items-center gap-2">
        {agent ? (
          <AgentGlyph agent={agent} className="h-4 w-4 shrink-0" />
        ) : (
          <HashIcon className="h-4 w-4 shrink-0 text-ink/35" />
        )}
        <div className="min-w-0 flex-1 truncate text-body-sm font-medium text-ink/75">{label}</div>
        <span className="shrink-0 text-meta text-ink/35">
          {t("astra.task_sessions", { count: sessions.length })}
        </span>
      </div>
      <div className="mt-2 grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-2">
        {sessions.slice().sort(compareReplaySessionTime).map((replaySession) => {
          const content = (
            <>
              <div className="truncate text-body-sm text-ink/75">
                {replaySession.session
                  ? sessionDisplayTitle(replaySession.session) ?? t("list.no_user_message")
                  : replaySession.sessionId}
              </div>
              <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-1">
                {replaySession.session ? (
                  <span className="text-meta text-ink/35">
                    {t("list.msgs", { count: replaySession.session.messageCount })}
                  </span>
                ) : (
                  <span className="text-meta text-ink/35">{t("thread.replay_reference")}</span>
                )}
                {replaySession.lastSeenAt && (
                  <span className="text-meta text-ink/25">
                    {formatDate(replaySession.lastSeenAt, lang)}
                  </span>
                )}
              </div>
              <div className="mt-1.5 flex min-w-0 flex-wrap gap-1">
                {replaySession.sources.map((source) => (
                  <span
                    key={replaySourceKey(source)}
                    title={replaySourceTitle(source)}
                    className="max-w-full truncate rounded bg-ink/[0.06] px-1.5 py-0.5 text-meta text-ink/40"
                  >
                    {source.label ?? t(`thread.replay_source.${source.kind}`)}
                  </span>
                ))}
              </div>
            </>
          );

          return replaySession.session ? (
            <button
              key={`${replaySession.agent}:${replaySession.sessionId}`}
              type="button"
              onClick={() => onSelectSession(replaySession.session!)}
              className="min-w-0 rounded-md border border-card-border/[0.10] bg-card px-2 py-1.5 text-left transition hover:bg-card-hover"
            >
              {content}
            </button>
          ) : (
            <div
              key={`${replaySession.agent}:${replaySession.sessionId}`}
              className="min-w-0 rounded-md border border-card-border/[0.10] bg-card px-2 py-1.5"
            >
              {content}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function AssistantSessionLane({
  label,
  agent,
  agentLabel,
  assistantColor,
  sessions,
  onSelectSession,
}: {
  label: string;
  agent: Agent | null;
  agentLabel?: string;
  assistantColor?: string | null;
  sessions: SessionInfo[];
  onSelectSession: (session: SessionInfo) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="rounded-md border border-card-border/[0.10] bg-card-panel px-2.5 py-2">
      <div className="flex min-w-0 items-center gap-2">
        {assistantColor ? (
          <AssistantBotIcon color={assistantColor} className="h-4 w-4 shrink-0 text-ink/35" />
        ) : agent ? (
          <AgentGlyph agent={agent} className="h-4 w-4 shrink-0" />
        ) : (
          <AssistantBotIcon className="h-4 w-4 shrink-0 text-ink/35" />
        )}
        <div className="min-w-0 flex-1 truncate text-body-sm font-medium text-ink/75">{label}</div>
        {(agent || agentLabel) && (
          <span className="shrink-0 text-meta text-ink/35">
            {agent ? AGENT_LABEL[agent] : agentLabel}
          </span>
        )}
      </div>
      {sessions.length === 0 ? (
        <div className="mt-2 flex items-center gap-1.5 rounded border border-dashed border-card-border/[0.10] px-2 py-2 text-caption text-ink/35">
          {agent ? (
            <AgentGlyph agent={agent} className="h-3.5 w-3.5 shrink-0" />
          ) : (
            <AssistantBotIcon className="h-3.5 w-3.5 shrink-0 text-ink/30" />
          )}
          {t("thread.no_assistant_sessions")}
        </div>
      ) : (
        <div className="mt-2 grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-2">
          {sessions.slice().sort(compareSessionTime).map((session) => (
            <button
              key={sessionIdentityKey(session)}
              type="button"
              onClick={() => onSelectSession(session)}
              className="min-w-0 rounded-md border border-card-border/[0.10] bg-card px-2 py-1.5 text-left transition hover:bg-card-hover"
            >
              <div className="truncate text-body-sm text-ink/75">
                {sessionDisplayTitle(session) ?? t("list.no_user_message")}
              </div>
              <div className="mt-0.5 text-meta text-ink/35">
                {t("list.msgs", { count: session.messageCount })}
              </div>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function issueSeverityDotClass(severity: IssueSeverity): string {
  switch (severity) {
    case "critical":
      return "bg-red-500";
    case "high":
      return "bg-amber-500";
    case "medium":
      return "bg-sky-500";
    case "low":
    default:
      return "bg-ink/25";
  }
}

function issueSeverityTextClass(severity: IssueSeverity): string {
  switch (severity) {
    case "critical":
      return "text-red-500";
    case "high":
      return "text-amber-500";
    case "medium":
      return "text-sky-500";
    case "low":
    default:
      return "text-ink/45";
  }
}

function threadAssistantCount(thread: ThreadInfo, stages: StageInfo[]): number {
  if (thread.kind !== "workflow") return thread.assistants.length;
  return new Set(stages.flatMap((stage) => stage.assistants.map((assistant) => assistant.assistantId))).size;
}

function agentFromId(value: string): Agent | null {
  return value === "astra-pi" || value === "codex" || value === "claude" || value === "gemini"
    ? value
    : null;
}

function compareSessionTime(a: SessionInfo, b: SessionInfo): number {
  const left = a.updatedAt ?? a.startedAt ?? 0;
  const right = b.updatedAt ?? b.startedAt ?? 0;
  return right - left;
}

function compareReplaySessionTime(a: ThreadReplaySessionInfo, b: ThreadReplaySessionInfo): number {
  const left = a.lastSeenAt ?? a.firstSeenAt ?? a.session?.updatedAt ?? a.session?.startedAt ?? 0;
  const right = b.lastSeenAt ?? b.firstSeenAt ?? b.session?.updatedAt ?? b.session?.startedAt ?? 0;
  return right - left;
}

type ReplaySessionGroup = {
  key: string;
  label: string;
  agent: Agent | null;
  sessions: ThreadReplaySessionInfo[];
};

function groupReplaySessionsByThreadKind(
  replay: ThreadReplayInfo,
  t: (key: string, vars?: Record<string, string | number>) => string,
): ReplaySessionGroup[] {
  const keyForSession = replay.kind === "workflow"
    ? (session: ThreadReplaySessionInfo) => workflowReplayGroupKey(session, t)
    : (session: ThreadReplaySessionInfo) => roundReplayGroupKey(session, t);
  const groups = new Map<string, ReplaySessionGroup>();
  for (const session of replay.sessions) {
    const seed = keyForSession(session);
    const group = groups.get(seed.key) ?? { ...seed, sessions: [] };
    group.sessions.push(session);
    groups.set(seed.key, group);
  }
  return Array.from(groups.values())
    .map((group) => ({
      ...group,
      sessions: group.sessions.slice().sort(compareReplaySessionTime),
    }))
    .sort(compareReplayGroups);
}

function workflowReplayGroupKey(
  session: ThreadReplaySessionInfo,
  t: (key: string, vars?: Record<string, string | number>) => string,
): ReplaySessionGroup {
  const stageSource = session.sources.find((source) => source.kind === "stage" || source.stageId);
  if (stageSource) {
    const label = stageSource.label ?? stageSource.stageId ?? t("thread.replay_source.stage");
    return {
      key: `stage:${stageSource.stageId ?? label}`,
      label,
      agent: null,
      sessions: [],
    };
  }
  return fallbackReplayGroupKey(session, t);
}

function roundReplayGroupKey(
  session: ThreadReplaySessionInfo,
  t: (key: string, vars?: Record<string, string | number>) => string,
): ReplaySessionGroup {
  const roundSource = session.sources.find((source) => source.planRoundId || source.kind === "plan_task");
  if (roundSource) {
    const value = roundSource.planRoundId
      ? shortSessionId(roundSource.planRoundId)
      : roundSource.label ?? roundSource.planTaskId ?? t("thread.replay_source.plan_task");
    return {
      key: `round:${roundSource.planRoundId ?? roundSource.planTaskId ?? value}`,
      label: t("thread.replay_group.round", { value }),
      agent: null,
      sessions: [],
    };
  }
  return fallbackReplayGroupKey(session, t);
}

function fallbackReplayGroupKey(
  session: ThreadReplaySessionInfo,
  t: (key: string, vars?: Record<string, string | number>) => string,
): ReplaySessionGroup {
  const source = session.sources[0] ?? null;
  if (source?.kind === "thread") {
    return {
      key: `thread:${session.agent}`,
      label: t("thread.replay_group.thread"),
      agent: session.agent,
      sessions: [],
    };
  }
  if (source?.kind === "astra_internal") {
    return {
      key: `astra:${source.astraRunId ?? session.agent}`,
      label: source.label ?? t("thread.replay_source.astra_internal"),
      agent: null,
      sessions: [],
    };
  }
  return {
    key: `agent:${session.agent}`,
    label: AGENT_LABEL[session.agent],
    agent: session.agent,
    sessions: [],
  };
}

function compareReplayGroups(a: ReplaySessionGroup, b: ReplaySessionGroup): number {
  const latestA = latestReplayGroupTime(a);
  const latestB = latestReplayGroupTime(b);
  return latestB - latestA || a.label.localeCompare(b.label);
}

function latestReplayGroupTime(group: ReplaySessionGroup): number {
  return group.sessions.reduce((latest, session) => {
    const time = session.lastSeenAt ?? session.firstSeenAt ?? session.session?.updatedAt ?? session.session?.startedAt ?? 0;
    return Math.max(latest, time);
  }, 0);
}

function replaySourceKey(source: ThreadReplaySessionSourceInfo): string {
  return [
    source.kind,
    source.stageId,
    source.planRoundId,
    source.planTaskId,
    source.astraRunId,
    source.role,
    source.createdAt,
  ].filter(Boolean).join(":");
}

function replaySourceTitle(source: ThreadReplaySessionSourceInfo): string {
  return [
    source.label,
    source.kind,
    source.role,
    source.planTaskId,
    source.planRoundId,
    source.stageId,
    source.astraRunId,
  ].filter(Boolean).join("\n");
}

function isAstraActive(status: AstraRunStatus): boolean {
  return status === "planning" || status === "thinking" || status === "awaiting_approval" || status === "dispatching" || status === "running";
}

function upsertRun(runs: AstraHandle[], run: AstraHandle): AstraHandle[] {
  const next = runs.some((item) => item.runId === run.runId)
    ? runs.map((item) => item.runId === run.runId ? run : item)
    : [run, ...runs];
  return next.slice().sort((a, b) => b.updatedAt - a.updatedAt);
}

function formatAstraStatus(status: AstraRunStatus): string {
  return status.replace(/_/g, " ");
}

function astraStatusClass(status: AstraRunStatus): string {
  switch (status) {
    case "awaiting_approval":
      return "bg-sky-500/[0.10] text-sky-500";
    case "thinking":
      return "bg-violet-500/[0.10] text-violet-500";
    case "dispatching":
    case "running":
      return "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]";
    case "errored":
      return "bg-red-500/[0.10] text-red-500";
    case "cancelled":
    case "interrupted":
      return "bg-ink/[0.08] text-ink/45";
    case "completed":
      return "bg-[rgb(var(--color-emerald)/0.12)] text-[rgb(var(--color-emerald)/0.95)]";
    case "planning":
    default:
      return "bg-amber-500/[0.10] text-amber-500";
  }
}

function astraRiskClass(risk: "low" | "medium" | "high"): string {
  switch (risk) {
    case "high":
      return "bg-red-500/[0.10] text-red-500";
    case "medium":
      return "bg-amber-500/[0.10] text-amber-500";
    case "low":
    default:
      return "bg-ink/[0.06] text-ink/45";
  }
}

function astraResultClass(status: PlanTaskStatus): string {
  switch (status) {
    case "completed":
      return "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]";
    case "failed":
    case "errored":
      return "bg-red-500/[0.10] text-red-500";
    case "cancelled":
    default:
      return "bg-ink/[0.08] text-ink/45";
  }
}

function astraTaskStatusClass(status: PlanTaskStatus): string {
  if (status === "running") {
    return "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]";
  }
  if (status === "planned") {
    return "bg-ink/[0.06] text-ink/45";
  }
  return astraResultClass(status);
}

type SnapshotChip = {
  kind: "stage" | "assistant" | "agent";
  label: string;
  title: string;
};

function planTaskSnapshotLabels(task: PlanTaskInfo, t: (key: string, vars?: Record<string, string | number>) => string): SnapshotChip[] {
  const chips: SnapshotChip[] = [];
  const stage = parseJsonObject(task.stageSnapshotJson);
  const assistant = parseJsonObject(task.assistantSnapshotJson);
  const agent = parseJsonObject(task.agentSnapshotJson);

  const stageName = stringField(stage, "name") ?? stringField(stage, "stageId") ?? stringField(stage, "id");
  if (stageName) {
    chips.push({
      kind: "stage",
      label: t("astra.snapshot.stage", { value: stageName }),
      title: task.stageSnapshotJson ?? stageName,
    });
  }

  const assistantName = stringField(assistant, "name") ?? stringField(assistant, "assistantId") ?? stringField(assistant, "id");
  if (assistantName) {
    const agentInfo = objectField(assistant, "agent");
    const model = stringField(agentInfo, "model");
    chips.push({
      kind: "assistant",
      label: t("astra.snapshot.assistant", {
        value: model ? `${assistantName} / ${model}` : assistantName,
      }),
      title: task.assistantSnapshotJson ?? assistantName,
    });
  }

  const agentInfo = objectField(agent, "agentInfo");
  const agentLabel = stringField(agentInfo, "displayName")
    ?? stringField(agentInfo, "name")
    ?? stringField(agent, "agent")
    ?? task.targetAgent;
  const model = stringField(agentInfo, "model");
  chips.push({
    kind: "agent",
    label: t("astra.snapshot.agent", {
      value: model ? `${agentLabel} / ${model}` : agentLabel,
    }),
    title: task.agentSnapshotJson,
  });

  return chips;
}

function parseJsonObject(value: string | null): Record<string, unknown> | null {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : null;
  } catch {
    return null;
  }
}

function objectField(value: Record<string, unknown> | null, key: string): Record<string, unknown> | null {
  const field = value?.[key];
  return field && typeof field === "object" && !Array.isArray(field)
    ? field as Record<string, unknown>
    : null;
}

function stringField(value: Record<string, unknown> | null, key: string): string | null {
  const field = value?.[key];
  return typeof field === "string" && field.trim() ? field : null;
}

function describeAstraDiagnostic(value: unknown, index: number): { key: string; label: string; code: string | null; detail: string | null; raw: string } {
  const record = value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
  const kind = diagnosticString(record, "kind") ?? "diagnostic";
  const backend = diagnosticString(record, "backend");
  const code = diagnosticString(record, "code");
  const message = diagnosticString(record, "message")
    ?? diagnosticString(record, "rawResponseSnippet")
    ?? diagnosticString(record, "sessionId");
  const raw = safeJsonPreview(value, 1200);
  return {
    key: `${kind}:${code ?? ""}:${index}`,
    label: backend ? `${kind} / ${backend}` : kind,
    code,
    detail: message,
    raw,
  };
}

function diagnosticString(record: Record<string, unknown> | null, key: string): string | null {
  const value = record?.[key];
  if (typeof value === "string" && value.trim()) return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return null;
}

function safeJsonPreview(value: unknown, maxLength: number): string {
  let text: string;
  try {
    text = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  } catch {
    text = String(value);
  }
  if (!text) return "";
  return text.length > maxLength ? `${text.slice(0, maxLength - 3)}...` : text;
}

function shortSessionId(sessionId: string): string {
  const trimmed = sessionId.trim();
  if (trimmed.length <= 18) return trimmed;
  return `${trimmed.slice(0, 8)}...${trimmed.slice(-6)}`;
}

function planRoundStatusClass(status: PlanRoundInfo["status"]): string {
  switch (status) {
    case "running":
      return "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]";
    case "completed":
      return "bg-[rgb(var(--color-emerald)/0.12)] text-[rgb(var(--color-emerald)/0.95)]";
    case "errored":
      return "bg-red-500/[0.10] text-red-500";
    case "cancelled":
      return "bg-ink/[0.08] text-ink/45";
    case "planned":
    default:
      return "bg-amber-500/[0.10] text-amber-500";
  }
}

function formatPlanStatus(status: string): string {
  return status.replace(/_/g, " ");
}

function knownAgent(value: string): Agent | null {
  return value === "codex" || value === "claude" || value === "gemini" ? value : null;
}

function formatDate(ts: number | null, lang: "en" | "zh"): string | null {
  if (!ts) return null;
  return new Date(ts).toLocaleString(localeTag(lang), {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
