import { invoke } from "@tauri-apps/api/core";
import type {
  BuildCanvasContextFileRequest,
  CanvasAnchorInfo,
  CanvasBlockRecord,
  CanvasDocumentState,
  CanvasKey,
  CanvasRevisionInfo,
  SaveCanvasDraftRequest,
  SaveCanvasRevisionRequest,
  UpdateCanvasBlocksRequest,
  UpsertCanvasAnchorRequest,
} from "./canvasTypes";
import type { Agent as GeneratedAgent } from "./bindings/Agent";
import type { ProcessTemplateInfo as GeneratedProcessTemplateInfo } from "./bindings/ProcessTemplateInfo";
import type { ProcessTemplateType as GeneratedProcessTemplateType } from "./bindings/ProcessTemplateType";
import type { ProjectInfo as GeneratedProjectInfo } from "./bindings/ProjectInfo";
import type { AgentAiProviderInfo as GeneratedAgentAiProviderInfo } from "./bindings/AgentAiProviderInfo";
import type { AgentCommandsInfo as GeneratedAgentCommandsInfo } from "./bindings/AgentCommandsInfo";
import type { AgentType as GeneratedAgentType } from "./bindings/AgentType";
import type { AstraConfig as GeneratedAstraConfig } from "./bindings/AstraConfig";
import type { RuntimeAgentOptionMetadata as GeneratedRuntimeAgentOptionMetadata } from "./bindings/RuntimeAgentOptionMetadata";

export type Agent = GeneratedAgent;

/// Single source of truth for runtime agent ids. Keep in sync with the
/// `Agent` enum on the Rust side. Adding a new agent here is the only TS
/// place callers should touch — `isAgent`, `Record<Agent, …>` literals,
/// and AGENTS-driven loops pick up the rest at compile time.
export const AGENTS = ["pi", "omp", "codex", "claude", "opencode"] as const;

// Compile-time guard: AGENTS must cover every Agent variant and only contain
// Agent variants. If either side drifts, TypeScript fails here.
type _AgentsExhaustive = Exclude<Agent, (typeof AGENTS)[number]> extends never
  ? Exclude<(typeof AGENTS)[number], Agent> extends never
    ? true
    : false
  : false;
const _agentsExhaustive: _AgentsExhaustive = true;
void _agentsExhaustive;

export function isAgent(value: unknown): value is Agent {
  return typeof value === "string" && (AGENTS as readonly string[]).includes(value);
}

export type ProcessTemplateType = GeneratedProcessTemplateType;

export type ProcessTemplateInfo = GeneratedProcessTemplateInfo;

export type ProjectInfo = GeneratedProjectInfo;

export interface TerminalSessionInfo {
  id: string;
  title: string;
  cwd: string;
  shell: string;
  cols: number;
  rows: number;
  output: string;
  running: boolean;
  exitCode: number | null;
  createdAtMs: number;
}

export type TerminalEvent =
  | {
      kind: "created";
      session: TerminalSessionInfo;
    }
  | {
      kind: "output";
      data: string;
    }
  | {
      kind: "resized";
      cols: number;
      rows: number;
    }
  | {
      kind: "closed";
      exitCode: number | null;
    }
  | {
      kind: "removed";
    };

export interface TerminalEventEnvelope {
  terminalId: string;
  event: TerminalEvent;
}

export interface SessioAppInfo {
  id: string;
  slug: string;
  directoryPath: string;
  htmlPath: string | null;
  htmlFileName: string | null;
  logoPath: string | null;
  nameZh: string | null;
  nameEn: string | null;
  permissions: SessioAppPermission[];
}

export type SessioAppPermission =
  | "autoplay"
  | "clipboardWrite"
  | "downloads"
  | "fullscreen"
  | "gamepad"
  | "modals"
  | "pointerLock"
  | "popups";

export interface SessioAppsCatalog {
  rootPath: string;
  apps: SessioAppInfo[];
}

export type AgentType = GeneratedAgentType;

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

export type AstraConfig = GeneratedAstraConfig;

export type AgentAiProviderInfo = GeneratedAgentAiProviderInfo;

export type AgentCommandsInfo = GeneratedAgentCommandsInfo;

export interface NetworkConfig {
  proxy: NetworkProxyConfig;
}

export interface NetworkProxyConfig {
  enabled: boolean;
  url: string | null;
  noProxy: string | null;
}

export interface McpSettings {
  servers: McpServerConfig[];
}

export type McpServerSource = "builtin" | "custom";
export type McpServerTransport = "http" | "sse" | "stdio";
export type McpServerInjectionMode = "always" | "sessionOptIn";
export type BuiltinMcpKind = "computerUse";

export interface McpKeyValue {
  name: string;
  value: string;
}

export interface McpServerConfig {
  id: string;
  name: string;
  description: string | null;
  enabled: boolean;
  source: McpServerSource;
  transport: McpServerTransport;
  injectionMode: McpServerInjectionMode;
  builtinKind: BuiltinMcpKind | null;
  url: string | null;
  headers: McpKeyValue[];
  command: string | null;
  args: string[];
  env: McpKeyValue[];
}

export type SkillSource = "builtin" | "user";
export type BuiltinSkillKind = "computerUse" | "createThread" | "workState";

export interface SkillMetadata {
  id: string;
  name: string;
  description: string;
  source: SkillSource;
  builtinKind: BuiltinSkillKind | null;
  skillMdPath: string;
  rootDir: string;
  skillDirName: string;
  frontmatter: Record<string, unknown>;
}

export interface InstallSkillRequest {
  sourcePath: string;
  directoryName?: string | null;
  overwrite?: boolean;
}

export interface AppshotConfig {
  shortcut: string;
}

export interface ConfigRecoveryNotice {
  path: string;
  backupPath: string | null;
  error: string;
  lineNumber: number | null;
  lineText: string | null;
  usedDefaults: boolean;
}

export interface ComputerUseSettings {
  enabled: boolean;
  mcpDescription?: string | null;
  approvedApps: string[];
}

export type AppshotPermissionKind = "screenshots" | "accessibility";

export interface AppshotPermissionState {
  granted: boolean;
  supported: boolean;
}

