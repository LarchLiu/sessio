import {
  parseWorkflowSnapshot,
  type WorkflowSnapshotAssistantView,
  type WorkflowSnapshotParticipantView,
  type WorkflowSnapshotRoundView,
  type WorkflowSnapshotStageView,
  type WorkflowSnapshotTaskView,
} from "./snapshot";
import type { WorkflowOverlay } from "../../workflowLiveProjection";
import AssistantBotIcon from "../../../../components/AssistantBotIcon";

export interface WorkflowCardHostProps {
  title: string;
  threadId: string;
  threadStageId: string;
  executionState: string;
  lastRunId: string;
  threadGoal: string;
  workflowSnapshotJson: string;
  workflowSummaryMarkdown: string;
  workflowOverlay?: WorkflowOverlay | null;
  onRunWorkflow: () => void;
  onOpenThread: () => void;
  interactionMode?: "block" | "overlay";
}

function stopOverlayInteraction(event: {
  preventDefault: () => void;
  stopPropagation: () => void;
}) {
  event.preventDefault();
  event.stopPropagation();
}

export function WorkflowCardHost({
  title,
  threadId,
  threadStageId,
  executionState,
  lastRunId,
  threadGoal,
  workflowSnapshotJson,
  workflowSummaryMarkdown,
  workflowOverlay = null,
  onRunWorkflow,
  onOpenThread,
  interactionMode = "block",
}: WorkflowCardHostProps) {
  const overlayRootClassName =
    interactionMode === "overlay" ? "pointer-events-none" : "";
  const overlayActionClassName =
    interactionMode === "overlay" ? "pointer-events-auto" : "";
  const snapshot = parseWorkflowSnapshot(workflowSnapshotJson);
  const stages = snapshot?.stages ?? [];
  const rounds = snapshot?.rounds ?? [];
  const assistants = snapshot?.assistants ?? [];
  const participants = snapshot?.participants ?? [];
  const kind = snapshot?.kind || (stages.length > 0 ? "process" : "");
  const displayStages = stages.map((stage) => mergeStageOverlay(stage, workflowOverlay));
  const displayGoal = threadGoal.trim() || snapshot?.goal.trim() || "";
  const displayId = threadStageId || threadId || snapshot?.threadId || "Unlinked thread";
  const memberTitle = kind === "brainstorm" || kind === "debate"
    ? "Participants"
    : kind === "teamwork"
      ? "Team"
      : "";
  const displayRounds = rounds.slice(-3).reverse();

  const summary = workflowSummaryMarkdown
    .split(/\r?\n/)
    .map(line => line.trim())
    .filter(Boolean)
    .slice(0, 6)
    .join(" ");

  return (
    <div className={"h-full w-full overflow-hidden rounded-[20px] border border-ink/10 bg-surface-panel/95 text-ink/80 shadow-[0_16px_40px_rgba(18,24,33,0.08)] " + overlayRootClassName}>
      <div className="relative flex items-start justify-between gap-3 px-4 py-3 after:pointer-events-none after:absolute after:bottom-0 after:left-0 after:right-0 after:h-px after:bg-ink/10 after:content-['']">
        <div className="min-w-0">
          <div className="truncate text-body-sm font-medium text-ink/88">{title || "Thread"}</div>
          <div className="truncate font-mono text-[11px] text-ink/48">{displayId}</div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            onPointerDown={stopOverlayInteraction}
            onMouseDown={stopOverlayInteraction}
            onClick={(event) => {
              stopOverlayInteraction(event);
              onRunWorkflow();
            }}
            className={"rounded-md border border-ink/10 px-2 py-1 text-[11px] text-ink/62 transition hover:bg-ink/5 " + overlayActionClassName}
          >
            Run
          </button>
          <button
            type="button"
            onPointerDown={stopOverlayInteraction}
            onMouseDown={stopOverlayInteraction}
            onClick={(event) => {
              stopOverlayInteraction(event);
              onOpenThread();
            }}
            className={"rounded-md border border-ink/10 px-2 py-1 text-[11px] text-ink/62 transition hover:bg-ink/5 " + overlayActionClassName}
          >
            Thread
          </button>
        </div>
      </div>
      <div className="flex h-[calc(100%-57px)] min-h-0 flex-col gap-2 px-4 py-3">
        <div className="flex shrink-0 flex-wrap items-center gap-1.5 text-[11px] uppercase text-ink/40">
          <span>{executionState || "idle"}</span>
          {lastRunId && (
            <>
              <span className="h-1 w-1 rounded-full bg-ink/18" />
              <span className="truncate">{lastRunId}</span>
            </>
          )}
          {snapshot?.rollup && (
            <>
              <span className="h-1 w-1 rounded-full bg-ink/18" />
              <span>{snapshot.rollup.completed}/{snapshot.rollup.total} done</span>
              {snapshot.rollup.openIssues > 0 && <span>{snapshot.rollup.openIssues} open issues</span>}
            </>
          )}
          {kind && (
            <>
              <span className="h-1 w-1 rounded-full bg-ink/18" />
              <span>{kindLabel(kind)}</span>
            </>
          )}
          {workflowOverlay?.activeCount ? (
            <>
              <span className="h-1 w-1 rounded-full bg-blue/35" />
              <span>{workflowOverlay.activeCount} live</span>
            </>
          ) : null}
        </div>
        <div className="shrink-0 truncate text-caption leading-5 text-ink/68">
          {displayGoal || "Thread goal is not available yet."}
        </div>
        {workflowOverlay?.currentAction && (
          <div className="shrink-0 truncate rounded bg-blue/8 px-2 py-1 text-[11px] leading-4 text-blue">
            {workflowOverlay.currentAction}
          </div>
        )}
        {displayStages.length > 0 && kind === "process" ? (
          <div className="min-h-0 flex-1 overflow-y-auto pr-1">
            <div className="grid gap-1.5">
              {displayStages.map((stage) => (
                <WorkflowStageRow key={stage.threadStageId} stage={stage} />
              ))}
            </div>
          </div>
        ) : kind === "teamwork" || kind === "brainstorm" || kind === "debate" ? (
          <div className="min-h-0 flex-1 overflow-y-auto pr-1">
            <div className="grid gap-2">
              {memberTitle && (
                <ThreadMemberSection
                  title={memberTitle}
                  assistants={kind === "teamwork" ? assistants : []}
                  participants={kind === "brainstorm" || kind === "debate" ? participants : []}
                />
              )}
              {displayRounds.length > 0 ? (
                <ThreadRoundSection rounds={displayRounds} />
              ) : (
                <div className="line-clamp-4 text-caption leading-6 text-ink/58">
                  {summary || "Thread activity is not available yet."}
                </div>
              )}
            </div>
          </div>
        ) : (
          <div className="line-clamp-4 text-caption leading-6 text-ink/58">
            {summary || "Thread summary is not available yet."}
          </div>
        )}
      </div>
    </div>
  );
}

