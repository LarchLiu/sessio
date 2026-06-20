export interface WorkflowCardHostProps {
  title: string;
  threadId: string;
  threadStageId: string;
  executionState: string;
  lastRunId: string;
  threadGoal: string;
  workflowSummaryMarkdown: string;
  onRunWorkflow: () => void;
  onOpenThread: () => void;
}

export function WorkflowCardHost({
  title,
  threadId,
  threadStageId,
  executionState,
  lastRunId,
  threadGoal,
  workflowSummaryMarkdown,
  onRunWorkflow,
  onOpenThread,
}: WorkflowCardHostProps) {
  const summary = workflowSummaryMarkdown
    .split(/\r?\n/)
    .map(line => line.trim())
    .filter(Boolean)
    .slice(0, 6)
    .join(" ");

  return (
    <div className="h-full w-full overflow-hidden rounded-[20px] border border-ink/10 bg-surface-panel/95 text-ink/80 shadow-[0_16px_40px_rgba(18,24,33,0.08)]">
      <div className="flex items-start justify-between gap-3 border-b border-ink/8 px-4 py-3">
        <div className="min-w-0">
          <div className="truncate text-body-sm font-medium text-ink/88">{title || "Workflow"}</div>
          <div className="truncate font-mono text-[11px] text-ink/48">{threadStageId || threadId || "Unlinked workflow"}</div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            onClick={onRunWorkflow}
            className="rounded-md border border-ink/10 px-2 py-1 text-[11px] text-ink/62 transition hover:bg-ink/5"
          >
            Run
          </button>
          <button
            type="button"
            onClick={onOpenThread}
            className="rounded-md border border-ink/10 px-2 py-1 text-[11px] text-ink/62 transition hover:bg-ink/5"
          >
            Thread
          </button>
        </div>
      </div>
      <div className="flex h-[calc(100%-57px)] flex-col gap-3 px-4 py-3">
        <div className="flex flex-wrap items-center gap-2 text-[11px] uppercase tracking-[0.08em] text-ink/40">
          <span>{executionState || "idle"}</span>
          {lastRunId && (
            <>
              <span className="h-1 w-1 rounded-full bg-ink/18" />
              <span className="truncate">{lastRunId}</span>
            </>
          )}
        </div>
        <div className="line-clamp-2 text-caption leading-6 text-ink/68">
          {threadGoal.trim() || "This card mirrors the linked thread workflow and can trigger a new run."}
        </div>
        <div className="line-clamp-4 text-caption leading-6 text-ink/58">
          {summary || "Workflow summary is not available yet."}
        </div>
      </div>
    </div>
  );
}
