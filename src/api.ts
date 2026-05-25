import { invoke } from "@tauri-apps/api/core";

export type Agent = "codex" | "claude" | "gemini";

export interface SessionInfo {
  id: string;
  agent: Agent;
  projectPath: string | null;
  projectName: string | null;
  startedAt: number | null;
  updatedAt: number | null;
  messageCount: number;
  title: string | null;
  firstUserMessage: string | null;
  filePath: string;
  fileSize: number;
  partial: boolean;
  available: boolean;
  archived: boolean;
  subagents: SubagentInfo[];
}

export interface SubagentInfo {
  id: string;
  agentType: string | null;
  description: string | null;
  startedAt: number | null;
  updatedAt: number | null;
  messageCount: number;
  firstUserMessage: string | null;
  filePath: string;
  fileSize: number;
  partial: boolean;
}

export interface SessionMessage {
  role: string;
  text: string;
  timestamp: number | null;
  toolCallId?: string | null;
}

export interface SessionMessagesResult {
  messages: SessionMessage[];
  messageCount: number;
}

export type IndexPhase = "idle" | "indexing" | "rebuilding";

export interface IndexStatus {
  phase: IndexPhase;
  lastError: string | null;
}

export interface MemoryBackendStatus {
  backend: string;
  available: boolean;
  error: string | null;
  details?: Record<string, unknown>;
}

export interface ProjectMemorySearchResult {
  title: string | null;
  snippet: string | null;
  score: number | null;
  recordId: string | null;
  artifactUri: string | null;
  raw: unknown;
}

export type RuntimeTransportKind = "acp" | "cliStreamJson" | "plainCli" | "fake";

export type RuntimeSessionStatus =
  | "starting"
  | "active"
  | "idle"
  | "cancelling"
  | "completed"
  | "errored"
  | "disconnected"
  | "ended";

export type RuntimeTurnStatus =
  | "pending"
  | "streaming"
  | "cancelling"
  | "completed"
  | "failed"
  | "cancelled";

export interface RuntimeCapabilitySet {
  supportsCancel: boolean;
  supportsPermissions: boolean;
  supportsToolDeltas: boolean;
  supportsLoadSession: boolean;
  supportsResume: boolean;
  supportsFork: boolean;
  supportsImageAttachments: boolean;
  supportsAudioAttachments: boolean;
  supportsEmbeddedContext: boolean;
  supportsAttachments: boolean;
  supportsModes: boolean;
}

export interface RuntimeStatus {
  agent: Agent;
  transport: RuntimeTransportKind;
  available: boolean;
  status: RuntimeSessionStatus;
  capabilities: RuntimeCapabilitySet;
  error: string | null;
  metadata: Record<string, unknown>;
}

export interface RuntimeAgentMetadata {
  agent: Agent;
  enabled: boolean;
  configured: boolean;
  transport: RuntimeTransportKind;
  sessionCommand: string | null;
  versionCommand: string | null;
  detectedVersion: string | null;
  capabilities: RuntimeCapabilitySet | null;
  updatedAt: number | null;
}

export interface StartAgentSessionRequest {
  agent: Agent;
  workspacePath: string;
  initialPrompt?: string | null;
  sourceSessionId?: string | null;
  sourceAgent?: Agent | null;
  options?: Record<string, unknown>;
}

export interface EnsureAgentRuntimeSessionRequest {
  agent: Agent;
  sessioRuntimeSessionId: string;
  workspacePath: string;
  agentRuntimeSessionId?: string | null;
  sourceAgent?: Agent | null;
}

export interface AgentSessionHandle {
  sessioRuntimeSessionId: string;
  agent: Agent;
  transport: RuntimeTransportKind;
  agentRuntimeSessionId: string;
  workspacePath: string;
  status: RuntimeSessionStatus;
  capabilities: RuntimeCapabilitySet;
}

export interface AgentAttachment {
  path: string;
  mimeType: string | null;
  kind: "image" | "file";
  previewDataUrl?: string | null;
}

export interface AgentInput {
  text: string;
  attachments?: AgentAttachment[];
  options?: Record<string, unknown>;
}

export interface AgentSessionConfigChange {
  configId: string;
  value: unknown;
}

export interface AgentTurnHandle {
  sessioRuntimeSessionId: string;
  turnId: string;
  status: RuntimeTurnStatus;
}

