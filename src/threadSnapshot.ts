import type {
  Agent,
  AstraHandle,
  PlanRoundInfo,
  PlanTaskInfo,
  StageInfo,
  StageStatus,
  ThreadInfo,
  ThreadWorkSnapshot,
  ThreadWorkSnapshotSessionRef,
  ThreadWorkSnapshotStage,
} from "./api";
import { sessionDisplayTitle } from "./appUtils";
import { buildSessioThreadPromptBlock } from "./historyMerge";

const COMPLETED: StageStatus[] = ["completed", "skipped"];
const MAX_CONTEXT_FIELD_CHARS = 700;

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
    kind: thread.kind,
    assistants: thread.assistants,
    agentParticipants: thread.agentParticipants,
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
  const stages = snapshot.stages ?? [];
  const rollup = snapshot.rollup ?? {
    completed: 0,
    incomplete: stages.length,
    blocked: stages.filter((stage) => stage.status === "blocked").length,
    openIssues: stages.reduce(
      (total, stage) => total + (stage.issues ?? []).filter((issue) => issue.status === "open").length,
      0,
    ),
    currentStage: null,
    total: stages.length,
  };
  lines.push("# Thread work-state snapshot");
  lines.push(`Goal: ${snapshot.goal}`);
  if (snapshot.description) lines.push(`Description: ${snapshot.description}`);
  lines.push(
    `Progress: ${rollup.completed}/${rollup.total} stages complete` +
      (rollup.blocked > 0 ? `, ${rollup.blocked} blocked` : "") +
      ((rollup.openIssues ?? 0) > 0 ? `, ${rollup.openIssues} open issues` : "") +
      (rollup.currentStage ? `, current stage: ${rollup.currentStage}` : ""),
  );
  lines.push("");
  lines.push("## Stages");
  for (const stage of stages) {
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
  const focusedStage = stages.find((stage) => stage.threadStageId === snapshot.focusedStageId) ?? null;
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
    lines.push("Use `~/.sessio/bin/sessio` for reliable CLI access; `sessio` is also acceptable if it is on PATH.");
    lines.push("When you begin work:");
    lines.push(`  ~/.sessio/bin/sessio stage set-status --id ${focusedId} --status in_progress --json`);
    lines.push("When the stage is complete:");
    lines.push(`  ~/.sessio/bin/sessio stage set-status --id ${focusedId} --status completed --json`);
    lines.push("If blocked, record why:");
    lines.push(
      `  ~/.sessio/bin/sessio stage set-status --id ${focusedId} --status blocked --summary "what is blocking" --json`,
    );
    lines.push("To add a structured issue:");
    lines.push(
      `  ~/.sessio/bin/sessio stage issue add --stage-id ${focusedId} --title "what is wrong" --severity medium --json`,
    );
  }
  return buildSessioThreadPromptBlock("work_context", lines.join("\n"), {
    thread_id: snapshot.threadId,
    focused_stage_id: snapshot.focusedStageId,
    target_agent: targetAgent,
  });
}

export function renderThreadOrchestrationContext({
  threadId,
  astraRuns,
  planRounds,
}: {
  threadId: string;
  astraRuns: AstraHandle[];
  planRounds: PlanRoundInfo[];
}): string {
  if (astraRuns.length === 0 && planRounds.length === 0) return "";

  const lines: string[] = [];
  lines.push("# Thread orchestration snapshot");

  if (astraRuns.length > 0) {
    lines.push("");
    lines.push("## Astra runs");
    for (const run of astraRuns.slice().sort((a, b) => a.createdAt - b.createdAt)) {
      lines.push(
        `- [${run.status}] ${run.runId}` +
          (run.roundIndex !== null ? ` round ${run.roundIndex}` : "") +
          (run.plannerBackend ? ` via ${run.plannerBackend}` : ""),
      );
      if (run.terminalReason) lines.push(`    terminal reason: ${compactContextField(run.terminalReason)}`);
      if (run.lastErrorMessage) lines.push(`    last error: ${compactContextField(run.lastErrorMessage)}`);
      else if (run.error) lines.push(`    error: ${compactContextField(run.error)}`);
      if (run.internalPlannerSessionIds.length > 0) {
        lines.push(`    planner sessions: ${run.internalPlannerSessionIds.join(", ")}`);
      }
    }
  }

  if (planRounds.length > 0) {
    lines.push("");
    lines.push("## Plan rounds");
    for (const round of planRounds.slice().sort((a, b) => a.roundIndex - b.roundIndex || a.createdAt - b.createdAt)) {
      lines.push(
        `- Round ${round.roundIndex} [${round.status}] ${round.mode}` +
          (round.astraRunId ? ` run=${round.astraRunId}` : "") +
          ` source=${round.source}`,
      );
      if (round.summary) lines.push(`    summary: ${compactContextField(round.summary)}`);
      for (const task of round.tasks.slice().sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt - b.createdAt)) {
        lines.push(`    - ${taskLine(task)}`);
        if (task.resultSummary) lines.push(`        result: ${compactContextField(task.resultSummary)}`);
        if (task.error) lines.push(`        error: ${compactContextField(task.error)}`);
        if (task.sessions.length > 0) {
          lines.push(
            `        sessions: ${task.sessions
              .map((session) => `${session.role}:${session.agent}/${session.sessionId}`)
              .join(", ")}`,
          );
        }
      }
    }
  }

  return buildSessioThreadPromptBlock("orchestration_context", lines.join("\n"), {
    thread_id: threadId,
  });
}

function taskLine(task: PlanTaskInfo): string {
  return `[${task.status}] ${task.title}` +
    ` -> ${task.targetAgent}` +
    (task.assistantId ? ` assistant=${task.assistantId}` : "") +
    (task.threadStageId ? ` stage=${task.threadStageId}` : "") +
    (task.expectedOutput ? `; expected: ${compactContextField(task.expectedOutput)}` : "");
}

function compactContextField(value: string): string {
  const compact = value.replace(/\s+/g, " ").trim();
  if (compact.length <= MAX_CONTEXT_FIELD_CHARS) return compact;
  return `${compact.slice(0, MAX_CONTEXT_FIELD_CHARS - 3).trimEnd()}...`;
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
