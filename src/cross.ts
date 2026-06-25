import {
  Agent,
  type SessionContentBlock,
  getSessionHistory,
  writeCrossPrompt,
} from "./api";
import { contentBlocksTextWithSessioAttachmentMarkers } from "./historyMerge";
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
  /// Optional tool calls collected for this turn. Used to surface high-signal
  /// tool state (currently TodoWrite / Update Plan snapshots) in the cross
  /// prompt; defaulting to undefined keeps callers that only pass `blocks`
  /// working unchanged.
  tools?: CrossPromptToolCall[];
}

export interface CrossPromptRenderBlock {
  kind: string;
  blocks?: unknown;
  [key: string]: unknown;
}

export interface CrossPromptToolCall {
  title?: string;
  kind?: string;
  rawInput?: unknown;
  updatedAt?: number;
}

interface CrossPromptEntry {
  role: "user" | "assistant";
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
  pi: (id) => `pi --resume ${id}`,
  codex: (id) => `codex resume ${id}`,
  claude: (id) => `claude --resume ${id}`,
  opencode: (id) => `opencode session resume ${id}`,
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
  const SEP_LEN = CROSS_PROMPT_SEP.length;
  const MIN_ANCHOR_BUDGET = 512;

  // Anchor on the latest user message and the very last entry so the
  // receiver always sees the immediate request being continued. We fan out
  // from the tail to capture nearby context, then — after packing — backfill
  // the most recent user message preceding the selection if the head of the
  // selection isn't already a user turn (so the receiver knows the topic).
  const lastUserIdx = lastIndexOfRole(filtered, "user");
  const anchorIndices = uniqueSortedAnchors(
    [lastUserIdx, formatted.length - 1].filter((idx) => idx >= 0),
  );

  const picked = new Map<number, string>();
  // Seed each anchor with at least a minimal slice so a runaway middle entry
  // cannot crowd them out before we even consider neighbors.
  for (const idx of anchorIndices) {
    const raw = formatted[idx];
    picked.set(
      idx,
      raw.length <= MIN_ANCHOR_BUDGET ? raw : truncateMessageTail(raw, MIN_ANCHOR_BUDGET),
    );
  }

  // Build the neighbor walk order: newest-first from the most recent anchor
  // backwards, then anything still missing before the earliest anchor (rare,
  // because anchors include `0` whenever there's a user at the very start).
  const newestAnchor = anchorIndices[anchorIndices.length - 1];
  const oldestAnchor = anchorIndices[0];
  const orderedNeighbors: number[] = [];
  for (let i = newestAnchor - 1; i > oldestAnchor; i--) orderedNeighbors.push(i);
  for (let i = newestAnchor + 1; i < formatted.length; i++) orderedNeighbors.push(i);
  for (let i = oldestAnchor - 1; i >= 0; i--) orderedNeighbors.push(i);

  for (const idx of orderedNeighbors) {
    if (picked.has(idx)) continue;
    const raw = formatted[idx];
    const remaining = CROSS_PROMPT_MAX - budgetUsedFromMap(picked, SEP_LEN) - SEP_LEN;
    if (remaining <= 0) break;
    if (raw.length <= remaining) {
      picked.set(idx, raw);
      continue;
    }
    const TRUNCATE_MIN_USEFUL = 256;
    if (remaining < TRUNCATE_MIN_USEFUL) continue;
    const truncated = truncateMessageTail(raw, remaining);
    if (truncated.length > remaining) continue;
    picked.set(idx, truncated);
  }

