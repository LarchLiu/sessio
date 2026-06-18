import type { RuntimeAgentSessionConfig } from "./api";
import type { AcpAvailableCommand, AcpAvailableCommandInput } from "./runtimeChat";

export interface ChatSlashCommandTrigger {
  query: string;
  raw: string;
}

export function parseChatSlashCommandTrigger(text: string): ChatSlashCommandTrigger | null {
  if (!text.startsWith("/") || /\s/.test(text)) return null;
  return {
    query: text.slice(1),
    raw: text,
  };
}

export function filterChatSlashCommands(
  commands: AcpAvailableCommand[],
  query: string,
): AcpAvailableCommand[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return commands;
  return commands.filter((command) => command.name.toLowerCase().startsWith(normalized));
}

export function formatChatSlashCommandText(
  command: Pick<AcpAvailableCommand, "name" | "input">,
): string {
  return `/${command.name}${command.input?.kind === "unstructured" ? " " : ""}`;
}

export function parseRuntimeSessionAvailableCommands(
  config: Pick<RuntimeAgentSessionConfig, "availableCommandsJson"> | null,
): AcpAvailableCommand[] {
  const raw = config?.availableCommandsJson?.trim();
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed
      .map(normalizeAvailableCommand)
      .filter((command): command is AcpAvailableCommand => Boolean(command));
  } catch {
    return [];
  }
}

function normalizeAvailableCommand(value: unknown): AcpAvailableCommand | null {
  if (!isRecord(value)) return null;
  const name = typeof value.name === "string" ? value.name.trim() : "";
  if (!name) return null;
  return {
    name,
    description: typeof value.description === "string" ? value.description : "",
    input: normalizeAvailableCommandInput(value.input),
    meta: value.meta ?? null,
  };
}

function normalizeAvailableCommandInput(value: unknown): AcpAvailableCommandInput | null {
  if (!isRecord(value) || typeof value.kind !== "string") return null;
  if (value.kind === "unstructured") {
    return {
      kind: "unstructured",
      hint: typeof value.hint === "string" ? value.hint : null,
      meta: value.meta ?? null,
      raw: value.raw ?? value,
    };
  }
  return {
    kind: "unknown",
    hint: typeof value.hint === "string" ? value.hint : null,
    meta: value.meta ?? null,
    raw: value.raw ?? value,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
