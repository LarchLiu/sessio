import { invoke } from "@tauri-apps/api/core";

export type Agent = "astra-pi" | "codex" | "claude" | "gemini";

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
  displayName: string;
  icon: string | null;
  aiProvider: string | null;
  aiProviders: AgentAiProviderInfo[];
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
  order: number;
  createdAt: number;
  updatedAt: number;
}

export interface AstraConfig {
  plannerAgent: string | null;
  plannerModel: string | null;
  plannerEffort: string | null;
  plannerPermissionMode: string | null;
  decisionAgent: string | null;
  decisionModel: string | null;
  decisionEffort: string | null;
  decisionPermissionMode: string | null;
  defaultModel: string | null;
  defaultEffort: string | null;
  defaultPermissionMode: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface AgentAiProviderInfo {
  id: string;
  displayName: string;
  provider: string;
  api: string | null;
  baseUrl: string | null;
  apiKey: string | null;
  model: string | null;
  models: RuntimeAgentOptionMetadata[];
  enabled: boolean;
  order: number;
}

export interface AgentCommandsInfo {
  session: string[];
  version: string[];
}

export interface NetworkConfig {
  proxy: NetworkProxyConfig;
}

export interface NetworkProxyConfig {
  enabled: boolean;
  url: string | null;
  noProxy: string | null;
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
  color: string | null;
  type: AssistantType;
  workflowId: string | null;
  projectId: string | null;
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface StageAssistantInfo {
  assistantId: string;
  name: string;
  color: string | null;
  agent: AssistantAgentInfo;
  systemPrompt?: string | null;
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

export type StageStatus =
  | "not_started"
  | "in_progress"
  | "blocked"
  | "needs_review"
  | "completed"
  | "skipped";

export type IssueStatus = "open" | "resolved" | "dismissed";

export type IssueSeverity = "low" | "medium" | "high" | "critical";

export interface StageIssueInfo {
  id: string;
  threadStageId: string;
  title: string;
  description: string | null;
  status: IssueStatus;
  severity: IssueSeverity;
  createdAt: number;
  updatedAt: number;
}

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
  icon: string | null;
  order: number;
  status: StageStatus;
  summary: string | null;
  outcome: string | null;
  enabled: boolean;
  allowEmptyAssistants: boolean;
  createdAt: number;
  updatedAt: number;
  sessions: SessionInfo[];
  issues: StageIssueInfo[];
}

export interface ProjectStageInfo {
  id: string;
  projectId: string | null;
  type: ProjectStageType;
  workflowId: string | null;
  kind: StageType | null;
  name: string | null;
  description: string | null;
  icon: string | null;
  order: number;
  enabled: boolean;
  allowEmptyAssistants: boolean;
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
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
  stages: StageInfo[];
  sessions: SessionInfo[];
}

export interface ThreadWorkState extends ThreadInfo {}

export type AstraRunStatus =
  | "planning"
  | "awaiting_approval"
  | "dispatching"
  | "running"
  | "completed"
  | "cancelled"
  | "errored"
  | "interrupted";

export type AstraTaskRisk = "low" | "medium" | "high";

export interface AstraTaskProposal {
  id: string;
  title: string;
  targetStageId: string | null;
  targetAgent: Agent;
  prompt: string;
  expectedOutput: string;
  risk: AstraTaskRisk;
}

export type AstraTaskResultStatus = "completed" | "failed" | "errored" | "cancelled";

export interface AstraTaskResult {
  taskId: string;
  threadStageId: string | null;
  sessioRuntimeSessionId: string;
  turnId: string | null;
  status: AstraTaskResultStatus;
  output: string;
  error: string | null;
  attemptCount: number;
  retryLimitReached: boolean;
  decisionAction?: string | null;
  decisionReason?: string | null;
  completedAt: number;
}

export interface AstraHandle {
  runId: string;
  threadId: string;
  projectId: string;
  status: AstraRunStatus;
  proposedTasks: AstraTaskProposal[];
  approvedTaskIds: string[];
  delegatedSessionIds: string[];
  taskResults: AstraTaskResult[];
  mode: string;
  currentStageId: string | null;
  currentTaskId: string | null;
  completedTaskIds: string[];
  stageAttemptCounts: Record<string, number>;
  retryLimit: number;
  plannerBackend: string | null;
  decisionBackend: string | null;
  roundIndex: number | null;
  roundLimit: number;
  terminalReason: string | null;
  lastErrorCode: string | null;
  lastErrorMessage: string | null;
  internalPlannerSessionIds: string[];
  internalDecisionSessionIds: string[];
  runDiagnostics: unknown[];
  error: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface AstraEvent {
  runId: string;
  threadId: string;
  status: AstraRunStatus;
  eventType: string;
  data: unknown;
  timestamp: number;
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
  renameTitle: string | null;
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

export type RuntimeTransportKind = "acp" | "cliStreamJson" | "plainCli" | "sidecar" | "fake";

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
  order: number;
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
  updatePreview: boolean;
}

export interface RuntimeAgentOptionMetadata {
  value: string;
  label: string;
  displayName: string;
  enabled: boolean;
  order: number;
}

export interface UpdateRuntimeAgentPreferencesRequest {
  agent: Agent;
  displayName?: string | null;
  enabled?: boolean | null;
  order?: number | null;
  aiProvider?: string | null;
  aiProviders?: AgentAiProviderInfo[];
  model?: string | null;
  effort?: string | null;
  permissionMode?: string | null;
  models?: RuntimeAgentOptionMetadata[];
  efforts?: RuntimeAgentOptionMetadata[];
  permissionModes?: RuntimeAgentOptionMetadata[];
}

export interface UpdateAgentPreferencesRequest {
  agentId: string;
  displayName?: string | null;
  enabled?: boolean | null;
  order?: number | null;
  aiProvider?: string | null;
  aiProviders?: AgentAiProviderInfo[];
  model?: string | null;
  effort?: string | null;
  permissionMode?: string | null;
  models?: RuntimeAgentOptionMetadata[];
  efforts?: RuntimeAgentOptionMetadata[];
  permissionModes?: RuntimeAgentOptionMetadata[];
}

export interface RuntimeAgentSelection {
  agent: Agent;
  model: string | null;
  effort: string | null;
  permissionMode: string | null;
  updatedAt: number;
}

export interface SetRuntimeAgentSelectionRequest {
  agent: Agent;
  model?: string | null;
  effort?: string | null;
  permissionMode?: string | null;
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
      metadata: Record<string, unknown>;
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

export async function updateSessionRenameTitle(
  agent: Agent,
  sessionId: string,
  renameTitle: string | null,
): Promise<void> {
  return invoke<void>("update_session_rename_title", { agent, sessionId, renameTitle });
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
  enabledStageIds?: string[] | null,
): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("add_existing_project", {
    path,
    name: name ?? null,
    workflowId: workflowId ?? null,
    enabledStageIds: enabledStageIds ?? null,
  });
}

export async function createProject(
  parentPath: string,
  name: string,
  workflowId?: string | null,
  enabledStageIds?: string[] | null,
): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("create_project", {
    parentPath,
    name,
    workflowId: workflowId ?? null,
    enabledStageIds: enabledStageIds ?? null,
  });
}

export async function createDefaultProject(
  name: string,
  workflowId?: string | null,
  enabledStageIds?: string[] | null,
): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("create_default_project", {
    name,
    workflowId: workflowId ?? null,
    enabledStageIds: enabledStageIds ?? null,
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

export async function getAstraConfig(): Promise<AstraConfig> {
  return invoke<AstraConfig>("get_astra_config");
}

export async function updateAstraConfig(config: Partial<AstraConfig>): Promise<AstraConfig> {
  return invoke<AstraConfig>("update_astra_config", { config });
}

export async function listAssistants(projectId?: string | null): Promise<AssistantInfo[]> {
  return invoke<AssistantInfo[]>("list_assistants", { projectId: projectId ?? null });
}

export async function createAssistant(input: {
  agent: AssistantAgentInfo;
  name: string;
  systemPrompt?: string | null;
  color?: string | null;
  type: AssistantType;
  workflowId?: string | null;
  projectId?: string | null;
}): Promise<AssistantInfo> {
  return invoke<AssistantInfo>("create_assistant", {
    req: {
      name: input.name,
      agent: input.agent,
      systemPrompt: input.systemPrompt ?? null,
      color: input.color ?? null,
      assistantType: input.type,
      workflowId: input.workflowId ?? null,
      projectId: input.projectId ?? null,
    },
  });
}

export async function updateAssistant(
  assistantId: string,
  patch: {
    name?: string | null;
    agent?: AssistantAgentInfo | null;
    systemPrompt?: string | null;
    color?: string | null;
    enabled?: boolean | null;
  },
): Promise<AssistantInfo> {
  return invoke<AssistantInfo>("update_assistant", {
    req: {
      assistantId,
      name: patch.name ?? null,
      agent: patch.agent ?? null,
      enabled: patch.enabled ?? null,
      color:
        Object.prototype.hasOwnProperty.call(patch, "color")
          ? patch.color === null
            ? null
            : patch.color
          : undefined,
      systemPrompt:
        Object.prototype.hasOwnProperty.call(patch, "systemPrompt")
          ? patch.systemPrompt === null
            ? null
            : patch.systemPrompt ?? ""
          : null,
    },
  });
}

export async function deleteAssistant(assistantId: string): Promise<void> {
  return invoke<void>("delete_assistant", { assistantId });
}

export async function listThreads(projectId: string): Promise<ThreadInfo[]> {
  return invoke<ThreadInfo[]>("list_threads", { projectId });
}

export async function getThreadWorkState(threadId: string): Promise<ThreadWorkState> {
  return invoke<ThreadWorkState>("get_thread_work_state", { threadId });
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
  patch: { goal?: string | null; description?: string | null; enabled?: boolean | null },
): Promise<ThreadInfo> {
  return invoke<ThreadInfo>("update_thread", {
    threadId,
    goal: patch.goal ?? null,
    enabled: patch.enabled ?? null,
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

export async function createAstraRun(
  threadId: string,
  prompt?: string | null,
): Promise<AstraHandle> {
  return invoke<AstraHandle>("create_astra_run", {
    req: { threadId, prompt: prompt ?? null },
  });
}

export async function cancelAstraRun(runId: string): Promise<AstraHandle> {
  return invoke<AstraHandle>("cancel_astra_run", { req: { runId } });
}

export async function listAstraRuns(threadId: string): Promise<AstraHandle[]> {
  return invoke<AstraHandle[]>("list_astra_runs", { threadId });
}

export async function getAstraRun(runId: string): Promise<AstraHandle> {
  return invoke<AstraHandle>("get_astra_run", { runId });
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
  icon?: string | null,
): Promise<ProjectStageInfo> {
  return invoke<ProjectStageInfo>("create_project_stage", {
    projectId,
    workflowId: workflowId ?? null,
    name,
    description: description ?? null,
    icon: icon ?? null,
  });
}

export async function updateProjectStage(
  stageId: string,
  patch: {
    name?: string | null;
    description?: string | null;
    icon?: string | null;
    order?: number | null;
    enabled?: boolean | null;
    allowEmptyAssistants?: boolean | null;
  },
): Promise<ProjectStageInfo> {
  const payload: {
    stageId: string;
    name?: string | null;
    description?: string | null;
    icon?: string | null;
    order?: number | null;
    enabled?: boolean | null;
    allowEmptyAssistants?: boolean | null;
  } = { stageId };
  if ("name" in patch) payload.name = patch.name ?? null;
  if ("description" in patch) payload.description = patch.description ?? null;
  if ("icon" in patch) payload.icon = patch.icon ?? null;
  if ("order" in patch) payload.order = patch.order ?? null;
  if ("enabled" in patch) payload.enabled = patch.enabled ?? null;
  if ("allowEmptyAssistants" in patch) payload.allowEmptyAssistants = patch.allowEmptyAssistants ?? null;
  return invoke<ProjectStageInfo>("update_project_stage", { req: payload });
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
    enabled?: boolean | null;
  },
): Promise<StageInfo> {
  return invoke<StageInfo>("update_thread_stage", {
    threadStageId,
    assistantIds: patch.assistantIds ?? null,
    order: patch.order ?? null,
    enabled: patch.enabled ?? null,
  });
}

export async function updateThreadStageState(
  threadStageId: string,
  patch: {
    status?: StageStatus;
    summary?: string | null;
    outcome?: string | null;
  },
): Promise<StageInfo> {
  return invoke<StageInfo>("update_thread_stage_state", {
    threadStageId,
    status: patch.status ?? null,
    summary: patch.summary === undefined ? null : patch.summary ?? "",
    outcome: patch.outcome === undefined ? null : patch.outcome ?? "",
  });
}

export async function listThreadStageIssues(
  threadStageId: string,
): Promise<StageIssueInfo[]> {
  return invoke<StageIssueInfo[]>("list_thread_stage_issues", { threadStageId });
}

export async function createThreadStageIssue(
  threadStageId: string,
  title: string,
  severity: IssueSeverity,
  description?: string | null,
): Promise<StageIssueInfo> {
  return invoke<StageIssueInfo>("create_thread_stage_issue", {
    threadStageId,
    title,
    severity,
    description: description ?? null,
  });
}

export async function updateThreadStageIssue(
  issueId: string,
  patch: {
    title?: string;
    description?: string | null;
    status?: IssueStatus;
    severity?: IssueSeverity;
  },
): Promise<StageIssueInfo> {
  return invoke<StageIssueInfo>("update_thread_stage_issue", {
    issueId,
    title: patch.title ?? null,
    description: patch.description === undefined ? null : patch.description ?? "",
    status: patch.status ?? null,
    severity: patch.severity ?? null,
  });
}

export async function deleteThreadStageIssue(issueId: string): Promise<void> {
  return invoke<void>("delete_thread_stage_issue", { issueId });
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

export async function linkThreadSession(
  threadId: string,
  agent: Agent,
  sessionId: string,
): Promise<ThreadInfo> {
  return invoke<ThreadInfo>("link_thread_session", { threadId, agent, sessionId });
}

export async function unlinkThreadSession(
  threadId: string,
  agent: Agent,
  sessionId: string,
): Promise<ThreadInfo> {
  return invoke<ThreadInfo>("unlink_thread_session", { threadId, agent, sessionId });
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

export interface ThreadWorkSnapshotStage {
  threadStageId: string;
  projectStageId?: string | null;
  name: string;
  kind: StageType | null;
  icon?: string | null;
  status: StageStatus;
  summary: string | null;
  outcome: string | null;
  assistants?: StageAssistantInfo[];
  issues?: StageIssueInfo[];
  sessionRefs: ThreadWorkSnapshotSessionRef[];
}

export interface ThreadWorkSnapshotSessionRef {
  agent: Agent;
  sessionId: string;
  title: string | null;
  filePath?: string | null;
  sourceKind?: "thread" | "stage";
  ancestorIndex?: number | null;
}

export interface ThreadWorkSnapshotDetailRefs {
  threadId: string;
  focusedStageId: string | null;
  stageIds: string[];
  issueIds: string[];
  sessionRefs: ThreadWorkSnapshotSessionRef[];
}

export interface ThreadWorkSnapshotRelatedContext {
  sessionExcerptRefs: ThreadWorkSnapshotSessionRef[];
}

export interface ThreadWorkSnapshot {
  threadId: string;
  projectId: string;
  goal: string;
  description: string | null;
  activeStageId: string | null;
  focusedStageId: string | null;
  stages: ThreadWorkSnapshotStage[];
  threadSessionRefs?: ThreadWorkSnapshotSessionRef[];
  relatedContext?: ThreadWorkSnapshotRelatedContext;
  detailRefs?: ThreadWorkSnapshotDetailRefs;
  rollup: {
    completed: number;
    incomplete: number;
    blocked: number;
    openIssues?: number;
    currentStage?: string | null;
    total: number;
  };
  capturedAt: number;
}

export interface ThreadWorkSnapshotResult {
  childAgent: Agent;
  childSessionId: string;
  threadId: string;
  stageId: string | null;
  version: number;
  createdAt: number;
  snapshot: ThreadWorkSnapshot;
}

export interface ThreadWorkSnapshotSourceRef {
  kind: string;
  id: string;
  label: string;
  threadId: string | null;
  threadStageId: string | null;
  issueId: string | null;
  agent: Agent | null;
  sessionId: string | null;
  filePath: string | null;
  ancestorIndex: number | null;
}

export interface ThreadWorkSnapshotSourcesResult {
  childAgent: Agent;
  childSessionId: string;
  threadId: string;
  stageId: string | null;
  sources: ThreadWorkSnapshotSourceRef[];
}

export async function saveThreadWorkSnapshot(
  childAgent: Agent,
  childSessionId: string,
  threadId: string,
  stageId: string | null,
  snapshot: ThreadWorkSnapshot,
): Promise<void> {
  return invoke<void>("save_thread_work_snapshot", {
    childAgent,
    childSessionId,
    threadId,
    stageId,
    snapshot,
  });
}

export async function getThreadWorkSnapshot(
  childAgent: Agent,
  childSessionId: string,
): Promise<ThreadWorkSnapshotResult | null> {
  return invoke<ThreadWorkSnapshotResult | null>("get_thread_work_snapshot", {
    childAgent,
    childSessionId,
  });
}

export async function getThreadWorkSnapshotSources(
  childAgent: Agent,
  childSessionId: string,
): Promise<ThreadWorkSnapshotSourcesResult | null> {
  return invoke<ThreadWorkSnapshotSourcesResult | null>("get_thread_work_snapshot_sources", {
    childAgent,
    childSessionId,
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

export async function getLastRuntimeAgentSelection(): Promise<RuntimeAgentSelection | null> {
  return invoke<RuntimeAgentSelection | null>("get_last_runtime_agent_selection");
}

export async function setLastRuntimeAgentSelection(
  req: SetRuntimeAgentSelectionRequest,
): Promise<RuntimeAgentSelection> {
  return invoke<RuntimeAgentSelection>("set_last_runtime_agent_selection", { req });
}

export async function getDebugConfig(): Promise<DebugConfig> {
  return invoke<DebugConfig>("get_debug_config");
}

export async function getNetworkConfig(): Promise<NetworkConfig> {
  return invoke<NetworkConfig>("get_network_config");
}

export async function updateNetworkConfig(config: NetworkConfig): Promise<NetworkConfig> {
  return invoke<NetworkConfig>("update_network_config", { config });
}

export async function updateRuntimeAgentPreferences(
  req: UpdateRuntimeAgentPreferencesRequest,
): Promise<RuntimeAgentMetadata> {
  return invoke<RuntimeAgentMetadata>("update_runtime_agent_preferences", { req });
}

export async function updateAgentPreferences(
  req: UpdateAgentPreferencesRequest,
): Promise<AgentInfo> {
  return invoke<AgentInfo>("update_agent_preferences", { req });
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
  "astra-pi": "Astra Pi",
  codex: "Codex",
  claude: "Claude Code",
  gemini: "Gemini",
};

const AGENT_COLOR_VAR: Record<Agent, string> = {
  "astra-pi": "--color-purple",
  codex: "--color-fg",
  claude: "--color-orange",
  gemini: "--color-blue",
};

export const AGENT_ACCENT: Record<Agent, string> = {
  "astra-pi": `rgb(var(${AGENT_COLOR_VAR["astra-pi"]}))`,
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
