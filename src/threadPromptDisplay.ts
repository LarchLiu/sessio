import type { AcpContentBlock } from "./runtimeChat";
import {
  stripSessioAssistantPromptBlocks,
  sessioThreadPromptBlockMetas,
  stripSessioThreadPromptBlocks,
} from "./historyMerge";

export function threadPromptDisplayContentBlocks(
  blocks: AcpContentBlock[],
  raw: unknown,
  showThreadPromptPlaceholders: boolean,
  fallbackPrompts: ThreadPromptDisplayMeta[] = [],
): AcpContentBlock[] {
  const out: AcpContentBlock[] = [];
  const hiddenPrompts = showThreadPromptPlaceholders
    ? threadPromptsFromUnknown(raw)
    : [];
  for (const block of blocks) {
    if (block.type !== "text") {
      out.push(block);
      continue;
    }
    if (showThreadPromptPlaceholders) {
      hiddenPrompts.push(
        ...threadPromptsFromBlockMeta(block),
        ...sessioThreadPromptBlockMetas(block.text).map((meta) => ({
          kind: meta.kind,
          attrs: meta.attrs,
          content: meta.content,
        })),
      );
    }
    const text = stripSessioAssistantPromptBlocks(
      stripSessioThreadPromptBlocks(block.text),
    );
    if (text.trim()) out.push({ ...block, text });
  }
  if (showThreadPromptPlaceholders && out.length === 0 && hiddenPrompts.length > 0) {
    out.push({ type: "text", text: threadPromptPlaceholderText(hiddenPrompts, fallbackPrompts) });
  }
  return out;
}

export interface ThreadPromptDisplayMeta {
  kind: string | null;
  attrs: Record<string, string>;
  content?: string;
}

function threadPromptsFromBlockMeta(block: AcpContentBlock): ThreadPromptDisplayMeta[] {
  const meta = asRecord(block.meta);
  const prompts = threadPromptsFromValue(meta.sessioThreadPrompts ?? meta.sessio_thread_prompts);
  if (prompts.length > 0) return prompts;
  return threadPromptKindsFromValue(meta.sessioThreadPromptKinds ?? meta.sessio_thread_prompt_kinds)
    .map((kind) => ({ kind, attrs: {} }));
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

function threadPromptsFromValue(value: unknown): ThreadPromptDisplayMeta[] {
  if (Array.isArray(value)) {
    return value.flatMap((item) => {
      if (typeof item === "string" && item.trim()) {
        return [{ kind: item.trim(), attrs: {} }];
      }
      const record = asRecord(item);
      if (Object.keys(record).length === 0) return [];
      const attrs = stringRecord(record.attrs);
      const kind = stringValue(record.kind) ?? stringValue(attrs.kind);
      return [{ kind, attrs: { ...attrs, ...(kind ? { kind } : {}) } }];
    });
  }
  if (typeof value === "string" && value.trim()) return [{ kind: value.trim(), attrs: {} }];
  return [];
}

function threadPromptsFromUnknown(value: unknown): ThreadPromptDisplayMeta[] {
  if (typeof value === "string") {
    return sessioThreadPromptBlockMetas(value).map((meta) => ({
      kind: meta.kind,
      attrs: meta.attrs,
      content: meta.content,
    }));
  }
  if (Array.isArray(value)) {
    return value.flatMap(threadPromptsFromUnknown);
  }
  const record = asRecord(value);
  if (Object.keys(record).length === 0) return [];
  const prompts = [
    ...threadPromptsFromValue(record.sessioThreadPrompts),
    ...threadPromptsFromValue(record.sessio_thread_prompts),
    ...threadPromptKindsFromValue(record.sessioThreadPromptKinds)
      .map((kind) => ({ kind, attrs: {} })),
    ...threadPromptKindsFromValue(record.sessio_thread_prompt_kinds)
      .map((kind) => ({ kind, attrs: {} })),
  ];
  const prompt = record.prompt;
  if (prompt !== value) prompts.push(...threadPromptsFromUnknown(prompt));
  const content = record.content;
  if (content !== value) prompts.push(...threadPromptsFromUnknown(content));
  const text = record.text;
  if (text !== value) prompts.push(...threadPromptsFromUnknown(text));
  return prompts;
}

function threadPromptPlaceholderText(
  prompts: ThreadPromptDisplayMeta[],
  fallbackPrompts: ThreadPromptDisplayMeta[],
): string {
  const labels = prompts
    .map((prompt) => threadPromptLabel(enrichPromptMeta(prompt, fallbackPrompts)))
    .filter(Boolean);
  const uniqueLabels = Array.from(new Set(labels));
  return uniqueLabels.join(", ") || "unknown";
}

function threadPromptLabel(prompt: ThreadPromptDisplayMeta): string {
  const attrs = prompt.attrs;
  const kind = prompt.kind?.trim() || attrs.kind || "unknown";
  const title =
    attrs.task_title ??
    attrs.title ??
    attrs.task ??
    attrs.user_prompt ??
    attrs.prompt_summary ??
    threadPromptTitleFromContent(prompt.content);
  const context = [
    attrs.assistant_name,
    attrs.stage_name,
  ]
    .filter(Boolean)
    .join(" / ");
  const parts = [kind, title, context].filter((part): part is string => Boolean(part?.trim()));
  return parts.join(" · ");
}

function enrichPromptMeta(
  prompt: ThreadPromptDisplayMeta,
  fallbackPrompts: ThreadPromptDisplayMeta[],
): ThreadPromptDisplayMeta {
  const fallback =
    fallbackPrompts.find((item) => item.kind && item.kind === prompt.kind) ??
    fallbackPrompts.find((item) => !item.kind) ??
    null;
  if (!fallback) return prompt;
  return {
    kind: prompt.kind ?? fallback.kind,
    attrs: {
      ...fallback.attrs,
      ...prompt.attrs,
      target_agent: prompt.attrs.target_agent ?? fallback.attrs.target_agent,
      task_title: prompt.attrs.task_title ?? fallback.attrs.task_title,
      assistant_name: prompt.attrs.assistant_name ?? fallback.attrs.assistant_name,
      stage_name: prompt.attrs.stage_name ?? fallback.attrs.stage_name,
      prompt_summary: prompt.attrs.prompt_summary ?? fallback.attrs.prompt_summary,
    },
    content: prompt.content ?? fallback.content,
  };
}

function threadPromptTitleFromContent(content: string | undefined): string | null {
  if (!content) return null;
  const jsonPrompt = promptSummaryFromJsonContent(content);
  if (jsonPrompt) return jsonPrompt;
  for (const line of content.split(/\r?\n/)) {
    const match = /^(?:Task title|Title|User prompt):\s*(.+)$/i.exec(line.trim());
    if (match?.[1]?.trim()) return match[1].trim();
  }
  return null;
}

function promptSummaryFromJsonContent(content: string): string | null {
  const trimmed = content.trim();
  if (!trimmed.startsWith("{")) return null;
  try {
    const record = asRecord(JSON.parse(trimmed) as unknown);
    return stringValue(record.userPrompt) ?? stringValue(record.prompt) ?? null;
  } catch {
    return null;
  }
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function stringRecord(value: unknown): Record<string, string> {
  const record = asRecord(value);
  const out: Record<string, string> = {};
  for (const [key, raw] of Object.entries(record)) {
    const value = stringValue(raw);
    if (value) out[key] = value;
  }
  return out;
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}