function ThreadMemberSection({
  title,
  assistants,
  participants,
}: {
  title: string;
  assistants: WorkflowSnapshotAssistantView[];
  participants: WorkflowSnapshotParticipantView[];
}) {
  const empty = assistants.length === 0 && participants.length === 0;
  return (
    <div className="min-w-0">
      <div className="mb-1 text-[11px] uppercase text-ink/38">{title}</div>
      {empty ? (
        <div className="rounded-md border border-ink/8 bg-card-panel/70 px-2 py-1.5 text-[11px] text-ink/45">
          No members configured
        </div>
      ) : (
        <div className="grid gap-1.5">
          {assistants.slice(0, 6).map((assistant) => (
            <AssistantMemberRow key={assistant.assistantId} assistant={assistant} />
          ))}
          {participants.slice(0, 6).map((participant) => (
            <ParticipantMemberRow key={participant.participantId} participant={participant} />
          ))}
        </div>
      )}
    </div>
  );
}

function AssistantMemberRow({ assistant }: { assistant: WorkflowSnapshotAssistantView }) {
  return (
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2 rounded-md border border-ink/8 bg-card-panel/70 px-2 py-1.5">
      <AssistantBotIcon color={assistant.color} className="h-5 w-5 shrink-0" />
      <div className="min-w-0">
        <div className="truncate text-caption font-medium text-ink/72">{assistant.name}</div>
        <div className="truncate text-[11px] leading-4 text-ink/42">{assistant.agentLabel ?? "Agent"}</div>
      </div>
      <span className="rounded border border-blue/15 bg-blue/8 px-1.5 py-0.5 text-[10px] leading-none text-blue">
        Team
      </span>
    </div>
  );
}

