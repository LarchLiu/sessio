import { useEffect, useRef, useState } from "react";
import { BLOCKSUITE_STYLE_SCOPE_CLASS } from "@blocksuite/std";
import { LoaderCircle } from "lucide-react";
import {
  createPageDocFromMarkdown,
  createPageEditor,
} from "./bootstrap";

export interface BlockSuiteDocPreviewProps {
  markdown: string;
  title?: string;
  emptyState?: string;
}

export default function BlockSuiteDocPreview({
  markdown,
  title,
  emptyState = "Structured preview is not available yet.",
}: BlockSuiteDocPreviewProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let cancelled = false;
    let editor: HTMLElement | null = null;
    let collectionDispose: (() => void) | null = null;

    const mount = async () => {
      setLoading(true);
      setError(null);
      try {
        const handle = await createPageDocFromMarkdown(markdown, title);
        if (cancelled) {
          handle.collection.dispose();
          return;
        }
        const nextEditor = createPageEditor(handle.doc) as HTMLElement;
        editor = nextEditor;
        collectionDispose = () => handle.collection.dispose();
        host.replaceChildren(nextEditor);
      } catch (nextError) {
        if (cancelled) return;
        setError(String(nextError));
        host.replaceChildren();
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    void mount();

    return () => {
      cancelled = true;
      editor?.remove();
      collectionDispose?.();
    };
  }, [markdown, title]);

  if (!markdown.trim() && !loading) {
    return (
      <div className="rounded-xl border border-ink/8 bg-ink/[0.03] px-3 py-2 text-caption text-ink/52">
        {emptyState}
      </div>
    );
  }

  return (
    <div className="overflow-hidden rounded-[20px] border border-ink/10 bg-surface-panel/95 shadow-[0_16px_40px_rgba(18,24,33,0.08)]">
      <div className="border-b border-ink/8 px-4 py-2.5 text-[11px] uppercase tracking-[0.08em] text-ink/42">
        Structured preview
      </div>
      {loading && (
        <div className="flex items-center gap-2 px-4 py-3 text-caption text-ink/52">
          <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
          Loading BlockSuite document…
        </div>
      )}
      {error && (
        <div className="px-4 py-3 text-caption text-status-error">
          {error}
        </div>
      )}
      <div
        ref={hostRef}
        className={loading || error ? `hidden ${BLOCKSUITE_STYLE_SCOPE_CLASS}` : `${BLOCKSUITE_STYLE_SCOPE_CLASS} h-[320px] overflow-hidden`}
      />
    </div>
  );
}