export interface RuntimeError {
  code: string;
  message: string;
  data: unknown | null;
}

export interface AcpProtocolMessage {
  direction: string;
  messageKind: string;
  method: string;
  protocolVersion: string | null;
  acpSessionId: string | null;
  turnId: string | null;
  requestId: string | null;
  updateType: string | null;
  data: unknown;
}

export type AgentRuntimeEventPayload =
  | {
      kind: "sessionStarted";
      agent: Agent;
      sessioRuntimeSessionId: string;
      agentRuntimeSessionId: string;
      transport: RuntimeTransportKind;
      workspacePath: string;
      capabilities: RuntimeCapabilitySet;
    }
  | { kind: "turnStarted"; sessioRuntimeSessionId: string; turnId: string }
  | { kind: "textDelta"; sessioRuntimeSessionId: string; turnId: string; text: string }
  | { kind: "reasoningDelta"; sessioRuntimeSessionId: string; turnId: string; text: string }
  | {
      kind: "toolStarted";
      sessioRuntimeSessionId: string;
      turnId: string;
      toolId: string;
      name: string;
      input: unknown | null;
      data: unknown;
    }
  | {
      kind: "toolInputDelta";
      sessioRuntimeSessionId: string;
      turnId: string;
      toolId: string;
      delta: string;
      data: unknown | null;
    }
  | {
      kind: "toolOutputDelta";
      sessioRuntimeSessionId: string;
      turnId: string;
      toolId: string;
      delta: string;
      data: unknown | null;
    }
  | {
      kind: "toolStatusChanged";
      sessioRuntimeSessionId: string;
      turnId: string;
      toolId: string;
      status: string;
      data: unknown | null;
    }
  | {
      kind: "sessionUpdate";
      sessioRuntimeSessionId: string;
      turnId: string;
      updateType: string;
      data: unknown;
    }
  | {
      kind: "acpProtocolMessage";
      sessioRuntimeSessionId: string;
      turnId?: string | null;
      message: AcpProtocolMessage;
    }
  | {
      kind: "permissionRequested";
      sessioRuntimeSessionId: string;
      turnId: string;
      requestId: string;
      toolName: string;
      input: unknown | null;
      data: unknown;
    }
  | {
      kind: "permissionResolved";
      sessioRuntimeSessionId: string;
      turnId: string;
      requestId: string;
      approved: boolean;
      optionId?: string | null;
    }
  | {
      kind: "turnCompleted";
      sessioRuntimeSessionId: string;
      turnId: string;
      result: unknown | null;
    }
  | {
      kind: "turnError";
      sessioRuntimeSessionId: string;
      turnId: string;
      error: RuntimeError;
    }
  | { kind: "turnCancelled"; sessioRuntimeSessionId: string; turnId: string }
  | { kind: "sessionEnded"; sessioRuntimeSessionId: string };

export type AgentRuntimeEvent = AgentRuntimeEventPayload & {
  sequence: number;
  timestamp: number;
};

export type SessionScope =
  | { kind: "all" }
  | { kind: "agent"; agent: Agent }
  | { kind: "project"; key: string };

export async function listSessions(): Promise<SessionInfo[]> {
  return invoke<SessionInfo[]>("list_sessions");
}

export async function getIndexStatus(): Promise<IndexStatus> {
  return invoke<IndexStatus>("get_index_status");
}

export async function getMemoryBackendStatus(): Promise<MemoryBackendStatus> {
  return invoke<MemoryBackendStatus>("get_memory_backend_status");
}

export async function searchProjectMemory(
  projectKey: string,
  query: string,
): Promise<ProjectMemorySearchResult[]> {
  return invoke<ProjectMemorySearchResult[]>("search_project_memory", {
    projectKey,
    query,
  });
}

export async function rebuildSessionIndex(): Promise<void> {
  return invoke<void>("rebuild_session_index");
}

export async function removeSessionFiles(session: SessionInfo): Promise<void> {
  return invoke<void>("remove_session_files", { session });
}

export async function removeSessionsByScope(scope: SessionScope): Promise<void> {
  return invoke<void>("remove_sessions_by_scope", { scope });
}

export async function getSessionMessages(
  agent: Agent,
  filePath: string,
  sessionId?: string
): Promise<SessionMessagesResult> {
  return invoke<SessionMessagesResult>("get_session_messages", {
    agent,
    filePath,
    sessionId: sessionId ?? null,
  });
}