  // Finalize each anchor with whatever extra room is now free, preferring full
  // content when possible. Walk newest → oldest so the most recent turn keeps
  // priority on growth.
  for (const idx of [...anchorIndices].reverse()) {
    const raw = formatted[idx];
    const current = picked.get(idx) as string;
    if (current === raw) continue;
    const sepCount = Math.max(picked.size - 1, 0);
    const otherTotal =
      budgetUsedFromMap(picked, SEP_LEN) - current.length - sepCount * SEP_LEN;
    const budget = CROSS_PROMPT_MAX - otherTotal - sepCount * SEP_LEN;
    if (raw.length <= budget) {
      picked.set(idx, raw);
    } else if (budget > current.length) {
      picked.set(idx, truncateMessageTail(raw, budget));
    }
  }

  // After the budget settles, make sure the very first entry the receiver
  // sees is a user message — that's the topic anchor. If the current top of
  // the selection is an assistant turn, walk backwards to the nearest user
  // message before it and squeeze it in (truncating if needed). This keeps
  // assistants from leading the prompt orphaned without their question.
  const sortedKeys = Array.from(picked.keys()).sort((a, b) => a - b);
  const topIdx = sortedKeys[0];
  if (topIdx !== undefined && filtered[topIdx]?.role !== "user") {
    let headUserIdx = -1;
    for (let i = topIdx - 1; i >= 0; i--) {
      if (filtered[i].role === "user") {
        headUserIdx = i;
        break;
      }
    }
    if (headUserIdx >= 0 && !picked.has(headUserIdx)) {
      const raw = formatted[headUserIdx];
      let remaining = CROSS_PROMPT_MAX - budgetUsedFromMap(picked, SEP_LEN) - SEP_LEN;
      if (remaining < MIN_ANCHOR_BUDGET) {
        // Make room by shrinking the newest non-anchor neighbor until the
        // head user can fit at MIN_ANCHOR_BUDGET (or we run out of victims).
        const anchorSet = new Set(anchorIndices);
        const neighborKeys = sortedKeys
          .filter((k) => !anchorSet.has(k))
          .sort((a, b) => b - a); // newest first
        for (const victim of neighborKeys) {
          if (remaining >= MIN_ANCHOR_BUDGET) break;
          const current = picked.get(victim) as string;
          if (current.length <= 64) continue;
          const need = MIN_ANCHOR_BUDGET + SEP_LEN - remaining;
          const newLen = Math.max(64, current.length - need);
          picked.set(victim, truncateMessageTail(formatted[victim], newLen));
          remaining = CROSS_PROMPT_MAX - budgetUsedFromMap(picked, SEP_LEN) - SEP_LEN;
        }
      }
      if (remaining >= MIN_ANCHOR_BUDGET) {
        picked.set(
          headUserIdx,
          raw.length <= remaining ? raw : truncateMessageTail(raw, remaining),
        );
      }
    }
  }

