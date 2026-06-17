import type { PartialBlock } from "@blocknote/core";

export type PlainEditorParseMode = "markdown" | "source-fallback";

export interface SerializedPlainEditorDoc {
  ok: boolean;
  content: string;
  reason?: "non_linear_source_fallback";
}

export interface RoundTripResult {
  safe: boolean;
  serialized: string;
}

export function normalizeEditorText(text: string): string {
  return text.replace(/\r\n/g, "\n");
}

export function roundTripMatches(original: string, serialized: string): RoundTripResult {
  const normalizedOriginal = normalizeEditorText(original);
  const normalizedSerialized = normalizeEditorText(serialized);
  const safe =
    normalizedOriginal === normalizedSerialized ||
    normalizeSafeMarkdownFormatting(normalizedOriginal) ===
      normalizeSafeMarkdownFormatting(normalizedSerialized);
  return {
    safe,
    serialized: normalizedSerialized,
  };
}

function normalizeSafeMarkdownFormatting(text: string): string {
  const lines = stripSingleTrailingNewline(text)
    .split("\n")
    .map((line) => normalizeSafeMarkdownLine(line));
  return removeSafeLooseListBlankLines(lines).join("\n");
}

function stripSingleTrailingNewline(text: string): string {
  return text.endsWith("\n") ? text.slice(0, -1) : text;
}

function normalizeSafeMarkdownLine(line: string): string {
  const unorderedList = line.match(/^(\s*)[-+]\s+(.+)$/);
  if (unorderedList) return `${unorderedList[1]}* ${unorderedList[2]}`;

  const textCodeFence = line.match(/^(\s*)```text\s*$/);
  if (textCodeFence) return `${textCodeFence[1]}\`\`\``;

  const thematicBreak = line.match(/^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/);
  if (thematicBreak) return "***";

  return line;
}

function removeSafeLooseListBlankLines(lines: string[]): string[] {
  const normalized: string[] = [];
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (
      line.trim() === "" &&
      isListItemLine(previousNonBlankLine(lines, index)) &&
      isListItemLine(nextNonBlankLine(lines, index))
    ) {
      continue;
    }
    normalized.push(line);
  }
  return normalized;
}

function previousNonBlankLine(lines: string[], startIndex: number): string | null {
  for (let index = startIndex - 1; index >= 0; index -= 1) {
    if (lines[index].trim() !== "") return lines[index];
  }
  return null;
}

function nextNonBlankLine(lines: string[], startIndex: number): string | null {
  for (let index = startIndex + 1; index < lines.length; index += 1) {
    if (lines[index].trim() !== "") return lines[index];
  }
  return null;
}

function isListItemLine(line: string | null): boolean {
  if (line === null) return false;
  return /^(\s*)(?:\*\s+|\d+[.)]\s+)(?:\[[ xX]\]\s+)?\S/.test(line);
}

export function serializeSourceLineBlocks(blocks: PartialBlock[]): SerializedPlainEditorDoc {
  const lines: string[] = [];
  for (const block of blocks) {
    if (block.type !== "paragraph") {
      return { ok: false, content: "", reason: "non_linear_source_fallback" };
    }
    if (block.children && block.children.length > 0) {
      return { ok: false, content: "", reason: "non_linear_source_fallback" };
    }
    const text = sourceParagraphText(block);
    if (text === null) {
      return { ok: false, content: "", reason: "non_linear_source_fallback" };
    }
    lines.push(text);
  }
  return { ok: true, content: lines.join("\n") };
}

function sourceParagraphText(block: PartialBlock): string | null {
  const content = block.content;
  if (content === undefined) return "";
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return null;

  let text = "";
  for (const item of content) {
    if (typeof item === "string") {
      text += item;
      continue;
    }
    if (!item || typeof item !== "object") return null;
    const maybeText = item as { type?: string; text?: unknown };
    if (maybeText.type !== "text" || typeof maybeText.text !== "string") {
      return null;
    }
    text += maybeText.text;
  }
  return text;
}
