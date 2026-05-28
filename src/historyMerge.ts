import type { SessionMessage } from "./api";
import type { AcpContentBlock, LiveRuntimeSession } from "./runtimeChat";

export function stripImagePlaceholders(s: string): string {
  return s
    .replace(/<image\b[^>]*>[\s\S]*?<\/image>/gi, "")
    .replace(/^\s*<image\b[^>]*>\s*$/gim, "")
    .replace(/^\s*<\/image>\s*$/gim, "")
    .replace(/\n{3,}/g, "\n\n");
}

export function stripInjectedContext(s: string): string {
  let text = s;
  for (;;) {
    const trimmed = text.trimStart();
    if (!trimmed.startsWith("<ide_")) break;
    const afterLt = trimmed.slice("<ide_".length);
    const closeIdx = afterLt.indexOf(">");
    if (closeIdx < 0) break;
    const tag = afterLt.slice(0, closeIdx);
    const close = `</ide_${tag}>`;
    const afterOpen = afterLt.slice(closeIdx + 1);
    const endIdx = afterOpen.indexOf(close);
    if (endIdx < 0) break;
    text = afterOpen.slice(endIdx + close.length);
  }
  const MARKER = "## My request for Codex:";
  const idx = text.indexOf(MARKER);
  if (idx >= 0) text = text.slice(idx + MARKER.length);
  return stripImagePlaceholders(text).trim();
}

export function stripSessioUploadWrapper(text: string): string {
  return text
    .replace(/^<sessio-upload-file\b[^>]*>\s*/i, "")
    .replace(/\s*<\/sessio-upload-file>\s*$/i, "")
    .replace(/^<!--\s*[^>]+-->\s*\n?/, "")
    .trim();
}

export function contentBlocksText(blocks: AcpContentBlock[]): string {
  return blocks.map((block) => {
    if (block.type === "text" && typeof block.text === "string") return block.text;
    if (block.type === "image") return `[image: ${String(block.mimeType ?? "")}]`;
    if (block.type === "resource_link") return `[resource: ${String(block.uri ?? "")}]`;
    return JSON.stringify(block);
  }).join("\n");
}

export function normalizedUserMessageText(text: string): string {
  return stripInjectedContext(stripSessioUploadWrapper(text))
    .replace(/\s+/g, " ")
    .trim();
}

export function normalizedReplayText(text: string): string {
  return stripInjectedContext(stripSessioUploadWrapper(text))
    .replace(/\s+/g, " ")
    .trim();
}

export function sameReplayMessage(a: SessionMessage, b: SessionMessage): boolean {
  return a.role === b.role &&
    normalizedReplayText(a.text) === normalizedReplayText(b.text);
}

export function liveSessionMessages(liveSession: LiveRuntimeSession): SessionMessage[] {
  const messages: SessionMessage[] = [];
  for (const turn of liveSession.turns) {
    for (const block of turn.blocks) {
      if (block.kind === "user" || block.kind === "assistant" || block.kind === "thought") {
        const text = contentBlocksText(block.blocks).trim();
        if (!text) continue;
        messages.push({
          role: block.kind === "thought" ? "thinking" : block.kind,
          text,
          timestamp: block.timestamp ?? turn.updatedAt ?? null,
        });
      }
    }
  }
  return messages;
}

// Splice current session messages onto the ancestor chain. If the current
// session starts with the same user message as the last ancestor user turn
// (a forked-from continuation), drop that duplicated tail to avoid replay.
export function forkVisibleHistoryMessages(
  ancestorMessages: SessionMessage[],
  currentMessages: SessionMessage[],
): SessionMessage[] {
  if (ancestorMessages.length === 0) return currentMessages;
  if (currentMessages.length === 0) return ancestorMessages;

  const firstCurrentUser = currentMessages.find((message) => message.role === "user");
  if (!firstCurrentUser) return [...ancestorMessages, ...currentMessages];

  let lastAncestorUserIndex = -1;
  for (let i = ancestorMessages.length - 1; i >= 0; i -= 1) {
    const role = ancestorMessages[i].role;
    if (role === "assistant") break;
    if (role === "user") {
      lastAncestorUserIndex = i;
      break;
    }
  }
  if (lastAncestorUserIndex < 0) return [...ancestorMessages, ...currentMessages];

  const lastAncestorUser = ancestorMessages[lastAncestorUserIndex];
  if (normalizedUserMessageText(lastAncestorUser.text) !== normalizedUserMessageText(firstCurrentUser.text)) {
    return [...ancestorMessages, ...currentMessages];
  }
  return [
    ...ancestorMessages.slice(0, lastAncestorUserIndex),
    ...currentMessages,
  ];
}

// Concatenate live messages onto history, removing the longest tail/head
// overlap so the same message isn't shown twice while it transitions from
// live to persisted.
export function mergeHistoryWithLiveMessages(
  historyMessages: SessionMessage[],
  liveMessages: SessionMessage[],
): SessionMessage[] {
  if (historyMessages.length === 0) return liveMessages;
  if (liveMessages.length === 0) return historyMessages;
  let overlap = 0;
  const maxOverlap = Math.min(historyMessages.length, liveMessages.length);
  for (let count = 1; count <= maxOverlap; count += 1) {
    const historyTail = historyMessages.slice(-count);
    const liveHead = liveMessages.slice(0, count);
    if (historyTail.every((message, index) => sameReplayMessage(message, liveHead[index]))) {
      overlap = count;
    }
  }
  return [...historyMessages, ...liveMessages.slice(overlap)];
}

export function crossContextMessages(
  ancestorMessages: SessionMessage[],
  currentMessages: SessionMessage[],
  liveSession: LiveRuntimeSession | null | undefined,
): SessionMessage[] {
  const historyMessages = forkVisibleHistoryMessages(ancestorMessages, currentMessages);
  if (!liveSession || liveSession.turns.length === 0) return historyMessages;
  return mergeHistoryWithLiveMessages(
    historyMessages,
    liveSessionMessages(liveSession),
  );
}
