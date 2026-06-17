import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { FileDiff, LoaderCircle } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { FileEditItem } from "../acpRenderItems";
import type { AcpViewModel, LiveTurn, AcpToolCall } from "../runtimeChat";
import { useI18n } from "../i18n";
import ScrollArea from "./ScrollArea";
import SessionFileEditsCard from "./SessionFileEditsCard";
import Tooltip from "./Tooltip";

export interface MinimalStripItem {
  icon?: LucideIcon | null;
  text: string;
  busy: boolean;
  lines?: string[];
  fullText?: string | null;
  streamKey?: string | null;
  scrollOnce?: boolean;
}

const STRIP_LINE_ADVANCE_MS = 1100;

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
    if (block.kind === "assistant") {
      const lines = contentBlocksToLines(block.blocks);
      if (lines.length > 0) {
        const isLiveAssistant = busy;
        return {
          icon: null,
          text: lines[lines.length - 1],
          lines: isLiveAssistant ? lines : undefined,
          fullText: lines.join("\n"),
          busy,
          scrollOnce: isLiveAssistant && lines.length > 1,
          streamKey: `${turn.turnId}:assistant:${i}`,
        };
      }
      continue;
    }
    if (block.kind === "thought") {
      const text = contentBlocksToLastLine(block.blocks);
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

function contentBlocksToLines(blocks: unknown): string[] {
  if (!Array.isArray(blocks)) return [];
  const lines: string[] = [];
  for (const entry of blocks) {
    const block = entry as Record<string, unknown> | undefined;
    if (!block || typeof block !== "object") continue;
    const text = block.text;
    if (typeof text !== "string" || !text.trim()) continue;
    for (const line of text.split("\n")) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      lines.push(clampText(trimmed));
    }
  }
  return lines;
}

function contentBlocksToLastLine(blocks: unknown): string {
  const lines = contentBlocksToLines(blocks);
  return lines[lines.length - 1] ?? "";
}

function clampText(text: string): string {
  return text.trim();
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
  const lines = item?.lines?.length ? item.lines : item ? [item.text] : [];
  const [activeLineIndex, setActiveLineIndex] = useState(0);
  const lastStreamKeyRef = useRef<string | null>(null);

  useEffect(() => {
    if (!item?.scrollOnce) {
      lastStreamKeyRef.current = item?.streamKey ?? null;
      setActiveLineIndex(0);
      return;
    }
    if (lastStreamKeyRef.current !== item.streamKey) {
      lastStreamKeyRef.current = item.streamKey ?? null;
      setActiveLineIndex(0);
      return;
    }
    setActiveLineIndex((current) => Math.min(current, Math.max(0, lines.length - 1)));
  }, [item?.scrollOnce, item?.streamKey, lines.length]);

  useEffect(() => {
    if (!item?.scrollOnce || lines.length <= 1) return;
    if (activeLineIndex >= lines.length - 1) return;
    const timer = window.setTimeout(() => {
      setActiveLineIndex((current) => Math.min(current + 1, lines.length - 1));
    }, STRIP_LINE_ADVANCE_MS);
    return () => window.clearTimeout(timer);
  }, [activeLineIndex, item?.scrollOnce, lines, lines.length]);

  if (!item) return null;
  const activeText = lines[activeLineIndex] ?? item.text;
  const textNode = (
    <span className="min-w-0 truncate">{activeText}</span>
  );
  return (
    <div className="flex h-7 w-full items-center gap-2 px-3 text-caption text-ink/60">
      {item.busy && <LoaderCircle className="h-3 w-3 shrink-0 animate-spin text-ink/45" />}
      {item.fullText ? (
        <Tooltip
          content={
            <div className="whitespace-pre-wrap break-words font-mono text-caption leading-relaxed">
              {item.fullText}
            </div>
          }
          placement="top"
          delayMs={250}
          interactive
          matchAnchorWidth
        >
          <div className="min-w-0 flex-1 overflow-hidden">{textNode}</div>
        </Tooltip>
      ) : (
        <div className="min-w-0 flex-1 overflow-hidden">{textNode}</div>
      )}
    </div>
  );
}

