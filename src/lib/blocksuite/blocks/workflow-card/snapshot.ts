import { isAgent, type Agent, type StageStatus } from "../../../../api";

export interface WorkflowSnapshotAssistantView {
  assistantId: string;
  name: string;
  color: string | null;
  agent: Agent | null;
  agentLabel: string | null;
  initial: string;
}

export interface WorkflowSnapshotStageView {
  threadStageId: string;
  name: string;
  status: StageStatus | string;
  summary: string | null;
  outcome: string | null;
  openIssues: number;
  assistants: WorkflowSnapshotAssistantView[];
  focused: boolean;
  active: boolean;
}

export interface WorkflowSnapshotRollupView {
  completed: number;
  total: number;
  blocked: number;
  openIssues: number;
  currentStage: string | null;
}

export interface WorkflowSnapshotView {
  threadId: string;
  goal: string;
  focusedStageId: string | null;
  activeStageId: string | null;
  stages: WorkflowSnapshotStageView[];
  rollup: WorkflowSnapshotRollupView | null;
}

export function parseWorkflowSnapshot(workflowSnapshotJson: string): WorkflowSnapshotView | null {
  const trimmed = workflowSnapshotJson.trim();
  if (!trimmed) return null;
  try {
    return workflowSnapshotToView(JSON.parse(trimmed));
  } catch {
    return null;
  }
}

export function workflowSnapshotToView(value: unknown): WorkflowSnapshotView | null {
  const record = asRecord(value);
  if (!record) return null;

  const threadId = pickString(record.threadId) ?? "";
  const goal = pickString(record.goal) ?? "";
  const focusedStageId = pickString(record.focusedStageId);
  const activeStageId = pickString(record.activeStageId);
  const stages = Array.isArray(record.stages)
    ? record.stages
        .map((stage) => stageToView(stage, focusedStageId, activeStageId))
        .filter((stage): stage is WorkflowSnapshotStageView => Boolean(stage))
    : [];

  return {
    threadId,
    goal,
    focusedStageId,
    activeStageId,
    stages,
    rollup: rollupToView(record.rollup, stages),
  };
}

function stageToView(
  value: unknown,
  focusedStageId: string | null,
  activeStageId: string | null,
): WorkflowSnapshotStageView | null {
  const record = asRecord(value);
  if (!record) return null;
  const threadStageId = pickString(record.threadStageId);
  const name = pickString(record.name);
  if (!threadStageId || !name) return null;
  const issues = Array.isArray(record.issues) ? record.issues : [];
  const assistants = Array.isArray(record.assistants)
    ? record.assistants
        .map(assistantToView)
        .filter((assistant): assistant is WorkflowSnapshotAssistantView => Boolean(assistant))
    : [];
  return {
    threadStageId,
    name,
    status: pickString(record.status) ?? "not_started",
    summary: pickString(record.summary),
    outcome: pickString(record.outcome),
    openIssues: issues.filter(isOpenIssue).length,
    assistants,
    focused: focusedStageId === threadStageId,
    active: activeStageId === threadStageId,
  };
}

function assistantToView(value: unknown): WorkflowSnapshotAssistantView | null {
  const record = asRecord(value);
  if (!record) return null;
  const assistantId = pickString(record.assistantId);
  const name = pickString(record.name);
  if (!assistantId || !name) return null;
  const agent = asRecord(record.agent);
  const agentId = pickString(agent?.id);
  const agentLabel = pickString(agent?.name) ?? pickString(agent?.id);
  return {
    assistantId,
    name,
    color: pickString(record.color),
    agent: isAgent(agentId) ? agentId : null,
    agentLabel,
    initial: name.trim().charAt(0).toUpperCase() || "?",
  };
}

function rollupToView(
  value: unknown,
  stages: WorkflowSnapshotStageView[],
): WorkflowSnapshotRollupView | null {
  const record = asRecord(value);
  const total = pickNumber(record?.total) ?? stages.length;
  if (total === 0 && stages.length === 0 && !record) return null;
  return {
    completed: pickNumber(record?.completed) ?? stages.filter((stage) => stage.status === "completed").length,
    total,
    blocked: pickNumber(record?.blocked) ?? stages.filter((stage) => stage.status === "blocked").length,
    openIssues: pickNumber(record?.openIssues) ?? stages.reduce((sum, stage) => sum + stage.openIssues, 0),
    currentStage: pickString(record?.currentStage)
      ?? stages.find((stage) => stage.active || stage.focused)?.name
      ?? null,
  };
}

function isOpenIssue(value: unknown): boolean {
  const record = asRecord(value);
  return record?.status === "open";
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function pickString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function pickNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}
