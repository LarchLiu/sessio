import type {
  AcpContentBlock,
  AcpRenderBlock,
  LiveTurn,
} from "./runtimeChat";

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
    if (block.type === "image") return imageMarkdown(block);
    if (block.type === "resource" || block.type === "resource_link") {
      return fileMarker(resourceDisplayName(block), resourceUri(block));
    }
    return JSON.stringify(block);
  }).join("\n");
}

export function normalizedUserMessageText(text: string): string {
  return normalizedMessageBodyText(text);
}

export function normalizedReplayText(text: string): string {
  return normalizedMessageBodyText(text);
}

function normalizedMessageBodyText(text: string): string {
  return stripAttachmentCompareMarkers(stripInjectedContext(stripSessioUploadWrapper(text)))
    .replace(/\s+/g, " ")
    .trim();
}

export function stripAttachmentCompareMarkers(text: string): string {
  return text
    .replace(/!\[[^\]]*\]\([^)]+\)/g, "")
    .replace(/\[file:\s*[^\]]+\]/gi, "")
    .replace(/\[image:\s*[^\]]*\]/gi, "")
    .replace(/\[resource:\s*[^\]]*\]/gi, "");
}

export function attachmentCompareSignature(text: string): string {
  const cleaned = stripInjectedContext(stripSessioUploadWrapper(text));
  return attachmentCompareIdentities(cleaned).sort().join("\u001f");
}

function attachmentCompareIdentities(text: string): string[] {
  const identities: string[] = [];
  collectRegexIdentities(text, /!\[[^\]]*\]\(([^)]+)\)/g, 1, identities);
  collectRegexIdentities(text, /\[file:\s*([^\]|]+)(?:\|([^\]]+))?\]/gi, 2, identities, 1);
  collectRegexIdentities(text, /\[resource:\s*([^\]]+)\]/gi, 1, identities);
  collectRegexIdentities(text, /\[image:\s*([^\]]+)\]/gi, 1, identities);
  return identities;
}

function collectRegexIdentities(
  text: string,
  pattern: RegExp,
  primaryGroup: number,
  identities: string[],
  fallbackGroup?: number,
): void {
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    const raw = match[primaryGroup] || (fallbackGroup ? match[fallbackGroup] : "");
    const identity = normalizeAttachmentIdentity(raw);
    if (identity) identities.push(identity);
  }
}

function normalizeAttachmentIdentity(value: string): string {
  const raw = value.trim().replace(/^<|>$/g, "");
  if (!raw) return "";
  const decoded = safeDecodeURIComponent(raw.startsWith("file://") ? raw.slice("file://".length) : raw);
  return decoded.replace(/\\/g, "/").replace(/\/{2,}/g, "/").replace(/\/$/, "");
}

