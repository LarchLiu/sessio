import type {
  AcpProtocolMessage,
  Agent,
  AgentRuntimeEvent,
  RuntimeCapabilitySet,
  RuntimeError,
  RuntimeTransportKind,
  RuntimeTurnStatus,
} from "./api";

export type LiveRuntimeAction =
  | { type: "runtime-event"; event: AgentRuntimeEvent }
  | { type: "ensure-session"; session: LiveRuntimeSession }
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
  sessionState: AcpSessionState;
  protocolMessages: AcpProtocolMessage[];
  ended: boolean;
}

export interface AcpSessionState {
  plan: unknown | null;
  availableCommands: unknown[];
  currentModeId: string | null;
  configOptions: unknown[];
  sessionInfo: Record<string, unknown> | null;
}

export interface LiveRuntimeStatus {
  sessioRuntimeSessionId: string;
  activeTurnId: string | null;
  ended: boolean;
}

export type LiveSessionActivity = "idle" | "running" | "failed" | "cancelled" | "updated";

export interface LiveTurn {
  turnId: string;
  status: RuntimeTurnStatus;
  blocks: AcpRenderBlock[];
  tools: AcpToolCall[];
  permissions: AcpPermissionRequest[];
  protocolMessages: AcpProtocolMessage[];
  stopReason: string | null;
  error: RuntimeError | null;
  startedAt: number;
  updatedAt: number;
}

export type AcpRenderBlock =
  | { kind: "user"; blocks: AcpContentBlock[]; raw: unknown }
  | { kind: "assistant"; blocks: AcpContentBlock[]; raw: unknown }
  | { kind: "thought"; blocks: AcpContentBlock[]; raw: unknown }
  | { kind: "tool"; toolId: string }
  | { kind: "permission"; requestId: string }
  | { kind: "sessionUpdate"; updateType: string; data: unknown }
  | { kind: "error"; error: RuntimeError };

export interface AcpToolCall {
  toolId: string;
  title: string;
  kind: string;
  status: string;
  content: unknown[];
  locations: unknown[];
  rawInput: unknown | null;
  rawOutput: unknown | null;
  meta: unknown | null;
  raw: unknown;
  updatedAt: number;
}

export interface AcpPermissionRequest {
  requestId: string;
  toolCall: unknown;
  toolName: string;
  input: unknown | null;
  options: AcpPermissionOption[];
  selectedOptionId: string | null;
  cancelled: boolean;
  raw: unknown;
}

export interface AcpPermissionOption {
  optionId: string;
  name: string;
  kind: string;
  meta: unknown | null;
}

export type AcpContentBlock = Record<string, unknown> & { type?: string };

export const emptyLiveRuntimeState: LiveRuntimeState = {
  sessions: {},
  lastSequence: 0,
};

export function liveSessionActivity(
  session: LiveRuntimeSession | null | undefined,
): LiveSessionActivity {
  const latest = latestLiveTurn(session);
  if (!latest) return "idle";
  if (["pending", "streaming", "cancelling"].includes(latest.status)) return "running";
  if (latest.status === "failed") return "failed";
  if (latest.status === "cancelled") return "cancelled";
  return "updated";
}

export function liveSessionUpdatedAt(
  session: LiveRuntimeSession | null | undefined,
): number | null {
  return latestLiveTurn(session)?.updatedAt ?? null;
}

