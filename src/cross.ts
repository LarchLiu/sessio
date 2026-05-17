import {
  Agent,
  SessionMessage,
  getSessionMessages,
  writeCrossPrompt,
} from "./api";

export const CROSS_PROMPT_MAX = 16 * 1024;

export interface CrossPromptSource {
  sourceAgent: Agent;
  sourceSessionId: string;
  sourceFilePath?: string;
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

export function buildCrossPrompt(
  messages: SessionMessage[],
  source?: CrossPromptSource,
): string {
  const filtered = messages.filter(
    (m) => m.role === "user" || m.role === "thinking" || m.role === "assistant",
  );
  if (filtered.length === 0) return "";
  const SEP = "\n\n";
  const formatted = filtered.map((m) => `[${m.role}]\n${m.text}`);
  let size = 0;
  let startIdx = filtered.length;
  for (let i = filtered.length - 1; i >= 0; i--) {
    const extra =
      formatted[i].length + (i === filtered.length - 1 ? 0 : SEP.length);
    if (size + extra > CROSS_PROMPT_MAX) break;
    size += extra;
    startIdx = i;
  }
  while (startIdx < filtered.length && filtered[startIdx].role !== "user") {
    startIdx++;
  }
  if (startIdx >= filtered.length) return "";
  const meta = source
    ? `<!-- sessio-cross:start source_agent="${htmlAttr(
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
  return header + formatted.slice(startIdx).join(SEP) + `\n\n<!-- sessio-cross:end -->`;
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
  const messages = await getSessionMessages(sourceAgent, filePath, sessionId);
  const prompt = buildCrossPrompt(messages, {
    sourceAgent,
    sourceSessionId: sessionId,
    sourceFilePath: filePath,
  });
  if (!prompt) return null;
  const path = await writeCrossPrompt(sessionId, prompt);
  return buildCrossCommand(targetAgent, path, placeholder);
}