export interface AppshotPermissionStatus {
  platform: "macos" | "windows" | "linux" | "other" | string;
  requiresPermission: boolean;
  screenshots: AppshotPermissionState;
  accessibility: AppshotPermissionState;
  canCapture: boolean;
}

/** A single OS-permission tier plus whether the platform gates it at all. */
export interface DesktopControlPermissionTier {
  granted: boolean;
  supported: boolean;
}

/**
 * Shared desktop-control permission status. Single source of truth for both
 * Appshot (screenshot tier) and computer use (all three tiers).
 */
export interface DesktopControlPermissionStatus {
  platform: "macos" | "windows" | "linux" | "other" | string;
  requiresPermission: boolean;
  screenshots: DesktopControlPermissionTier;
  accessibility: DesktopControlPermissionTier;
  /** Capture screenshots / visual state. */
  canObserve: boolean;
  /** Inspect the accessibility / UI hierarchy. */
  canInspect: boolean;
  /** Inject input under the current platform/provider policy. */
  canControl: boolean;
}

export interface ComputerUseStatus {
  enabled: boolean;
  sessionApproved: boolean;
  hasLease: boolean;
  canObserve: boolean;
  canInspect: boolean;
  canControl: boolean;
  foregroundActive: boolean;
  activeAppId: string | null;
  activeAppApproved: boolean;
}

export interface ImBridgeConfig {
  enabled: boolean;
  idleTimeoutSecs: number;
  telegram: TelegramBridgeConfig | null;
  discord: DiscordBridgeConfig | null;
  feishu: FeishuBridgeConfig | null;
  wechat: WechatBridgeConfig | null;
}

export interface ImBridgeWorkspaceBinding {
  platform: string;
  chatId: string;
  workspacePath: string;
}

// --- Scheduled ("auto") tasks ---

export type ScheduleKind = "interval" | "daily" | "weekly" | "cron";

export type Schedule =
  | { kind: "interval"; everySecs: number }
  | { kind: "daily"; hour: number; minute: number }
  | { kind: "weekly"; weekday: number; hour: number; minute: number }
  | { kind: "cron"; expr: string };

export type ScheduledTaskMode = "chat" | ThreadKind;
export type ScheduledTaskStatus = "active" | "paused";
export type ScheduledTaskRunStatus = "running" | "completed" | "failed" | "cancelled";
export type ScheduledTaskPushStatus = "pending" | "summarizing" | "sent" | "failed";
export type ScheduledTaskRunTrigger = "scheduled" | "manual";

export interface ScheduledTaskRun {
  id: string;
  taskId: string;
  mode: ScheduledTaskMode;
  trigger: ScheduledTaskRunTrigger;
  status: ScheduledTaskRunStatus;
  startedAtMs: number;
  scheduledForMs: number | null;
  completedAtMs: number | null;
  sessionAgent: Agent | null;
  sessionId: string | null;
  /**
   * Real ACP / jsonl session id stamped after the runtime publishes it. Use
   * this when joining back to `SessionInfo` (the `sessionId` field carries the
   * runtime's internal handle, dead after a process restart).
   */
  agentSessionId: string | null;
  threadId: string | null;
  astraRunId: string | null;
  pushPlatform: string | null;
  pushChatId: string | null;
  pushStatus: ScheduledTaskPushStatus | null;
  pushSummary: string | null;
  pushError: string | null;
  pushSentAtMs: number | null;
  error: string | null;
}

export interface TaskImPush {
  enabled: boolean;
  platform: string;
  chatId: string;
}

export interface TaskTargetBase {
  projectId: string;
  imPush: TaskImPush | null;
}

export interface ChatTaskTarget extends TaskTargetBase {
  mode: "chat";
  prompt: string;
  agent: Agent;
  model: string | null;
  effort: string | null;
  permissionMode: string | null;
}

export interface ProcessTaskTarget extends TaskTargetBase {
  mode: "process";
  goal: string;
  description: string | null;
  stageIds: string[];
}

export interface TeamworkTaskTarget extends TaskTargetBase {
  mode: "teamwork";
  goal: string;
  description: string | null;
  assistantIds: string[];
}

export interface AgentThreadTaskTarget extends TaskTargetBase {
  mode: "brainstorm" | "debate";
  goal: string;
  description: string | null;
  agentParticipants: ThreadAgentInfo[];
}

export type TaskTarget =
  | ChatTaskTarget
  | ProcessTaskTarget
  | TeamworkTaskTarget
  | AgentThreadTaskTarget;

export interface ScheduledTask {
  id: string;
  name: string;
  status: ScheduledTaskStatus;
  schedule: Schedule;
  target: TaskTarget;
  createdAtMs: number;
  updatedAtMs: number;
  lastRunAtMs: number | null;
  runs: ScheduledTaskRun[];
}

export interface TelegramBridgeConfig {
  enabled: boolean;
  agent: Agent | null;
  model: string | null;
  effort: string | null;
  defaultWorkspace: string | null;
  allowedWorkspaces: string[];
  workspaceBindings: ImBridgeWorkspaceBinding[];
  botToken: string;
  allowedUserIds: number[];
  pollTimeoutSecs: number;
  apiBase: string | null;
}

export interface DiscordBridgeConfig {
  enabled: boolean;
  agent: Agent | null;
  model: string | null;
  effort: string | null;
  defaultWorkspace: string | null;
  allowedWorkspaces: string[];
  workspaceBindings: ImBridgeWorkspaceBinding[];
  botToken: string;
  allowedServerIds: string[];
  allowedChannelIds: string[];
  mentionOnly: boolean;
  apiBase: string | null;
  gatewayUrl: string | null;
}

export interface FeishuBridgeConfig {
  enabled: boolean;
  agent: Agent | null;
  model: string | null;
  effort: string | null;
  defaultWorkspace: string | null;
  allowedWorkspaces: string[];
  workspaceBindings: ImBridgeWorkspaceBinding[];
  appId: string;
  appSecret: string;
  domain: string | null;
}

export interface WechatBridgeConfig {
  enabled: boolean;
  agent: Agent | null;
  model: string | null;
  effort: string | null;
  defaultWorkspace: string | null;
  allowedWorkspaces: string[];
  workspaceBindings: ImBridgeWorkspaceBinding[];
  botToken: string;
  botId: string | null;
  userId: string | null;
  baseUrl: string | null;
  pollTimeoutSecs: number;
}

