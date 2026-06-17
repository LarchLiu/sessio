import { useMemo } from "react";
import { FileDiff, LoaderCircle } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { AcpViewModel, LiveTurn, AcpToolCall } from "../runtimeChat";
import { useI18n } from "../i18n";

export interface MinimalStripItem {
  icon?: LucideIcon | null;
  text: string;
  busy: boolean;
}

const MAX_TEXT_LENGTH = 240;

export function pickLatestStripItem(
  viewModel: AcpViewModel,
  workingTurnId: string | null,
): MinimalStripItem | null {
  for (let i = viewModel.turns.length - 1; i >= 0; i -= 1) {
    const turn = viewModel.turns[i];
    const item = pickFromTurn(turn, workingTurnId);
    if (item) return item;
  }
  return null;
}

function pickFromTurn(turn: LiveTurn, workingTurnId: string | null): MinimalStripItem | null {
  const busy = turn.turnId === workingTurnId;
  for (let i = turn.blocks.length - 1; i >= 0; i -= 1) {
    const block = turn.blocks[i];
    if (block.kind === "permission" || block.kind === "error") continue;
    if (block.kind === "tool") {
      const tool = turn.tools.find((t) => t.toolId === block.toolId);
      if (!tool) continue;
      return { icon: null, text: toolStripText(tool), busy };
    }
    if (block.kind === "user") continue;
    if (block.kind === "sessionUpdate") {
      if (block.updateType === "file_edit") continue;
      const text = sessionUpdateText(block.data);
      if (text) return { icon: null, text: clampText(text), busy };
      continue;
    }
    if (block.kind === "assistant" || block.kind === "thought") {
      const text = contentBlocksToText(block.blocks);
      if (text.trim()) return { icon: null, text: clampText(text), busy };
    }
  }
  if (turn.tools.length > 0) {
    const tool = turn.tools[turn.tools.length - 1];
    return { icon: null, text: toolStripText(tool), busy };
  }
  return null;
}

function toolStripText(tool: AcpToolCall): string {
  const name = tool.title || tool.kind || "tool";
  const detail = pickToolDetail(tool);
  return detail ? `${name} · ${detail}` : name;
}

function pickToolDetail(tool: AcpToolCall): string {
  const raw = tool.rawInput;
  if (!raw || typeof raw !== "object") return "";
  const record = raw as Record<string, unknown>;
  const candidates = [
    "file_path",
    "filePath",
    "path",
    "command",
    "query",
    "pattern",
    "url",
    "description",
  ];
  for (const key of candidates) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) {
      return clampText(value.trim().split("\n")[0]);
    }
  }
  return "";
}

function sessionUpdateText(data: unknown): string {
  if (typeof data === "string") return data.split("\n")[0];
  if (data && typeof data === "object") {
    const record = data as Record<string, unknown>;
    const text = record.text ?? record.message ?? record.summary;
    if (typeof text === "string") return text.split("\n")[0];
  }
  return "";
}

function contentBlocksToText(blocks: unknown): string {
  if (!Array.isArray(blocks)) return "";
  for (let i = blocks.length - 1; i >= 0; i -= 1) {
    const block = blocks[i] as Record<string, unknown> | undefined;
    if (!block || typeof block !== "object") continue;
    const text = block.text;
    if (typeof text === "string" && text.trim()) {
      return text.split("\n").filter((line) => line.trim()).pop() ?? text;
    }
  }
  return "";
}

function clampText(text: string): string {
  const trimmed = text.trim();
  if (trimmed.length <= MAX_TEXT_LENGTH) return trimmed;
  return trimmed.slice(0, MAX_TEXT_LENGTH - 1) + "…";
}

/**
 * A vertically stacked group of slim attachments that sits above the
 * ChatComposer. The whole group is a single rounded-top panel a bit
 * narrower than the composer, with no bottom border so it visually
 * tucks into the composer's top edge.
 */
export function ComposerTopAttachments({ children }: { children: React.ReactNode }) {
  const visibleChildren = Array.isArray(children)
    ? children.filter(Boolean)
    : children
      ? [children]
      : [];
  if (visibleChildren.length === 0) return null;
  return (
    <div className="-mb-2 flex justify-center px-16">
      <div className="flex w-full flex-col overflow-hidden rounded-t-xl border border-b-0 border-ink/[0.10] bg-ink/[0.05]">
        {visibleChildren.map((child, index) => (
          <div
            key={index}
            className={index > 0 ? "border-t border-ink/[0.06]" : ""}
          >
            {child}
          </div>
        ))}
      </div>
    </div>
  );
}

export function MinimalMessageStrip({
  viewModel,
  workingTurnId,
}: {
  viewModel: AcpViewModel;
  workingTurnId: string | null;
}) {
  const item = useMemo(
    () => pickLatestStripItem(viewModel, workingTurnId),
    [viewModel, workingTurnId],
  );
  if (!item) return null;
  return (
    <div className="flex h-7 w-full items-center gap-2 px-3 text-caption text-ink/60">
      {item.busy && <LoaderCircle className="h-3 w-3 shrink-0 animate-spin text-ink/45" />}
      <span className="min-w-0 truncate">{item.text}</span>
    </div>
  );
}

export function EditedFilesBar({
  fileCount,
  additions,
  deletions,
  onClick,
}: {
  fileCount: number;
  additions: number;
  deletions: number;
  onClick?: () => void;
}) {
  const { t } = useI18n();
  if (fileCount === 0) return null;
  const label =
    fileCount === 1
      ? t("chat.files.count_one")
      : t("chat.files.count", { count: fileCount });
  const Tag = onClick ? "button" : "div";
  return (
    <Tag
      type={onClick ? "button" : undefined}
      onClick={onClick}
      className={
        "flex h-7 w-full items-center gap-2 px-3 text-caption text-ink/65 transition-colors " +
        (onClick ? "cursor-pointer text-left hover:text-ink/85" : "")
      }
    >
      <FileDiff className="h-3.5 w-3.5 shrink-0 text-ink/45" />
      <span className="shrink-0 font-medium">{label}</span>
      <span className="shrink-0 font-mono">
        <span className="text-[rgb(var(--color-emerald))]">+{additions}</span>
        <span className="text-ink/25"> </span>
        <span className="text-status-error">-{deletions}</span>
      </span>
    </Tag>
  );
}
