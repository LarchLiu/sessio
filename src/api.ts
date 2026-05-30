import { invoke } from "@tauri-apps/api/core";

export type Agent = "codex" | "claude" | "gemini";

export type WorkflowType = "builtin" | "custom";

export interface WorkflowInfo {
  id: string;
  name: string;
  description: string | null;
  type: WorkflowType;
  createdAt: number;
  updatedAt: number;
}

export interface ProjectInfo {
  id: string;
  path: string;
  name: string;
  workflowId: string;
  createdAt: number;
  updatedAt: number;
  sessionCount: number;
}

export type AgentType = "builtin" | "custom";

export interface AgentInfo {
  id: string;
  name: string;
  icon: string | null;
  model: string | null;
  models: RuntimeAgentOptionMetadata[];
  effort: string | null;
  efforts: RuntimeAgentOptionMetadata[];
  permissionMode: string | null;
  permissionModes: RuntimeAgentOptionMetadata[];
  type: AgentType;
  enabled: boolean;
  transport: RuntimeTransportKind;
  commands: AgentCommandsInfo;
  createdAt: number;
  updatedAt: number;
}

export interface AgentCommandsInfo {
  session: string[];
  version: string[];
}

export type AssistantType = "builtin" | "custom";

export interface AssistantAgentInfo {
  id: string;
  name: string;
  model: string;
  mode: string;
  effort: string;
}

