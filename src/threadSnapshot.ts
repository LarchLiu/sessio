import type {
  StageInfo,
  StageStatus,
  ThreadInfo,
  ThreadWorkSnapshot,
  ThreadWorkSnapshotStage,
} from "./api";

const COMPLETED: StageStatus[] = ["completed", "skipped"];

function snapshotStage(stage: StageInfo): ThreadWorkSnapshotStage {
  return {
    threadStageId: stage.id,
    name: stage.name ?? stage.kind ?? stage.stageId,
    kind: stage.kind,
    status: stage.status,
    summary: stage.summary,
    outcome: stage.outcome,
    sessionRefs: stage.sessions.map((session) => ({
      agent: session.agent,
      sessionId: session.id,
      title: session.title ?? session.firstUserMessage,
    })),
  };
}

/**
 * Build the structured work-state envelope captured the moment a thread/stage
 * chat is created. Reads explicit stage status (Phase 1) — no inference.
 */
export function buildThreadWorkSnapshot(
  thread: ThreadInfo,
  focusedStage: StageInfo | null,
  capturedAt: number,
): ThreadWorkSnapshot {
  const stages = thread.stages
    .slice()
    .sort((a, b) => a.order - b.order)
    .map(snapshotStage);
  const completed = stages.filter((stage) => COMPLETED.includes(stage.status)).length;
  const blocked = stages.filter((stage) => stage.status === "blocked").length;
  return {
    threadId: thread.id,
    projectId: thread.projectId,
    goal: thread.goal,
    description: thread.description,
    activeStageId: thread.stageId,
    focusedStageId: focusedStage?.id ?? null,
    stages,
    rollup: {
      completed,
      incomplete: stages.length - completed,
      blocked,
      total: stages.length,
    },
    capturedAt,
  };
}

function statusLabel(status: StageStatus): string {
  switch (status) {
    case "completed":
      return "[done]";
    case "in_progress":
      return "[active]";
    case "blocked":
      return "[blocked]";
    case "needs_review":
      return "[needs review]";
    case "skipped":
      return "[skipped]";
    case "not_started":
    default:
      return "[not started]";
  }
}

/**
 * Render the agent-facing context markdown. Tells the agent where it is working
 * (threadStageId) and how to report progress back through the Sessio CLI.
 */
export function renderThreadWorkContext(snapshot: ThreadWorkSnapshot): string {
  const lines: string[] = [];
  lines.push("# Thread work-state snapshot");
  lines.push(`Goal: ${snapshot.goal}`);
  if (snapshot.description) lines.push(`Description: ${snapshot.description}`);
  lines.push(
    `Progress: ${snapshot.rollup.completed}/${snapshot.rollup.total} stages complete` +
      (snapshot.rollup.blocked > 0 ? `, ${snapshot.rollup.blocked} blocked` : ""),
  );
  lines.push("");
  lines.push("## Stages");
  for (const stage of snapshot.stages) {
    const focus = stage.threadStageId === snapshot.focusedStageId ? " <- you are here" : "";
    lines.push(`- ${statusLabel(stage.status)} ${stage.name}${focus}`);
    if (stage.summary) lines.push(`    summary: ${stage.summary}`);
    if (stage.outcome) lines.push(`    outcome: ${stage.outcome}`);
    for (const ref of stage.sessionRefs) {
      lines.push(`    [${ref.agent}:${ref.sessionId}] ${ref.title ?? ""}`.trimEnd());
    }
  }

  const focusedId = snapshot.focusedStageId ?? snapshot.activeStageId;
  if (focusedId) {
    lines.push("");
    lines.push("## Report progress back to Sessio");
    lines.push(`You are working in Sessio thread stage ${focusedId}.`);
    lines.push("When you begin work:");
    lines.push(`  sessio stage set-status --id ${focusedId} --status in_progress --json`);
    lines.push("When the stage is complete:");
    lines.push(`  sessio stage set-status --id ${focusedId} --status completed --json`);
    lines.push("If blocked, record why:");
    lines.push(
      `  sessio stage set-status --id ${focusedId} --status blocked --summary "what is blocking" --json`,
    );
    lines.push("(sessio resolves to ~/.sessio/bin/sessio)");
  }
  return lines.join("\n");
}
