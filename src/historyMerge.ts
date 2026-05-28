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

export function sanitizeSessioAttachmentText(text: string): string {
  const withoutFileLinks = removeFileMarkdownLinks(text);
  return replaceXmlishBlocks(
    replaceXmlishBlocks(withoutFileLinks, "sessio-upload-file", (attrs) => {
      const uri = attrs.uri;
      const name = attrs.name ?? basenameFromUri(uri ?? "");
      return fileMarker(name, uri);
    }),
    "context",
    (attrs) => fileMarker(basenameFromUri(attrs.ref ?? ""), attrs.ref),
  );
}

export function removeFileMarkdownLinks(text: string): string {
  let out = text;
  let searchFrom = 0;
  for (;;) {
    const closeLabelRel = out.slice(searchFrom).indexOf("](");
    if (closeLabelRel < 0) break;
    const closeLabel = searchFrom + closeLabelRel;
    const openLabelRel = out.slice(searchFrom, closeLabel).lastIndexOf("[");
    if (openLabelRel < 0) {
      searchFrom = closeLabel + 2;
      continue;
    }
    const openLabel = searchFrom + openLabelRel;
    const targetStart = closeLabel + 2;
    const closeTarget = out.indexOf(")", targetStart);
    if (closeTarget < 0) break;
    const target = out.slice(targetStart, closeTarget).trim().replace(/^<|>$/g, "");
    const label = out.slice(openLabel + 1, closeLabel);
    const isAtPrefix = label.trimStart().startsWith("@");
    const isCrossContext = target.includes("sessio-cross-context");
    if (!target.startsWith("file://") || (!isAtPrefix && !isCrossContext)) {
      searchFrom = closeTarget + 1;
      continue;
    }
    const dropStart = openLabel > 0 && out[openLabel - 1] === "!" ? openLabel - 1 : openLabel;
    out = out.slice(0, dropStart) + out.slice(closeTarget + 1);
    searchFrom = dropStart;
  }
  return collapseBlankLines(out);
}

function replaceXmlishBlocks(
  text: string,
  tag: string,
  marker: (attrs: Record<string, string>) => string,
): string {
  let out = "";
  let rest = text;
  const openPrefix = `<${tag}`;
  const closeTag = `</${tag}>`;
  for (;;) {
    const openStart = rest.indexOf(openPrefix);
    if (openStart < 0) break;
    out += rest.slice(0, openStart);
    const afterOpen = rest.slice(openStart);
    const openEnd = afterOpen.indexOf(">");
    if (openEnd < 0) {
      out += afterOpen;
      return collapseBlankLines(out);
    }
    const afterTag = afterOpen.slice(openEnd + 1);
    const closeStart = afterTag.indexOf(closeTag);
    if (closeStart < 0) {
      out += afterOpen;
      return collapseBlankLines(out);
    }
    out += marker(parseXmlishAttrs(afterOpen.slice(openPrefix.length, openEnd)));
    rest = afterTag.slice(closeStart + closeTag.length);
  }
  out += rest;
  return collapseBlankLines(out);
}

function parseXmlishAttrs(input: string): Record<string, string> {
  const attrs: Record<string, string> = {};
  const pattern = /([A-Za-z0-9_:-]+)\s*=\s*(["'])(.*?)\2/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(input)) !== null) {
    attrs[match[1]] = unescapeXmlAttr(match[3]);
  }
  return attrs;
}

function fileMarker(name?: string | null, uri?: string | null): string {
  const safeName = name?.trim() || "attachment";
  const safeUri = uri?.trim();
  return safeUri ? `[file: ${safeName}|${safeUri}]` : `[file: ${safeName}]`;
}

function basenameFromUri(uri: string): string | null {
  if (!uri) return null;
  const path = uri.startsWith("file://") ? decodeURIComponent(uri.slice("file://".length)) : uri;
  return path.split(/[/\\]/).filter(Boolean).pop() || null;
}

function unescapeXmlAttr(value: string): string {
  return value
    .replace(/&quot;/g, "\"")
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

function collapseBlankLines(text: string): string {
  return text
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n")
    .replace(/\n{3,}/g, "\n\n");
}
