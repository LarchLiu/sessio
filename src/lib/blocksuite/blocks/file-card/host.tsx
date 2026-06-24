import { useEffect, useMemo, useState } from "react";
import { ArrowUpRight, ChevronDown } from "lucide-react";
import PlainMarkdownPreview from "../../../../components/PlainMarkdownPreview";
import FileViewer from "../../../../components/FileViewer";
import { readWorkspaceTextFile } from "../../../../api";
import { isPlainEditorMarkdownDocumentPath } from "../../../../hooks/plainEditorFileTypes";
import { languageFromPath } from "../../../../hooks/useFileContent";

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
  onTogglePreviewCollapsed: (nextCollapsed: boolean) => void;
  onOpenFile?: (path: string) => void;
  onHeaderPointerDown?: (event: React.PointerEvent<HTMLDivElement>) => void;
  interactionMode?: "block" | "overlay";
}

export function FileCardHost({
  workspacePath,
  blockId,
  selected = false,
  title,
  sourcePath,
  subtitle,
  contentVersion,
  previewCollapsed,
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
    <div className={"h-full w-full overflow-hidden rounded-[20px] border border-ink/10 bg-surface-panel/95 text-ink/80 shadow-[0_16px_40px_rgba(18,24,33,0.08)] " + overlayRootClassName}>
      <div
        onPointerDown={onHeaderPointerDown}
        className={
          "flex items-center justify-between gap-3 px-4 py-3 " +
          (!previewCollapsed ? "border-b border-ink/8 " : "") +
          overlayContentClassName
        }
      >
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <div className="min-w-0 flex-1">
            <div className="truncate text-body-sm font-medium text-ink/88">{title || "File card"}</div>
            {!previewCollapsed && (
              <div className="truncate font-mono text-[11px] text-ink/48">{subtitle || sourcePath}</div>
            )}
          </div>
        </div>
        <div className={"flex shrink-0 items-center gap-1 self-start " + overlayActionClassName}>
          <button
            type="button"
            onPointerDown={stopOverlayInteraction}
            onMouseDown={stopOverlayInteraction}
            onClick={(event) => {
              stopOverlayInteraction(event);
              if (!resolvedSourcePath) return;
              onOpenFile?.(resolvedSourcePath);
            }}
            className="inline-flex h-6 w-6 items-center justify-center rounded-md text-ink/45 transition hover:bg-ink/[0.05] hover:text-ink/80"
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
            className="inline-flex h-6 w-6 items-center justify-center rounded-md text-ink/45 transition hover:bg-ink/[0.05] hover:text-ink/80"
            aria-label={previewCollapsed ? "Open preview" : "Collapse preview"}
            aria-expanded={!previewCollapsed}
          >
            <ChevronDown
              className={
                "h-3.5 w-3.5 transition-transform " +
                (previewCollapsed ? "-rotate-90" : "")
              }
            />
          </button>
        </div>
      </div>
      {!previewCollapsed && (
        <div
          className={
            "flex h-[calc(100%-57px)] min-h-0 overflow-hidden overscroll-contain p-2 " +
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
              <FileViewer
                fileKey={`${blockId}:${contentVersion}`}
                text={content}
                language={previewLanguage}
                mode="code"
                workspacePath={workspacePath}
                path={resolvedSourcePath}
                mtimeMs={null}
                contentVersion={contentVersion}
                savedScrollTop={0}
                codePadding="12px 6px"
                codeGutterMarginRight="0"
                codeLineNumberPaddingRight="0"
                codeShowLineNumbers={false}
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
