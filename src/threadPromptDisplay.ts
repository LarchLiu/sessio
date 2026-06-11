import type { AcpContentBlock } from "./runtimeChat";
import {
  sessioThreadPromptBlockMetas,
  stripSessioThreadPromptBlocks,
} from "./historyMerge";

export function threadPromptDisplayContentBlocks(
  blocks: AcpContentBlock[],
  raw: unknown,
  showThreadPromptPlaceholders: boolean,
): AcpContentBlock[] {
  const out: AcpContentBlock[] = [];
  const hiddenKinds = showThreadPromptPlaceholders
    ? threadPromptKindsFromUnknown(raw)
    : [];
  for (const block of blocks) {
    if (block.type !== "text") {
      out.push(block);
      continue;
    }
    if (showThreadPromptPlaceholders) {
      hiddenKinds.push(
        ...threadPromptKindsFromBlockMeta(block),
        ...sessioThreadPromptBlockMetas(block.text)
          .map((meta) => meta.kind)
          .filter((kind): kind is string => Boolean(kind)),
      );
    }
    const text = stripSessioThreadPromptBlocks(block.text);
    if (text.trim()) out.push({ ...block, text });
  }
  if (showThreadPromptPlaceholders && out.length === 0 && hiddenKinds.length > 0) {
    out.push({ type: "text", text: threadPromptPlaceholderText(hiddenKinds) });
  }
  return out;
}

function threadPromptKindsFromBlockMeta(block: AcpContentBlock): string[] {
  const meta = asRecord(block.meta);
  const value = meta.sessioThreadPromptKinds ?? meta.sessio_thread_prompt_kinds;
  return threadPromptKindsFromValue(value);
}

function threadPromptKindsFromValue(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value
      .map((item) => typeof item === "string" ? item.trim() : "")
      .filter(Boolean);
  }
  if (typeof value === "string" && value.trim()) return [value.trim()];
  return [];
}

function threadPromptKindsFromUnknown(value: unknown): string[] {
  if (typeof value === "string") {
    return sessioThreadPromptBlockMetas(value)
      .map((meta) => meta.kind)
      .filter((kind): kind is string => Boolean(kind));
  }
  if (Array.isArray(value)) {
    return value.flatMap(threadPromptKindsFromUnknown);
  }
  const record = asRecord(value);
  if (Object.keys(record).length === 0) return [];
  const kinds = [
    ...threadPromptKindsFromValue(record.sessioThreadPromptKinds),
    ...threadPromptKindsFromValue(record.sessio_thread_prompt_kinds),
  ];
  const prompt = record.prompt;
  if (prompt !== value) kinds.push(...threadPromptKindsFromUnknown(prompt));
  const content = record.content;
  if (content !== value) kinds.push(...threadPromptKindsFromUnknown(content));
  const text = record.text;
  if (text !== value) kinds.push(...threadPromptKindsFromUnknown(text));
  return kinds;
}

function threadPromptPlaceholderText(kinds: string[]): string {
  const uniqueKinds = Array.from(new Set(kinds.map((kind) => kind.trim()).filter(Boolean)));
  return `Thread prompt: ${uniqueKinds.join(", ") || "unknown"}`;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}