function latestLiveTurn(session: LiveRuntimeSession | null | undefined): LiveTurn | null {
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
  if (action.type === "runtime-event") return applyRuntimeEvent(state, action.event);

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
    const turns = session.turns.map(cloneTurn);
    const index = turns.findIndex((turn) => turn.turnId === action.from);
    if (index < 0) return state;
    const existing = turns.findIndex((turn) => turn.turnId === action.to);
    if (existing >= 0 && existing !== index) {
      turns[existing] = mergeTurns(turns[index], turns[existing], action.to);
      turns.splice(index, 1);
    } else {
      turns[index] = { ...turns[index], turnId: action.to };
    }
    return updateSession(state, { ...session, turns });
  }

  if (action.type === "reconcile-indexed-session") {
    return state;
  }

  const session = state.sessions[action.sessioRuntimeSessionId];
  if (!session) return state;
  const { turns, turn } = upsertTurn(session.turns, action.turnId, action.timestamp);
  if (action.type === "optimistic-user-message") {
    turn.status = "streaming";
    turn.updatedAt = action.timestamp;
    if (!turn.blocks.some((block) => block.kind === "user")) {
      turn.blocks.push({
        kind: "user",
        blocks: [{ type: "text", text: action.text }],
        raw: { optimistic: true, prompt: [{ type: "text", text: action.text }] },
      });
    }
  } else {
    turn.status = "failed";
    turn.error = action.error;
    turn.updatedAt = action.timestamp;
    turn.blocks.push({ kind: "error", error: action.error });
  }
  return updateSession(state, { ...session, turns });
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
  let next: LiveRuntimeState = { sessions: { ...state.sessions }, lastSequence: event.sequence };

  if (event.kind === "sessionStarted") {
    const existing = next.sessions[event.sessioRuntimeSessionId];
    next.sessions[event.sessioRuntimeSessionId] = {
      sessioRuntimeSessionId: event.sessioRuntimeSessionId,
      agent: event.agent,
      agentRuntimeSessionId: event.agentRuntimeSessionId,
      transport: event.transport,
      workspacePath: event.workspacePath,
      capabilities: event.capabilities,
      turns: existing?.turns ?? [],
      sessionState: existing?.sessionState ?? emptyAcpSessionState(),
      protocolMessages: existing?.protocolMessages ?? [],
      ended: false,
    };
    return next;
  }

  const session = next.sessions[event.sessioRuntimeSessionId];
  if (!session) return next;

  if (event.kind === "sessionEnded") {
    return updateSession(next, { ...session, ended: true });
  }

  if (event.kind === "turnStarted") {
    const { turns, turn } = upsertTurn(session.turns, event.turnId, event.timestamp);
    turn.status = "streaming";
    turn.updatedAt = event.timestamp;
    return updateSession(next, { ...session, turns });
  }

  if (event.kind === "turnCompleted") {
    const { turns, turn } = upsertTurn(session.turns, event.turnId, event.timestamp);
    turn.status = "completed";
    turn.updatedAt = event.timestamp;
    return updateSession(next, { ...session, turns });
  }

  if (event.kind === "turnCancelled") {
    const { turns, turn } = upsertTurn(session.turns, event.turnId, event.timestamp);
    turn.status = "cancelled";
    turn.updatedAt = event.timestamp;
    return updateSession(next, { ...session, turns });
  }

  if (event.kind === "turnError") {
    const { turns, turn } = upsertTurn(session.turns, event.turnId, event.timestamp);
    turn.status = "failed";
    turn.error = event.error;
    turn.updatedAt = event.timestamp;
    turn.blocks.push({ kind: "error", error: event.error });
    return updateSession(next, { ...session, turns });
  }

  if (event.kind === "permissionResolved") {
    const { turns, turn } = upsertTurn(session.turns, event.turnId, event.timestamp);
    const permission = turn.permissions.find((item) => item.requestId === event.requestId);
    if (permission) {
      permission.cancelled = false;
      permission.selectedOptionId = permission.options.find((option) =>
        event.approved ? option.kind.startsWith("allow") : option.kind.startsWith("reject"),
      )?.optionId ?? null;
    }
    turn.updatedAt = event.timestamp;
    return updateSession(next, { ...session, turns });
  }

  if (event.kind !== "acpProtocolMessage") {
    return next;
  }

  const message = event.message;
  const protocolMessages = appendProtocolMessage(session.protocolMessages, message);
  if (!event.turnId) {
    return updateSession(next, {
      ...session,
      protocolMessages,
      sessionState: applySessionLevelMessage(session.sessionState, message),
    });
  }

  const { turns, turn } = upsertTurn(session.turns, event.turnId, event.timestamp);
  turn.protocolMessages = appendProtocolMessage(turn.protocolMessages, message);
  turn.updatedAt = event.timestamp;
  applyAcpMessageToTurn(turn, message, event.timestamp);
  return updateSession(next, {
    ...session,
    turns,
    protocolMessages,
    sessionState: applySessionLevelMessage(session.sessionState, message),
  });
}

