import type { PartialBlock } from "@blocknote/core";

export type NotionParseMode = "markdown" | "source-fallback";

export interface SerializedNotionDoc {
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
  return {
    safe: normalizedOriginal === normalizedSerialized,
    serialized: normalizedSerialized,
  };
}

export function serializeSourceLineBlocks(blocks: PartialBlock[]): SerializedNotionDoc {
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