export interface WechatQrCode {
  qrcodeId: string;
  qrcodeContent: string;
  qrcodeImageContent: string | null;
}

export interface WechatQrStatus {
  status: string;
  botToken: string | null;
  botId: string | null;
  userId: string | null;
  baseUrl: string | null;
  redirectHost: string | null;
  error: string | null;
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
  selectedSkillIds: string[];
  selectedMcpIds: string[];
  type: AssistantType;
  processTemplateId: string | null;
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
  selectedSkillIds?: string[];
  selectedMcpIds?: string[];
  order: number;
}

export type ThreadKind = "process" | "teamwork" | "brainstorm" | "debate";

export interface ThreadAssistantInfo {
  assistantId: string;
  name: string;
  color: string | null;
  agent: AssistantAgentInfo;
  systemPrompt?: string | null;
  selectedSkillIds?: string[];
  selectedMcpIds?: string[];
  order: number;
}

export interface ThreadAgentInfo {
  participantId: string;
  agent: Agent;
  model: string;
  effort: string;
  permissionMode: string;
  order: number;
  createdAt?: number;
  updatedAt?: number;
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
  processTemplateId: string | null;
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
  processTemplateId: string | null;
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
  kind: ThreadKind;
  enabled: boolean;
  /**
   * `manual` for user-created threads, `scheduled_task` for threads spawned
   * by an auto task. The sidebar overlays a `CalendarClock` badge on
   * `scheduled_task` threads.
   */
  origin: ThreadOrigin;
  /** Populated when `origin === "scheduled_task"`. */
  scheduledTaskId: string | null;
  createdAt: number;
  updatedAt: number;
  assistants: ThreadAssistantInfo[];
  agentParticipants: ThreadAgentInfo[];
  stages: StageInfo[];
  sessions: SessionInfo[];
}

export type ThreadOrigin = "manual" | "scheduled_task";

export interface ThreadWorkState extends ThreadInfo {}

export type ThreadReplaySessionSourceKind = "thread" | "stage" | "plan_task" | "astra_internal";

export interface ThreadReplaySessionSourceInfo {
  kind: ThreadReplaySessionSourceKind;
  threadId: string | null;
  stageId: string | null;
  planRoundId: string | null;
  planTaskId: string | null;
  astraRunId: string | null;
  role: PlanTaskSessionRole | null;
  label: string | null;
  stageSnapshotJson: string | null;
  assistantSnapshotJson: string | null;
  agentSnapshotJson: string | null;
  createdAt: number | null;
}

export interface ThreadReplaySessionInfo {
  agent: Agent;
  sessionId: string;
  session: SessionInfo | null;
  sources: ThreadReplaySessionSourceInfo[];
  firstSeenAt: number | null;
  lastSeenAt: number | null;
}

export interface ThreadReplayInfo {
  threadId: string;
  kind: ThreadKind;
  sessions: ThreadReplaySessionInfo[];
}

export interface ThreadIndexItemInfo {
  threadId: string;
  projectId: string;
  goal: string;
  kind: ThreadKind;
  origin: ThreadOrigin;
  scheduledTaskId: string | null;
  createdAt: number;
  updatedAt: number;
  time: number;
  sessionKeys: string[];
}

export type PlanRoundMode = "parallel" | "sequential";
export type PlanRoundSource = "astra" | "manual" | "agent";
export type PlanRoundStatus = "planned" | "running" | "completed" | "cancelled" | "errored";
export type PlanTaskStatus =
  | "planned"
  | "running"
  | "completed"
  | "failed"
  | "errored"
  | "cancelled";
export type PlanTaskRisk = "low" | "medium" | "high";
export type PlanTaskSessionRole =
  | "primary"
  | "delegated"
  | "runtime"
  | "planner"
  | "synthesis"
  | "cross_check"
  | "diagnostic";

export interface PlanTaskSessionInfo {
  taskId: string;
  agent: Agent;
  sessionId: string;
  role: PlanTaskSessionRole;
  attemptId?: string;
  attemptCount: number;
  supersededAt?: number;
  createdAt: number;
  updatedAt: number;
}

export interface PlanTaskInfo {
  id: string;
  roundId: string;
  threadStageId: string | null;
  assistantId: string | null;
  agentParticipantId: string | null;
  targetAgent: Agent;
  stageSnapshotJson: string | null;
  assistantSnapshotJson: string | null;
  agentSnapshotJson: string;
  title: string;
  prompt: string;
  expectedOutput: string | null;
  risk: PlanTaskRisk;
  sortOrder: number;
  status: PlanTaskStatus;
  resultSummary: string | null;
  error: string | null;
  startedAt: number | null;
  completedAt: number | null;
  createdAt: number;
  updatedAt: number;
  sessions: PlanTaskSessionInfo[];
}

export interface PlanRoundInfo {
  id: string;
  threadId: string;
  astraRunId: string | null;
  roundIndex: number;
  summary: string | null;
  mode: PlanRoundMode;
  source: PlanRoundSource;
  status: PlanRoundStatus;
  createdAt: number;
  updatedAt: number;
  tasks: PlanTaskInfo[];
}

export interface CreatePlanTaskInput {
  threadStageId?: string | null;
  assistantId?: string | null;
  agentParticipantId?: string | null;
  targetAgent: Agent;
  stageSnapshotJson?: string | null;
  assistantSnapshotJson?: string | null;
  agentSnapshotJson: string;
  title: string;
  prompt: string;
  expectedOutput?: string | null;
  risk: PlanTaskRisk;
  sortOrder: number;
  status: PlanTaskStatus;
}

export interface CreatePlanRoundInput {
  threadId: string;
  astraRunId?: string | null;
  roundIndex?: number | null;
  summary?: string | null;
  mode: PlanRoundMode;
  source: PlanRoundSource;
  status: PlanRoundStatus;
  tasks: CreatePlanTaskInput[];
}

export type AstraRunStatus =
  | "planning"
  | "thinking"
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
  planTaskId?: string | null;
  assistantId?: string | null;
  agentParticipantId?: string | null;
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
  completedAt: number;
}