export interface AssistantInfo {
  id: string;
  name: string;
  agent: AssistantAgentInfo;
  systemPrompt: string | null;
  type: AssistantType;
  workflowId: string | null;
  projectId: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface StageAssistantInfo {
  assistantId: string;
  name: string;
  agent: AssistantAgentInfo;
  order: number;
}

export type StageType =
  | "research"
  | "plan"
  | "develop"
  | "build"
  | "writing"
  | "editing"
  | "review"
  | "proofreading"
  | "screenplay"
  | "storyboard"
  | "design"
  | "production"
  | "human"
  | "done";

export type ProjectStageType = "builtin" | "custom";

export interface StageInfo {
  id: string;
  threadId: string;
  stageId: string;
  projectId: string;
  assistantIds: string[];
  assistants: StageAssistantInfo[];
  type: ProjectStageType;
  workflowId: string | null;
  kind: StageType | null;
  name: string | null;
  description: string | null;
  order: number;
  createdAt: number;
  updatedAt: number;
  sessions: SessionInfo[];
}

export interface ProjectStageInfo {
  id: string;
  projectId: string | null;
  type: ProjectStageType;
  workflowId: string | null;
  kind: StageType | null;
  name: string | null;
  description: string | null;
  order: number;
  createdAt: number;
  updatedAt: number;
  assistants: StageAssistantInfo[];
}

export interface ThreadInfo {
  id: string;
  projectId: string;
  goal: string;
  description: string | null;
  stageId: string | null;
  createdAt: number;
  updatedAt: number;
  stages: StageInfo[];
}

export type KanbanStatus =
  | "todo"
  | "in_progress"
  | "canceled"
  | "agent_review"
  | "human_review"
  | "done";

export interface KanbanItem {
  id: string;
  projectId: string;
  title: string;
  description: string | null;
  status: KanbanStatus;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
  sessions: SessionInfo[];
}

export interface SessionInfo {
  id: string;
  agent: Agent;
  forkedFromAgent?: Agent | null;
  forkedFromId?: string | null;
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

export type SessionContentBlock =
  | {
      type: "text";
      text: string;
      annotations?: unknown | null;
      meta?: unknown | null;
    }
  | {
      type: "image" | "audio";
      uri?: string;
      data?: string;
      mimeType?: string;
      annotations?: unknown | null;
      meta?: unknown | null;
    }
  | {
      type: "resource";
      uri?: string;
      name?: string;
      mimeType?: string;
      text?: string;
      blob?: string;
      resource?: unknown;
      annotations?: unknown | null;
      meta?: unknown | null;
    }
  | {
      type: "resource_link";
      uri: string;
      name?: string;
      title?: string;
      description?: string;
      mimeType?: string;
      size?: number;
      annotations?: unknown | null;
      meta?: unknown | null;
    }
  | {
      type: "unknown";
      uri?: string;
      name?: string;
      title?: string;
      description?: string;
      mimeType?: string;
      size?: number;
      text?: string;
      blob?: string;
      resource?: unknown;
      annotations?: unknown | null;
      meta?: unknown | null;
    }
  | (Record<string, unknown> & {
      type: string;
      meta?: unknown | null;
    });

export interface SessionHistoryResult {
  messageCount: number;
  indexedThrough: number | null;
  turns: SessionHistoryTurn[];
}

export interface SessionHistorySnapshotGroup {
  ancestorAgent: Agent;
  ancestorSessionId: string;
  ancestorIndex: number;
  turns: SessionHistoryTurn[];
}

export interface SessionHistorySnapshotsResult {
  hasSnapshot: boolean;
  groups: SessionHistorySnapshotGroup[];
}

export interface SessionHistoryTurn {
  turnId: string;
  status: RuntimeTurnStatus;
  blocks: SessionHistoryRenderBlock[];
  tools: SessionHistoryToolCall[];
  permissions: SessionHistoryPermissionRequest[];
  protocolMessages: AcpProtocolMessage[];
  stopReason: string | null;
  error: RuntimeError | null;
  startedAt: number;
  updatedAt: number;
}

export type SessionHistoryRenderBlock =
  | { kind: "user"; blocks: SessionContentBlock[]; raw: unknown; timestamp?: number }
  | { kind: "assistant"; blocks: SessionContentBlock[]; raw: unknown; timestamp?: number }
  | { kind: "thought"; blocks: SessionContentBlock[]; raw: unknown; timestamp?: number }
  | { kind: "tool"; toolId: string; timestamp?: number }
  | { kind: "permission"; requestId: string; timestamp?: number }
  | { kind: "sessionUpdate"; updateType: string; data: unknown; timestamp?: number }
  | { kind: "error"; error: RuntimeError; timestamp?: number };

export interface SessionHistoryToolCall {
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

export interface SessionHistoryPermissionRequest {
  requestId: string;
  toolCall: unknown;
  toolName: string;
  input: unknown | null;
  options: unknown[];
  selectedOptionId: string | null;
  cancelled: boolean;
  raw: unknown;
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
  model: string | null;
  models: RuntimeAgentOptionMetadata[];
  effort: string | null;
  efforts: RuntimeAgentOptionMetadata[];
  permissionMode: string | null;
  permissionModes: RuntimeAgentOptionMetadata[];
  sessionCommand: string | null;
  versionCommand: string | null;
  detectedVersion: string | null;
  capabilities: RuntimeCapabilitySet | null;
  updatedAt: number | null;
}

export interface DebugConfig {
  acpConfig: boolean;
}

export interface RuntimeAgentOptionMetadata {
  value: string;
  label: string;
}

export interface UpdateRuntimeAgentPreferencesRequest {
  agent: Agent;
  model?: string | null;
  effort?: string | null;
  permissionMode?: string | null;
  models?: RuntimeAgentOptionMetadata[];
  efforts?: RuntimeAgentOptionMetadata[];
  permissionModes?: RuntimeAgentOptionMetadata[];
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
  displayName?: string | null;
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

export async function listProjects(): Promise<ProjectInfo[]> {
  return invoke<ProjectInfo[]>("list_projects");
}

export async function listWorkflows(): Promise<WorkflowInfo[]> {
  return invoke<WorkflowInfo[]>("list_workflows");
}

export async function createWorkflow(name: string, description?: string | null): Promise<WorkflowInfo> {
  return invoke<WorkflowInfo>("create_workflow", { name, description: description ?? null });
}

export async function updateWorkflow(
  workflowId: string,
  patch: { name?: string | null; description?: string | null },
): Promise<WorkflowInfo> {
  return invoke<WorkflowInfo>("update_workflow", {
    workflowId,
    name: patch.name ?? null,
    description: patch.description === undefined ? undefined : patch.description,
  });
}

export async function deleteWorkflow(workflowId: string): Promise<void> {
  return invoke<void>("delete_workflow", { workflowId });
}

export async function addExistingProject(
  path: string,
  name?: string | null,
  workflowId?: string | null,
): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("add_existing_project", {
    path,
    name: name ?? null,
    workflowId: workflowId ?? null,
  });
}

export async function createProject(
  parentPath: string,
  name: string,
  workflowId?: string | null,
): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("create_project", {
    parentPath,
    name,
    workflowId: workflowId ?? null,
  });
}

export async function createDefaultProject(
  name: string,
  workflowId?: string | null,
): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("create_default_project", {
    name,
    workflowId: workflowId ?? null,
  });
}

export async function updateProject(
  projectId: string,
  patch: { name?: string | null; workflowId?: string | null },
): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("update_project", {
    projectId,
    name: patch.name ?? null,
    workflowId: patch.workflowId ?? null,
  });
}

export async function archiveProject(projectId: string): Promise<void> {
  return invoke<void>("archive_project", { projectId });
}

