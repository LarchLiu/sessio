import { useEffect, useMemo, useState } from "react";
import PlainMarkdownPreview from "../../../../components/PlainMarkdownPreview";
import { readWorkspaceTextFile } from "../../../../api";

function stopOverlayInteraction(event: {
  preventDefault: () => void;
  stopPropagation: () => void;
}) {
  event.preventDefault();
  event.stopPropagation();
}

export interface MarkdownPreviewHostProps {
  workspacePath: string | null;
  blockId: string;
  selected?: boolean;
  title: string;
  sourcePath: string;
  excerpt: string;
  contentVersion: string;
  renderMode: "summary" | "preview";
  onToggleRenderMode: (nextMode: "summary" | "preview") => void;
  onOpenFile?: (path: string) => void;
  onHeaderPointerDown?: (event: React.PointerEvent<HTMLDivElement>) => void;
  interactionMode?: "block" | "overlay";
}

export function MarkdownPreviewHost({
  workspacePath,
  selected = false,
  title,
  sourcePath,
  excerpt,
  contentVersion,
  renderMode,
  onToggleRenderMode,
  onOpenFile,
  onHeaderPointerDown,
  interactionMode = "block",
}: MarkdownPreviewHostProps) {
  const overlayRootClassName =
    interactionMode === "overlay" ? "pointer-events-none" : "";
  const overlayActionClassName =
    interactionMode === "overlay" ? "pointer-events-auto" : "";
  const overlayContentClassName =
    interactionMode === "overlay" ? "pointer-events-auto" : "";

  const [content, setContent] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const shouldLoadPreview = renderMode === "preview";
  const resolvedSourcePath = useMemo(
    () => resolveWorkspaceFilePath(sourcePath, workspacePath),
    [sourcePath, workspacePath],
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

  const summary = useMemo(
    () => excerpt.trim() || "No markdown summary available.",
    [excerpt],
  );

  return (
    <div className={"h-full w-full overflow-hidden rounded-[20px] border border-ink/10 bg-surface-panel/95 text-ink/80 shadow-[0_16px_40px_rgba(18,24,33,0.08)] " + overlayRootClassName}>
      <div
        onPointerDown={onHeaderPointerDown}
        className={"flex items-start justify-between gap-3 border-b border-ink/8 px-4 py-3 " + overlayContentClassName}
      >
        <div className="min-w-0">
          <div className="truncate text-body-sm font-medium text-ink/88">{title || "Markdown preview"}</div>
          <div className="truncate font-mono text-[11px] text-ink/48">{sourcePath}</div>
        </div>
        <button
          type="button"
          onPointerDown={stopOverlayInteraction}
          onMouseDown={stopOverlayInteraction}
          onClick={(event) => {
            stopOverlayInteraction(event);
            if (!resolvedSourcePath) {
              setError("Markdown source path is unavailable.");
              return;
            }
            onOpenFile?.(resolvedSourcePath);
          }}
          className={"shrink-0 rounded-md border border-ink/10 px-2 py-1 text-[11px] text-ink/62 transition hover:bg-ink/5 " + overlayActionClassName}
        >
          Open
        </button>
        <button
          type="button"
          onPointerDown={stopOverlayInteraction}
          onMouseDown={stopOverlayInteraction}
          onClick={(event) => {
            stopOverlayInteraction(event);
            onToggleRenderMode(renderMode === "preview" ? "summary" : "preview");
          }}
          className={"shrink-0 rounded-md border border-ink/10 px-2 py-1 text-[11px] text-ink/62 transition hover:bg-ink/5 " + overlayActionClassName}
        >
          {renderMode === "preview" ? "Summary" : "Preview"}
        </button>
      </div>
      {!shouldLoadPreview && (
        <div className="px-4 py-3 text-caption leading-6 text-ink/62">
          {summary}
        </div>
      )}
      {shouldLoadPreview && (
        <div
          className={
            "h-[calc(100%-57px)] overflow-auto overscroll-contain px-4 py-3 " +
            overlayContentClassName
          }
        >
          {loading && <div className="text-caption text-ink/52">Loading markdown preview…</div>}
          {!loading && error && <div className="text-caption text-status-error">{error}</div>}
          {!loading && !error && content !== null && (
            <PlainMarkdownPreview
              text={content}
              filePath={resolvedSourcePath}
              interactionMode={
                interactionMode !== "overlay"
                  ? "default"
                  : selected
                    ? "capture-wheel"
                    : "thumbs-only"
              }
            />
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
