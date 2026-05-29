import {
  Agent,
  type SessionContentBlock,
  getSessionHistory,
  writeCrossPrompt,
} from "./api";
import { contentBlocksText } from "./historyMerge";
import type { AcpContentBlock } from "./runtimeChat";

export const CROSS_PROMPT_MAX = 16 * 1024;
const CROSS_PROMPT_SEP = "\n\n";

export interface CrossPromptSource {
  sourceAgent: Agent;
  sourceSessionId: string;
  sourceFilePath?: string;
}

export interface CrossPromptTurn {
  blocks: CrossPromptRenderBlock[];
}

export interface CrossPromptRenderBlock {
  kind: string;
  blocks?: unknown;
  [key: string]: unknown;
}

interface CrossPromptEntry {
  role: "user" | "thinking" | "assistant";
  text: string;
}

function htmlAttr(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export const IS_WIN =
  typeof navigator !== "undefined" && /Win/i.test(navigator.platform);

export const RESUME_CMD: Record<Agent, (id: string) => string> = {
  codex: (id) => `codex resume ${id}`,
  claude: (id) => `claude --resume ${id}`,
  gemini: (id) => `gemini --resume ${id}`,
};

function bashQuote(s: string): string {
  return `'${s.replace(/'/g, "'\\''")}'`;
}

function pwshQuote(s: string): string {
  return `'${s.replace(/'/g, "''")}'`;
}

function truncateMessageTail(formatted: string, maxLength = CROSS_PROMPT_MAX): string {
  if (formatted.length <= maxLength) return formatted;
  const roleEnd = formatted.indexOf("\n");
  if (roleEnd < 0) return formatted.slice(-maxLength);
  const prefix = formatted.slice(0, roleEnd + 1);
  const marker = "[...truncated...]\n";
  if (maxLength <= prefix.length + marker.length) return (prefix + marker).slice(0, maxLength);
  const budget = maxLength - prefix.length - marker.length;
  return prefix + marker + formatted.slice(-budget);
}

function fitFormattedSelection(selection: string[]): string[] {
  const fitted = [...selection];
  for (;;) {
    const joined = fitted.join(CROSS_PROMPT_SEP);
    if (joined.length <= CROSS_PROMPT_MAX) return fitted;
    if (fitted.length === 1) return [truncateMessageTail(fitted[0])];

    const lastIndex = fitted.length - 1;
    const beforeLast = fitted.slice(0, lastIndex).join(CROSS_PROMPT_SEP);
    const lastBudget = CROSS_PROMPT_MAX - beforeLast.length - CROSS_PROMPT_SEP.length;
    if (lastBudget > 64) {
      fitted[lastIndex] = truncateMessageTail(fitted[lastIndex], lastBudget);
      if (fitted.join(CROSS_PROMPT_SEP).length <= CROSS_PROMPT_MAX) return fitted;
    }

    if (fitted.length > 2) {
      fitted.splice(1, 1);
      continue;
    }
    return [truncateMessageTail(fitted[0])];
  }
}

function buildCrossPromptEntries(
  entries: CrossPromptEntry[],
  source?: CrossPromptSource,
): string {
  const filtered = entries.filter((entry) => entry.text.trim());
  if (filtered.length === 0) return "";
  const formatted = filtered.map((entry) => `[${entry.role}]\n${entry.text}`);
  let size = 0;
  let startIdx = filtered.length;
  for (let i = filtered.length - 1; i >= 0; i--) {
    const extra =
      formatted[i].length + (i === filtered.length - 1 ? 0 : CROSS_PROMPT_SEP.length);
    if (size + extra > CROSS_PROMPT_MAX) break;
    size += extra;
    startIdx = i;
  }
  while (startIdx < filtered.length && filtered[startIdx].role !== "user") {
    startIdx++;
  }
  if (startIdx >= filtered.length) {
    startIdx = filtered.map((message) => message.role).lastIndexOf("user");
  }
  if (startIdx < 0) return "";
  const selected = fitFormattedSelection(formatted.slice(startIdx));
  const meta = source
    ? `\n\n<!-- sessio-cross:start source_agent="${htmlAttr(
        source.sourceAgent,
      )}" source_session_id="${htmlAttr(source.sourceSessionId)}"${
        source.sourceFilePath
          ? ` source_file_path="${htmlAttr(source.sourceFilePath)}"`
          : ""
      } -->\n\n`
    : `<!-- sessio-cross:start -->\n\n`;
  const header =
    meta +
    `# Continued session from agent\n` +
    `The dialogue below is the recent context of an in-progress session ` +
    `(oldest → latest). Pick up from the last turn and continue helping ` +
    `the user.\n\n`;
  return header + selected.join(CROSS_PROMPT_SEP) + `\n\n<!-- sessio-cross:end -->`;
}

export function buildCrossPromptFromTurns(
  turns: CrossPromptTurn[],
  source?: CrossPromptSource,
): string {
  return buildCrossPromptEntries(crossPromptEntriesFromTurns(turns), source);
}

function crossPromptEntriesFromTurns(turns: CrossPromptTurn[]): CrossPromptEntry[] {
  return turns.flatMap((turn) =>
    turn.blocks.flatMap((block) => {
      const role = crossPromptRole(block.kind);
      if (!role) return [];
      const text = contentBlocksText(normalizeCrossContentBlocks(block.blocks)).trim();
      return text ? [{ role, text }] : [];
    }),
  );
}

function crossPromptRole(kind: string): CrossPromptEntry["role"] | null {
  if (kind === "user" || kind === "assistant") return kind;
  if (kind === "thought") return "thinking";
  return null;
}

function normalizeCrossContentBlocks(blocks: unknown): AcpContentBlock[] {
  if (!Array.isArray(blocks)) return [];
  const out: AcpContentBlock[] = [];
  for (const block of blocks) {
    if (!block || typeof block !== "object") continue;
    const record = block as SessionContentBlock & Record<string, unknown>;
    const type = typeof record.type === "string" ? record.type : "unknown";
    if (type === "text") {
      const text = typeof record.text === "string" ? record.text : "";
      if (text.trim()) out.push({ type: "text", text });
      continue;
    }
    if (type === "image" || type === "audio") {
      out.push({
        type,
        uri: typeof record.uri === "string" ? record.uri : undefined,
        data: typeof record.data === "string" ? record.data : undefined,
        mimeType: typeof record.mimeType === "string" ? record.mimeType : undefined,
      } as AcpContentBlock);
      continue;
    }
    if (type === "resource_link") {
      out.push({
        type: "resource_link",
        uri: typeof record.uri === "string" ? record.uri : "",
        name: typeof record.name === "string" ? record.name : undefined,
        title: typeof record.title === "string" ? record.title : undefined,
        description: typeof record.description === "string" ? record.description : undefined,
        mimeType: typeof record.mimeType === "string" ? record.mimeType : undefined,
        size: typeof record.size === "number" ? record.size : undefined,
      });
      continue;
    }
    if (type === "resource") {
      out.push({
        type: "resource",
        uri: typeof record.uri === "string" ? record.uri : undefined,
        name: typeof record.name === "string" ? record.name : undefined,
        mimeType: typeof record.mimeType === "string" ? record.mimeType : undefined,
        text: typeof record.text === "string" ? record.text : undefined,
        blob: typeof record.blob === "string" ? record.blob : undefined,
        resource: record.resource,
      });
      continue;
    }
    out.push({ ...record, type: "unknown", originalType: type, meta: record.meta ?? null });
  }
  return out;
}

export function buildCrossCommand(
  targetAgent: Agent,
  filePath: string,
  placeholder: string,
): string {
  if (IS_WIN) {
    return `${targetAgent} "<${placeholder}>$(Get-Content -Raw ${pwshQuote(
      filePath,
    )})"`;
  }
  return `${targetAgent} "<${placeholder}>$(cat ${bashQuote(filePath)})"`;
}

// Materializes the cross prompt for a given source session into a temp file
// and returns the shell command to feed it into the target agent. Returns
// null when the source has no replayable user message.
export async function buildCrossCommandForSession(
  sourceAgent: Agent,
  targetAgent: Agent,
  sessionId: string,
  filePath: string,
  placeholder: string,
): Promise<string | null> {
  const { turns } = await getSessionHistory(sourceAgent, filePath, sessionId);
  const prompt = buildCrossPromptFromTurns(turns, {
    sourceAgent,
    sourceSessionId: sessionId,
    sourceFilePath: filePath,
  });
  if (!prompt) return null;
  const path = await writeCrossPrompt(sessionId, prompt);
  return buildCrossCommand(targetAgent, path, placeholder);
}
