import type {
  AcpContentBlock,
  AcpRenderBlock,
  LiveTurn,
} from "./runtimeChat";

const SESSIO_ATTACHMENT_MARKER = "__sessio_attachment__:";
const SESSIO_THREAD_PROMPT_START = "<!-- sessio-thread-prompt:start";
const SESSIO_THREAD_PROMPT_END = "<!-- sessio-thread-prompt:end";

export interface SessioThreadPromptBlockMeta {
  kind: string | null;
  attrs: Record<string, string>;
  content: string;
}

function htmlAttr(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export function buildSessioThreadPromptBlock(
  kind: string,
  content: string,
  attrs: Record<string, string | null | undefined> = {},
): string {
  const body = content.trim();
  if (!body) return "";
  const nonce = threadPromptNonce();
  const attrText = [
    ["nonce", nonce],
    ["kind", kind],
    ...Object.entries(attrs),
  ]
    .filter((entry): entry is [string, string] =>
      /^[A-Za-z_][A-Za-z0-9_-]*$/.test(entry[0]) && Boolean(entry[1]?.trim()))
    .map(([key, value]) => ` ${key}="${htmlAttr(value)}"`)
    .join("");
  return `${SESSIO_THREAD_PROMPT_START}${attrText} -->\n\n${body}\n\n${SESSIO_THREAD_PROMPT_END} nonce="${htmlAttr(nonce)}" -->`;
}

function threadPromptNonce(): string {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  const bytes = new Uint8Array(16);
  if (globalThis.crypto?.getRandomValues) {
    globalThis.crypto.getRandomValues(bytes);
    return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

export function sessioThreadPromptBlockMetas(input: string): SessioThreadPromptBlockMeta[] {
  const metas: SessioThreadPromptBlockMeta[] = [];
  let cursor = 0;
  for (;;) {
    const start = input.indexOf(SESSIO_THREAD_PROMPT_START, cursor);
    if (start < 0) break;
    const startCommentEnd = input.indexOf("-->", start);
    if (startCommentEnd < 0) break;
    const startComment = input.slice(start, startCommentEnd + "-->".length);
    const nonce = commentAttr(startComment, "nonce");
    if (!nonce) {
      cursor = startCommentEnd + "-->".length;
      continue;
    }
    const endMarker = `${SESSIO_THREAD_PROMPT_END} nonce="${nonce}" -->`;
    const end = input.indexOf(endMarker, startCommentEnd + "-->".length);
    if (end < 0) {
      cursor = startCommentEnd + "-->".length;
      continue;
    }
    const attrs = commentAttrs(startComment);
    const content = input.slice(startCommentEnd + "-->".length, end).trim();
    metas.push({
      kind: attrs.kind?.trim() || null,
      attrs,
      content,
    });
    cursor = end + endMarker.length;
  }
  return metas;
}

export function stripSessioThreadPromptBlocks(input: string): string {
  let out = "";
  let cursor = 0;
  let changed = false;
  for (;;) {
    const start = input.indexOf(SESSIO_THREAD_PROMPT_START, cursor);
    if (start < 0) {
      out += input.slice(cursor);
      break;
    }
    const startCommentEnd = input.indexOf("-->", start);
    if (startCommentEnd < 0) {
      out += input.slice(cursor);
      break;
    }
    const startComment = input.slice(start, startCommentEnd + "-->".length);
    const nonce = commentAttr(startComment, "nonce");
    if (!nonce) {
      out += input.slice(cursor, startCommentEnd + "-->".length);
      cursor = startCommentEnd + "-->".length;
      continue;
    }
    const endMarker = `${SESSIO_THREAD_PROMPT_END} nonce="${nonce}" -->`;
    const end = input.indexOf(endMarker, startCommentEnd + "-->".length);
    if (end < 0) {
      out += input.slice(cursor, startCommentEnd + "-->".length);
      cursor = startCommentEnd + "-->".length;
      continue;
    }
    changed = true;
    out += input.slice(cursor, start);
    cursor = end + endMarker.length;
  }
  if (!changed) return input;
  return out
    .replace(/^\s*---+\s*/, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function commentAttr(comment: string, key: string): string | null {
  const match = new RegExp(`\\s${key}="([^"]*)"`).exec(comment);
  return match?.[1] ?? null;
}

function commentAttrs(comment: string): Record<string, string> {
  const attrs: Record<string, string> = {};
  const pattern = /\s([A-Za-z_][A-Za-z0-9_-]*)="([^"]*)"/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(comment))) {
    attrs[match[1]] = htmlUnattr(match[2]);
  }
  return attrs;
}

function htmlUnattr(s: string): string {
  return s
    .replace(/&quot;/g, "\"")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

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
  return stripSessioThreadPromptBlocks(stripImagePlaceholders(text)).trim();
}

export function stripSessioUploadWrapper(text: string): string {
  return text
    .replace(/^<sessio-upload-file\b[^>]*>\s*/i, "")
    .replace(/\s*<\/sessio-upload-file>\s*$/i, "")
    .replace(/^<!--\s*[^>]+-->\s*\n?/, "")
    .trim();
}

export function contentBlocksText(blocks: AcpContentBlock[]): string {
  return blocks
    .map((block) => {
      if (block.type === "text" && typeof block.text === "string") return block.text;
      if (block.type === "image") return imageMarkdown(block);
      if (block.type === "resource" || block.type === "resource_link") {
        return fileMarker(resourceDisplayName(block), resourceUri(block));
      }
      return JSON.stringify(block);
    })
    .join("\n");
}

export function contentBlocksTextWithSessioAttachmentMarkers(blocks: AcpContentBlock[]): string {
  return blocks
    .map((block) => {
      if (block.type === "text" && typeof block.text === "string") return block.text;
      if (block.type === "image") return imageMarkdown(block, true);
      if (block.type === "resource" || block.type === "resource_link") {
        return fileMarker(resourceDisplayName(block), resourceUri(block), true);
      }
      return JSON.stringify(block);
    })
    .join("\n");
}

export function normalizedUserMessageText(text: string): string {
  return normalizedMessageBodyText(text);
}

function normalizedMessageBodyText(text: string): string {
  return stripAttachmentCompareMarkers(
    stripInjectedContext(stripSessioUploadWrapper(text)),
  )
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
  const historyIds = new Set(historyTurns.map((turn) => turn.turnId));
  const nextLiveTurns = removeDuplicatedTailLiveUser(
    historyTurns,
    liveTurns.filter((turn) => !historyIds.has(turn.turnId)),
  );
  return [...historyTurns, ...nextLiveTurns];
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

function removeDuplicatedTailLiveUser(
  historyTurns: LiveTurn[],
  liveTurns: LiveTurn[],
): LiveTurn[] {
  if (liveTurns.length === 0) return liveTurns;
  const historyUser = lastTailUserBlock(historyTurns);
  if (!historyUser) return liveTurns;

  const firstLiveBlocks = liveTurns[0].blocks;
  const liveUserIndex = firstLiveBlocks.findIndex((block) => block.kind === "user");
  if (liveUserIndex < 0) return liveTurns;
  const liveUser = firstLiveBlocks[liveUserIndex] as UserRenderBlock;
  if (!sameAcpUserBlocks(historyUser.block, liveUser)) return liveTurns;

  const firstLiveTurn = {
    ...liveTurns[0],
    blocks: firstLiveBlocks.filter((_, index) => index !== liveUserIndex),
  };
  if (isEmptyRenderableTurn(firstLiveTurn)) {
    return liveTurns.slice(1);
  }
  return [firstLiveTurn, ...liveTurns.slice(1)];
}

function isEmptyRenderableTurn(turn: LiveTurn): boolean {
  if (
    turn.status === "pending" ||
    turn.status === "streaming" ||
    turn.status === "cancelling"
  ) {
    return false;
  }
  return turn.blocks.length === 0 &&
    turn.tools.length === 0 &&
    turn.permissions.length === 0 &&
    !turn.error;
}

export function sanitizeSessioAttachmentText(text: string): string {
  const withoutFileLinks = removeFileMarkdownLinks(text);
  return replaceXmlishBlocks(
    replaceXmlishBlocks(withoutFileLinks, "sessio-upload-file", (attrs) => {
      const uri = attrs.uri;
      const name = attrs.name ?? basenameFromUri(uri ?? "");
      return fileMarker(name, uri, true);
    }),
    "context",
    (attrs) => fileMarker(basenameFromUri(attrs.ref ?? ""), attrs.ref, true),
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

function fileMarker(name?: string | null, uri?: string | null, marked = false): string {
  const safeName = name?.trim() || "attachment";
  const safeUri = uri?.trim();
  const displayName = marked ? `${SESSIO_ATTACHMENT_MARKER}${safeName}` : safeName;
  return safeUri ? `[file: ${displayName}|${safeUri}]` : `[file: ${displayName}]`;
}

function imageMarkdown(
  block: Extract<AcpContentBlock, { type: "image" }>,
  marked = false,
): string {
  const mimeType = block.mimeType?.trim() || "image";
  const src = block.uri?.trim() ||
    (block.data?.trim()
      ? block.data.trim().startsWith("data:")
        ? block.data.trim()
        : `data:${mimeType};base64,${block.data.trim()}`
      : "");
  const label = marked ? `${SESSIO_ATTACHMENT_MARKER}${mimeType}` : mimeType;
  return src ? `![${label}](${src})` : `[image: ${mimeType}]`;
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
