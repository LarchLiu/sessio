import { parseWorkflowSnapshot, type WorkflowSnapshotStageView } from "./snapshot";
import type { WorkflowOverlay } from "../../workflowLiveProjection";

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
  const displayStages = stages.map((stage) => mergeStageOverlay(stage, workflowOverlay));
  const displayGoal = threadGoal.trim() || snapshot?.goal.trim() || "";
  const displayId = threadStageId || threadId || snapshot?.threadId || "Unlinked workflow";

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
          <div className="truncate text-body-sm font-medium text-ink/88">{title || "Workflow"}</div>
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
          {workflowOverlay?.activeCount ? (
            <>
              <span className="h-1 w-1 rounded-full bg-blue/35" />
              <span>{workflowOverlay.activeCount} live</span>
            </>
          ) : null}
        </div>
        <div className="shrink-0 truncate text-caption leading-5 text-ink/68">
          {displayGoal || "Workflow goal is not available yet."}
        </div>
        {workflowOverlay?.currentAction && (
          <div className="shrink-0 truncate rounded bg-blue/8 px-2 py-1 text-[11px] leading-4 text-blue">
            {workflowOverlay.currentAction}
          </div>
        )}
        {displayStages.length > 0 ? (
          <div className="min-h-0 flex-1 overflow-y-auto pr-1">
            <div className="grid gap-1.5">
              {displayStages.map((stage) => (
                <WorkflowStageRow key={stage.threadStageId} stage={stage} />
              ))}
            </div>
          </div>
        ) : (
          <div className="line-clamp-4 text-caption leading-6 text-ink/58">
            {summary || "Workflow summary is not available yet."}
          </div>
        )}
      </div>
    </div>
  );
}

type WorkflowStageDisplay = WorkflowSnapshotStageView & {
  currentAction?: string | null;
  activeAssistantIds?: string[];
};

function WorkflowStageRow({ stage }: { stage: WorkflowStageDisplay }) {
  const openIssueLabel = stage.openIssues === 1 ? "1 issue" : `${stage.openIssues} issues`;
  const stageDetail = stage.currentAction ?? stage.summary ?? stage.outcome;
  const visibleAssistants = stage.assistants.slice(0, 4);
  const extraAssistants = stage.assistants.length - visibleAssistants.length;
  const activeAssistantIds = new Set(stage.activeAssistantIds ?? []);
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
          {visibleAssistants.map((assistant) => (
            <span
              key={assistant.assistantId}
              title={assistant.agentLabel ?? assistant.name}
              className={
                "inline-flex max-w-[120px] items-center gap-1 rounded px-1.5 py-0.5 text-[10px] leading-3 " +
                (activeAssistantIds.has(assistant.assistantId)
                  ? "bg-blue/10 text-blue"
                  : "bg-ink/[0.055] text-ink/55")
              }
            >
              <span
                className={
                  "grid h-3.5 w-3.5 shrink-0 place-items-center rounded-full text-[8px] font-medium text-white " +
                  (activeAssistantIds.has(assistant.assistantId) ? "ring-2 ring-blue/25" : "")
                }
                style={{ backgroundColor: assistant.color ?? "rgb(var(--color-blue))" }}
              >
                {assistant.initial}
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
  };
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
