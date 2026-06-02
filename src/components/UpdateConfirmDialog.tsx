import { Download, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useI18n } from "../i18n";
import ScrollArea from "./ScrollArea";

type UpdateConfirmDialogProps = {
  open: boolean;
  currentVersion: string;
  latestVersion: string;
  releaseNotes: string | null;
  canInstall: boolean;
  installing: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  onExited: () => void;
};

export default function UpdateConfirmDialog({
  open,
  currentVersion,
  latestVersion,
  releaseNotes,
  canInstall,
  installing,
  onCancel,
  onConfirm,
  onExited,
}: UpdateConfirmDialogProps) {
  const { t } = useI18n();
  const confirmRef = useRef<HTMLButtonElement>(null);
  const notes = releaseNotes?.trim() ?? "";

  useEffect(() => {
    if (!open) return;
    const id = requestAnimationFrame(() => confirmRef.current?.focus());
    return () => cancelAnimationFrame(id);
  }, [open]);

  return createPortal(
    <div
      className={
          "update-confirm-dialog absolute inset-0 z-[80] flex items-center justify-center bg-black/42 px-4 backdrop-blur-md " +
        (open ? "update-confirm-dialog-in" : "update-confirm-dialog-out")
      }
      onClick={installing ? undefined : onCancel}
      onAnimationEnd={(event) => {
        if (!open && event.currentTarget === event.target) onExited();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="update-confirm-title"
        className={
          "update-confirm-panel w-full max-w-[360px] overflow-hidden rounded-lg border border-ink/10 bg-surface-panel shadow-[0_24px_80px_rgba(0,0,0,0.34)] " +
          (open ? "update-confirm-panel-in" : "update-confirm-panel-out")
        }
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape" && !installing) onCancel();
        }}
      >
        <div className="px-5 pt-5">
          <div className="mb-1 flex items-center justify-between gap-3">
            <h2 id="update-confirm-title" className="text-body font-semibold text-ink">
              {t("update_dialog.title")}
            </h2>
            <button
              type="button"
              aria-label={t("detail.close")}
              disabled={installing}
              onClick={onCancel}
              className="rounded-md p-1 text-ink/38 transition hover:bg-ink/5 hover:text-ink disabled:opacity-35"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
          <div className="text-body-sm text-ink/58">
            {t("update_dialog.subtitle")}
          </div>
          <div className="mt-4 text-body-sm tabular-nums text-ink/78">
            {currentVersion} <span className="px-1.5 text-ink/42">-&gt;</span> {latestVersion}
          </div>
          <div className="mt-5 text-body-sm font-medium text-ink/82">
            {t("update_dialog.notes")}
          </div>
        </div>
        <ScrollArea className="mx-5 mb-4 mt-2 max-h-[min(44vh,360px)]" viewportClassName="pr-3">
          {notes ? (
            <div className="update-release-notes text-body-sm leading-relaxed text-ink/64">
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                components={{
                  a: ({ children, href }) => (
                    <a
                      href={href}
                      target="_blank"
                      rel="noreferrer"
                      className="text-blue hover:underline"
                    >
                      {children}
                    </a>
                  ),
                  h1: ({ children }) => (
                    <div className="mb-2 mt-0 text-title font-semibold text-ink">
                      {children}
                    </div>
                  ),
                  h2: ({ children }) => (
                    <div className="mb-2 mt-4 text-body font-semibold text-ink/90 first:mt-0">
                      {children}
                    </div>
                  ),
                  h3: ({ children }) => (
                    <div className="mb-1.5 mt-3 text-body-sm font-semibold text-ink/82 first:mt-0">
                      {children}
                    </div>
                  ),
                  code: ({ children }) => (
                    <code className="rounded bg-ink/7 px-1 py-0.5 font-mono text-[12px] text-ink/76">
                      {children}
                    </code>
                  ),
                  pre: ({ children }) => (
                    <pre className="my-2 overflow-x-auto rounded-md bg-ink/7 p-2 font-mono text-[12px] text-ink/76">
                      {children}
                    </pre>
                  ),
                }}
              >
                {notes}
              </ReactMarkdown>
            </div>
          ) : (
            <div className="text-body-sm text-ink/45">
              {t("update_dialog.no_notes")}
            </div>
          )}
        </ScrollArea>
        <div className="flex items-center justify-end gap-2 border-t border-ink/10 bg-ink/[0.035] px-5 py-3">
          <button
            type="button"
            disabled={installing}
            onClick={onCancel}
            className="rounded-md px-3 py-1.5 text-body-sm font-medium text-ink/72 transition hover:bg-ink/5 hover:text-ink disabled:opacity-45"
          >
            {t("update_dialog.cancel")}
          </button>
          <button
            ref={confirmRef}
            type="button"
            disabled={installing}
            onClick={onConfirm}
            className="inline-flex h-8 items-center gap-1.5 rounded-md bg-blue px-3 text-body-sm font-semibold text-white shadow-[0_6px_18px_rgba(66,133,244,0.26)] transition hover:bg-blue/92 disabled:opacity-70"
          >
            <Download className="h-4 w-4" />
            <span>
              {installing
                ? t("sidebar.update_installing")
                : canInstall
                  ? t("update_dialog.install_now")
                  : t("update_dialog.download_now")}
            </span>
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
