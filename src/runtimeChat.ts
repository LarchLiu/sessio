import type {
  AcpProtocolMessage,
  Agent,
  AgentSessionHandle,
  AgentRuntimeEvent,
  RuntimeCapabilitySet,
  RuntimeError,
  RuntimeTransportKind,
  RuntimeTurnStatus,
} from "./api";

export type LiveRuntimeAction =
  | { type: "runtime-event"; event: AgentRuntimeEvent }
  | { type: "runtime-turn-snapshot"; event: LiveRuntimeTurnSnapshotEvent }
  | { type: "ensure-session"; session: LiveRuntimeSession }
  | {
      type: "reconcile-indexed-session";
      sessioRuntimeSessionId: string;
      indexedThrough: number;
    };

export interface LiveRuntimeState {
  sessions: Record<string, LiveRuntimeSession>;
  lastSequence: number;
  sessionSequences?: Record<string, number>;
}

export interface LiveRuntimeSession {
  sessioRuntimeSessionId: string;
  agent: Agent;
  agentRuntimeSessionId: string;
  transport: RuntimeTransportKind;
  workspacePath: string;
  capabilities: RuntimeCapabilitySet;
  metadata?: Record<string, unknown>;
  turns: LiveTurn[];
  sessionState: AcpSessionState;
  protocolMessages: AcpProtocolMessage[];
  ended: boolean;
}

export interface LiveRuntimeTurnSnapshotEvent {
  sequence: number;
  timestamp: number;
  session: LiveRuntimeSession;
}

export interface AcpViewModel {
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
  | { kind: "user"; blocks: AcpContentBlock[]; raw: unknown; timestamp?: number }
  | { kind: "assistant"; blocks: AcpContentBlock[]; raw: unknown; timestamp?: number }
  | { kind: "thought"; blocks: AcpContentBlock[]; raw: unknown; timestamp?: number }
  | { kind: "tool"; toolId: string; timestamp?: number }
  | { kind: "permission"; requestId: string; timestamp?: number }
  | { kind: "sessionUpdate"; updateType: string; data: unknown; timestamp?: number }
  | { kind: "error"; error: RuntimeError; timestamp?: number };

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
  commandType?: "agent_builtin" | "app";
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
  sessionSequences: {},
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
    turns: session.turns,
    sessionState: session.sessionState,
    protocolMessages: session.protocolMessages,
    ended: session.ended,
  };
}

export function historyTurnsToAcpViewModel(turns: LiveTurn[]): AcpViewModel {
  return {
    turns,
    sessionState: emptyAcpSessionState(),
    protocolMessages: [],
    ended: true,
  };
}

export function dispatchSessionStartedFallback({
  dispatch,
  handle,
  liveState,
  sequenceRef,
  timestamp,
  metadata,
}: {
  dispatch: React.Dispatch<LiveRuntimeAction>;
  handle: AgentSessionHandle;
  liveState: LiveRuntimeState;
  sequenceRef: { current: number };
  timestamp: number;
  metadata?: Record<string, unknown>;
}): void {
  if (liveState.sessions[handle.sessioRuntimeSessionId]) return;
  sequenceRef.current += 1;
  dispatch({
    type: "runtime-event",
    event: {
      kind: "sessionStarted",
      sequence: liveState.lastSequence + sequenceRef.current,
      timestamp,
      agent: handle.agent,
      sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
      agentRuntimeSessionId: handle.agentRuntimeSessionId,
      transport: handle.transport,
      workspacePath: handle.workspacePath,
      capabilities: handle.capabilities,
      metadata: metadata ?? {},
    },
  });
}

export function liveSessionActivity(
  session: LiveRuntimeSession | null | undefined,
): LiveSessionActivity {
  const latest = latestLiveTurn(session);
  if (!latest) return "idle";
  if (latest.permissions.some((permission) => !permission.cancelled && !permission.selectedOptionId)) return "permission";
  if (!session?.ended && ["pending", "streaming", "cancelling"].includes(latest.status)) return "running";
  if (latest.status === "failed") return "failed";
  if (latest.status === "cancelled") return "cancelled";
  return "updated";
}

const SESSION_ACTIVITY_PRIORITY: Record<LiveSessionActivity, number> = {
  idle: 0,
  cancelled: 1,
  updated: 2,
  failed: 3,
  running: 4,
  permission: 5,
};

/**
 * Collapse the activity of several live sessions into one status for rows that
 * fan out across multiple sessions (e.g. a thread). Surfaces the most
 * attention-worthy lane: a pending permission on any session outranks a running
 * one, which outranks a failure. Returns "idle" when nothing is live.
 */
