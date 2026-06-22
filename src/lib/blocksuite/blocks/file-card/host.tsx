function stopOverlayInteraction(event: {
  preventDefault: () => void;
  stopPropagation: () => void;
}) {
  event.preventDefault();
  event.stopPropagation();
}

export interface FileCardHostProps {
  title: string;
  sourcePath: string;
  sourceType: string;
  subtitle: string;
  summary: string;
  status: string;
  onPromoteToMarkdown: () => void;
  onOpenFile?: (path: string) => void;
  interactionMode?: "block" | "overlay";
}

export function FileCardHost({
  title,
  sourcePath,
  sourceType,
  subtitle,
  summary,
  status,
  onPromoteToMarkdown,
  onOpenFile,
  interactionMode = "block",
}: FileCardHostProps) {
  const overlayRootClassName =
    interactionMode === "overlay" ? "pointer-events-none" : "";
  const overlayActionClassName =
    interactionMode === "overlay" ? "pointer-events-auto" : "";

  return (
    <div className={"h-full w-full overflow-hidden rounded-[20px] border border-ink/10 bg-surface-panel/95 text-ink/80 shadow-[0_16px_40px_rgba(18,24,33,0.08)] " + overlayRootClassName}>
      <div className="flex items-start justify-between gap-3 border-b border-ink/8 px-4 py-3">
        <div className="min-w-0">
          <div className="truncate text-body-sm font-medium text-ink/88">{title || "File card"}</div>
          <div className="truncate font-mono text-[11px] text-ink/48">{subtitle || sourcePath}</div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            onPointerDown={stopOverlayInteraction}
            onMouseDown={stopOverlayInteraction}
            onClick={(event) => {
              stopOverlayInteraction(event);
              onOpenFile?.(sourcePath);
            }}
            className={"rounded-md border border-ink/10 px-2 py-1 text-[11px] text-ink/62 transition hover:bg-ink/5 " + overlayActionClassName}
          >
            Open
          </button>
          <button
            type="button"
            onPointerDown={stopOverlayInteraction}
            onMouseDown={stopOverlayInteraction}
            onClick={(event) => {
              stopOverlayInteraction(event);
              onPromoteToMarkdown();
            }}
            className={"rounded-md border border-ink/10 px-2 py-1 text-[11px] text-ink/62 transition hover:bg-ink/5 " + overlayActionClassName}
          >
            Preview
          </button>
        </div>
      </div>
      <div className="flex h-[calc(100%-57px)] flex-col gap-3 px-4 py-3">
        <div className="flex items-center gap-2 text-[11px] uppercase tracking-[0.08em] text-ink/40">
          <span>{sourceType.replaceAll("_", " ")}</span>
          <span className="h-1 w-1 rounded-full bg-ink/18" />
          <span>{status || "idle"}</span>
        </div>
        <div className="line-clamp-4 text-caption leading-6 text-ink/64">
          {summary.trim() || "Use this card to organize a workspace file, then promote it into a markdown preview when you need the full body."}
        </div>
      </div>
    </div>
  );
}