export async function updateSessionMessageCount(
  agent: Agent,
  filePath: string,
  messageCount: number,
  sessionId?: string
): Promise<void> {
  return invoke<void>("update_session_message_count", {
    agent,
    filePath,
    sessionId: sessionId ?? null,
    messageCount,
  });
}

export async function readLocalImageDataUrl(path: string): Promise<string> {
  return invoke<string>("read_local_image_data_url", { path });
}

export async function readLocalTextFile(path: string): Promise<string> {
  return invoke<string>("read_local_text_file", { path });
}

export async function writeCrossPrompt(
  sessionId: string,
  content: string,
): Promise<string> {
  return invoke<string>("write_cross_prompt", { sessionId, content });
}

export async function getAgentRuntimeStatus(agent: Agent): Promise<RuntimeStatus> {
  return invoke<RuntimeStatus>("get_agent_runtime_status", { agent });
}

export async function listRuntimeAgents(): Promise<RuntimeAgentMetadata[]> {
  return invoke<RuntimeAgentMetadata[]>("list_runtime_agents");
}

export async function startAgentSession(
  req: StartAgentSessionRequest,
): Promise<AgentSessionHandle> {
  return invoke<AgentSessionHandle>("start_agent_session", { req });
}

export async function createPendingSession(
  session: SessionInfo,
): Promise<void> {
  return invoke<void>("create_pending_session", { session });
}

export async function forkAgentSession(
  req: StartAgentSessionRequest,
): Promise<AgentSessionHandle> {
  return invoke<AgentSessionHandle>("fork_agent_session", { req });
}

export async function loadAgentSession(
  agent: Agent,
  runtimeSessionId: string,
  workspacePath: string,
  agentRuntimeSessionId?: string | null,
  sourceAgent?: Agent | null,
): Promise<AgentSessionHandle> {
  return invoke<AgentSessionHandle>("load_agent_session", {
    agent,
    runtimeSessionId,
    workspacePath,
    agentRuntimeSessionId: agentRuntimeSessionId ?? null,
    sourceAgent: sourceAgent ?? null,
  });
}

export async function ensureAgentRuntimeSession(
  req: EnsureAgentRuntimeSessionRequest,
): Promise<AgentSessionHandle> {
  return invoke<AgentSessionHandle>("ensure_agent_runtime_session", { req });
}

export async function sendAgentInput(
  sessioRuntimeSessionId: string,
  input: AgentInput,
): Promise<AgentTurnHandle> {
  return invoke<AgentTurnHandle>("send_agent_input", {
    sessioRuntimeSessionId,
    input,
  });
}

export async function cancelAgentTurn(
  sessioRuntimeSessionId: string,
  turnId: string,
): Promise<void> {
  return invoke<void>("cancel_agent_turn", {
    sessioRuntimeSessionId,
    turnId,
  });
}

export async function setAgentSessionConfigOption(
  sessioRuntimeSessionId: string,
  change: AgentSessionConfigChange,
): Promise<void> {
  return invoke<void>("set_agent_session_config_option", {
    sessioRuntimeSessionId,
    change,
  });
}

export async function respondAgentPermission(
  sessioRuntimeSessionId: string,
  requestId: string,
  optionId: string,
): Promise<void> {
  return invoke<void>("respond_agent_permission", {
    sessioRuntimeSessionId,
    requestId,
    optionId,
  });
}

export const AGENT_LABEL: Record<Agent, string> = {
  codex: "Codex",
  claude: "Claude Code",
  gemini: "Gemini",
};

const AGENT_COLOR_VAR: Record<Agent, string> = {
  codex: "--color-fg",
  claude: "--color-orange",
  gemini: "--color-blue",
};

export const AGENT_ACCENT: Record<Agent, string> = {
  codex: `rgb(var(${AGENT_COLOR_VAR.codex}))`,
  claude: `rgb(var(${AGENT_COLOR_VAR.claude}))`,
  gemini: `rgb(var(${AGENT_COLOR_VAR.gemini}))`,
};

export function agentTint(a: Agent, alpha: number): string {
  return `rgb(var(${AGENT_COLOR_VAR[a]}) / ${alpha})`;
}

export function agentColorVar(a: Agent): string {
  return `var(${AGENT_COLOR_VAR[a]})`;
}
