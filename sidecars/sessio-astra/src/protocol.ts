export const PROTOCOL_VERSION = 1;

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };

export interface ProtocolRequest {
  protocolVersion: number;
  id: string;
  method: string;
  params?: Record<string, unknown>;
}

export interface ProtocolResponse {
  protocolVersion: number;
  id: string;
  result?: unknown;
  error?: ProtocolError;
}

export interface ProtocolEvent {
  protocolVersion: number;
  method: "event";
  params: {
    runId: string;
    type: string;
    data?: unknown;
  };
}

export interface ToolCallRequest {
  protocolVersion: number;
  id: string;
  method: "tool/call";
  params: {
    runId: string;
    name: string;
    args?: Record<string, unknown>;
  };
}

export interface ProtocolError {
  code: string;
  message: string;
  data?: unknown;
}

export interface StartParams {
  runId: string;
  thread: ThreadSnapshot;
  snapshot?: unknown;
  prompt?: string | null;
}

export interface ConfirmParams {
  runId: string;
  approvedTaskIds: string[];
  tasks?: AstraTaskProposal[];
}

export interface CancelParams {
  runId: string;
}

export interface TaskResultParams {
  runId: string;
  result: {
    taskId: string;
    sessioRuntimeSessionId: string;
    turnId?: string | null;
    status: "completed" | "failed" | "errored" | "cancelled";
    output?: string;
    error?: string | null;
    completedAt?: number;
  };
}

export interface ThreadSnapshot {
  id: string;
  projectId: string;
  goal: string;
  description?: string | null;
  stages?: StageSnapshot[];
  sessions?: unknown[];
}

export interface StageSnapshot {
  id: string;
  stageId?: string;
  name?: string | null;
  description?: string | null;
  status?: string;
  order?: number;
  assistants?: AssistantSnapshot[];
  issues?: unknown[];
}

export interface AssistantSnapshot {
  assistantId?: string;
  name?: string;
  agent?: {
    id?: string;
    name?: string;
  };
}

export interface AstraTaskProposal {
  id: string;
  title: string;
  targetStageId?: string | null;
  targetAgent: "codex" | "claude" | "gemini";
  prompt: string;
  expectedOutput: string;
  risk: "low" | "medium" | "high";
}

export interface AstraTaskResult {
  taskId: string;
  threadStageId?: string | null;
  sessioRuntimeSessionId: string;
  turnId?: string | null;
  status: "completed" | "failed" | "errored" | "cancelled";
  output?: string;
  error?: string | null;
  attemptCount?: number;
  retryLimitReached?: boolean;
  completedAt?: number;
}

export interface AstraStageMutationResult {
  ok: boolean;
  stage?: unknown;
  issue?: unknown;
  error?: string | null;
  appliedAt?: number;
}

export interface AstraPlan {
  summary: string;
  tasks: AstraTaskProposal[];
}

export function isRequest(value: unknown): value is ProtocolRequest {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<ProtocolRequest>;
  return (
    candidate.protocolVersion === PROTOCOL_VERSION &&
    typeof candidate.id === "string" &&
    candidate.id.length > 0 &&
    typeof candidate.method === "string" &&
    candidate.method.length > 0
  );
}

export function response(id: string, result: unknown = {}): ProtocolResponse {
  return { protocolVersion: PROTOCOL_VERSION, id, result };
}

export function errorResponse(id: string, code: string, message: string, data?: unknown): ProtocolResponse {
  return {
    protocolVersion: PROTOCOL_VERSION,
    id,
    error: data === undefined ? { code, message } : { code, message, data },
  };
}

export function event(runId: string, type: string, data?: unknown): ProtocolEvent {
  return {
    protocolVersion: PROTOCOL_VERSION,
    method: "event",
    params: { runId, type, data },
  };
}