export async function listAgents(): Promise<AgentInfo[]> {
  return invoke<AgentInfo[]>("list_agents");
}

export async function listAssistants(projectId?: string | null): Promise<AssistantInfo[]> {
  return invoke<AssistantInfo[]>("list_assistants", { projectId: projectId ?? null });
}

export async function createAssistant(input: {
  agent: AssistantAgentInfo;
  name: string;
  systemPrompt?: string | null;
  type: AssistantType;
  workflowId?: string | null;
  projectId?: string | null;
}): Promise<AssistantInfo> {
  return invoke<AssistantInfo>("create_assistant", {
    name: input.name,
    agent: input.agent,
    systemPrompt: input.systemPrompt ?? null,
    assistantType: input.type,
    workflowId: input.workflowId ?? null,
    projectId: input.projectId ?? null,
  });
}

export async function updateAssistant(
  assistantId: string,
  patch: {
    name?: string | null;
    agent?: AssistantAgentInfo | null;
    systemPrompt?: string | null;
  },
): Promise<AssistantInfo> {
  return invoke<AssistantInfo>("update_assistant", {
    assistantId,
    name: patch.name ?? null,
    agent: patch.agent ?? null,
    systemPrompt:
      Object.prototype.hasOwnProperty.call(patch, "systemPrompt")
        ? patch.systemPrompt === null
          ? null
          : patch.systemPrompt ?? ""
        : null,
  });
}

export async function deleteAssistant(assistantId: string): Promise<void> {
  return invoke<void>("delete_assistant", { assistantId });
}

export async function listThreads(projectId: string): Promise<ThreadInfo[]> {
  return invoke<ThreadInfo[]>("list_threads", { projectId });
}

export async function createThread(
  projectId: string,
  goal: string,
  description?: string | null,
): Promise<ThreadInfo> {
  return invoke<ThreadInfo>("create_thread", {
    projectId,
    goal,
    description: description ?? null,
  });
}

export async function updateThread(
  threadId: string,
  patch: { goal?: string | null; description?: string | null },
): Promise<ThreadInfo> {
  return invoke<ThreadInfo>("update_thread", {
    threadId,
    goal: patch.goal ?? null,
    description:
      Object.prototype.hasOwnProperty.call(patch, "description")
        ? patch.description === null
          ? null
          : patch.description ?? ""
        : null,
  });
}

export async function deleteThread(threadId: string): Promise<void> {
  return invoke<void>("delete_thread", { threadId });
}

export async function listProjectStages(projectId: string): Promise<ProjectStageInfo[]> {
  return invoke<ProjectStageInfo[]>("list_project_stages", { projectId });
}

export async function listWorkflowStages(workflowId: string): Promise<ProjectStageInfo[]> {
  return invoke<ProjectStageInfo[]>("list_workflow_stages", { workflowId });
}

export async function createProjectStage(
  projectId: string,
  name: string,
  description?: string | null,
  workflowId?: string | null,
): Promise<ProjectStageInfo> {
  return invoke<ProjectStageInfo>("create_project_stage", {
    projectId,
    workflowId: workflowId ?? null,
    name,
    description: description ?? null,
  });
}

export async function updateProjectStage(
  stageId: string,
  patch: {
    name?: string | null;
    description?: string | null;
    order?: number | null;
  },
): Promise<ProjectStageInfo> {
  const payload: {
    stageId: string;
    name?: string | null;
    description?: string | null;
    order?: number | null;
  } = { stageId };
  if ("name" in patch) payload.name = patch.name ?? null;
  if ("description" in patch) payload.description = patch.description ?? null;
  if ("order" in patch) payload.order = patch.order ?? null;
  return invoke<ProjectStageInfo>("update_project_stage", payload);
}

export async function updateProjectStageAssistants(
  stageId: string,
  assistantIds: string[],
): Promise<ProjectStageInfo> {
  return invoke<ProjectStageInfo>("update_project_stage_assistants", {
    stageId,
    assistantIds,
  });
}

export async function deleteProjectStage(stageId: string): Promise<void> {
  return invoke<void>("delete_project_stage", { stageId });
}

export async function addThreadStage(
  threadId: string,
  stageId: string,
  assistantIds: string[],
): Promise<StageInfo> {
  return invoke<StageInfo>("add_thread_stage", { threadId, stageId, assistantIds });
}

export async function updateThreadStage(
  threadStageId: string,
  patch: {
    assistantIds?: string[] | null;
    order?: number | null;
  },
): Promise<StageInfo> {
  return invoke<StageInfo>("update_thread_stage", {
    threadStageId,
    assistantIds: patch.assistantIds ?? null,
    order: patch.order ?? null,
  });
}

