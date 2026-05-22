import type {
  Agent,
  AgentRuntimeEvent,
  RuntimeCapabilitySet,
  RuntimeError,
  RuntimeTransportKind,
  RuntimeTurnStatus,
} from "./api";

export interface LiveRuntimeState {
  sessions: Record<string, LiveRuntimeSession>;
  lastSequence: number;
}

export interface LiveRuntimeSession {
  sessioRuntimeSessionId: string;
  agent: Agent;
  agentRuntimeSessionId: string;
  transport: RuntimeTransportKind;
  workspacePath: string;
  capabilities: RuntimeCapabilitySet;
  turns: LiveTurn[];
  ended: boolean;
}

export interface LiveRuntimeStatus {
  sessioRuntimeSessionId: string;
  activeTurnId: string | null;
  ended: boolean;
}

export interface LiveTurn {
  turnId: string;
  status: RuntimeTurnStatus;
  userText: string;
  assistantText: string;
  reasoningText: string;
  parts: LiveMessagePart[];
  tools: LiveToolCall[];
  permissions: LivePermissionRequest[];
  error: RuntimeError | null;
  startedAt: number;
  updatedAt: number;
}

export type LiveMessagePart =
  | { kind: "user"; text: string }
  | { kind: "assistantText"; text: string }
  | { kind: "reasoning"; text: string }
  | { kind: "tool"; toolId: string }
  | { kind: "permission"; requestId: string }
  | { kind: "error"; error: RuntimeError };

export interface LiveToolCall {
  toolId: string;
  name: string;
  input: unknown | null;
  inputText: string;
  outputText: string;
}

export interface LivePermissionRequest {
  requestId: string;
  toolName: string;
  input: unknown | null;
  approved: boolean | null;
}

export const emptyLiveRuntimeState: LiveRuntimeState = {
  sessions: {},
  lastSequence: 0,
};

export function applyRuntimeEvent(
  state: LiveRuntimeState,
  event: AgentRuntimeEvent,
): LiveRuntimeState {
  if (event.sequence <= state.lastSequence) return state;

  const next: LiveRuntimeState = {
    sessions: { ...state.sessions },
    lastSequence: event.sequence,
  };

  if (event.kind === "sessionStarted") {
    next.sessions[event.sessioRuntimeSessionId] = {
      sessioRuntimeSessionId: event.sessioRuntimeSessionId,
      agent: event.agent,
      agentRuntimeSessionId: event.agentRuntimeSessionId,
      transport: event.transport,
      workspacePath: event.workspacePath,
      capabilities: event.capabilities,
      turns: next.sessions[event.sessioRuntimeSessionId]?.turns ?? [],
      ended: false,
    };
    return next;
  }

  const session = next.sessions[event.sessioRuntimeSessionId];
  if (!session) return next;

  if (event.kind === "sessionEnded") {
    next.sessions[event.sessioRuntimeSessionId] = { ...session, ended: true };
    return next;
  }

  const turns = session.turns.slice();
  const existingIndex =
    "turnId" in event ? turns.findIndex((turn) => turn.turnId === event.turnId) : -1;
  const turn =
    existingIndex >= 0
      ? cloneTurn(turns[existingIndex])
      : "turnId" in event
        ? newTurn(event.turnId, event.timestamp)
        : null;
  if (!turn) return next;

  turn.updatedAt = event.timestamp;

  switch (event.kind) {
    case "turnStarted":
      turn.status = "streaming";
      break;
    case "userMessage":
      turn.userText = event.text;
      turn.parts.push({ kind: "user", text: event.text });
      break;
    case "textDelta":
      turn.assistantText += event.text;
      appendTextPart(turn.parts, "assistantText", event.text);
      turn.status = "streaming";
      break;
    case "reasoningDelta":
      turn.reasoningText += event.text;
      appendTextPart(turn.parts, "reasoning", event.text);
      break;
    case "toolStarted":
      turn.tools.push({
        toolId: event.toolId,
        name: event.name,
        input: event.input,
        inputText: "",
        outputText: "",
      });
      turn.parts.push({ kind: "tool", toolId: event.toolId });
      break;
    case "toolInputDelta":
      upsertTool(turn, event.toolId).inputText += event.delta;
      break;
    case "toolOutputDelta":
      upsertTool(turn, event.toolId).outputText += event.delta;
      break;
    case "permissionRequested":
      turn.permissions.push({
        requestId: event.requestId,
        toolName: event.toolName,
        input: event.input,
        approved: null,
      });
      turn.parts.push({ kind: "permission", requestId: event.requestId });
      break;
    case "permissionResolved":
      turn.permissions = turn.permissions.map((permission) =>
        permission.requestId === event.requestId
          ? { ...permission, approved: event.approved }
          : permission,
      );
      break;
    case "turnCompleted":
      turn.status = "completed";
      break;
    case "turnError":
      turn.status = "failed";
      turn.error = event.error;
      turn.parts.push({ kind: "error", error: event.error });
      break;
    case "turnCancelled":
      turn.status = "cancelled";
      break;
    default:
      break;
  }

  if (existingIndex >= 0) {
    turns[existingIndex] = turn;
  } else {
    turns.push(turn);
  }
  next.sessions[event.sessioRuntimeSessionId] = { ...session, turns };
  return next;
}

function newTurn(turnId: string, timestamp: number): LiveTurn {
  return {
    turnId,
    status: "pending",
    userText: "",
    assistantText: "",
    reasoningText: "",
    parts: [],
    tools: [],
    permissions: [],
    error: null,
    startedAt: timestamp,
    updatedAt: timestamp,
  };
}

function cloneTurn(turn: LiveTurn): LiveTurn {
  return {
    ...turn,
    parts: turn.parts.map((part) => ({ ...part })),
    tools: turn.tools.map((tool) => ({ ...tool })),
    permissions: turn.permissions.map((permission) => ({ ...permission })),
  };
}

function appendTextPart(
  parts: LiveMessagePart[],
  kind: "assistantText" | "reasoning",
  text: string,
): void {
  const last = parts[parts.length - 1];
  if (last?.kind === kind) {
    last.text += text;
    return;
  }
  parts.push({ kind, text });
}

function upsertTool(turn: LiveTurn, toolId: string): LiveToolCall {
  let tool = turn.tools.find((item) => item.toolId === toolId);
  if (!tool) {
    tool = {
      toolId,
      name: "tool",
      input: null,
      inputText: "",
      outputText: "",
    };
    turn.tools.push(tool);
  }
  return tool;
}