export function aggregateLiveSessionActivity(
  sessions: Array<LiveRuntimeSession | null | undefined>,
): LiveSessionActivity {
  let best: LiveSessionActivity = "idle";
  for (const session of sessions) {
    const activity = liveSessionActivity(session);
    if (SESSION_ACTIVITY_PRIORITY[activity] > SESSION_ACTIVITY_PRIORITY[best]) {
      best = activity;
    }
  }
  return best;
}

export function liveThreadActivity(
  threadId: string,
  sessionKeys: string[],
  liveSessions: Record<string, LiveRuntimeSession>,
  runtimeSessionAliases: Record<string, string>,
): LiveSessionActivity {
  const threadLiveSessions: Array<LiveRuntimeSession | null | undefined> = [];
  const seenRuntimeSessionIds = new Set<string>();
  const linkedSessionKeys = new Set(sessionKeys);

  for (const sessionKey of sessionKeys) {
    const runtimeId = runtimeSessionAliases[sessionKey];
    if (!runtimeId || seenRuntimeSessionIds.has(runtimeId)) continue;
    threadLiveSessions.push(liveSessions[runtimeId]);
    seenRuntimeSessionIds.add(runtimeId);
  }

  for (const liveSession of Object.values(liveSessions)) {
    const agentSessionId = liveSession.agentRuntimeSessionId.trim();
    const matchesLinkedSession = agentSessionId
      ? linkedSessionKeys.has(`${liveSession.agent}:${agentSessionId}`)
      : false;
    if (!matchesLinkedSession && !isThreadPlannerLiveSession(liveSession, threadId)) continue;
    if (seenRuntimeSessionIds.has(liveSession.sessioRuntimeSessionId)) continue;
    threadLiveSessions.push(liveSession);
    seenRuntimeSessionIds.add(liveSession.sessioRuntimeSessionId);
  }

  return aggregateLiveSessionActivity(threadLiveSessions);
}

export function isThreadPlannerLiveSession(
  liveSession: LiveRuntimeSession,
  threadId: string | undefined,
): boolean {
  if (!threadId) return false;
  const metadata = liveSession.metadata ?? {};
  const metadataThreadId = stringMeta(metadata, "astraThreadId")
    ?? stringMeta(metadata, "threadId");
  if (metadataThreadId !== threadId) return false;
  if (stringMeta(metadata, "astraPurpose") === "orchestration") return true;
  return Boolean(metadata.astraInternal && stringMeta(metadata, "astraRunId"));
}