export interface AstraHandle {
  runId: string;
  threadId: string;
  projectId: string;
  continuedFromRunId: string | null;
  status: AstraRunStatus;
  mode: string;
  plannerBackend: string | null;
  roundIndex: number | null;
  roundLimit: number;
  terminalReason: string | null;
  lastErrorCode: string | null;
  lastErrorMessage: string | null;
  internalPlannerSessionIds: string[];
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
  channel?: ChannelSessionInfo | null;
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
  /**
   * Where the session was spawned. Sidebar surfaces `chat` and `channel`
   * directly; `thread` is represented by the parent thread item. Auxiliary
   * sessions (see {@link isAuxiliary}) are filtered regardless of origin.
   */
  origin: SessionOrigin;
  /**
   * Set when the session is directly attached to an auto task — chat-mode
   * task sessions and the summary-push session that posts to a channel.
   * Thread-mode auto task sessions live under their thread, so look at
   * {@link ThreadInfo.scheduledTaskId} instead.
   */
  scheduledTaskId: string | null;
  /**
   * True for system-internal helpers (codex guardian, Astra delegated, pi
   * fake, scheduled-task summary push). Auxiliary sessions never appear in
   * the sidebar.
   */
  isAuxiliary: boolean;
  subagents: SubagentInfo[];
}

export type SessionOrigin = "chat" | "thread" | "channel";

export interface ChannelSessionInfo {
  platform: string;
  channelId: string;
  channelType: string | null;
  userId: string | null;
  teamId: string | null;
  threadId: string | null;
  displayName: string | null;
  agent: Agent;
  agentSessionId: string;
  sessioRuntimeSessionId: string;
  workspacePath: string;
  metadata: Record<string, unknown>;
  createdAt: number;
  updatedAt: number;
  lastActivityAt: number;
  endedAt: number | null;
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

export type RuntimeTransportKind = "acp" | "piRpc" | "fake";

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
  mcpInjection?: McpInjectionCapabilities;
}

export interface McpInjectionCapabilities {
  http: boolean;
  sse: boolean;
  acp: boolean;
  nativeExtension: boolean;
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
  computerUseEligible?: boolean;
  updatedAt: number | null;
}

export interface RuntimeAgentSessionConfig {
  agent: Agent;
  adapterVersion: string;
  availableCommandsJson: string;
  configOptionsJson: string;
  createdAt: number;
  updatedAt: number;
}

export interface DebugConfig {
  acpConfig: boolean;
  updatePreview: boolean;
}

export type RuntimeAgentOptionMetadata = GeneratedRuntimeAgentOptionMetadata;