function safeDecodeURIComponent(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

export function forkVisibleHistoryTurns(
  ancestorTurns: LiveTurn[],
  currentTurns: LiveTurn[],
): LiveTurn[] {
  if (ancestorTurns.length === 0) return currentTurns;
  if (currentTurns.length === 0) return ancestorTurns;

  const firstCurrentUser = firstUserBlock(currentTurns);
  if (!firstCurrentUser) return [...ancestorTurns, ...currentTurns];

  const forkPoint = lastTailUserBlock(ancestorTurns);
  if (!forkPoint || !sameAcpUserBlocks(forkPoint.block, firstCurrentUser.block)) {
    return [...ancestorTurns, ...currentTurns];
  }
  return [...ancestorTurns.slice(0, forkPoint.turnIndex), ...currentTurns];
}

export function mergeHistoryWithLiveTurns(
  historyTurns: LiveTurn[],
  liveTurns: LiveTurn[],
): LiveTurn[] {
  if (historyTurns.length === 0) return liveTurns;
  if (liveTurns.length === 0) return historyTurns;
  return [...historyTurns, ...trimLiveReplayPrefix(historyTurns, liveTurns)];
}

export function trimLiveReplayPrefix(
  historyTurns: LiveTurn[],
  liveTurns: LiveTurn[],
): LiveTurn[] {
  const overlap = liveReplayPrefixOverlap(historyTurns, liveTurns);
  if (overlap === 0) return liveTurns;

  let remaining = overlap;
  const out: LiveTurn[] = [];
  for (const turn of liveTurns) {
    if (remaining <= 0) {
      out.push(turn);
      continue;
    }

    const messageBlockCount = turn.blocks.filter(isAcpMessageBlock).length;
    if (messageBlockCount === 0) {
      out.push(turn);
      continue;
    }

    if (remaining >= messageBlockCount) {
      remaining -= messageBlockCount;
      if (isReplayTurnFinished(turn)) continue;
      const blocks = turn.blocks.filter((block) => !isAcpMessageBlock(block));
      if (blocks.length > 0 || turn.tools.length > 0 || turn.permissions.length > 0 || turn.error) {
        out.push({ ...turn, blocks });
      }
      continue;
    }

    const blocks: LiveTurn["blocks"] = [];
    for (const block of turn.blocks) {
      if (remaining > 0 && isAcpMessageBlock(block)) {
        remaining -= 1;
        continue;
      }
      blocks.push(block);
    }
    out.push({ ...turn, blocks });
  }
  return out;
}

type UserRenderBlock = Extract<AcpRenderBlock, { kind: "user" }>;

function firstUserBlock(turns: LiveTurn[]): { turnIndex: number; block: UserRenderBlock } | null {
  for (let turnIndex = 0; turnIndex < turns.length; turnIndex += 1) {
    for (const block of turns[turnIndex].blocks) {
      if (block.kind === "user") return { turnIndex, block };
    }
  }
  return null;
}

function lastTailUserBlock(turns: LiveTurn[]): { turnIndex: number; block: UserRenderBlock } | null {
  for (let turnIndex = turns.length - 1; turnIndex >= 0; turnIndex -= 1) {
    const blocks = turns[turnIndex].blocks;
    for (let blockIndex = blocks.length - 1; blockIndex >= 0; blockIndex -= 1) {
      const block = blocks[blockIndex];
      if (block.kind === "assistant") return null;
      if (block.kind === "user") return { turnIndex, block };
    }
  }
  return null;
}

export function sameAcpUserBlocks(a: UserRenderBlock, b: UserRenderBlock): boolean {
  const left = contentBlocksText(a.blocks);
  const right = contentBlocksText(b.blocks);
  return normalizedUserMessageText(left) === normalizedUserMessageText(right) &&
    attachmentCompareSignature(left) === attachmentCompareSignature(right);
}

type TurnMessageRole = "user" | "assistant" | "thinking";

interface TurnMessageRef {
  role: TurnMessageRole;
  text: string;
  timestamp: number | null;
}

function liveReplayPrefixOverlap(historyTurns: LiveTurn[], liveTurns: LiveTurn[]): number {
  const historyMessages = turnMessageRefs(historyTurns);
  const liveMessages = turnMessageRefs(liveTurns);
  let overlap = 0;
  const maxOverlap = Math.min(historyMessages.length, liveMessages.length);
  for (let count = 1; count <= maxOverlap; count += 1) {
    const historyTail = historyMessages.slice(-count);
    const liveHead = liveMessages.slice(0, count);
    if (historyTail.every((item, index) => sameAcpReplayRef(item, liveHead[index]))) {
      overlap = count;
    }
  }
  return overlap;
}

function turnMessageRefs(turns: LiveTurn[]): TurnMessageRef[] {
  const refs: TurnMessageRef[] = [];
  turns.forEach((turn) => {
    turn.blocks.forEach((block) => {
      if (!isAcpMessageBlock(block)) return;
      const text = contentBlocksText(block.blocks).trim();
      if (!text) return;
      refs.push({
        role: block.kind === "thought" ? "thinking" : block.kind,
        text,
        timestamp: block.timestamp ?? turn.updatedAt ?? null,
      });
    });
  });
  return refs;
}

function sameAcpReplayRef(a: TurnMessageRef, b: TurnMessageRef): boolean {
  return a.role === b.role &&
    normalizedReplayText(a.text) === normalizedReplayText(b.text) &&
    attachmentCompareSignature(a.text) === attachmentCompareSignature(b.text);
}

function isAcpMessageBlock(
  block: AcpRenderBlock,
): block is Extract<AcpRenderBlock, { kind: "user" | "assistant" | "thought" }> {
  return block.kind === "user" || block.kind === "assistant" || block.kind === "thought";
}

function isReplayTurnFinished(turn: LiveTurn): boolean {
  return turn.status === "completed" || turn.status === "failed" || turn.status === "cancelled";
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

function imageMarkdown(block: Extract<AcpContentBlock, { type: "image" }>): string {
  const mimeType = block.mimeType?.trim() || "image";
  const src = block.uri?.trim() ||
    (block.data?.trim()
      ? block.data.trim().startsWith("data:")
        ? block.data.trim()
        : `data:${mimeType};base64,${block.data.trim()}`
      : "");
  return src ? `![${mimeType}](${src})` : `[image: ${mimeType}]`;
}

function resourceDisplayName(block: Extract<AcpContentBlock, { type: "resource" | "resource_link" }>): string {
  if (block.type === "resource_link") {
    return block.title ?? block.name ?? basenameFromUri(block.uri) ?? "Resource";
  }
  return block.name ?? basenameFromUri(block.uri ?? "") ?? "Embedded resource";
}

function resourceUri(block: Extract<AcpContentBlock, { type: "resource" | "resource_link" }>): string | null {
  return block.uri?.trim() || null;
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