export async function updateThreadStageAssistantAgent(
  threadStageId: string,
  assistantId: string,
  agent: AssistantAgentInfo,
): Promise<StageInfo> {
  return invoke<StageInfo>("update_thread_stage_assistant_agent", { threadStageId, assistantId, agent });
}

export async function deleteThreadStage(threadStageId: string): Promise<void> {
  return invoke<void>("delete_thread_stage", { threadStageId });
}

export async function setThreadStage(
  threadId: string,
  stageId: string,
): Promise<ThreadInfo> {
  return invoke<ThreadInfo>("set_thread_stage", { threadId, stageId });
}

export async function linkStageSession(
  stageId: string,
  agent: Agent,
  sessionId: string,
): Promise<StageInfo> {
  return invoke<StageInfo>("link_stage_session", { stageId, agent, sessionId });
}

export async function unlinkStageSession(
  stageId: string,
  agent: Agent,
  sessionId: string,
): Promise<StageInfo> {
  return invoke<StageInfo>("unlink_stage_session", { stageId, agent, sessionId });
}

export async function listKanbanItems(projectId: string): Promise<KanbanItem[]> {
  return invoke<KanbanItem[]>("list_kanban_items", { projectId });
}

export async function createKanbanItem(
  projectId: string,
  title: string,
  description?: string | null,
): Promise<KanbanItem> {
  return invoke<KanbanItem>("create_kanban_item", {
    projectId,
    title,
    description: description ?? null,
  });
}

export async function updateKanbanItem(
  itemId: string,
  patch: {
    title?: string | null;
    description?: string | null;
    status?: KanbanStatus | null;
  },
): Promise<KanbanItem> {
  return invoke<KanbanItem>("update_kanban_item", {
    itemId,
    title: patch.title ?? null,
    description:
      Object.prototype.hasOwnProperty.call(patch, "description")
        ? patch.description === null
          ? null
          : patch.description ?? ""
        : null,
    status: patch.status ?? null,
  });
}

export async function updateKanbanItemStatus(
  itemId: string,
  status: KanbanStatus,
): Promise<KanbanItem> {
  return invoke<KanbanItem>("update_kanban_item_status", { itemId, status });
}

export async function deleteKanbanItem(itemId: string): Promise<void> {
  return invoke<void>("delete_kanban_item", { itemId });
}

export async function linkKanbanItemSession(
  itemId: string,
  agent: Agent,
  sessionId: string,
): Promise<KanbanItem> {
  return invoke<KanbanItem>("link_kanban_item_session", { itemId, agent, sessionId });
}

export async function unlinkKanbanItemSession(
  itemId: string,
  agent: Agent,
  sessionId: string,
): Promise<KanbanItem> {
  return invoke<KanbanItem>("unlink_kanban_item_session", { itemId, agent, sessionId });
}

export async function getSessionAncestors(
  agent: Agent,
  sessionId: string,
): Promise<SessionInfo[]> {
  return invoke<SessionInfo[]>("get_session_ancestors", { agent, sessionId });
}

export async function getSessionHistorySnapshots(
  childAgent: Agent,
  childSessionId: string,
): Promise<SessionHistorySnapshotsResult> {
  return invoke<SessionHistorySnapshotsResult>("get_session_history_snapshots", {
    childAgent,
    childSessionId,
  });
}

export async function saveSessionHistorySnapshots(
  childAgent: Agent,
  childSessionId: string,
  groups: SessionHistorySnapshotGroup[],
): Promise<void> {
  return invoke<void>("save_session_history_snapshots", {
    childAgent,
    childSessionId,
    groups,
  });
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

export async function getSessionHistory(
  agent: Agent,
  filePath: string,
  sessionId?: string
): Promise<SessionHistoryResult> {
  return invoke<SessionHistoryResult>("get_session_history", {
    agent,
    filePath,
    sessionId: sessionId ?? null,
  });
}

export async function updateSessionHistoryCount(
  agent: Agent,
  filePath: string,
  messageCount: number,
  sessionId?: string
): Promise<void> {
  return invoke<void>("update_session_history_count", {
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

export async function getDebugConfig(): Promise<DebugConfig> {
  return invoke<DebugConfig>("get_debug_config");
}

export async function updateRuntimeAgentPreferences(
  req: UpdateRuntimeAgentPreferencesRequest,
): Promise<RuntimeAgentMetadata> {
  return invoke<RuntimeAgentMetadata>("update_runtime_agent_preferences", { req });
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
