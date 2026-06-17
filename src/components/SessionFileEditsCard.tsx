import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { MultiFileDiff, PatchDiff } from "@pierre/diffs/react";
import { ChevronDown, FileDiff } from "lucide-react";
import type { FileEditContentDiff, FileEditItem } from "../acpRenderItems";
import ScrollArea from "./ScrollArea";
import { useEffectiveThemeType } from "./shikiHighlight";
import { isNearScrollBottom } from "./useMessageStreamScrollController";

export interface SessionFileEditsCardProps {
  edits: FileEditItem[];
  additions: number;
  deletions: number;
  fileCount?: number;
  compact?: boolean;
  showAllFiles?: boolean;
}

export default function SessionFileEditsCard({
  edits,
  additions,
  deletions,
  fileCount = edits.length,
  compact = false,
  showAllFiles = false,
}: SessionFileEditsCardProps) {
  const stateKey = useMemo(
    () =>
      edits
        .map((edit) =>
          [
            edit.path ?? "",
            edit.displayPath ?? "",
            edit.additions ?? 0,
            edit.deletions ?? 0,
            edit.patch ?? "",
            edit.patches?.join("\n") ?? "",
            edit.oldContent ?? "",
            edit.newContent ?? "",
            edit.contentDiffs
              ?.map((diff) => `${diff.oldContent ?? ""}\u0000${diff.newContent ?? ""}`)
              .join("\n") ?? "",
            edit.detail ?? "",
            edit.details?.join("\n") ?? "",
          ].join("\u0001"),
        )
        .join("\u0002"),
    [edits],
  );
  const [expandedState, setExpandedState] = useState(() => ({
    key: stateKey,
    expanded: edits.length <= 3,
  }));
  const expanded = showAllFiles
    ? true
    : expandedState.key === stateKey
      ? expandedState.expanded
      : edits.length <= 3;
  const [openDetails, setOpenDetails] = useState<Set<string>>(() => new Set());
  const pendingScrollKeyRef = useRef<string | null>(null);
  const pendingBottomStickRef = useRef(false);
  const fileEditBlockRef = useRef<HTMLDivElement>(null);
  const detailRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const visibleEdits = expanded ? edits : edits.slice(0, 3);
  const hiddenCount = showAllFiles
    ? 0
    : Math.max(0, edits.length - visibleEdits.length);
  const setExpanded = (nextExpanded: boolean) => {
    setExpandedState({ key: stateKey, expanded: nextExpanded });
  };

  useEffect(() => {
    setOpenDetails(new Set());
    pendingScrollKeyRef.current = null;
    pendingBottomStickRef.current = false;
    detailRefs.current.clear();
  }, [stateKey]);

  const toggleDetail = (key: string) => {
    setOpenDetails((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        const scroller = findScroller(fileEditBlockRef.current);
        pendingBottomStickRef.current = scroller instanceof HTMLDivElement
          ? isNearScrollBottom(scroller)
          : false;
        pendingScrollKeyRef.current = key;
        next.add(key);
      }
      return next;
    });
  };

  useLayoutEffect(() => {
    const key = pendingScrollKeyRef.current;
    if (!key || !openDetails.has(key)) return;
    pendingScrollKeyRef.current = null;
    const node = detailRefs.current.get(key);
    if (!node) return;
    const stickToBottom = pendingBottomStickRef.current;
    pendingBottomStickRef.current = false;
    const align = () => {
      const scroller = findScroller(node);
      if (!scroller) {
        node.scrollIntoView({ block: "end", behavior: "auto" });
        return;
      }
      if (stickToBottom) {
        scroller.scrollTop = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
        return;
      }
      const nodeBottom = node.getBoundingClientRect().bottom;
      const scrollerBottom = scroller.getBoundingClientRect().bottom;
      const delta = nodeBottom - scrollerBottom + 12;
      if (delta > 0) {
        scroller.scrollTop += delta;
      }
    };
    window.requestAnimationFrame(() => {
      align();
      window.requestAnimationFrame(align);
    });
    const timers = [80, 180, 360].map((delay) => window.setTimeout(align, delay));
    return () => {
      timers.forEach((timer) => window.clearTimeout(timer));
    };
  }, [openDetails]);

  return (
    <div ref={fileEditBlockRef} className="overflow-hidden rounded-md bg-ink/[0.035]">
      <div className={"flex items-center justify-between gap-3 " + (compact ? "px-2 py-1.5" : "px-2.5 py-2")}>
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-bg-panel text-ink/70">
            <FileDiff className="h-4 w-4" />
          </span>
          <div className="min-w-0">
            <div className="text-body-sm font-medium text-ink/80">
              Edited {fileCount} {fileCount === 1 ? "file" : "files"}
            </div>
            <div className="font-mono text-caption leading-tight">
              <span className="text-[rgb(var(--color-emerald))]">+{additions}</span>
              <span className="text-ink/25"> </span>
              <span className="text-status-error">-{deletions}</span>
            </div>
          </div>
        </div>
      </div>
      {edits.length > 0 && (
        <div className="border-t border-ink/[0.07]">
          {visibleEdits.map((edit, index) => {
            const label = edit.displayPath || edit.path || "(unknown file)";
            const detailKey = `${index}:${edit.path || edit.displayPath || label}`;
            const detail = normalizeEditDetails(edit).join("\n\n");
            const hasDetail = hasRenderableEditDetail(edit);
            const detailOpen = openDetails.has(detailKey);
            const rowContent = (
              <>
                <span className="min-w-0 truncate text-ink/80">{label}</span>
                <div className="flex shrink-0 items-center gap-2">
                  <span className="font-mono text-caption">
                    <span className="text-[rgb(var(--color-emerald))]">+{edit.additions ?? 0}</span>
                    <span className="text-ink/25"> </span>
                    <span className="text-status-error">-{edit.deletions ?? 0}</span>
                  </span>
                  {hasDetail && (
                    <ChevronDown
                      className={
                        "h-3.5 w-3.5 text-ink/55 transition-transform " +
                        (detailOpen ? "rotate-180" : "")
                      }
                    />
                  )}
                </div>
              </>
            );
            return (
              <div key={`${label}-${index}`}>
                {hasDetail ? (
                  <button
                    type="button"
                    className="grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-2.5 py-1.5 text-left text-body-sm hover:bg-ink/[0.04]"
                    aria-expanded={detailOpen}
                    aria-label={
                      detailOpen
                        ? `Hide changes for ${label}`
                        : `Show changes for ${label}`
                    }
                    onClick={() => toggleDetail(detailKey)}
                  >
                    {rowContent}
                  </button>
                ) : (
                  <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-2.5 py-1.5 text-body-sm">
                    {rowContent}
                  </div>
                )}
                {detailOpen && hasDetail && (
                  <div
                    ref={(node) => {
                      if (node) {
                        detailRefs.current.set(detailKey, node);
                      } else {
                        detailRefs.current.delete(detailKey);
                      }
                    }}
                  >
                    <DiffPreview edit={edit} fallback={detail} />
                  </div>
                )}
              </div>
            );
          })}
          {!showAllFiles && hiddenCount > 0 && (
            <button
              type="button"
              className="flex w-full items-center gap-1 px-2.5 py-1.5 text-left text-body-sm text-ink/75 hover:bg-ink/[0.04]"
              onClick={() => setExpanded(true)}
            >
              <span>
                Show {hiddenCount} more {hiddenCount === 1 ? "file" : "files"}
              </span>
              <ChevronDown className="h-3.5 w-3.5" />
            </button>
          )}
          {!showAllFiles && expanded && edits.length > 3 && (
            <button
              type="button"
              className="flex w-full items-center gap-1 px-2.5 py-1.5 text-left text-body-sm text-ink/75 hover:bg-ink/[0.04]"
              onClick={() => {
                scrollBlockStartIntoView(fileEditBlockRef.current);
                setExpanded(false);
              }}
            >
              <span>Collapse files</span>
              <ChevronDown className="h-3.5 w-3.5 rotate-180" />
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function hasRenderableEditDetail(edit: FileEditItem): boolean {
  return Boolean(
    (typeof edit.patch === "string" && edit.patch.trim()) ||
      normalizeEditPatches(edit).length > 0 ||
      normalizeEditDetails(edit).length > 0 ||
      normalizeContentDiffs(edit).length > 0 ||
      typeof edit.oldContent === "string" ||
      typeof edit.newContent === "string",
  );
}

function diffPreviewOptions(themeType: "light" | "dark") {
  return {
    diffStyle: "unified" as const,
    overflow: "scroll" as const,
    theme: {
      dark: "github-dark",
      light: "github-light",
    },
    themeType,
    disableFileHeader: true,
    hunkSeparators: "line-info-basic" as const,
    lineDiffType: "word" as const,
  };
}

function DiffPreview({
  edit,
  fallback,
}: {
  edit: FileEditItem;
  fallback: string;
}) {
  const name = edit.displayPath || edit.path || "file";
  const themeType = useEffectiveThemeType();
  const options = useMemo(() => diffPreviewOptions(themeType), [themeType]);
  const contentDiffs = normalizeContentDiffs(edit);
  const patch = typeof edit.patch === "string" ? edit.patch : "";
  const patches = normalizeEditPatches(edit);
  return (
    <ScrollArea
      className="mx-2.5 mb-2 max-h-72 rounded bg-bg-panel-alt"
      viewportClassName="p-0"
      orientation="both"
      persistScrollbars
    >
      <div className="min-w-max text-caption sessio-diff-preview">
        {patches.length > 0 || patch.trim() ? (
          <>
            {(patches.length > 0 ? patches : [patch]).map((patchItem, index) => (
              <PatchDiff
                key={index}
                patch={patchItem}
                options={options}
                disableWorkerPool
              />
            ))}
          </>
        ) : contentDiffs.length > 0 ? (
          <>
            {contentDiffs.map((contentDiff, index) => (
              <MultiFileDiff
                key={index}
                oldFile={{ name, contents: contentDiff.oldContent ?? "" }}
                newFile={{ name, contents: contentDiff.newContent ?? "" }}
                options={options}
                disableWorkerPool
              />
            ))}
          </>
        ) : (
          <pre className="px-2.5 py-2 font-mono text-caption leading-relaxed text-ink/75">
            <code>{fallback}</code>
          </pre>
        )}
      </div>
    </ScrollArea>
  );
}

function normalizeEditPatches(edit: FileEditItem): string[] {
  const patches = Array.isArray(edit.patches)
    ? edit.patches.filter((item): item is string => Boolean(item.trim()))
    : [];
  if (typeof edit.patch === "string" && edit.patch.trim()) {
    patches.push(edit.patch);
  }
  return Array.from(new Set(patches));
}

function normalizeEditDetails(edit: FileEditItem): string[] {
  const details = Array.isArray(edit.details)
    ? edit.details.filter((item): item is string => Boolean(item.trim()))
    : [];
  if (typeof edit.detail === "string" && edit.detail.trim()) {
    details.push(edit.detail);
  }
  return Array.from(new Set(details));
}

function normalizeContentDiffs(edit: FileEditItem): FileEditContentDiff[] {
  const diffs = Array.isArray(edit.contentDiffs)
    ? edit.contentDiffs.filter(
        (item) =>
          typeof item.oldContent === "string" ||
          typeof item.newContent === "string",
      )
    : [];
  if (
    typeof edit.oldContent === "string" ||
    typeof edit.newContent === "string"
  ) {
    diffs.push({
      oldContent: edit.oldContent,
      newContent: edit.newContent,
    });
  }
  const seen = new Set<string>();
  return diffs.filter((diff) => {
    const key = `${diff.oldContent ?? ""}\u0000${diff.newContent ?? ""}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function findScroller(el: HTMLElement | null): HTMLElement | null {
  let current: HTMLElement | null = el?.parentElement ?? null;
  while (current) {
    const style = window.getComputedStyle(current);
    if (
      (style.overflowY === "auto" || style.overflowY === "scroll") &&
      current.scrollHeight > current.clientHeight + 1
    ) {
      return current;
    }
    current = current.parentElement;
  }
  return null;
}

function scrollBlockStartIntoView(el: HTMLElement | null) {
  if (!el) return;
  window.requestAnimationFrame(() => {
    const scroller = findScroller(el);
    if (scroller) {
      const top = el.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop;
      scroller.scrollTop = Math.max(0, top - 12);
      return;
    }
    el.scrollIntoView({ block: "start", behavior: "auto" });
  });
}
