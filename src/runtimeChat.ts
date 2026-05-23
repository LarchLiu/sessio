import type {
  Agent,
  AgentRuntimeEvent,
  RuntimeCapabilitySet,
  RuntimeError,
  RuntimeTransportKind,
  RuntimeTurnStatus,
} from "./api";

export type LiveRuntimeAction =
  | { type: "runtime-event"; event: AgentRuntimeEvent }
  | {
      type: "ensure-session";
      session: LiveRuntimeSession;
    }
  | {
      type: "optimistic-user-message";
      sessioRuntimeSessionId: string;
      turnId: string;
      text: string;
      timestamp: number;
    }
  | {
      type: "replace-turn-id";
      sessioRuntimeSessionId: string;
      from: string;
      to: string;
    }
  | {
      type: "turn-error";
      sessioRuntimeSessionId: string;
      turnId: string;
      error: RuntimeError;
      timestamp: number;
    }
  | {
      type: "reconcile-indexed-session";
      sessioRuntimeSessionId: string;
      indexedThrough: number;
    };

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

export type LiveSessionActivity =
  | "idle"
  | "running"
  | "failed"
  | "cancelled"
  | "updated";

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

export function liveSessionActivity(
  session: LiveRuntimeSession | null | undefined,
): LiveSessionActivity {
  if (!session || session.turns.length === 0) return "idle";
  const latest = latestLiveTurn(session);
  if (!latest) return "idle";
  if (
    latest.status === "pending" ||
    latest.status === "streaming" ||
    latest.status === "cancelling"
  ) {
    return "running";
  }
  if (latest.status === "failed") return "failed";
  if (latest.status === "cancelled") return "cancelled";
  return "updated";
}

export function liveSessionUpdatedAt(
  session: LiveRuntimeSession | null | undefined,
): number | null {
  return latestLiveTurn(session)?.updatedAt ?? null;
}

function latestLiveTurn(
  session: LiveRuntimeSession | null | undefined,
): LiveTurn | null {
  if (!session || session.turns.length === 0) return null;
  return session.turns.reduce((latest, turn) =>
    turn.updatedAt > latest.updatedAt ? turn : latest,
  );
}

export function normalizeAgentRuntimeEvent(raw: unknown): AgentRuntimeEvent {
  return camelizeKeys(raw) as AgentRuntimeEvent;
}

export function applyRuntimeAction(
  state: LiveRuntimeState,
  action: LiveRuntimeAction,
): LiveRuntimeState {
  if (action.type === "runtime-event") {
    return applyRuntimeEvent(state, action.event);
  }

  if (action.type === "ensure-session") {
    return {
      ...state,
      sessions: {
        ...state.sessions,
        [action.session.sessioRuntimeSessionId]:
          state.sessions[action.session.sessioRuntimeSessionId] ?? action.session,
      },
    };
  }

  if (action.type === "replace-turn-id") {
    const session = state.sessions[action.sessioRuntimeSessionId];
    if (!session) return state;
    const fromIndex = session.turns.findIndex((turn) => turn.turnId === action.from);
    if (fromIndex < 0) return state;
    const toIndex = session.turns.findIndex((turn) => turn.turnId === action.to);
    const turns = session.turns.map(cloneTurn);
    if (toIndex >= 0 && toIndex !== fromIndex) {
      turns[toIndex] = mergeTurns(turns[fromIndex], turns[toIndex], action.to);
      turns.splice(fromIndex, 1);
    } else {
      turns[fromIndex] = { ...turns[fromIndex], turnId: action.to };
    }
    return {
      ...state,
      sessions: {
        ...state.sessions,
        [session.sessioRuntimeSessionId]: { ...session, turns },
      },
    };
  }

  if (action.type === "reconcile-indexed-session") {
    const session = state.sessions[action.sessioRuntimeSessionId];
    if (!session) return state;
    const turns = session.turns.filter(
      (turn) =>
        !(
          turn.status === "completed" &&
          turn.updatedAt <= action.indexedThrough
        ),
    );
    if (turns.length === session.turns.length) return state;
    return {
      ...state,
      sessions: {
        ...state.sessions,
        [session.sessioRuntimeSessionId]: { ...session, turns },
      },
    };
  }

  const session = state.sessions[action.sessioRuntimeSessionId];
  if (!session) return state;
  const turns = session.turns.slice();
  const existingIndex = turns.findIndex((turn) => turn.turnId === action.turnId);
  const turn =
    existingIndex >= 0
      ? cloneTurn(turns[existingIndex])
      : newTurn(action.turnId, action.timestamp);

  if (action.type === "optimistic-user-message") {
    turn.userText = action.text;
    turn.status = "streaming";
    turn.updatedAt = action.timestamp;
    if (!turn.parts.some((part) => part.kind === "user" && part.text === action.text)) {
      turn.parts.push({ kind: "user", text: action.text });
    }
  } else {
    turn.status = "failed";
    turn.error = action.error;
    turn.updatedAt = action.timestamp;
    turn.parts.push({ kind: "error", error: action.error });
  }

  if (existingIndex >= 0) turns[existingIndex] = turn;
  else turns.push(turn);
  return {
    ...state,
    sessions: {
      ...state.sessions,
      [session.sessioRuntimeSessionId]: { ...session, turns },
    },
  };
}

function camelizeKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(camelizeKeys);
  if (!value || typeof value !== "object") return value;
  const out: Record<string, unknown> = {};
  for (const [key, child] of Object.entries(value)) {
    out[snakeToCamel(key)] = camelizeKeys(child);
  }
  return out;
}

function snakeToCamel(value: string): string {
  return value.replace(/_([a-z])/g, (_, ch: string) => ch.toUpperCase());
}

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
  let existingIndex =
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

function mergeTurns(localTurn: LiveTurn, runtimeTurn: LiveTurn, turnId: string): LiveTurn {
  return {
    ...runtimeTurn,
    turnId,
    status: runtimeTurn.status === "pending" ? localTurn.status : runtimeTurn.status,
    userText: localTurn.userText || runtimeTurn.userText,
    assistantText: localTurn.assistantText + runtimeTurn.assistantText,
    reasoningText: localTurn.reasoningText + runtimeTurn.reasoningText,
    parts: [...localTurn.parts, ...runtimeTurn.parts],
    tools: mergeTools(localTurn.tools, runtimeTurn.tools),
    permissions: mergePermissions(localTurn.permissions, runtimeTurn.permissions),
    error: runtimeTurn.error ?? localTurn.error,
    startedAt: Math.min(localTurn.startedAt, runtimeTurn.startedAt),
    updatedAt: Math.max(localTurn.updatedAt, runtimeTurn.updatedAt),
  };
}

function mergeTools(localTools: LiveToolCall[], runtimeTools: LiveToolCall[]): LiveToolCall[] {
  const merged = localTools.map((tool) => ({ ...tool }));
  for (const tool of runtimeTools) {
    const index = merged.findIndex((item) => item.toolId === tool.toolId);
    if (index >= 0) {
      merged[index] = { ...merged[index], ...tool };
    } else {
      merged.push({ ...tool });
    }
  }
  return merged;
}

function mergePermissions(
  localPermissions: LivePermissionRequest[],
  runtimePermissions: LivePermissionRequest[],
): LivePermissionRequest[] {
  const merged = localPermissions.map((permission) => ({ ...permission }));
  for (const permission of runtimePermissions) {
    const index = merged.findIndex((item) => item.requestId === permission.requestId);
    if (index >= 0) {
      merged[index] = { ...merged[index], ...permission };
    } else {
      merged.push({ ...permission });
    }
  }
  return merged;
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