export function EditedFilesBar({
  fileCount,
  additions,
  deletions,
  edits = [],
  onClick,
}: {
  fileCount: number;
  additions: number;
  deletions: number;
  edits?: FileEditItem[];
  onClick?: () => void;
}) {
  const { t } = useI18n();
  const anchorRef = useRef<HTMLButtonElement | HTMLDivElement | null>(null);
  const popupRef = useRef<HTMLDivElement | null>(null);
  const popupContentRef = useRef<HTMLDivElement | null>(null);
  const hideTimerRef = useRef<number | null>(null);
  const [hoverOpen, setHoverOpen] = useState(false);
  const [popupPos, setPopupPos] = useState<{
    top: number;
    left: number;
    width: number;
    height: number;
  } | null>(null);

  const clearHideTimer = () => {
    if (hideTimerRef.current === null) return;
    window.clearTimeout(hideTimerRef.current);
    hideTimerRef.current = null;
  };

  const scheduleHide = () => {
    clearHideTimer();
    hideTimerRef.current = window.setTimeout(() => {
      hideTimerRef.current = null;
      setHoverOpen(false);
    }, 120);
  };

  const openHover = () => {
    clearHideTimer();
    if (edits.length > 0) setHoverOpen(true);
  };

  const closeIfOutside = () => {
    if (popupRef.current?.matches(":hover")) return;
    scheduleHide();
  };

  useLayoutEffect(() => {
    if (!hoverOpen) return;
    const updatePosition = () => {
      const anchor = anchorRef.current;
      const popup = popupRef.current;
      if (!anchor || !popup) return;
      const rect = anchor.getBoundingClientRect();
      const margin = 12;
      const gap = 8;
      const availableBelow = window.innerHeight - margin - rect.bottom - gap;
      const availableAbove = rect.top - margin - gap;
      const width = Math.min(rect.width, window.innerWidth - margin * 2);

      popup.style.width = `${width}px`;
      const content = popupContentRef.current;
      const computed = window.getComputedStyle(popup);
      const verticalPadding =
        Number.parseFloat(computed.paddingTop || "0") +
        Number.parseFloat(computed.paddingBottom || "0");
      const naturalHeight = content
        ? content.scrollHeight + verticalPadding
        : popup.scrollHeight;
      const fitsBelow = naturalHeight <= availableBelow;
      const fitsAbove = naturalHeight <= availableAbove;
      const placeBelow = fitsBelow || (!fitsAbove && availableBelow >= availableAbove);
      const availableHeight = Math.max(0, placeBelow ? availableBelow : availableAbove);
      const finalHeight = Math.min(520, availableHeight, naturalHeight);

      popup.style.height = `${finalHeight}px`;
      popup.style.maxHeight = `${finalHeight}px`;

      const left = Math.min(
        Math.max(margin, rect.left + rect.width / 2 - width / 2),
        window.innerWidth - margin - width,
      );
      const top = placeBelow
        ? Math.min(rect.bottom + gap, window.innerHeight - margin - finalHeight)
        : Math.max(margin, rect.top - gap - finalHeight);

      setPopupPos({
        top,
        left,
        width,
        height: finalHeight,
      });
    };
    updatePosition();
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    const content = popupContentRef.current;
    const ro = content ? new ResizeObserver(() => updatePosition()) : null;
    if (content && ro) ro.observe(content);
    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
      ro?.disconnect();
    };
  }, [hoverOpen]);

  useEffect(() => {
    return () => {
      clearHideTimer();
    };
  }, []);

  if (fileCount === 0) return null;
  const label =
    fileCount === 1
      ? t("chat.files.count_one")
      : t("chat.files.count", { count: fileCount });
  const Tag = onClick ? "button" : "div";
  const popupStyle = popupPos ?? { top: 0, left: 0, width: 0, height: 0 };

  return (
    <>
      <Tag
        ref={anchorRef as never}
        type={onClick ? "button" : undefined}
        onClick={onClick}
        onMouseEnter={openHover}
        onMouseLeave={closeIfOutside}
        onFocus={openHover}
        onBlur={scheduleHide}
        className={
          "flex h-7 w-full items-center gap-2 px-3 text-caption text-ink/65 transition-colors " +
          (onClick ? "cursor-pointer text-left hover:text-ink/85" : "hover:text-ink/85")
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
      {hoverOpen && edits.length > 0
        ? createPortal(
            <div
              ref={popupRef}
              style={{
                position: "fixed",
                top: popupStyle.top,
                left: popupStyle.left,
                width: popupStyle.width,
                height: popupStyle.height,
                visibility: popupPos ? "visible" : "hidden",
              }}
              onMouseEnter={openHover}
              onMouseLeave={closeIfOutside}
              className="chat-bottom-edited-files-popup z-50 min-h-0 overflow-hidden rounded-xl border border-ink/[0.10] bg-surface-panel p-2 shadow-[0_16px_48px_rgba(0,0,0,0.22)]"
            >
              <ScrollArea className="h-full" persistScrollbars>
                <div ref={popupContentRef}>
                  <SessionFileEditsCard
                    edits={edits}
                    additions={additions}
                    deletions={deletions}
                    fileCount={fileCount}
                    compact
                    showAllFiles
                  />
                </div>
              </ScrollArea>
            </div>,
            document.body,
          )
        : null}
    </>
  );
}