function applyAcpMessageToTurn(turn: LiveTurn, message: AcpProtocolMessage, timestamp: number): void {
  if (message.method === "session/prompt" && message.direction === "client_to_agent") {
    const prompt = asRecord(message.data).prompt;
    turn.status = "streaming";
    replaceOrAppendUserBlock(turn, {
      kind: "user",
      blocks: normalizeContentBlocks(prompt),
      raw: message.data,
    });
    return;
  }

  if (message.method === "session/prompt" && message.direction === "agent_to_client") {
    const stopReason = stringField(message.data, "stopReason");
    turn.stopReason = stopReason;
    turn.status = stopReason === "cancelled" ? "cancelled" : "completed";
    return;
  }

  if (message.method === "session/request_permission") {
    if (message.messageKind === "request") {
      const permission = permissionFromMessage(message);
      if (permission) upsertPermission(turn, permission);
      return;
    }
    if (message.messageKind === "response") {
      const optionId = selectedPermissionOptionId(message.data);
      const latest = turn.permissions[turn.permissions.length - 1];
      if (latest) {
        latest.selectedOptionId = optionId;
        latest.cancelled = !optionId;
      }
      return;
    }
  }

  if (message.method !== "session/update") return;
  const update = asRecord(message.data).update;
  const updateType = stringField(update, "sessionUpdate") ?? message.updateType ?? "unknown";

  switch (updateType) {
    case "user_message_chunk":
      appendContentBlock(turn, "user", normalizeContentBlocks(asRecord(update).content), update);
      break;
    case "agent_message_chunk":
      appendContentBlock(turn, "assistant", normalizeContentBlocks(asRecord(update).content), update);
      turn.status = turn.status === "pending" ? "streaming" : turn.status;
      break;
    case "agent_thought_chunk":
      appendContentBlock(turn, "thought", normalizeContentBlocks(asRecord(update).content), update);
      break;
    case "tool_call": {
      const tool = toolFromValue(update, timestamp);
      upsertTool(turn, tool);
      ensureBlock(turn, { kind: "tool", toolId: tool.toolId });
      break;
    }
    case "tool_call_update": {
      const tool = toolUpdateFromValue(update, timestamp);
      upsertTool(turn, tool);
      ensureBlock(turn, { kind: "tool", toolId: tool.toolId });
      break;
    }
    default:
      turn.blocks.push({ kind: "sessionUpdate", updateType, data: update });
      break;
  }
}

function emptyAcpSessionState(): AcpSessionState {
  return {
    plan: null,
    availableCommands: [],
    currentModeId: null,
    configOptions: [],
    sessionInfo: null,
  };
}

function applySessionLevelMessage(state: AcpSessionState, message: AcpProtocolMessage): AcpSessionState {
  if (message.method !== "session/update") return state;
  const update = asRecord(message.data).update;
  const updateType = stringField(update, "sessionUpdate") ?? message.updateType;
  if (!updateType) return state;
  switch (updateType) {
    case "plan":
      return { ...state, plan: update };
    case "available_commands":
      return { ...state, availableCommands: arrayField(update, "availableCommands") };
    case "current_mode":
      return { ...state, currentModeId: stringField(update, "currentModeId") };
    case "config_options":
      return { ...state, configOptions: arrayField(update, "configOptions") };
    case "session_info":
      return { ...state, sessionInfo: asRecord(update) };
    default:
      return state;
  }
}

function upsertTurn(turns: LiveTurn[], turnId: string, timestamp: number): { turns: LiveTurn[]; turn: LiveTurn } {
  const next = turns.map(cloneTurn);
  let index = next.findIndex((turn) => turn.turnId === turnId);
  if (index < 0) {
    next.push(newTurn(turnId, timestamp));
    index = next.length - 1;
  }
  return { turns: next, turn: next[index] };
}