function ParticipantMemberRow({ participant }: { participant: WorkflowSnapshotParticipantView }) {
  return (
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2 rounded-md border border-ink/8 bg-card-panel/70 px-2 py-1.5">
      <span className="grid h-5 w-5 shrink-0 place-items-center rounded-full bg-ink/[0.06] text-[10px] font-medium text-ink/58">
        {participant.initial}
      </span>
      <div className="min-w-0">
        <div className="truncate text-caption font-medium text-ink/72">{participant.agentLabel}</div>
        <div className="truncate text-[11px] leading-4 text-ink/42">{participant.model || participant.name}</div>
      </div>
      <span className="rounded border border-accent-purple/15 bg-accent-purple/8 px-1.5 py-0.5 text-[10px] leading-none text-accent-purple">
        Lane
      </span>
    </div>
  );
}

function ThreadRoundSection({ rounds }: { rounds: WorkflowSnapshotRoundView[] }) {
  return (
    <div className="min-w-0">
      <div className="mb-1 text-[11px] uppercase text-ink/38">Rounds</div>
      <div className="grid gap-1.5">
        {rounds.map((round) => (
          <ThreadRoundRow key={round.roundId} round={round} />
        ))}
      </div>
    </div>
  );
}

function ThreadRoundRow({ round }: { round: WorkflowSnapshotRoundView }) {
  const visibleTasks = round.tasks.slice(0, 3);
  return (
    <div className="min-w-0 rounded-md border border-ink/8 bg-card-panel/70 px-2 py-1.5">
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-2">
        <div className="truncate text-caption font-medium text-ink/72">Round {round.roundIndex}</div>
        <div className="flex shrink-0 items-center gap-1">
          <span className="rounded border border-ink/10 bg-ink/[0.04] px-1.5 py-0.5 text-[10px] leading-none text-ink/45">
            {round.mode}
          </span>
          <span className={"rounded border px-1.5 py-0.5 text-[10px] leading-none " + taskStatusClassName(round.status)}>
            {statusLabel(round.status)}
          </span>
        </div>
      </div>
      {round.summary && <div className="mt-0.5 truncate text-[11px] leading-4 text-ink/42">{round.summary}</div>}
      {visibleTasks.length > 0 && (
        <div className="mt-1 grid gap-1">
          {visibleTasks.map((task) => (
            <ThreadTaskRow key={task.taskId} task={task} />
          ))}
        </div>
      )}
    </div>
  );
}

function ThreadTaskRow({ task }: { task: WorkflowSnapshotTaskView }) {
  const detail = task.error || task.resultSummary || task.targetLabel;
  return (
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-2 rounded bg-ink/[0.035] px-2 py-1">
      <div className="min-w-0">
        <div className="truncate text-[11px] leading-4 text-ink/62">{task.title}</div>
        {detail && <div className="truncate text-[10px] leading-3 text-ink/38">{detail}</div>}
      </div>
      <span className={"rounded border px-1.5 py-0.5 text-[10px] leading-none " + taskStatusClassName(task.status)}>
        {statusLabel(task.status)}
      </span>
    </div>
  );
}

type WorkflowStageDisplay = WorkflowSnapshotStageView & {
  currentAction?: string | null;
  activeAssistantIds?: string[];
  activeAgents?: string[];
};

