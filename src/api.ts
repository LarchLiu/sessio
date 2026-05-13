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

export async function listSessions(): Promise<SessionInfo[]> {
  return invoke<SessionInfo[]>("list_sessions");
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

export const AGENT_LABEL: Record<Agent, string> = {
  codex: "Codex",
  claude: "Claude Code",
  gemini: "Gemini",
};

export const AGENT_ACCENT: Record<Agent, string> = {
  codex: "rgb(var(--color-agent-codex))",
  claude: "rgb(var(--color-agent-claude))",
  gemini: "rgb(var(--color-agent-gemini))",
};

export function agentTint(a: Agent, alpha: number): string {
  return `rgb(var(--color-agent-${a}) / ${alpha})`;
}