function newTurn(turnId: string, timestamp: number): LiveTurn {
  return {
    turnId,
    status: "pending",
    blocks: [],
    tools: [],
    permissions: [],
    protocolMessages: [],
    stopReason: null,
    error: null,
    startedAt: timestamp,
    updatedAt: timestamp,
  };
}

function cloneTurn(turn: LiveTurn): LiveTurn {
  return {
    ...turn,
    blocks: turn.blocks.map((block) => ({ ...block })),
    tools: turn.tools.map((tool) => ({ ...tool })),
    permissions: turn.permissions.map((permission) => ({ ...permission, options: permission.options.map((option) => ({ ...option })) })),
    protocolMessages: turn.protocolMessages.map((message) => ({ ...message })),
  };
}

function mergeTurns(localTurn: LiveTurn, runtimeTurn: LiveTurn, turnId: string): LiveTurn {
  return {
    ...runtimeTurn,
    turnId,
    blocks: [...localTurn.blocks, ...runtimeTurn.blocks],
    tools: mergeBy(localTurn.tools, runtimeTurn.tools, (tool) => tool.toolId),
    permissions: mergeBy(localTurn.permissions, runtimeTurn.permissions, (permission) => permission.requestId),
    protocolMessages: [...localTurn.protocolMessages, ...runtimeTurn.protocolMessages],
    error: runtimeTurn.error ?? localTurn.error,
    startedAt: Math.min(localTurn.startedAt, runtimeTurn.startedAt),
    updatedAt: Math.max(localTurn.updatedAt, runtimeTurn.updatedAt),
  };
}

function mergeBy<T>(left: T[], right: T[], key: (item: T) => string): T[] {
  const merged = left.map((item) => ({ ...item }));
  for (const item of right) {
    const index = merged.findIndex((existing) => key(existing) === key(item));
    if (index >= 0) merged[index] = { ...merged[index], ...item };
    else merged.push({ ...item });
  }
  return merged;
}

function updateSession(state: LiveRuntimeState, session: LiveRuntimeSession): LiveRuntimeState {
  return {
    ...state,
    sessions: {
      ...state.sessions,
      [session.sessioRuntimeSessionId]: session,
    },
  };
}

function appendProtocolMessage(messages: AcpProtocolMessage[], message: AcpProtocolMessage): AcpProtocolMessage[] {
  return [...messages, message].slice(-240);
}

function replaceOrAppendUserBlock(turn: LiveTurn, block: Extract<AcpRenderBlock, { kind: "user" }>): void {
  const index = turn.blocks.findIndex((item) => item.kind === "user");
  if (index >= 0) turn.blocks[index] = block;
  else turn.blocks.unshift(block);
}

function appendContentBlock(
  turn: LiveTurn,
  kind: "user" | "assistant" | "thought",
  blocks: AcpContentBlock[],
  raw: unknown,
): void {
  const last = turn.blocks[turn.blocks.length - 1];
  if (last?.kind === kind) {
    last.blocks.push(...blocks);
    return;
  }
  turn.blocks.push({ kind, blocks, raw });
}

function ensureBlock(turn: LiveTurn, block: Extract<AcpRenderBlock, { kind: "tool" } | { kind: "permission" }>): void {
  if (block.kind === "tool" && turn.blocks.some((item) => item.kind === "tool" && item.toolId === block.toolId)) return;
  if (block.kind === "permission" && turn.blocks.some((item) => item.kind === "permission" && item.requestId === block.requestId)) return;
  turn.blocks.push(block);
}

function normalizeContentBlocks(value: unknown): AcpContentBlock[] {
  if (Array.isArray(value)) return value.flatMap(normalizeContentBlocks);
  if (!value || typeof value !== "object") return [];
  const record = value as Record<string, unknown>;
  if (record.content && typeof record.content === "object" && !Array.isArray(record.content)) {
    return [record.content as AcpContentBlock];
  }
  if (typeof record.type === "string") return [record as AcpContentBlock];
  return [];
}