export function stringMeta(metadata: Record<string, unknown> | undefined, key: string): string | null {
  const value = metadata?.[key];
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

export function liveSessionUpdatedAt(
  session: LiveRuntimeSession | null | undefined,
): number | null {
  return latestLiveTurn(session)?.updatedAt ?? null;
}

export function effectiveRuntimeCapabilities(
  liveCapabilities: RuntimeCapabilitySet | null | undefined,
  fallbackCapabilities: RuntimeCapabilitySet | null | undefined,
): RuntimeCapabilitySet | null {
  if (!liveCapabilities) return fallbackCapabilities ?? null;
  if (fallbackCapabilities && isPlaceholderRuntimeCapabilities(liveCapabilities)) {
    return fallbackCapabilities;
  }
  return liveCapabilities;
}

function latestLiveTurn(session: LiveRuntimeSession | null | undefined): LiveTurn | null {
  if (!session || session.turns.length === 0) return null;
  return session.turns.reduce((latest, turn) =>
    turn.updatedAt > latest.updatedAt ? turn : latest,
  );
}

export function normalizeAgentRuntimeEvent(raw: unknown): AgentRuntimeEvent {
  if (isCamelRuntimeEvent(raw)) return raw;
  return camelizeKeys(raw) as AgentRuntimeEvent;
}

export function normalizeRuntimeTurnSnapshot(raw: unknown): LiveRuntimeTurnSnapshotEvent {
  if (isCamelRuntimeTurnSnapshot(raw)) return raw;
  return camelizeKeys(raw) as LiveRuntimeTurnSnapshotEvent;
}

function isCamelRuntimeEvent(value: unknown): value is AgentRuntimeEvent {
  if (!isRecord(value) || typeof value.kind !== "string") return false;
  if (value.kind === "sessionStarted") {
    return typeof value.sessioRuntimeSessionId === "string"
      && typeof value.agentRuntimeSessionId === "string";
  }
  return typeof value.sessioRuntimeSessionId === "string";
}

function isCamelRuntimeTurnSnapshot(value: unknown): value is LiveRuntimeTurnSnapshotEvent {
  if (!isRecord(value) || typeof value.sequence !== "number") return false;
  const session = value.session;
  return isRecord(session) && typeof session.sessioRuntimeSessionId === "string";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export function applyRuntimeAction(
  state: LiveRuntimeState,
  action: LiveRuntimeAction,
): LiveRuntimeState {
  if (action.type === "runtime-turn-snapshot") return applyRuntimeTurnSnapshot(state, action.event);
  if (action.type === "runtime-event") return applyRuntimeEventEnvelope(state, action.event);

  if (action.type === "ensure-session") {
    const sessionSequences = state.sessionSequences ?? {};
    return {
      ...state,
      sessions: {
        ...state.sessions,
        [action.session.sessioRuntimeSessionId]:
          state.sessions[action.session.sessioRuntimeSessionId] ?? action.session,
      },
      sessionSequences: {
        ...sessionSequences,
        [action.session.sessioRuntimeSessionId]:
          sessionSequences[action.session.sessioRuntimeSessionId] ?? state.lastSequence,
      },
    };
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

  return state;
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

function applyRuntimeTurnSnapshot(
  state: LiveRuntimeState,
  event: LiveRuntimeTurnSnapshotEvent,
): LiveRuntimeState {
  const sessionId = event.session.sessioRuntimeSessionId;
  const sessionSequences = state.sessionSequences ?? {};
  if (event.sequence < (sessionSequences[sessionId] ?? 0)) return state;
  const existing = state.sessions[sessionId];
  const session = existing
    ? mergeRuntimeSessionSnapshot(existing, event.session)
    : event.session;
  return {
    sessions: {
      ...state.sessions,
      [session.sessioRuntimeSessionId]: session,
    },
    sessionSequences: {
      ...sessionSequences,
      [session.sessioRuntimeSessionId]: event.sequence,
    },
    lastSequence: Math.max(state.lastSequence, event.sequence),
  };
}

function applyRuntimeEventEnvelope(
  state: LiveRuntimeState,
  event: AgentRuntimeEvent,
): LiveRuntimeState {
  const sessionSequences = state.sessionSequences ?? {};
  if (event.sequence < (sessionSequences[event.sessioRuntimeSessionId] ?? 0)) return state;
  if (event.kind !== "sessionStarted" && event.kind !== "sessionEnded") {
    return { ...state, lastSequence: Math.max(state.lastSequence, event.sequence) };
  }
  if (event.kind === "sessionEnded") {
    const session = state.sessions[event.sessioRuntimeSessionId];
    if (!session) return { ...state, lastSequence: Math.max(state.lastSequence, event.sequence) };
    return {
      sessions: {
        ...state.sessions,
        [session.sessioRuntimeSessionId]: { ...session, ended: true },
      },
      sessionSequences: {
        ...sessionSequences,
        [session.sessioRuntimeSessionId]: event.sequence,
      },
      lastSequence: Math.max(state.lastSequence, event.sequence),
    };
  }
  const existing = state.sessions[event.sessioRuntimeSessionId];
  const capabilities =
    existing?.agentRuntimeSessionId === "pending" && isPlaceholderRuntimeCapabilities(event.capabilities)
      ? existing.capabilities
      : event.capabilities;
  const session: LiveRuntimeSession = {
    sessioRuntimeSessionId: event.sessioRuntimeSessionId,
    agent: event.agent,
    agentRuntimeSessionId: event.agentRuntimeSessionId,
    transport: event.transport,
    workspacePath: event.workspacePath,
    capabilities,
    metadata: event.metadata ?? {},
    turns: existing?.turns ?? [],
    sessionState: existing?.sessionState ?? emptyAcpSessionState(),
    protocolMessages: existing?.protocolMessages ?? [],
    ended: false,
  };
  return {
    sessions: {
      ...state.sessions,
      [session.sessioRuntimeSessionId]: session,
    },
    sessionSequences: {
      ...sessionSequences,
      [session.sessioRuntimeSessionId]: event.sequence,
    },
    lastSequence: Math.max(state.lastSequence, event.sequence),
  };
}

function mergeRuntimeSessionSnapshot(
  existing: LiveRuntimeSession,
  snapshot: LiveRuntimeSession,
): LiveRuntimeSession {
  const capabilities =
    existing.agentRuntimeSessionId === "pending" && isPlaceholderRuntimeCapabilities(snapshot.capabilities)
      ? existing.capabilities
      : snapshot.capabilities;
  return {
    ...snapshot,
    capabilities,
    metadata: {
      ...(existing.metadata ?? {}),
      ...(snapshot.metadata ?? {}),
    },
  };
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
