import {
  AGENT_LABEL,
  isAgent,
  type Agent,
  type PlanRoundStatus,
  type PlanTaskStatus,
  type StageStatus,
  type ThreadKind,
} from "../../../../api";

export interface WorkflowSnapshotAssistantView {
  assistantId: string;
  name: string;
  color: string | null;
  agent: Agent | null;
  agentLabel: string | null;
  initial: string;
  order: number;
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

export interface WorkflowSnapshotParticipantView {
  participantId: string;
  name: string;
  agent: Agent;
  agentLabel: string;
  model: string;
  effort: string | null;
  permissionMode: string | null;
  initial: string;
  order: number;
}

export interface WorkflowSnapshotTaskView {
  taskId: string;
  title: string;
  status: PlanTaskStatus | string;
  targetAgent: Agent | null;
  targetLabel: string;
  assistantId: string | null;
  agentParticipantId: string | null;
  resultSummary: string | null;
  error: string | null;
}

export interface WorkflowSnapshotRoundView {
  roundId: string;
  roundIndex: number;
  status: PlanRoundStatus | string;
  mode: string;
  summary: string | null;
  tasks: WorkflowSnapshotTaskView[];
}

export interface WorkflowSnapshotView {
  threadId: string;
  kind: ThreadKind | "";
  goal: string;
  focusedStageId: string | null;
  activeStageId: string | null;
  assistants: WorkflowSnapshotAssistantView[];
  participants: WorkflowSnapshotParticipantView[];
  stages: WorkflowSnapshotStageView[];
  rounds: WorkflowSnapshotRoundView[];
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
  const kind = threadKindToView(pickString(record.kind));
  const goal = pickString(record.goal) ?? "";
  const focusedStageId = pickString(record.focusedStageId);
  const activeStageId = pickString(record.activeStageId);
  const assistants = Array.isArray(record.assistants)
    ? record.assistants
        .map(assistantToView)
        .filter((assistant): assistant is WorkflowSnapshotAssistantView => Boolean(assistant))
        .sort((a, b) => a.order - b.order)
    : [];
  const participants = Array.isArray(record.agentParticipants)
    ? record.agentParticipants
        .map(participantToView)
        .filter((participant): participant is WorkflowSnapshotParticipantView => Boolean(participant))
        .sort((a, b) => a.order - b.order)
    : [];
  const stages = Array.isArray(record.stages)
    ? record.stages
        .map((stage) => stageToView(stage, focusedStageId, activeStageId))
        .filter((stage): stage is WorkflowSnapshotStageView => Boolean(stage))
    : [];
  const rounds = Array.isArray(record.planRounds)
    ? record.planRounds
        .map(roundToView)
        .filter((round): round is WorkflowSnapshotRoundView => Boolean(round))
        .sort((a, b) => a.roundIndex - b.roundIndex)
    : [];

  return {
    threadId,
    kind,
    goal,
    focusedStageId,
    activeStageId,
    assistants,
    participants,
    stages,
    rounds,
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
    order: pickNumber(record.order) ?? 0,
  };
}

function participantToView(value: unknown): WorkflowSnapshotParticipantView | null {
  const record = asRecord(value);
  if (!record) return null;
  const participantId = pickString(record.participantId);
  const agentId = pickString(record.agent);
  const model = pickString(record.model) ?? "";
  if (!participantId || !isAgent(agentId)) return null;
  const agentLabel = AGENT_LABEL[agentId];
  const name = model ? `${agentLabel} ${model}` : agentLabel;
  return {
    participantId,
    name,
    agent: agentId,
    agentLabel,
    model,
    effort: pickString(record.effort),
    permissionMode: pickString(record.permissionMode),
    initial: agentLabel.trim().charAt(0).toUpperCase() || "?",
    order: pickNumber(record.order) ?? 0,
  };
}

function roundToView(value: unknown): WorkflowSnapshotRoundView | null {
  const record = asRecord(value);
  if (!record) return null;
  const roundId = pickString(record.id);
  if (!roundId) return null;
  return {
    roundId,
    roundIndex: pickNumber(record.roundIndex) ?? 0,
    status: pickString(record.status) ?? "planned",
    mode: pickString(record.mode) ?? "parallel",
    summary: pickString(record.summary),
    tasks: Array.isArray(record.tasks)
      ? record.tasks
          .map(taskToView)
          .filter((task): task is WorkflowSnapshotTaskView => Boolean(task))
          .sort((a, b) => a.title.localeCompare(b.title))
      : [],
  };
}

function taskToView(value: unknown): WorkflowSnapshotTaskView | null {
  const record = asRecord(value);
  if (!record) return null;
  const taskId = pickString(record.id);
  if (!taskId) return null;
  const targetAgent = pickString(record.targetAgent);
  const validTargetAgent = isAgent(targetAgent) ? targetAgent : null;
  return {
    taskId,
    title: pickString(record.title) ?? "Task",
    status: pickString(record.status) ?? "planned",
    targetAgent: validTargetAgent,
    targetLabel: validTargetAgent ? AGENT_LABEL[validTargetAgent] : pickString(record.targetAgent) ?? "Agent",
    assistantId: pickString(record.assistantId),
    agentParticipantId: pickString(record.agentParticipantId),
    resultSummary: pickString(record.resultSummary),
    error: pickString(record.error),
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

function threadKindToView(value: string | null): WorkflowSnapshotView["kind"] {
  switch (value) {
    case "process":
    case "teamwork":
    case "brainstorm":
    case "debate":
      return value;
    default:
      return "";
  }
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
