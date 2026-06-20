import { useEffect, useMemo, useState } from "react";
import { openPath } from "@tauri-apps/plugin-opener";
import { PlainMarkdownPreviewContent } from "../../../../components/PlainMarkdownPreview";
import { useEffectiveThemeType } from "../../../../components/shikiHighlight";
import { readWorkspaceTextFile } from "../../../../api";

export interface MarkdownPreviewHostProps {
  workspacePath: string | null;
  blockId: string;
  title: string;
  sourcePath: string;
  excerpt: string;
  contentVersion: string;
  renderMode: "summary" | "preview";
  focused: boolean;
  onToggleRenderMode: (nextMode: "summary" | "preview") => void;
}

export function MarkdownPreviewHost({
  workspacePath,
  title,
  sourcePath,
  excerpt,
  contentVersion,
  renderMode,
  focused,
  onToggleRenderMode,
}: MarkdownPreviewHostProps) {
  const themeType = useEffectiveThemeType();
  const [content, setContent] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const shouldLoadPreview = focused && renderMode === "preview";

  useEffect(() => {
    if (!shouldLoadPreview) return;
    if (!workspacePath || !sourcePath) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    readWorkspaceTextFile(workspacePath, sourcePath)
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
  }, [contentVersion, shouldLoadPreview, sourcePath, workspacePath]);

  const summary = useMemo(() => excerpt.trim() || "Open preview to load markdown content.", [excerpt]);

  return (
    <div className="h-full w-full overflow-hidden rounded-[20px] border border-ink/10 bg-surface-panel/95 text-ink/80 shadow-[0_16px_40px_rgba(18,24,33,0.08)]">
      <div className="flex items-start justify-between gap-3 border-b border-ink/8 px-4 py-3">
        <div className="min-w-0">
          <div className="truncate text-body-sm font-medium text-ink/88">{title || "Markdown preview"}</div>
          <div className="truncate font-mono text-[11px] text-ink/48">{sourcePath}</div>
        </div>
        <button
          type="button"
          onClick={() => {
            void openPath(sourcePath).catch(() => {});
          }}
          className="shrink-0 rounded-md border border-ink/10 px-2 py-1 text-[11px] text-ink/62 transition hover:bg-ink/5"
        >
          Open
        </button>
        <button
          type="button"
          onClick={() => onToggleRenderMode(renderMode === "preview" ? "summary" : "preview")}
          className="shrink-0 rounded-md border border-ink/10 px-2 py-1 text-[11px] text-ink/62 transition hover:bg-ink/5"
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
        <div className="h-[calc(100%-57px)] overflow-auto px-4 py-3">
          {loading && <div className="text-caption text-ink/52">Loading markdown preview…</div>}
          {!loading && error && <div className="text-caption text-status-error">{error}</div>}
          {!loading && !error && content !== null && (
            <article className="markdown-content" data-theme-type={themeType}>
              <PlainMarkdownPreviewContent
                text={content}
                filePath={sourcePath}
                themeType={themeType}
              />
            </article>
          )}
        </div>
      )}
    </div>
  );
}
