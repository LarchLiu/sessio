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
): Promise<SessionMessage[]> {
  return invoke<SessionMessage[]>("get_session_messages", {
    agent,
    filePath,
    sessionId: sessionId ?? null,
  });
}

export async function writeCrossPrompt(
  sessionId: string,
  content: string,
): Promise<string> {
  return invoke<string>("write_cross_prompt", { sessionId, content });
}

export const AGENT_LABEL: Record<Agent, string> = {
  codex: "Codex",
  claude: "Claude Code",
  gemini: "Gemini",
};

const AGENT_COLOR_VAR: Record<Agent, string> = {
  codex: "--color-emerald",
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
