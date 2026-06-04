import type {
  Agent,
  StageInfo,
  StageStatus,
  ThreadInfo,
  ThreadWorkSnapshot,
  ThreadWorkSnapshotSessionRef,
  ThreadWorkSnapshotStage,
} from "./api";
import { sessionDisplayTitle } from "./appUtils";

const COMPLETED: StageStatus[] = ["completed", "skipped"];

function snapshotStage(stage: StageInfo): ThreadWorkSnapshotStage {
  return {
    threadStageId: stage.id,
    projectStageId: stage.stageId,
    name: stage.name ?? stage.kind ?? stage.stageId,
    kind: stage.kind,
    icon: stage.icon,
    status: stage.status,
    summary: stage.summary,
    outcome: stage.outcome,
    assistants: stage.assistants,
    issues: stage.issues ?? [],
    sessionRefs: stage.sessions.map((session) => ({
      agent: session.agent,
      sessionId: session.id,
      title: sessionDisplayTitle(session),
      filePath: session.filePath || null,
      sourceKind: "stage",
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
  const openIssues = stages.reduce(
    (total, stage) => total + (stage.issues ?? []).filter((issue) => issue.status === "open").length,
    0,
  );
  const threadSessionRefs = thread.sessions.map((session) => ({
    agent: session.agent,
    sessionId: session.id,
    title: sessionDisplayTitle(session),
    filePath: session.filePath || null,
    sourceKind: "thread" as const,
  }));
  const stageSessionRefs = stages.flatMap((stage) => stage.sessionRefs);
  const allSessionRefs = dedupeSessionRefs([...threadSessionRefs, ...stageSessionRefs]);
  return {
    threadId: thread.id,
    projectId: thread.projectId,
    goal: thread.goal,
    description: thread.description,
    activeStageId: thread.stageId,
    focusedStageId: focusedStage?.id ?? null,
    stages,
    threadSessionRefs,
    relatedContext: {
      sessionExcerptRefs: allSessionRefs,
    },
    detailRefs: {
      threadId: thread.id,
      focusedStageId: focusedStage?.id ?? null,
      stageIds: stages.map((stage) => stage.threadStageId),
      issueIds: stages.flatMap((stage) => (stage.issues ?? []).map((issue) => issue.id)),
      sessionRefs: allSessionRefs,
    },
    rollup: {
      completed,
      incomplete: stages.length - completed,
      blocked,
      openIssues,
      currentStage: focusedStage
        ? focusedStage.name ?? focusedStage.kind ?? focusedStage.stageId
        : null,
      total: stages.length,
    },
    capturedAt,
  };
}

function dedupeSessionRefs(refs: ThreadWorkSnapshotSessionRef[]): ThreadWorkSnapshotSessionRef[] {
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
export function renderThreadWorkContext(snapshot: ThreadWorkSnapshot, targetAgent?: Agent | null): string {
  const lines: string[] = [];
  lines.push("# Thread work-state snapshot");
  lines.push(`Goal: ${snapshot.goal}`);
  if (snapshot.description) lines.push(`Description: ${snapshot.description}`);
  lines.push(
    `Progress: ${snapshot.rollup.completed}/${snapshot.rollup.total} stages complete` +
      (snapshot.rollup.blocked > 0 ? `, ${snapshot.rollup.blocked} blocked` : "") +
      ((snapshot.rollup.openIssues ?? 0) > 0 ? `, ${snapshot.rollup.openIssues} open issues` : "") +
      (snapshot.rollup.currentStage ? `, current stage: ${snapshot.rollup.currentStage}` : ""),
  );
  lines.push("");
  lines.push("## Stages");
  for (const stage of snapshot.stages) {
    const focus = stage.threadStageId === snapshot.focusedStageId ? " <- you are here" : "";
    lines.push(`- ${statusLabel(stage.status)} ${stage.name}${focus}`);
    if (stage.summary) lines.push(`    summary: ${stage.summary}`);
    if (stage.outcome) lines.push(`    outcome: ${stage.outcome}`);
    for (const issue of (stage.issues ?? []).filter((item) => item.status === "open")) {
      lines.push(`    issue [${issue.severity}] ${issue.title}`);
      if (issue.description) lines.push(`      ${issue.description}`);
    }
    for (const ref of stage.sessionRefs) {
      lines.push(`    [${ref.agent}:${ref.sessionId}] ${ref.title ?? ""}`.trimEnd());
    }
  }
  const focusedStage = snapshot.stages.find((stage) => stage.threadStageId === snapshot.focusedStageId) ?? null;
  const assistantInstructions = focusedStage
    ? stageAssistantInstructions(focusedStage, targetAgent)
    : [];
  if (assistantInstructions.length > 0) {
    lines.push("");
    lines.push("## Stage assistant instructions");
    for (const instruction of assistantInstructions) {
      lines.push(`### ${instruction.name}`);
      lines.push(instruction.prompt);
    }
  }
  const focusedId = snapshot.focusedStageId;
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
    lines.push("To add a structured issue:");
    lines.push(
      `  sessio stage issue add --stage-id ${focusedId} --title "what is wrong" --severity medium --json`,
    );
    lines.push("(sessio resolves to ~/.sessio/bin/sessio)");
  }
  return lines.join("\n");
}

function stageAssistantInstructions(
  stage: ThreadWorkSnapshotStage,
  targetAgent?: Agent | null,
): { name: string; prompt: string }[] {
  if (!targetAgent) return [];
  return (stage.assistants ?? [])
    .filter((assistant) => assistant.agent.id === targetAgent)
    .map((assistant) => ({
      name: assistant.name,
      prompt: assistant.systemPrompt?.trim() ?? "",
    }))
    .filter((instruction) => instruction.prompt.length > 0);
}
