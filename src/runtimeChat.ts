import type {
  AcpProtocolMessage,
  Agent,
  AgentAttachment,
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
      attachments?: AgentAttachment[];
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

export interface AcpViewModel {
  source: "live" | "history";
  turns: LiveTurn[];
  sessionState: AcpSessionState;
  protocolMessages: AcpProtocolMessage[];
  ended: boolean;
}

export interface AcpSessionState {
  plan: AcpPlan | null;
  availableCommands: AcpAvailableCommand[];
  currentModeId: string | null;
  configOptions: AcpSessionConfigOption[];
  sessionInfo: AcpSessionInfo | null;
}

export interface LiveRuntimeStatus {
  sessioRuntimeSessionId: string;
  activeTurnId: string | null;
  ended: boolean;
}

export type LiveSessionActivity = "idle" | "permission" | "running" | "failed" | "cancelled" | "updated";

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
  content: AcpToolCallContent[];
  locations: AcpToolCallLocation[];
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

export type AcpContentBlock =
  | AcpTextContent
  | AcpImageContent
  | AcpAudioContent
  | AcpResourceLink
  | AcpEmbeddedResource
  | AcpUnknownContentBlock;

export interface AcpBaseTypedValue {
  meta?: unknown | null;
  annotations?: unknown | null;
}

export interface AcpTextContent extends AcpBaseTypedValue {
  type: "text";
  text: string;
}

export interface AcpImageContent extends AcpBaseTypedValue {
  type: "image";
  uri?: string;
  data?: string;
  mimeType?: string;
}

export interface AcpAudioContent extends AcpBaseTypedValue {
  type: "audio";
  uri?: string;
  data?: string;
  mimeType?: string;
}

export interface AcpResourceLink extends AcpBaseTypedValue {
  type: "resource_link";
  uri: string;
  name?: string;
  title?: string;
  description?: string;
  mimeType?: string;
  size?: number;
}

export interface AcpEmbeddedResource extends AcpBaseTypedValue {
  type: "resource";
  uri?: string;
  name?: string;
  mimeType?: string;
  text?: string;
  blob?: string;
  resource?: unknown;
}

export type AcpUnknownContentBlock = Record<string, unknown> & {
  type: "unknown";
  originalType?: string;
  meta?: unknown | null;
};

export type AcpToolCallContent =
  | AcpToolContentBlock
  | AcpToolDiffContent
  | AcpToolTerminalContent
  | AcpUnknownToolContent;

export interface AcpToolContentBlock {
  type: "content";
  content: AcpContentBlock;
  meta?: unknown | null;
}

export interface AcpToolDiffContent {
  type: "diff";
  path?: string;
  oldText?: string | null;
  newText?: string;
  meta?: unknown | null;
}

export interface AcpToolTerminalContent {
  type: "terminal";
  terminalId: string;
  meta?: unknown | null;
}

export type AcpUnknownToolContent = Record<string, unknown> & {
  type: "unknown";
  originalType?: string;
  meta?: unknown | null;
};

export interface AcpToolCallLocation {
  path?: string;
  line?: number;
  column?: number;
  [key: string]: unknown;
}

export interface AcpPlan {
  entries: AcpPlanEntry[];
  meta?: unknown | null;
}

export interface AcpPlanEntry {
  content: string;
  priority: string;
  status: string;
  meta?: unknown | null;
}

export interface AcpAvailableCommand {
  name: string;
  description: string;
  input?: AcpAvailableCommandInput | null;
  meta?: unknown | null;
}

export interface AcpAvailableCommandInput {
  kind: "unstructured" | "unknown";
  hint?: string | null;
  meta?: unknown | null;
  raw: unknown;
}

export interface AcpSessionConfigOption {
  id: string;
  name: string;
  description?: string | null;
  category?: string | null;
  type?: string;
  currentValue?: string | boolean | null;
  options?: AcpSessionConfigChoice[];
  groups?: AcpSessionConfigChoiceGroup[];
  meta?: unknown | null;
  raw: unknown;
}

export interface AcpSessionConfigChoice {
  value: string;
  name: string;
  description?: string | null;
  meta?: unknown | null;
}

export interface AcpSessionConfigChoiceGroup {
  group: string;
  name: string;
  options: AcpSessionConfigChoice[];
  meta?: unknown | null;
}

export interface AcpSessionInfo {
  title?: string | null;
  updatedAt?: string | null;
  meta?: unknown | null;
  raw: Record<string, unknown>;
}

export const emptyLiveRuntimeState: LiveRuntimeState = {
  sessions: {},
  lastSequence: 0,
};

export function emptyAcpSessionState(): AcpSessionState {
  return {
    plan: null,
    availableCommands: [],
    currentModeId: null,
    configOptions: [],
    sessionInfo: null,
  };
}

export function liveSessionToAcpViewModel(session: LiveRuntimeSession): AcpViewModel {
  return {
    source: "live",
    turns: session.turns,
    sessionState: session.sessionState,
    protocolMessages: session.protocolMessages,
    ended: session.ended,
  };
}

export function historyTurnsToAcpViewModel(turns: LiveTurn[]): AcpViewModel {
  return {
    source: "history",
    turns,
    sessionState: emptyAcpSessionState(),
    protocolMessages: [],
    ended: true,
  };
}

export function liveSessionActivity(
  session: LiveRuntimeSession | null | undefined,
): LiveSessionActivity {
  const latest = latestLiveTurn(session);
  if (!latest) return "idle";
  if (latest.permissions.some((permission) => !permission.cancelled && !permission.selectedOptionId)) return "permission";
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
    const session = state.sessions[action.sessioRuntimeSessionId];
    if (!session) return state;
    const turns = session.turns.filter((turn) => {
      if (turn.status === "pending" || turn.status === "streaming" || turn.status === "cancelling") {
        return true;
      }
      return turn.updatedAt > action.indexedThrough;
    });
    if (turns.length === session.turns.length) return state;
    return updateSession(state, { ...session, turns });
  }

  const session = state.sessions[action.sessioRuntimeSessionId];
  if (!session) return state;
  const { turns, turn } = upsertTurn(session.turns, action.turnId, action.timestamp);
  if (action.type === "optimistic-user-message") {
    turn.status = "streaming";
    turn.updatedAt = action.timestamp;
    if (!turn.blocks.some((block) => block.kind === "user")) {
      const blocks = optimisticUserContentBlocks(action.text, action.attachments ?? []);
      turn.blocks.push({
        kind: "user",
        blocks,
        raw: { optimistic: true, prompt: blocks },
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

function optimisticUserContentBlocks(
  text: string,
  attachments: AgentAttachment[],
): AcpContentBlock[] {
  const blocks: AcpContentBlock[] = [{ type: "text", text }];
  for (const attachment of attachments) {
    if (attachment.kind === "image") {
      blocks.push({
        type: "image",
        uri: attachment.previewDataUrl ?? attachment.path,
        mimeType: attachment.mimeType ?? undefined,
      });
      continue;
    }
    blocks.push({
      type: "resource",
      uri: attachment.path,
      name: attachment.displayName?.trim() || basenameFromPath(attachment.path),
      mimeType: attachment.mimeType ?? undefined,
    });
  }
  return blocks;
}

function basenameFromPath(path: string): string {
  return path.split(/[/\\]/).filter(Boolean).pop() || "attachment";
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
    const capabilities =
      existing?.agentRuntimeSessionId === "pending" && isPlaceholderRuntimeCapabilities(event.capabilities)
        ? existing.capabilities
        : event.capabilities;
    next.sessions[event.sessioRuntimeSessionId] = {
      sessioRuntimeSessionId: event.sessioRuntimeSessionId,
      agent: event.agent,
      agentRuntimeSessionId: event.agentRuntimeSessionId,
      transport: event.transport,
      workspacePath: event.workspacePath,
      capabilities,
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
      permission.selectedOptionId =
        event.optionId ??
        permission.options.find((option) =>
          event.approved ? option.kind.startsWith("allow") : option.kind.startsWith("reject"),
        )?.optionId ??
        null;
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

function applySessionLevelMessage(state: AcpSessionState, message: AcpProtocolMessage): AcpSessionState {
  if (message.method !== "session/update") return state;
  const update = asRecord(message.data).update;
  const updateType = stringField(update, "sessionUpdate") ?? message.updateType;
  if (!updateType) return state;
  switch (updateType) {
    case "plan":
      return { ...state, plan: normalizePlan(update) };
    case "available_commands":
      return { ...state, availableCommands: arrayField(update, "availableCommands").map(normalizeAvailableCommand) };
    case "current_mode":
      return { ...state, currentModeId: stringField(update, "currentModeId") };
    case "config_options":
      return { ...state, configOptions: arrayField(update, "configOptions").map(normalizeSessionConfigOption) };
    case "session_info":
      return { ...state, sessionInfo: normalizeSessionInfo(update) };
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
    blocks: mergeTurnBlocks(localTurn.blocks, runtimeTurn.blocks),
    tools: mergeBy(localTurn.tools, runtimeTurn.tools, (tool) => tool.toolId),
    permissions: mergeBy(localTurn.permissions, runtimeTurn.permissions, (permission) => permission.requestId),
    protocolMessages: [...localTurn.protocolMessages, ...runtimeTurn.protocolMessages],
    error: runtimeTurn.error ?? localTurn.error,
    startedAt: Math.min(localTurn.startedAt, runtimeTurn.startedAt),
    updatedAt: Math.max(localTurn.updatedAt, runtimeTurn.updatedAt),
  };
}

function isPlaceholderRuntimeCapabilities(capabilities: RuntimeCapabilitySet): boolean {
  return capabilities.supportsCancel
    && capabilities.supportsPermissions
    && capabilities.supportsToolDeltas
    && capabilities.supportsLoadSession
    && !capabilities.supportsResume
    && !capabilities.supportsFork
    && !capabilities.supportsImageAttachments
    && !capabilities.supportsAudioAttachments
    && !capabilities.supportsEmbeddedContext
    && !capabilities.supportsAttachments
    && !capabilities.supportsModes;
}

function mergeTurnBlocks(localBlocks: AcpRenderBlock[], runtimeBlocks: AcpRenderBlock[]): AcpRenderBlock[] {
  if (runtimeBlocks.some((block) => block.kind === "user")) {
    return [
      ...localBlocks.filter((block) => block.kind !== "user"),
      ...runtimeBlocks,
    ];
  }
  return [...localBlocks, ...runtimeBlocks];
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
  if (blocks.length === 0) return;
  const last = turn.blocks[turn.blocks.length - 1];
  if (last?.kind === kind) {
    appendBlocksToContentBlockGroup(last, blocks);
    return;
  }
  turn.blocks.push({ kind, blocks: mergeAdjacentTextBlocks(blocks), raw });
}

function appendBlocksToContentBlockGroup(
  target: Extract<AcpRenderBlock, { kind: "user" | "assistant" | "thought" }>,
  blocks: AcpContentBlock[],
): void {
  target.blocks = mergeAdjacentTextBlocks([...target.blocks, ...blocks]);
}

function mergeAdjacentTextBlocks(blocks: AcpContentBlock[]): AcpContentBlock[] {
  const merged: AcpContentBlock[] = [];
  for (const block of blocks) {
    const previous = merged[merged.length - 1];
    if (
      previous &&
      previous.type === "text" &&
      block.type === "text" &&
      typeof previous.text === "string" &&
      typeof block.text === "string" &&
      sameTextBlockShape(previous, block)
    ) {
      previous.text += block.text;
    } else {
      merged.push({ ...block });
    }
  }
  return merged;
}

function sameTextBlockShape(left: AcpContentBlock, right: AcpContentBlock): boolean {
  return JSON.stringify(textBlockMetadata(left)) === JSON.stringify(textBlockMetadata(right));
}

function textBlockMetadata(block: AcpContentBlock): Record<string, unknown> {
  const metadata = { ...(block as unknown as Record<string, unknown>) };
  delete metadata.text;
  return metadata;
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
    return [normalizeContentBlock(record.content)];
  }
  if (typeof record.type === "string") return [normalizeContentBlock(record)];
  return [];
}

function normalizeContentBlock(value: unknown): AcpContentBlock {
  const record = asRecord(value);
  const type = stringField(record, "type") ?? "unknown";
  const meta = record.meta ?? record._meta ?? null;
  if (type === "text") {
    return {
      ...record,
      type,
      text: stringField(record, "text") ?? "",
      annotations: record.annotations ?? null,
      meta,
    };
  }
  if (type === "image" || type === "audio") {
    return {
      ...record,
      type,
      uri: stringField(record, "uri") ?? undefined,
      data: stringField(record, "data") ?? undefined,
      mimeType: stringField(record, "mimeType") ?? undefined,
      annotations: record.annotations ?? null,
      meta,
    };
  }
  if (type === "resource_link") {
    return {
      ...record,
      type,
      uri: stringField(record, "uri") ?? "",
      name: stringField(record, "name") ?? undefined,
      title: stringField(record, "title") ?? undefined,
      description: stringField(record, "description") ?? undefined,
      mimeType: stringField(record, "mimeType") ?? undefined,
      size: numberField(record, "size") ?? undefined,
      annotations: record.annotations ?? null,
      meta,
    };
  }
  if (type === "resource") {
    const resource = asRecord(record.resource);
    return {
      ...record,
      type,
      uri: stringField(resource, "uri") ?? stringField(record, "uri") ?? undefined,
      name: stringField(record, "name") ?? undefined,
      mimeType: stringField(resource, "mimeType") ?? stringField(record, "mimeType") ?? undefined,
      text: stringField(resource, "text") ?? stringField(record, "text") ?? undefined,
      blob: stringField(resource, "blob") ?? stringField(record, "blob") ?? undefined,
      resource: record.resource ?? null,
      annotations: record.annotations ?? null,
      meta,
    };
  }
  return { ...record, type: "unknown", originalType: type, meta } as AcpUnknownContentBlock;
}

function toolFromValue(value: unknown, timestamp: number): AcpToolCall {
  const record = asRecord(value);
  const id = stringField(record, "toolCallId") ?? `tool-${timestamp}`;
  return {
    toolId: id,
    title: stringField(record, "title") ?? "tool",
    kind: stringField(record, "kind") ?? "other",
    status: stringField(record, "status") ?? "pending",
    content: arrayField(record, "content").map(normalizeToolCallContent),
    locations: arrayField(record, "locations").map(normalizeToolCallLocation),
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
    content: arrayField(record, "content").map(normalizeToolCallContent),
    locations: arrayField(record, "locations").map(normalizeToolCallLocation),
    rawInput: record.rawInput ?? null,
    rawOutput: record.rawOutput ?? null,
    meta: record.meta ?? null,
    raw: value,
    updatedAt: timestamp,
  };
}

function normalizeToolCallContent(value: unknown): AcpToolCallContent {
  const record = asRecord(value);
  const type = stringField(record, "type") ?? "unknown";
  const meta = record.meta ?? record._meta ?? null;
  if (type === "content") {
    return {
      ...record,
      type,
      content: normalizeContentBlock(record.content),
      meta,
    };
  }
  if (type === "diff") {
    return {
      ...record,
      type,
      path: stringField(record, "path") ?? stringField(record, "filePath") ?? undefined,
      oldText: stringField(record, "oldText") ?? stringField(record, "old_text") ?? null,
      newText: stringField(record, "newText") ?? stringField(record, "new_text") ?? "",
      meta,
    };
  }
  if (type === "terminal") {
    return {
      ...record,
      type,
      terminalId: stringField(record, "terminalId") ?? stringField(record, "terminal_id") ?? "",
      meta,
    };
  }
  return { ...record, type: "unknown", originalType: type, meta } as AcpUnknownToolContent;
}

function normalizeToolCallLocation(value: unknown): AcpToolCallLocation {
  return asRecord(value) as AcpToolCallLocation;
}

function normalizePlan(value: unknown): AcpPlan {
  const record = asRecord(value);
  return {
    entries: arrayField(record, "entries").map((entry) => {
      const item = asRecord(entry);
      return {
        content: stringField(item, "content") ?? "",
        priority: stringField(item, "priority") ?? "medium",
        status: stringField(item, "status") ?? "pending",
        meta: item.meta ?? item._meta ?? null,
      };
    }),
    meta: record.meta ?? record._meta ?? null,
  };
}

function normalizeAvailableCommand(value: unknown): AcpAvailableCommand {
  const record = asRecord(value);
  return {
    name: stringField(record, "name") ?? "command",
    description: stringField(record, "description") ?? "",
    input: normalizeAvailableCommandInput(record.input),
    meta: record.meta ?? record._meta ?? null,
  };
}

function normalizeAvailableCommandInput(value: unknown): AcpAvailableCommandInput | null {
  if (value === null || value === undefined) return null;
  const record = asRecord(value);
  const hint = stringField(record, "hint");
  return {
    kind: hint !== undefined ? "unstructured" : "unknown",
    hint,
    meta: record.meta ?? record._meta ?? null,
    raw: value,
  };
}

function normalizeSessionConfigOption(value: unknown): AcpSessionConfigOption {
  const record = asRecord(value);
  const type = stringField(record, "type") ?? undefined;
  const option: AcpSessionConfigOption = {
    id: stringField(record, "id") ?? "",
    name: stringField(record, "name") ?? "Option",
    description: stringField(record, "description"),
    category: stringField(record, "category"),
    type,
    currentValue: record.currentValue as string | boolean | null | undefined,
    meta: record.meta ?? record._meta ?? null,
    raw: value,
  };
  if (type === "select") {
    option.options = normalizeConfigChoices(record.options).options;
    option.groups = normalizeConfigChoices(record.options).groups;
  }
  if (type === "boolean" && typeof record.currentValue === "boolean") {
    option.currentValue = record.currentValue;
  }
  return option;
}

function normalizeConfigChoices(value: unknown): {
  options?: AcpSessionConfigChoice[];
  groups?: AcpSessionConfigChoiceGroup[];
} {
  if (!Array.isArray(value)) return {};
  const first = asRecord(value[0]);
  if ("options" in first && "group" in first) {
    return {
      groups: value.map((groupValue) => {
        const group = asRecord(groupValue);
        return {
          group: stringField(group, "group") ?? "",
          name: stringField(group, "name") ?? "Group",
          options: arrayField(group, "options").map(normalizeConfigChoice),
          meta: group.meta ?? group._meta ?? null,
        };
      }),
    };
  }
  return { options: value.map(normalizeConfigChoice) };
}

function normalizeConfigChoice(value: unknown): AcpSessionConfigChoice {
  const record = asRecord(value);
  return {
    value: stringField(record, "value") ?? "",
    name: stringField(record, "name") ?? "Option",
    description: stringField(record, "description"),
    meta: record.meta ?? record._meta ?? null,
  };
}

function normalizeSessionInfo(value: unknown): AcpSessionInfo {
  const record = asRecord(value);
  return {
    title: stringField(record, "title"),
    updatedAt: stringField(record, "updatedAt"),
    meta: record.meta ?? record._meta ?? null,
    raw: record,
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

function numberField(value: unknown, key: string): number | null {
  const item = asRecord(value)[key];
  return typeof item === "number" ? item : null;
}

function arrayField(value: unknown, key: string): unknown[] {
  const item = asRecord(value)[key];
  return Array.isArray(item) ? item : [];
}