  const orderedSelection = Array.from(picked.keys())
    .sort((a, b) => a - b)
    .map((idx) => picked.get(idx) as string);
  const selected = fitFormattedSelection(orderedSelection);

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

function lastIndexOfRole(
  entries: CrossPromptEntry[],
  role: CrossPromptEntry["role"],
): number {
  for (let i = entries.length - 1; i >= 0; i--) {
    if (entries[i].role === role) return i;
  }
  return -1;
}

function uniqueSortedAnchors(indices: number[]): number[] {
  const set = new Set<number>();
  for (const idx of indices) if (idx >= 0) set.add(idx);
  return Array.from(set).sort((a, b) => a - b);
}

function budgetUsedFromMap(picked: Map<number, string>, sepLen: number): number {
  if (picked.size === 0) return 0;
  let total = 0;
  for (const value of picked.values()) total += value.length;
  return total + (picked.size - 1) * sepLen;
}

export function buildCrossPromptFromTurns(
  turns: CrossPromptTurn[],
  source?: CrossPromptSource,
): string {
  return buildCrossPromptEntries(crossPromptEntriesFromTurns(turns), source);
}

function crossPromptEntriesFromTurns(turns: CrossPromptTurn[]): CrossPromptEntry[] {
  const out: CrossPromptEntry[] = [];
  for (const turn of turns) {
    for (const block of turn.blocks) {
      const role = crossPromptRole(block.kind);
      if (!role) continue;
      const text = contentBlocksTextWithSessioAttachmentMarkers(
        normalizeCrossContentBlocks(block.blocks),
      ).trim();
      if (text) out.push({ role, text });
    }
    const todoEntry = todoEntryFromTurnTools(turn.tools);
    if (todoEntry) out.push(todoEntry);
  }
  return out;
}

function crossPromptRole(kind: string): CrossPromptEntry["role"] | null {
  // Skip `thought` blocks intentionally: a receiving agent does its own
  // reasoning, and the source's chain-of-thought tends to dominate the budget
  // (each thinking block can run several KB), crowding out the actual
  // user/assistant exchange. Preserve the conversation skeleton instead.
  if (kind === "user" || kind === "assistant") return kind;
  return null;
}

/// Render a single `[assistant]` entry per turn that snapshots the latest
/// todo/plan tool call. Captures both Claude's `TodoWrite` and
/// Codex-style `update_plan` / `TaskUpdate` so the receiving agent inherits
/// the active work plan rather than rediscovering it.
function todoEntryFromTurnTools(
  tools: CrossPromptToolCall[] | undefined,
): CrossPromptEntry | null {
  if (!Array.isArray(tools) || tools.length === 0) return null;
  // Walk from the end so we report the most recent snapshot first when a turn
  // updated the list multiple times.
  for (let i = tools.length - 1; i >= 0; i--) {
    const tool = tools[i];
    if (!tool || typeof tool !== "object") continue;
    const text = renderTodoToolText(tool);
    if (text) return { role: "assistant", text };
  }
  return null;
}

function renderTodoToolText(tool: CrossPromptToolCall): string | null {
  const title = typeof tool.title === "string" ? tool.title : "";
  const kind = typeof tool.kind === "string" ? tool.kind : "";
  const isTodo =
    title === "TodoWrite" || title === "todo_write" || kind === "todo";
  const isPlan =
    title === "TaskUpdate" ||
    title === "update_plan" ||
    title === "automation_update" ||
    kind === "task_list";
  if (!isTodo && !isPlan) return null;

  const entries = extractTodoEntries(tool.rawInput);
  if (entries.length === 0) return null;

  const header = isPlan ? "Plan" : "Todos";
  const lines = entries.map((entry) => formatTodoLine(entry));
  return `[${header}]\n${lines.join("\n")}`;
}

interface CrossTodoEntry {
  content: string;
  status?: string;
}

function extractTodoEntries(raw: unknown): CrossTodoEntry[] {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return [];
  const record = raw as Record<string, unknown>;
  const candidates: unknown[] = [
    record.entries,
    record.todos,
    record.plan,
    record.tasks,
  ];
  let items: unknown[] | null = null;
  for (const candidate of candidates) {
    if (Array.isArray(candidate)) {
      items = candidate;
      break;
    }
  }
  if (!items) return [];
  const out: CrossTodoEntry[] = [];
  for (const item of items) {
    if (!item || typeof item !== "object" || Array.isArray(item)) continue;
    const obj = item as Record<string, unknown>;
    const content =
      pickStringField(obj.content) ??
      pickStringField(obj.activeForm) ??
      pickStringField(obj.step) ??
      pickStringField(obj.title) ??
      pickStringField(obj.text);
    if (!content) continue;
    out.push({
      content,
      status: pickStringField(obj.status) ?? undefined,
    });
  }
  return out;
}

function pickStringField(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}

function formatTodoLine(entry: CrossTodoEntry): string {
  const status = (entry.status ?? "").toLowerCase();
  const marker =
    status === "completed" || status === "complete" || status === "done"
      ? "[x]"
      : status === "in_progress" ||
        status === "in-progress" ||
        status === "active" ||
        status === "running"
      ? "[~]"
      : "[ ]";
  return `${marker} ${entry.content}`;
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
