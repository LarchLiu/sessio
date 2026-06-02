import { useCallback, useEffect, useMemo, useState } from "react";
import { Check, Circle, CircleAlert, CircleDot, LoaderCircle, MessageSquarePlus, MinusCircle } from "lucide-react";
import HashIcon from "@iconify-react/mynaui/hash";
import type { Agent, ProjectInfo, SessionInfo, StageInfo, StageStatus, ThreadInfo } from "../api";
import { AGENT_LABEL, listThreads, updateThreadStageState } from "../api";
import { AgentGlyph } from "../components/AgentIcon";
import AssistantBotIcon from "../components/AssistantBotIcon";
import ScrollArea from "../components/ScrollArea";
import { localeTag, useI18n } from "../i18n";
import { sessionIdentityKey } from "../appUtils";
import { projectStageIcon } from "../utils/stageDisplay";

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
  const [loading, setLoading] = useState(true);

  const reload = useCallback(() => {
    return listThreads(project.id)
      .then((rows) => setThreads(rows))
      .catch((err) => onError(String(err)));
  }, [onError, project.id]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    listThreads(project.id)
      .then((rows) => {
        if (!cancelled) setThreads(rows);
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
  }, [onError, project.id]);

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
              <ThreadStat label={t("assistant.title")} value={String(uniqueAssistantCount(sortedStages))} />
              <ThreadStat label={t("thread.chats")} value={String(thread.sessions.length)} />
              <ThreadStat label={t("meta.updated")} value={formatDate(thread.updatedAt, lang) ?? "-"} />
            </div>

            {thread.description && (
              <p className="max-w-[820px] whitespace-pre-wrap text-body-sm leading-relaxed text-ink/55">
                {thread.description}
              </p>
            )}

            {thread.sessions.length > 0 && (
              <ThreadLinkedSessions sessions={thread.sessions} onSelectSession={onSelectSession} />
            )}

            {sortedStages.length === 0 ? (
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
  onStatusChange,
}: {
  stage: StageInfo;
  previousStatus: StageStatus | null;
  first: boolean;
  last: boolean;
  onSelectSession: (session: SessionInfo) => void;
  onNewChat: () => void;
  onStatusChange: (status: StageStatus) => void;
}) {
  const { t } = useI18n();
  const visual = stageStatusVisual(stage.status);
  const Icon = visual.icon;
  const previousComplete = previousStatus === "completed";
  const nextComplete = stage.status === "completed";
  const completeLineClass = "bg-[rgb(var(--color-emerald)/0.75)]";
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
                <span className="truncate">{stageLabel(stage, t)}</span>
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
        </div>
      </div>
    </section>
  );
}

function ThreadLinkedSessions({
  sessions,
  onSelectSession,
}: {
  sessions: SessionInfo[];
  onSelectSession: (session: SessionInfo) => void;
}) {
  const { t } = useI18n();
  const groups = groupSessionsByAgent(sessions);
  return (
    <section className="rounded-lg border border-card-border/[0.12] bg-card p-3">
      <div className="mb-3 flex items-center gap-2 text-body-sm font-medium text-ink/75">
        <HashIcon className="h-4 w-4 text-ink/40" />
        {t("thread.linked_sessions")}
      </div>
      <div className="grid gap-2">
        {groups.map(({ agent, sessions: agentSessions }) => (
          <AssistantSessionLane
            key={agent}
            label={AGENT_LABEL[agent]}
            agent={agent}
            sessions={agentSessions}
            onSelectSession={onSelectSession}
          />
        ))}
      </div>
    </section>
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
                {session.title ?? session.firstUserMessage ?? t("list.no_user_message")}
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

const STAGE_STATUS_ORDER: StageStatus[] = [
  "not_started",
  "in_progress",
  "needs_review",
  "blocked",
  "completed",
  "skipped",
];

type StageStatusVisual = {
  icon: typeof Circle;
  markerClass: string;
  textClass: string;
};

function stageStatusVisual(status: StageStatus): StageStatusVisual {
  switch (status) {
    case "completed":
      return {
        icon: Check,
        markerClass:
          "border-[rgb(var(--color-emerald)/0.80)] bg-[rgb(var(--color-emerald))] text-[rgb(var(--color-bg-panel))]",
        textClass: "text-[rgb(var(--color-emerald))]",
      };
    case "in_progress":
      return {
        icon: CircleDot,
        markerClass:
          "border-[rgb(var(--color-emerald)/0.80)] bg-surface-panel text-[rgb(var(--color-emerald))]",
        textClass: "text-[rgb(var(--color-emerald))]",
      };
    case "needs_review":
      return {
        icon: CircleDot,
        markerClass: "border-sky-500/70 bg-surface-panel text-sky-500",
        textClass: "text-sky-500",
      };
    case "blocked":
      return {
        icon: CircleAlert,
        markerClass: "border-amber-500/70 bg-surface-panel text-amber-500",
        textClass: "text-amber-500",
      };
    case "skipped":
      return {
        icon: MinusCircle,
        markerClass: "border-ink/15 bg-surface-panel text-ink/30",
        textClass: "text-ink/40",
      };
    case "not_started":
    default:
      return {
        icon: Circle,
        markerClass: "border-ink/15 bg-surface-panel text-ink/30",
        textClass: "text-ink/45",
      };
  }
}

function stageLabel(stage: StageInfo, t: (key: string) => string): string {
  if (stage.type === "custom") return stage.name ?? t("stage.custom");
  return stage.kind ? t(`stage.type.${stage.kind}`) : stage.name ?? t("stage.type");
}

function uniqueAssistantCount(stages: StageInfo[]): number {
  return new Set(stages.flatMap((stage) => stage.assistants.map((assistant) => assistant.assistantId))).size;
}

function compareSessionTime(a: SessionInfo, b: SessionInfo): number {
  const left = a.updatedAt ?? a.startedAt ?? 0;
  const right = b.updatedAt ?? b.startedAt ?? 0;
  return right - left;
}

function groupSessionsByAgent(sessions: SessionInfo[]): Array<{ agent: Agent; sessions: SessionInfo[] }> {
  const byAgent = new Map<Agent, SessionInfo[]>();
  for (const session of sessions) {
    const group = byAgent.get(session.agent) ?? [];
    group.push(session);
    byAgent.set(session.agent, group);
  }
  return Array.from(byAgent, ([agent, rows]) => ({
    agent,
    sessions: rows.slice().sort(compareSessionTime),
  }));
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
