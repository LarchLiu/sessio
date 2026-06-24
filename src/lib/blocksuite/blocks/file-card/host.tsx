import { startTransition, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { GrammarState } from "shiki";
import { ArrowUpRight, ChevronDown, ChevronUp, FileDiff } from "lucide-react";
import PlainMarkdownPreview from "../../../../components/PlainMarkdownPreview";
import ScrollArea from "../../../../components/ScrollArea";
import { readWorkspaceTextFile } from "../../../../api";
import { isPlainEditorMarkdownDocumentPath } from "../../../../hooks/plainEditorFileTypes";
import { languageFromPath } from "../../../../hooks/useFileContent";
import {
  getShikiHighlighter,
  renderShikiLine,
  shikiLanguage,
  shikiTheme,
  type ShikiHighlightedLine,
  useEffectiveThemeType,
} from "../../../../components/shikiHighlight";

function stopOverlayInteraction(event: {
  preventDefault: () => void;
  stopPropagation: () => void;
}) {
  event.preventDefault();
  event.stopPropagation();
}

export interface FileCardHostProps {
  workspacePath: string | null;
  blockId: string;
  selected?: boolean;
  title: string;
  sourcePath: string;
  subtitle: string;
  contentVersion: string;
  previewCollapsed: boolean;
  isLatestEditedFile?: boolean;
  onTogglePreviewCollapsed: (nextCollapsed: boolean) => void;
  onOpenFile?: (path: string) => void;
  onHeaderPointerDown?: (event: React.PointerEvent<HTMLDivElement>) => void;
  interactionMode?: "block" | "overlay";
}

export function FileCardHost({
  workspacePath,
  selected = false,
  title,
  sourcePath,
  subtitle,
  contentVersion,
  previewCollapsed,
  isLatestEditedFile = false,
  onTogglePreviewCollapsed,
  onOpenFile,
  onHeaderPointerDown,
  interactionMode = "block",
}: FileCardHostProps) {
  const overlayRootClassName =
    interactionMode === "overlay" ? "pointer-events-none" : "";
  const overlayActionClassName =
    interactionMode === "overlay" ? "pointer-events-auto" : "";
  const overlayContentClassName =
    interactionMode === "overlay" ? "pointer-events-auto" : "";

  const [content, setContent] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const shouldLoadPreview = !previewCollapsed;
  const resolvedSourcePath = useMemo(
    () => resolveWorkspaceFilePath(sourcePath, workspacePath),
    [sourcePath, workspacePath],
  );
  const displayPath = useMemo(
    () => resolveWorkspaceRelativePath(subtitle || sourcePath, workspacePath),
    [sourcePath, subtitle, workspacePath],
  );
  const previewPath = resolvedSourcePath ?? sourcePath;
  const renderMarkdownPreview = useMemo(
    () => isPlainEditorMarkdownDocumentPath(previewPath),
    [previewPath],
  );
  const previewLanguage = useMemo(
    () => languageFromPath(previewPath),
    [previewPath],
  );
  useEffect(() => {
    if (!shouldLoadPreview) return;
    if (!workspacePath || !resolvedSourcePath) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    readWorkspaceTextFile(workspacePath, resolvedSourcePath)
      .then((file) => {
        if (cancelled) return;
        setContent(file.content);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [contentVersion, resolvedSourcePath, shouldLoadPreview, workspacePath]);

  const capturePreviewWheel = shouldLoadPreview && selected;
  const stopPreviewWheelPropagation = capturePreviewWheel
    ? (event: React.WheelEvent<HTMLDivElement>) => {
        event.stopPropagation();
      }
    : undefined;

  return (
    <div className={"flex h-full w-full min-h-0 flex-col overflow-hidden rounded-[20px] border border-ink/10 bg-surface-panel/95 text-ink/80 shadow-[0_16px_40px_rgba(18,24,33,0.08)] " + overlayRootClassName}>
      <div
        onPointerDown={onHeaderPointerDown}
        className={
          "shrink-0 px-4 py-3 " +
          (!previewCollapsed
            ? "relative after:pointer-events-none after:absolute after:bottom-0 after:left-0 after:right-0 after:h-px after:bg-ink/10 after:content-[''] "
            : "") +
          overlayContentClassName
        }
      >
        <div className="flex w-full items-center justify-between gap-3">
          <div className="flex min-w-0 flex-1 items-center gap-1.5 text-body-sm font-medium text-ink/88">
            {isLatestEditedFile && <FileDiff className="h-3.5 w-3.5 shrink-0 text-ink/45" />}
            <span className="min-w-0 flex-1 truncate">{title || "File card"}</span>
          </div>
          <div className={"flex shrink-0 items-center gap-2 " + overlayActionClassName}>
            <button
              type="button"
              onPointerDown={stopOverlayInteraction}
              onMouseDown={stopOverlayInteraction}
              onClick={(event) => {
                stopOverlayInteraction(event);
                if (!resolvedSourcePath) return;
                onOpenFile?.(resolvedSourcePath);
              }}
              className="flex h-5 w-5 items-center justify-center rounded text-ink/45 transition hover:bg-ink/[0.06] hover:text-ink/80"
              aria-label={`Open ${title || sourcePath}`}
            >
              <ArrowUpRight className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              onPointerDown={stopOverlayInteraction}
              onMouseDown={stopOverlayInteraction}
              onClick={(event) => {
                stopOverlayInteraction(event);
                onTogglePreviewCollapsed(!previewCollapsed);
              }}
              className="flex h-5 w-5 items-center justify-center rounded text-ink/45 transition hover:bg-ink/[0.06] hover:text-ink/80"
              aria-label={previewCollapsed ? "Open preview" : "Collapse preview"}
              aria-expanded={!previewCollapsed}
            >
              {previewCollapsed ? (
                <ChevronDown className="h-3.5 w-3.5 text-ink/55" />
              ) : (
                <ChevronUp className="h-3.5 w-3.5 text-ink/55" />
              )}
            </button>
          </div>
        </div>
        {!previewCollapsed && (
          <div className="mt-1 w-full break-all font-mono text-[11px] leading-4 text-ink/48">
            {displayPath}
          </div>
        )}
      </div>
      {!previewCollapsed && (
        <div
          className={
            "flex min-h-0 min-w-0 flex-1 overflow-hidden overscroll-contain p-2 " +
            overlayContentClassName
          }
          onWheelCapture={stopPreviewWheelPropagation}
          onWheel={stopPreviewWheelPropagation}
        >
          {loading && <div className="text-caption text-ink/52">Loading preview...</div>}
          {!loading && error && <div className="text-caption text-status-error">{error}</div>}
          {!loading && !error && content !== null && (
            renderMarkdownPreview ? (
              <PlainMarkdownPreview
                text={content}
                filePath={resolvedSourcePath}
                interactionMode={
                  capturePreviewWheel
                    ? "capture-wheel"
                    : "thumbs-only"
                }
                scrollbarInset="flush"
              />
            ) : (
              <FileCardVirtualizedCodePreview
                code={content}
                language={previewLanguage}
                interactionMode={
                  capturePreviewWheel
                    ? "capture-wheel"
                    : "thumbs-only"
                }
              />
            )
          )}
        </div>
      )}
    </div>
  );
}

function resolveWorkspaceFilePath(
  path: string,
  workspacePath: string | null,
): string | null {
  if (!path) return null;
  if (/^([a-zA-Z]:[\\/]|\/)/.test(path)) return path;
  if (!workspacePath) return null;
  const separator = workspacePath.includes("\\") ? "\\" : "/";
  const trimmedRoot = workspacePath.replace(/[\\/]+$/, "");
  const trimmedPath = path.replace(/^[\\/]+/, "");
  return `${trimmedRoot}${separator}${trimmedPath}`;
}

function resolveWorkspaceRelativePath(
  path: string,
  workspacePath: string | null,
): string {
  if (!path) return "";
  if (!workspacePath) return path;
  const normalizedPath = path.replace(/\\/g, "/");
  const normalizedWorkspace = workspacePath.replace(/\\/g, "/").replace(/\/+$/, "");
  if (normalizedPath === normalizedWorkspace) return "";
  if (!normalizedPath.startsWith(`${normalizedWorkspace}/`)) return path;
  return normalizedPath.slice(normalizedWorkspace.length + 1);
}

const FILE_CARD_CODE_LINE_HEIGHT = 20;
const FILE_CARD_CODE_OVERSCAN = 40;
const FILE_CARD_HIGHLIGHT_CHUNK_LINES = 120;
const FILE_CARD_HIGHLIGHT_NEAR_VIEWPORT_BURST = 4;
const FILE_CARD_HIGHLIGHT_BACKGROUND_BURST = 1;
const FILE_CARD_HIGHLIGHT_NEAR_VIEWPORT_BUDGET_MS = 8;
const FILE_CARD_HIGHLIGHT_BACKGROUND_BUDGET_MS = 4;

function FileCardVirtualizedCodePreview({
  code,
  language,
  interactionMode,
}: {
  code: string;
  language: string;
  interactionMode: "default" | "thumbs-only" | "capture-wheel";
}) {
  const themeType = useEffectiveThemeType();
  const [viewportElement, setViewportElement] = useState<HTMLDivElement | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);

  const handleViewportRef = useCallback((node: HTMLDivElement | null) => {
    setViewportElement(node);
  }, []);

  useEffect(() => {
    if (!viewportElement) return;
    const updateMetrics = () => {
      setScrollTop(viewportElement.scrollTop);
      setViewportHeight(viewportElement.clientHeight);
    };
    updateMetrics();
    const resizeObserver = new ResizeObserver(updateMetrics);
    resizeObserver.observe(viewportElement);
    return () => resizeObserver.disconnect();
  }, [viewportElement]);

  const handleScroll = useCallback((viewport: HTMLDivElement) => {
    setScrollTop(viewport.scrollTop);
    setViewportHeight(viewport.clientHeight);
  }, []);

  const plainLines = useMemo(() => code.split("\n"), [code]);
  const lineCount = plainLines.length;
  const visibleLineCount = Math.max(
    1,
    Math.ceil(viewportHeight / FILE_CARD_CODE_LINE_HEIGHT),
  );
  const startLine = Math.max(
    0,
    Math.floor(scrollTop / FILE_CARD_CODE_LINE_HEIGHT) - FILE_CARD_CODE_OVERSCAN,
  );
  const endLine = Math.min(
    lineCount,
    startLine + visibleLineCount + FILE_CARD_CODE_OVERSCAN * 2,
  );
  const {
    highlightedChunks,
  } = useProgressiveFileCardHighlightedCode(
    plainLines,
    language,
    themeType,
    endLine,
  );
  const visibleRows = useMemo(
    () =>
      Array.from({ length: Math.max(0, endLine - startLine) }, (_, offset) => {
        const lineIndex = startLine + offset;
        const chunkIndex = Math.floor(lineIndex / FILE_CARD_HIGHLIGHT_CHUNK_LINES);
        const chunkLineIndex = lineIndex - chunkIndex * FILE_CARD_HIGHLIGHT_CHUNK_LINES;
        return {
          lineIndex,
          plainText: plainLines[lineIndex] ?? "",
          tokens: highlightedChunks.get(chunkIndex)?.[chunkLineIndex] ?? null,
        };
      }),
    [endLine, highlightedChunks, plainLines, startLine],
  );
  const totalHeight = lineCount * FILE_CARD_CODE_LINE_HEIGHT;
  const offsetTop = startLine * FILE_CARD_CODE_LINE_HEIGHT;

  return (
    <ScrollArea
      ref={handleViewportRef}
      className="min-h-0 flex-1 overflow-hidden rounded-md bg-ink/[0.055]"
      viewportClassName="p-2"
      orientation="both"
      interactionMode={interactionMode}
      scrollbarInset="flush"
      onScroll={handleScroll}
    >
      <div
        className="relative min-w-full"
        style={{
          height: `${Math.max(totalHeight, viewportHeight)}px`,
        }}
      >
        <div
          className="absolute left-0 top-0 min-w-full"
          style={{ transform: `translateY(${offsetTop}px)` }}
        >
          {visibleRows.map(({ lineIndex, plainText, tokens }) => {
            const content =
              tokens && tokens.length > 0
                ? renderShikiLine(tokens, lineIndex)
                : (plainText || "\u00a0");
            return (
              <div
                key={lineIndex}
                className="w-max min-w-full bg-transparent font-mono text-caption text-ink/80"
                style={{
                  height: `${FILE_CARD_CODE_LINE_HEIGHT}px`,
                  lineHeight: `${FILE_CARD_CODE_LINE_HEIGHT}px`,
                  whiteSpace: "pre",
                }}
              >
                {content}
              </div>
            );
          })}
        </div>
      </div>
    </ScrollArea>
  );
}

function useProgressiveFileCardHighlightedCode(
  plainLines: string[],
  language: string,
  themeType: "light" | "dark",
  priorityLine: number,
) {
  const lineCount = plainLines.length;
  const chunkCount = Math.max(
    1,
    Math.ceil(lineCount / FILE_CARD_HIGHLIGHT_CHUNK_LINES),
  );
  const lang = useMemo(() => shikiLanguage(language), [language]);
  const theme = useMemo(() => shikiTheme(themeType), [themeType]);
  const [highlightedChunks, setHighlightedChunks] = useState<
    Map<number, ShikiHighlightedLine[]>
  >(new Map());
  const priorityChunkRef = useRef(0);

  priorityChunkRef.current = Math.min(
    chunkCount - 1,
    Math.max(0, Math.floor(priorityLine / FILE_CARD_HIGHLIGHT_CHUNK_LINES)),
  );

  useEffect(() => {
    let cancelled = false;
    let frameId: number | null = null;
    let grammarState: GrammarState | undefined;
    let nextChunkIndex = 0;
    const queuedChunks = new Map<number, ShikiHighlightedLine[]>();

    const flushQueuedChunks = () => {
      if (!queuedChunks.size) return;
      const entries = Array.from(queuedChunks.entries());
      queuedChunks.clear();
      startTransition(() => {
        setHighlightedChunks((current) => {
          const next = new Map(current);
          for (const [chunkIndex, tokens] of entries) {
            next.set(chunkIndex, tokens);
          }
          return next;
        });
      });
    };

    startTransition(() => {
      setHighlightedChunks(new Map());
    });

    getShikiHighlighter()
      .then((highlighter) => {
        if (cancelled) return;

        const step = () => {
          if (cancelled) return;

          const priorityChunk = priorityChunkRef.current;
          const nearViewport = nextChunkIndex <= priorityChunk + 1;
          const chunkBurst = nearViewport
            ? FILE_CARD_HIGHLIGHT_NEAR_VIEWPORT_BURST
            : FILE_CARD_HIGHLIGHT_BACKGROUND_BURST;
          const frameBudget = nearViewport
            ? FILE_CARD_HIGHLIGHT_NEAR_VIEWPORT_BUDGET_MS
            : FILE_CARD_HIGHLIGHT_BACKGROUND_BUDGET_MS;
          const frameStart = performance.now();
          let processedChunks = 0;

          while (
            nextChunkIndex < chunkCount &&
            processedChunks < chunkBurst &&
            performance.now() - frameStart < frameBudget
          ) {
            const chunkStartLine = nextChunkIndex * FILE_CARD_HIGHLIGHT_CHUNK_LINES;
            const chunkEndLine = Math.min(
              lineCount,
              chunkStartLine + FILE_CARD_HIGHLIGHT_CHUNK_LINES,
            );
            const chunkCode = plainLines
              .slice(chunkStartLine, chunkEndLine)
              .join("\n");
            const tokens = highlighter.codeToTokensBase(chunkCode, {
              lang,
              theme,
              grammarState,
            });

            grammarState = highlighter.getLastGrammarState(tokens);
            queuedChunks.set(nextChunkIndex, tokens);
            nextChunkIndex += 1;
            processedChunks += 1;
          }

          flushQueuedChunks();

          if (nextChunkIndex < chunkCount) {
            frameId = window.requestAnimationFrame(step);
          }
        };

        step();
      })
      .catch((err) => {
        console.error("highlight file card chunk failed", err);
      });

    return () => {
      cancelled = true;
      if (frameId !== null) window.cancelAnimationFrame(frameId);
    };
  }, [chunkCount, lang, lineCount, plainLines, theme]);

  return {
    highlightedChunks,
  };
}