function toolFromValue(value: unknown, timestamp: number): AcpToolCall {
  const record = asRecord(value);
  const id = stringField(record, "toolCallId") ?? `tool-${timestamp}`;
  return {
    toolId: id,
    title: stringField(record, "title") ?? "tool",
    kind: stringField(record, "kind") ?? "other",
    status: stringField(record, "status") ?? "pending",
    content: arrayField(record, "content"),
    locations: arrayField(record, "locations"),
    rawInput: record.rawInput ?? null,
    rawOutput: record.rawOutput ?? null,
    meta: record.meta ?? null,
    raw: value,
    updatedAt: timestamp,
  };
}

function toolUpdateFromValue(value: unknown, timestamp: number): AcpToolCall {
  const record = asRecord(value);
  return {
    toolId: stringField(record, "toolCallId") ?? `tool-${timestamp}`,
    title: stringField(record, "title") ?? "tool",
    kind: stringField(record, "kind") ?? "other",
    status: stringField(record, "status") ?? "pending",
    content: arrayField(record, "content"),
    locations: arrayField(record, "locations"),
    rawInput: record.rawInput ?? null,
    rawOutput: record.rawOutput ?? null,
    meta: record.meta ?? null,
    raw: value,
    updatedAt: timestamp,
  };
}

function upsertTool(turn: LiveTurn, nextTool: AcpToolCall): void {
  const index = turn.tools.findIndex((tool) => tool.toolId === nextTool.toolId);
  if (index < 0) {
    turn.tools.push(nextTool);
    return;
  }
  const current = turn.tools[index];
  turn.tools[index] = {
    ...current,
    ...nextTool,
    title: nextTool.title === "tool" ? current.title : nextTool.title,
    kind: nextTool.kind === "other" ? current.kind : nextTool.kind,
    status: nextTool.status === "pending" && current.status ? current.status : nextTool.status,
    content: nextTool.content.length > 0 ? nextTool.content : current.content,
    locations: nextTool.locations.length > 0 ? nextTool.locations : current.locations,
    rawInput: nextTool.rawInput ?? current.rawInput,
    rawOutput: nextTool.rawOutput ?? current.rawOutput,
  };
}

function permissionFromMessage(message: AcpProtocolMessage): AcpPermissionRequest | null {
  const data = asRecord(message.data);
  const requestId = message.requestId ?? stringField(asRecord(data.toolCall), "toolCallId");
  if (!requestId) return null;
  const toolCall = asRecord(data.toolCall);
  const fields = asRecord(toolCall.fields ?? toolCall);
  return {
    requestId,
    toolCall: data.toolCall ?? null,
    toolName: stringField(fields, "title") ?? "tool",
    input: fields.rawInput ?? null,
    options: arrayField(data, "options").map((item) => {
      const option = asRecord(item);
      return {
        optionId: stringField(option, "optionId") ?? "",
        name: stringField(option, "name") ?? "Option",
        kind: stringField(option, "kind") ?? "unknown",
        meta: option.meta ?? null,
      };
    }),
    selectedOptionId: null,
    cancelled: false,
    raw: message.data,
  };
}

function upsertPermission(turn: LiveTurn, permission: AcpPermissionRequest): void {
  const index = turn.permissions.findIndex((item) => item.requestId === permission.requestId);
  if (index >= 0) turn.permissions[index] = { ...turn.permissions[index], ...permission };
  else turn.permissions.push(permission);
  ensureBlock(turn, { kind: "permission", requestId: permission.requestId });
}

function selectedPermissionOptionId(value: unknown): string | null {
  const outcome = asRecord(value).outcome;
  const record = asRecord(outcome);
  if (stringField(record, "outcome") === "cancelled") return null;
  return stringField(record, "optionId");
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function stringField(value: unknown, key: string): string | null {
  const item = asRecord(value)[key];
  return typeof item === "string" ? item : null;
}

function arrayField(value: unknown, key: string): unknown[] {
  const item = asRecord(value)[key];
  return Array.isArray(item) ? item : [];
}