function WorkflowStageRow({ stage }: { stage: WorkflowStageDisplay }) {
  const openIssueLabel = stage.openIssues === 1 ? "1 issue" : `${stage.openIssues} issues`;
  const stageDetail = stage.currentAction ?? stage.summary ?? stage.outcome;
  const activeAssistantIds = new Set(stage.activeAssistantIds ?? []);
  const activeAgents = new Set(stage.activeAgents ?? []);
  const orderedAssistants = stage.assistants
    .map((assistant, order) => ({
      assistant,
      order,
      active: assistantIsActive(assistant, activeAssistantIds, activeAgents),
    }))
    .sort((a, b) => Number(b.active) - Number(a.active) || a.order - b.order);
  const visibleAssistants = orderedAssistants.slice(0, 4);
  const extraAssistants = Math.max(0, stage.assistants.length - visibleAssistants.length);
  return (
    <div
      className={
        "min-w-0 rounded-md border px-2 py-1.5 " +
        (stage.focused || stage.active
          ? "border-emerald/25 bg-emerald/8"
          : "border-ink/8 bg-card-panel/70")
      }
    >
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-2">
        <div className="min-w-0 truncate text-caption font-medium text-ink/74">{stage.name}</div>
        <div className="flex shrink-0 items-center gap-1">
          {stage.openIssues > 0 && (
            <span className="rounded border border-status-warn/20 bg-status-warn/10 px-1.5 py-0.5 text-[10px] leading-none text-status-warn">
              {openIssueLabel}
            </span>
          )}
          <span className={"rounded border px-1.5 py-0.5 text-[10px] leading-none " + statusClassName(stage.status)}>
            {statusLabel(stage.status)}
          </span>
        </div>
      </div>
      {stageDetail && (
        <div className="mt-0.5 truncate text-[11px] leading-4 text-ink/42">{stageDetail}</div>
      )}
      {visibleAssistants.length > 0 && (
        <div className="mt-1.5 flex min-w-0 flex-wrap gap-1">
          {visibleAssistants.map(({ assistant, active }) => (
            <span
              key={assistant.assistantId}
              title={assistant.agentLabel ?? assistant.name}
              className={
                "inline-flex max-w-[120px] items-center gap-1 rounded px-1.5 py-0.5 text-[10px] leading-3 " +
                (active
                  ? "bg-blue/10 text-blue"
                  : "bg-ink/[0.055] text-ink/55")
              }
            >
              <span
                className={
                  "grid h-3.5 w-3.5 shrink-0 place-items-center rounded-full " +
                  (active ? "ring-2 ring-blue/25" : "")
                }
              >
                <AssistantBotIcon color={assistant.color} className="h-3.5 w-3.5 shrink-0" />
              </span>
              <span className="truncate">{assistant.name}</span>
            </span>
          ))}
          {extraAssistants > 0 && (
            <span className="rounded bg-ink/[0.055] px-1.5 py-0.5 text-[10px] leading-3 text-ink/45">
              +{extraAssistants}
            </span>
          )}
        </div>
      )}
    </div>
  );
}

function mergeStageOverlay(
  stage: WorkflowSnapshotStageView,
  workflowOverlay: WorkflowOverlay | null,
): WorkflowStageDisplay {
  const overlay = workflowOverlay?.stages[stage.threadStageId];
  if (!overlay) return stage;
  return {
    ...stage,
    status: overlay.status,
    active: stage.active || overlay.active,
    currentAction: overlay.currentAction,
    activeAssistantIds: overlay.activeAssistantIds,
    activeAgents: overlay.activeAgents,
  };
}

function assistantIsActive(
  assistant: WorkflowStageDisplay["assistants"][number],
  activeAssistantIds: Set<string>,
  activeAgents: Set<string>,
): boolean {
  return activeAssistantIds.has(assistant.assistantId)
    || Boolean(assistant.agent && activeAgents.has(assistant.agent));
}

function statusLabel(status: string) {
  switch (status) {
    case "not_started":
      return "Not started";
    case "in_progress":
      return "In progress";
    case "needs_review":
      return "Needs review";
    case "completed":
      return "Completed";
    case "blocked":
      return "Blocked";
    case "skipped":
      return "Skipped";
    default:
      return status.replace(/_/g, " ");
  }
}

function statusClassName(status: string) {
  switch (status) {
    case "completed":
      return "border-emerald/20 bg-emerald/10 text-emerald";
    case "in_progress":
      return "border-blue/20 bg-blue/10 text-blue";
    case "blocked":
      return "border-status-error/20 bg-status-error/10 text-status-error";
    case "needs_review":
      return "border-accent-purple/20 bg-accent-purple/10 text-accent-purple";
    case "skipped":
      return "border-ink/10 bg-ink/[0.04] text-ink/38";
    default:
      return "border-ink/10 bg-ink/[0.05] text-ink/45";
  }
}

function taskStatusClassName(status: string) {
  switch (status) {
    case "completed":
      return "border-emerald/20 bg-emerald/10 text-emerald";
    case "running":
    case "planned":
      return "border-blue/20 bg-blue/10 text-blue";
    case "failed":
    case "errored":
      return "border-status-error/20 bg-status-error/10 text-status-error";
    case "cancelled":
      return "border-ink/10 bg-ink/[0.04] text-ink/38";
    default:
      return "border-ink/10 bg-ink/[0.05] text-ink/45";
  }
}

function kindLabel(kind: string) {
  switch (kind) {
    case "process":
      return "Process";
    case "teamwork":
      return "Teamwork";
    case "brainstorm":
      return "Brainstorm";
    case "debate":
      return "Debate";
    default:
      return kind;
  }
}