export interface UpdateRuntimeAgentPreferencesRequest {
  agent: Agent;
  displayName?: string | null;
  enabled?: boolean | null;
  order?: number | null;
  aiProvider?: string | null;
  aiProviders?: AgentAiProviderInfo[];
  commands?: AgentCommandsInfo;
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
  commands?: AgentCommandsInfo;
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
  options?: Record<string, unknown>;
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

export interface SavePastedAttachmentRequest {
  fileName: string | null;
  mimeType: string | null;
  dataBase64: string;
}

export interface SavedPastedAttachment {
  path: string;
}

export interface CaptureWindowAreaRequest {
  fileName?: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ScreenshotCaptureRequest {
  fileName?: string;
  hideSelf?: boolean;
}

export interface ScreenshotOverlayCaptureRequest {
  requestId: string;
  fileName?: string;
  hideSelf?: boolean;
}

export interface ScreenshotOverlayCompleteRequest {
  requestId: string;
  path?: string;
  cancelled?: boolean;
}

export interface ScreenshotOverlayWindow {
  label: string;
}

export interface ScreenshotOverlayInitialSelection {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ScreenshotOverlaySource {
  requestId: string;
  sourcePath: string;
  fileName: string;
  mode?: "interactive" | "selection";
  windows: ScreenshotOverlayWindowCandidate[];
  initialSelection?: ScreenshotOverlayInitialSelection | null;
}

export interface ComputerUsePointerOverlayReadyRequest {
  label: string;
}

export interface ScreenshotOverlayWindowCandidate {
  id: string;
  appName: string;
  title?: string | null;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface AgentInput {
  text: string;
  attachments?: AgentAttachment[];
  options?: Record<string, unknown>;
}

export interface SavedCanvasDraft {
  document: CanvasDocumentState["document"];
}

export interface SavedCanvasRevision {
  document: CanvasDocumentState["document"];
  revision: CanvasRevisionInfo;
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

export async function listSessioApps(): Promise<SessioAppsCatalog> {
  return invoke<SessioAppsCatalog>("list_sessio_apps");
}

export interface SessioAppFileWriteRequest {
  appDirectoryPath: string;
  relativePath: string;
  data: string;
  encoding?: "utf8" | "base64";
  overwrite?: boolean;
}

export interface SessioAppFileWriteResult {
  relativePath: string;
  bytesWritten: number;
}

export async function writeSessioAppFile(
  request: SessioAppFileWriteRequest,
): Promise<SessioAppFileWriteResult> {
  return invoke<SessioAppFileWriteResult>("write_sessio_app_file", { request });
}

export async function listSessioAppSessions(appId: string): Promise<SessionInfo[]> {
  return invoke<SessionInfo[]>("list_sessio_app_sessions", { appId });
}

export async function linkSessioAppSession(
  appId: string,
  agent: Agent,
  sessionId: string,
): Promise<void> {
  return invoke<void>("link_sessio_app_session", { appId, agent, sessionId });
}

export async function listTerminals(): Promise<TerminalSessionInfo[]> {
  return invoke<TerminalSessionInfo[]>("list_terminals");
}

export async function createTerminal(input: {
  cwd?: string | null;
  cols?: number | null;
  rows?: number | null;
  shell?: string | null;
}): Promise<TerminalSessionInfo> {
  return invoke<TerminalSessionInfo>("create_terminal", {
    req: {
      cwd: input.cwd ?? null,
      cols: input.cols ?? null,
      rows: input.rows ?? null,
      shell: input.shell ?? null,
    },
  });
}

export async function writeTerminalInput(
  terminalId: string,
  data: string,
): Promise<void> {
  return invoke<void>("write_terminal_input", {
    req: { terminalId, data },
  });
}

export async function resizeTerminal(
  terminalId: string,
  cols: number,
  rows: number,
): Promise<void> {
  return invoke<void>("resize_terminal", {
    req: { terminalId, cols, rows },
  });
}

export async function closeTerminal(terminalId: string): Promise<void> {
  return invoke<void>("close_terminal", {
    req: { terminalId },
  });
}

export async function listChannelSessions(): Promise<ChannelSessionInfo[]> {
  return invoke<ChannelSessionInfo[]>("list_channel_sessions");
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

export async function listProjectFiles(path: string): Promise<string[]> {
  return invoke<string[]>("list_project_files", { path });
}

export type ProjectGitStatus = "added" | "deleted" | "ignored" | "modified" | "renamed" | "untracked";

export interface ProjectGitStatusEntry {
  path: string;
  status: ProjectGitStatus;
}

export async function getProjectGitStatus(path: string): Promise<ProjectGitStatusEntry[]> {
  return invoke<ProjectGitStatusEntry[]>("get_project_git_status", { path });
}

export interface ProjectGitSummary {
  isRepo: boolean;
  root: string | null;
  branch: string | null;
  head: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  hasChanges: boolean;
  stagedCount: number;
  unstagedCount: number;
  untrackedCount: number;
}

export interface ProjectGitChange {
  path: string;
  originalPath: string | null;
  status: ProjectGitStatus;
  staged: boolean;
  indexStatus: string;
  worktreeStatus: string;
}

export interface ProjectGitState {
  summary: ProjectGitSummary;
  changes: ProjectGitChange[];
}

export interface ProjectGitCommit {
  hash: string;
  shortHash: string;
  parents: string[];
  author: string;
  timestamp: number;
  refs: string[];
  subject: string;
  message: string;
  pushed: boolean;
}

export interface ProjectGitCommitPage {
  commits: ProjectGitCommit[];
  hasMore: boolean;
}

export type ProjectGitAction =
  | "fetch"
  | "pull"
  | "push"
  | "sync"
  | "stage"
  | "unstage"
  | "discard"
  | "clean"
  | "stageAll"
  | "unstageAll"
  | "discardAll"
  | "cleanAll"
  | "commit";

export interface ProjectGitActionResult {
  stdout: string;
  stderr: string;
}

export async function getProjectGitSummary(path: string): Promise<ProjectGitSummary> {
  return invoke<ProjectGitSummary>("get_project_git_summary", { path });
}

export async function getProjectGitState(path: string): Promise<ProjectGitState> {
  return invoke<ProjectGitState>("get_project_git_state", { path });
}

export async function listProjectGitCommits(
  path: string,
  offset: number,
  limit: number,
): Promise<ProjectGitCommitPage> {
  return invoke<ProjectGitCommitPage>("list_project_git_commits", { path, offset, limit });
}

export async function runProjectGitAction(
  path: string,
  action: ProjectGitAction,
  options: { paths?: string[]; message?: string | null } = {},
): Promise<ProjectGitActionResult> {
  return invoke<ProjectGitActionResult>("run_project_git_action", {
    path,
    action,
    paths: options.paths ?? null,
    message: options.message ?? null,
  });
}

export interface FileGitDiff {
  status: ProjectGitStatus | "clean";
  patch: string | null;
}

export async function getFileGitDiff(
  workspacePath: string,
  filePath: string,
): Promise<FileGitDiff> {
  return invoke<FileGitDiff>("get_file_git_diff", { workspacePath, filePath });
}

export async function listProcessTemplates(): Promise<ProcessTemplateInfo[]> {
  return invoke<ProcessTemplateInfo[]>("list_process_templates");
}

export async function createProcessTemplate(name: string, description?: string | null): Promise<ProcessTemplateInfo> {
  return invoke<ProcessTemplateInfo>("create_process_template", { name, description: description ?? null });
}

export async function updateProcessTemplate(
  processTemplateId: string,
  patch: { name?: string | null; description?: string | null },
): Promise<ProcessTemplateInfo> {
  return invoke<ProcessTemplateInfo>("update_process_template", {
    processTemplateId,
    name: patch.name ?? null,
    description: patch.description === undefined ? undefined : patch.description,
  });
}

export async function deleteProcessTemplate(processTemplateId: string): Promise<void> {
  return invoke<void>("delete_process_template", { processTemplateId });
}

export async function addExistingProject(
  path: string,
  name?: string | null,
  processTemplateId?: string | null,
  enabledStageIds?: string[] | null,
): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("add_existing_project", {
    path,
    name: name ?? null,
    processTemplateId: processTemplateId ?? null,
    enabledStageIds: enabledStageIds ?? null,
  });
}

export async function createProject(
  parentPath: string,
  name: string,
  processTemplateId?: string | null,
  enabledStageIds?: string[] | null,
): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("create_project", {
    parentPath,
    name,
    processTemplateId: processTemplateId ?? null,
    enabledStageIds: enabledStageIds ?? null,
  });
}

export async function createDefaultProject(
  name: string,
  processTemplateId?: string | null,
  enabledStageIds?: string[] | null,
): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("create_default_project", {
    name,
    processTemplateId: processTemplateId ?? null,
    enabledStageIds: enabledStageIds ?? null,
  });
}

export async function updateProject(
  projectId: string,
  patch: { name?: string | null; processTemplateId?: string | null },
): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("update_project", {
    projectId,
    name: patch.name ?? null,
    processTemplateId: patch.processTemplateId ?? null,
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
  selectedSkillIds?: string[];
  selectedMcpIds?: string[];
  type: AssistantType;
  processTemplateId?: string | null;
  projectId?: string | null;
}): Promise<AssistantInfo> {
  return invoke<AssistantInfo>("create_assistant", {
    req: {
      name: input.name,
      agent: input.agent,
      systemPrompt: input.systemPrompt ?? null,
      color: input.color ?? null,
      selectedSkillIds: input.selectedSkillIds ?? [],
      selectedMcpIds: input.selectedMcpIds ?? [],
      assistantType: input.type,
      processTemplateId: input.processTemplateId ?? null,
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
    selectedSkillIds?: string[] | null;
    selectedMcpIds?: string[] | null;
    enabled?: boolean | null;
  },
): Promise<AssistantInfo> {
  return invoke<AssistantInfo>("update_assistant", {
    req: {
      assistantId,
      name: patch.name ?? null,
      agent: patch.agent ?? null,
      enabled: patch.enabled ?? null,
      selectedSkillIds:
        Object.prototype.hasOwnProperty.call(patch, "selectedSkillIds")
          ? patch.selectedSkillIds ?? []
          : null,
      selectedMcpIds:
        Object.prototype.hasOwnProperty.call(patch, "selectedMcpIds")
          ? patch.selectedMcpIds ?? []
          : null,
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

export async function getThreadReplay(threadId: string): Promise<ThreadReplayInfo> {
  return invoke<ThreadReplayInfo>("get_thread_replay", { threadId });
}

export async function listThreadIndex(projectId?: string | null): Promise<ThreadIndexItemInfo[]> {
  return invoke<ThreadIndexItemInfo[]>("list_thread_index", {
    projectId: projectId ?? null,
  });
}

export async function createThread(
  projectId: string,
  goal: string,
  description?: string | null,
  kind?: ThreadKind,
  assistantIds?: string[],
  agentParticipants?: ThreadAgentInfo[],
): Promise<ThreadInfo> {
  return invoke<ThreadInfo>("create_thread", {
    projectId,
    goal,
    description: description ?? null,
    kind: kind ?? null,
    assistantIds: assistantIds ?? null,
    agentParticipants: agentParticipants ?? null,
  });
}

export async function updateThread(
  threadId: string,
  patch: {
    goal?: string | null;
    description?: string | null;
    enabled?: boolean | null;
    kind?: ThreadKind | null;
    assistantIds?: string[] | null;
    agentParticipants?: ThreadAgentInfo[] | null;
  },
): Promise<ThreadInfo> {
  return invoke<ThreadInfo>("update_thread", {
    threadId,
    goal: patch.goal ?? null,
    enabled: patch.enabled ?? null,
    kind: patch.kind ?? null,
    assistantIds:
      Object.prototype.hasOwnProperty.call(patch, "assistantIds")
        ? patch.assistantIds ?? []
        : null,
    agentParticipants:
      Object.prototype.hasOwnProperty.call(patch, "agentParticipants")
        ? patch.agentParticipants ?? []
        : null,
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

export async function createPlanRound(input: CreatePlanRoundInput): Promise<PlanRoundInfo> {
  return invoke<PlanRoundInfo>("create_plan_round", {
    req: {
      threadId: input.threadId,
      astraRunId: input.astraRunId ?? null,
      roundIndex: input.roundIndex ?? null,
      summary: input.summary ?? null,
      mode: input.mode,
      source: input.source,
      status: input.status,
      tasks: input.tasks.map((task) => ({
        threadStageId: task.threadStageId ?? null,
        assistantId: task.assistantId ?? null,
        agentParticipantId: task.agentParticipantId ?? null,
        targetAgent: task.targetAgent,
        stageSnapshotJson: task.stageSnapshotJson ?? null,
        assistantSnapshotJson: task.assistantSnapshotJson ?? null,
        agentSnapshotJson: task.agentSnapshotJson,
        title: task.title,
        prompt: task.prompt,
        expectedOutput: task.expectedOutput ?? null,
        risk: task.risk,
        sortOrder: task.sortOrder,
        status: task.status,
      })),
    },
  });
}

export async function getPlanRound(roundId: string): Promise<PlanRoundInfo | null> {
  return invoke<PlanRoundInfo | null>("get_plan_round", { roundId });
}

export async function listPlanRounds(threadId: string): Promise<PlanRoundInfo[]> {
  return invoke<PlanRoundInfo[]>("list_plan_rounds", { threadId });
}

export async function updatePlanTaskStatus(
  taskId: string,
  patch: {
    status: PlanTaskStatus;
    resultSummary?: string | null;
    error?: string | null;
  },
): Promise<PlanTaskInfo> {
  return invoke<PlanTaskInfo>("update_plan_task_status", {
    req: {
      taskId,
      status: patch.status,
      resultSummary: Object.prototype.hasOwnProperty.call(patch, "resultSummary")
        ? patch.resultSummary ?? null
        : undefined,
      error: Object.prototype.hasOwnProperty.call(patch, "error")
        ? patch.error ?? null
        : undefined,
    },
  });
}

export async function completePlanTaskAndStartNext(
  taskId: string,
  patch: {
    status: PlanTaskStatus;
    resultSummary?: string | null;
    error?: string | null;
  },
): Promise<PlanRoundInfo> {
  return invoke<PlanRoundInfo>("complete_plan_task_and_start_next", {
    req: {
      taskId,
      status: patch.status,
      resultSummary: Object.prototype.hasOwnProperty.call(patch, "resultSummary")
        ? patch.resultSummary ?? null
        : undefined,
      error: Object.prototype.hasOwnProperty.call(patch, "error")
        ? patch.error ?? null
        : undefined,
    },
  });
}

export async function linkPlanTaskSession(input: {
  taskId: string;
  agent: Agent;
  sessionId: string;
  role: PlanTaskSessionRole;
}): Promise<PlanTaskSessionInfo> {
  return invoke<PlanTaskSessionInfo>("link_plan_task_session", {
    req: input,
  });
}

export async function listPlanTaskSessions(taskId: string): Promise<PlanTaskSessionInfo[]> {
  return invoke<PlanTaskSessionInfo[]>("list_plan_task_sessions", { taskId });
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

export async function listProcessTemplateStages(processTemplateId: string): Promise<ProjectStageInfo[]> {
  return invoke<ProjectStageInfo[]>("list_process_template_stages", { processTemplateId });
}

export async function createProjectStage(
  projectId: string,
  name: string,
  description?: string | null,
  processTemplateId?: string | null,
  icon?: string | null,
): Promise<ProjectStageInfo> {
  return invoke<ProjectStageInfo>("create_project_stage", {
    projectId,
    processTemplateId: processTemplateId ?? null,
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
  kind?: ThreadKind;
  assistants?: ThreadAssistantInfo[];
  agentParticipants?: ThreadAgentInfo[];
  activeStageId?: string | null;
  focusedStageId?: string | null;
  stages?: ThreadWorkSnapshotStage[];
  threadSessionRefs?: ThreadWorkSnapshotSessionRef[];
  planRounds?: PlanRoundInfo[];
  relatedContext?: ThreadWorkSnapshotRelatedContext;
  detailRefs?: ThreadWorkSnapshotDetailRefs;
  rollup?: {
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

export async function savePastedAttachment(
  req: SavePastedAttachmentRequest,
): Promise<SavedPastedAttachment> {
  return invoke<SavedPastedAttachment>("save_pasted_attachment", { req });
}

export async function captureWindowAreaPng(
  req: CaptureWindowAreaRequest,
): Promise<SavedPastedAttachment> {
  return invoke<SavedPastedAttachment>("capture_window_area_png", { req });
}

export async function captureFrontmostAppWindowPng(
  req: ScreenshotCaptureRequest,
): Promise<SavedPastedAttachment> {
  return invoke<SavedPastedAttachment>("capture_frontmost_app_window_png", { req });
}

export async function captureSelectedScreenAreaPng(
  req: ScreenshotCaptureRequest,
): Promise<SavedPastedAttachment> {
  return invoke<SavedPastedAttachment>("capture_selected_screen_area_png", { req });
}

export async function captureInteractiveScreenPng(
  req: ScreenshotCaptureRequest,
): Promise<SavedPastedAttachment> {
  return invoke<SavedPastedAttachment>("capture_interactive_screen_png", { req });
}

export async function openScreenshotOverlayCapture(
  req: ScreenshotOverlayCaptureRequest,
): Promise<ScreenshotOverlayWindow> {
  return invoke<ScreenshotOverlayWindow>("open_screenshot_overlay_capture", { req });
}

export async function getScreenshotOverlaySource(): Promise<ScreenshotOverlaySource> {
  return invoke<ScreenshotOverlaySource>("get_screenshot_overlay_source");
}

export async function computerUsePointerOverlayReady(
  req: ComputerUsePointerOverlayReadyRequest,
): Promise<void> {
  return invoke<void>("computer_use_pointer_overlay_ready", { payload: req });
}

export async function finishScreenshotOverlay(): Promise<void> {
  return invoke<void>("finish_screenshot_overlay");
}

export async function completeScreenshotOverlayCapture(
  req: ScreenshotOverlayCompleteRequest,
): Promise<void> {
  return invoke<void>("complete_screenshot_overlay_capture", { req });
}

export async function readLocalTextFile(path: string): Promise<string> {
  return invoke<string>("read_local_text_file", { path });
}

export interface WorkspaceTextFile {
  content: string;
  mtimeMs: number;
}

export async function readWorkspaceTextFile(
  workspacePath: string,
  path: string,
): Promise<WorkspaceTextFile> {
  return invoke<WorkspaceTextFile>("read_workspace_text_file", {
    workspacePath,
    path,
  });
}

export interface WorkspaceTextFileWrite {
  mtimeMs: number;
}

export async function writeWorkspaceTextFile(
  workspacePath: string,
  path: string,
  content: string,
  expectedMtimeMs: number,
): Promise<WorkspaceTextFileWrite> {
  return invoke<WorkspaceTextFileWrite>("write_workspace_text_file", {
    workspacePath,
    path,
    content,
    expectedMtimeMs,
  });
}

export async function watchPreviewFile(path: string): Promise<void> {
  return invoke<void>("watch_preview_file", { path });
}

export async function unwatchPreviewFile(path: string): Promise<void> {
  return invoke<void>("unwatch_preview_file", { path });
}

export async function writeCrossPrompt(
  sessionId: string,
  content: string,
): Promise<string> {
  return invoke<string>("write_cross_prompt", { sessionId, content });
}

export async function getCanvas(canvasKey: CanvasKey): Promise<CanvasDocumentState> {
  return invoke<CanvasDocumentState>("get_canvas", { canvasKey });
}

export async function saveCanvasDraft(
  req: SaveCanvasDraftRequest,
): Promise<SavedCanvasDraft> {
  return invoke<SavedCanvasDraft>("save_canvas_draft", { req });
}

export async function saveCanvasRevision(
  req: SaveCanvasRevisionRequest,
): Promise<SavedCanvasRevision> {
  return invoke<SavedCanvasRevision>("save_canvas_revision", { req });
}

export async function updateCanvasBlocks(
  req: UpdateCanvasBlocksRequest,
): Promise<CanvasBlockRecord[]> {
  return invoke<CanvasBlockRecord[]>("update_canvas_blocks", { req });
}

export async function createCanvasContextFile(
  req: BuildCanvasContextFileRequest,
): Promise<string> {
  return invoke<string>("create_canvas_context_file", { req });
}

export async function createCanvasAnchor(
  req: UpsertCanvasAnchorRequest,
): Promise<CanvasAnchorInfo> {
  return invoke<CanvasAnchorInfo>("create_canvas_anchor", { req });
}

export async function getAgentRuntimeStatus(agent: Agent): Promise<RuntimeStatus> {
  return invoke<RuntimeStatus>("get_agent_runtime_status", { agent });
}

export async function listRuntimeAgents(): Promise<RuntimeAgentMetadata[]> {
  return invoke<RuntimeAgentMetadata[]>("list_runtime_agents");
}

export async function getRuntimeAgentSessionConfig(
  agent: Agent,
): Promise<RuntimeAgentSessionConfig | null> {
  return invoke<RuntimeAgentSessionConfig | null>("get_runtime_agent_session_config", { agent });
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

export async function getMcpSettings(): Promise<McpSettings> {
  return invoke<McpSettings>("get_mcp_settings");
}

export async function listSkills(): Promise<SkillMetadata[]> {
  return invoke<SkillMetadata[]>("list_skills");
}

export async function installSkill(req: InstallSkillRequest): Promise<SkillMetadata> {
  return invoke<SkillMetadata>("install_skill", { req });
}

export async function updateMcpSettings(settings: McpSettings): Promise<McpSettings> {
  return invoke<McpSettings>("update_mcp_settings", { settings });
}

export async function getAppshotConfig(): Promise<AppshotConfig> {
  return invoke<AppshotConfig>("get_appshot_config");
}

export async function takeConfigRecoveryNotice(): Promise<ConfigRecoveryNotice | null> {
  return invoke<ConfigRecoveryNotice | null>("take_config_recovery_notice");
}

export async function getComputerUseSettings(): Promise<ComputerUseSettings> {
  return invoke<ComputerUseSettings>("get_computer_use_settings");
}

export async function getAppshotPermissionStatus(): Promise<AppshotPermissionStatus> {
  return invoke<AppshotPermissionStatus>("get_appshot_permission_status");
}

export async function getDesktopControlPermissionStatus(): Promise<DesktopControlPermissionStatus> {
  return invoke<DesktopControlPermissionStatus>("get_desktop_control_permission_status");
}

export async function getComputerUseStatus(
  sessioRuntimeSessionId: string,
): Promise<ComputerUseStatus | null> {
  return invoke<ComputerUseStatus | null>("get_computer_use_status", {
    sessioRuntimeSessionId,
  });
}

export async function setComputerUseAppApproval(
  sessioRuntimeSessionId: string,
  appId: string,
  approved: boolean,
): Promise<ComputerUseSettings> {
  return invoke<ComputerUseSettings>("set_computer_use_app_approval", {
    sessioRuntimeSessionId,
    appId,
    approved,
  });
}

export async function setComputerUseSessionApproval(
  sessioRuntimeSessionId: string,
  approved: boolean,
): Promise<void> {
  return invoke<void>("set_computer_use_session_approval", {
    sessioRuntimeSessionId,
    approved,
  });
}

export async function computerUseAbort(sessioRuntimeSessionId: string): Promise<void> {
  return invoke<void>("computer_use_abort", {
    sessioRuntimeSessionId,
  });
}

export async function requestAppshotPermission(
  permission: AppshotPermissionKind,
): Promise<AppshotPermissionStatus> {
  return invoke<AppshotPermissionStatus>("request_appshot_permission", { permission });
}

export async function openAppshotPermissionsPanel(): Promise<void> {
  return invoke<void>("open_appshot_permissions_panel");
}

export async function updateAppshotConfig(config: AppshotConfig): Promise<AppshotConfig> {
  return invoke<AppshotConfig>("update_appshot_config", { config });
}

export async function setAppshotShortcutRecording(recording: boolean): Promise<void> {
  return invoke<void>("set_appshot_shortcut_recording", { recording });
}

export async function updateComputerUseSettings(
  settings: ComputerUseSettings,
): Promise<ComputerUseSettings> {
  return invoke<ComputerUseSettings>("update_computer_use_settings", { settings });
}

export async function getImBridgeConfig(): Promise<ImBridgeConfig> {
  return invoke<ImBridgeConfig>("get_im_bridge_config");
}

export async function updateImBridgeConfig(config: ImBridgeConfig): Promise<ImBridgeConfig> {
  return invoke<ImBridgeConfig>("update_im_bridge_config", { config });
}

export async function getScheduledTasks(): Promise<ScheduledTask[]> {
  return invoke<ScheduledTask[]>("get_scheduled_tasks");
}

export async function saveScheduledTasks(tasks: ScheduledTask[]): Promise<ScheduledTask[]> {
  return invoke<ScheduledTask[]>("save_scheduled_tasks", { tasks });
}

export async function runScheduledTaskNow(id: string): Promise<void> {
  return invoke<void>("run_scheduled_task_now", { id });
}

export async function forceUnlockScheduledTask(id: string): Promise<void> {
  return invoke<void>("force_unlock_scheduled_task", { id });
}

export async function detectTelegramUserIds(botToken: string, apiBase: string | null): Promise<number[]> {
  return invoke<number[]>("detect_telegram_user_ids", { botToken, apiBase });
}

export async function testTelegramBotConnection(botToken: string, apiBase: string | null): Promise<void> {
  return invoke<void>("test_telegram_bot_connection", { botToken, apiBase });
}

export async function testDiscordBotConnection(botToken: string, apiBase: string | null): Promise<void> {
  return invoke<void>("test_discord_bot_connection", { botToken, apiBase });
}

export async function testFeishuBotConnection(appId: string, appSecret: string, domain: string | null): Promise<void> {
  return invoke<void>("test_feishu_bot_connection", { appId, appSecret, domain });
}

export async function testWechatBotConnection(botToken: string, baseUrl: string | null): Promise<void> {
  return invoke<void>("test_wechat_bot_connection", { botToken, baseUrl });
}

export async function getWechatQrcode(baseUrl: string | null): Promise<WechatQrCode> {
  return invoke<WechatQrCode>("get_wechat_qrcode", { baseUrl });
}

export async function pollWechatQrcodeStatus(qrcode: string, baseUrl: string | null): Promise<WechatQrStatus> {
  return invoke<WechatQrStatus>("poll_wechat_qrcode_status", { qrcode, baseUrl });
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

export async function disposeAgentRuntimeSession(
  sessioRuntimeSessionId: string,
): Promise<void> {
  return invoke<void>("dispose_agent_runtime_session", {
    sessioRuntimeSessionId,
  });
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
  pi: "Pi",
  omp: "OMP",
  codex: "Codex",
  claude: "Claude Code",
  opencode: "OpenCode",
};

/// Short single-word product names used in dense UI chips/dropdowns where the
/// "Claude Code" suffix doesn't fit. Defaults to AGENT_LABEL
/// when no override is set.
export const AGENT_SHORT_LABEL: Record<Agent, string> = {
  pi: "Pi",
  omp: "OMP",
  codex: "Codex",
  claude: "Claude",
  opencode: "OpenCode",
};

const AGENT_COLOR_VAR: Record<Agent, string> = {
  pi: "--color-purple",
  omp: "--color-brand",
  codex: "--color-fg",
  claude: "--color-orange",
  opencode: "--color-fg",
};

export const AGENT_ACCENT: Record<Agent, string> = {
  pi: `rgb(var(${AGENT_COLOR_VAR.pi}))`,
  omp: `rgb(var(${AGENT_COLOR_VAR.omp}))`,
  codex: `rgb(var(${AGENT_COLOR_VAR.codex}))`,
  claude: `rgb(var(${AGENT_COLOR_VAR.claude}))`,
  opencode: `rgb(var(${AGENT_COLOR_VAR.opencode}))`,
};

export function agentTint(a: Agent, alpha: number): string {
  return `rgb(var(${AGENT_COLOR_VAR[a]}) / ${alpha})`;
}

export function agentColorVar(a: Agent): string {
  return `var(${AGENT_COLOR_VAR[a]})`;
}
