import { CircleCheck, Download, RotateCcw, X } from "lucide-react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useI18n } from "../i18n";
import { formatVersionLabel } from "../updater";
import ScrollArea from "./ScrollArea";

type UpdateConfirmDialogProps = {
  open: boolean;
  currentVersion: string;
  latestVersion: string;
  releaseNotes: string | null;
  canInstall: boolean;
  installing: boolean;
  updateReady: boolean;
  downloadedBytes: number;
  totalBytes: number | null;
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
  updateReady,
  downloadedBytes,
  totalBytes,
  onCancel,
  onConfirm,
  onExited,
}: UpdateConfirmDialogProps) {
  const { t } = useI18n();
  const [hasOpened, setHasOpened] = useState(open);
  useEffect(() => {
    if (open) setHasOpened(true);
  }, [open]);
  const notes = (releaseNotes?.trim() ?? "").replace(/<\/?samp>/g, "`");
  const progress =
    totalBytes && totalBytes > 0
      ? Math.max(0, Math.min(100, Math.round((downloadedBytes / totalBytes) * 100)))
      : null;
  const confirmButtonClass =
    "inline-flex h-8 items-center gap-1.5 rounded-md bg-blue px-3 text-body-sm font-semibold text-white shadow-[0_6px_18px_rgb(var(--color-blue)/0.26)] outline-none transition duration-150 hover:bg-blue/92 hover:shadow-[0_9px_24px_rgb(var(--color-blue)/0.34)] focus-visible:ring-2 focus-visible:ring-blue/45 disabled:opacity-70 disabled:shadow-[0_6px_18px_rgb(var(--color-blue)/0.18)]";

  return createPortal(
    <div
      className={
        "update-confirm-dialog absolute inset-0 z-[80] flex items-center justify-center bg-black/42 px-4 backdrop-blur-md " +
        (open ? "update-confirm-dialog-in" : hasOpened ? "update-confirm-dialog-out" : "opacity-0")
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
          "update-confirm-panel w-full max-w-[560px] overflow-hidden rounded-lg border border-ink/10 bg-surface-panel shadow-[0_24px_80px_rgba(0,0,0,0.34)] " +
          (open ? "update-confirm-panel-in" : hasOpened ? "update-confirm-panel-out" : "opacity-0")
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
            {formatVersionLabel(currentVersion)} <span className="px-1.5 text-ink/42">-&gt;</span> {formatVersionLabel(latestVersion)}
          </div>
          <div className="mt-5 text-body-sm font-medium text-ink/82">
            {t("update_dialog.notes")}
          </div>
        </div>
        <ScrollArea
          className="update-release-notes-scroll mx-5 mb-4 mt-2"
          viewportClassName="update-release-notes-viewport pr-3"
        >
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
        {(installing || updateReady) && (
          <div className="mx-5 mb-4">
            {updateReady ? (
              <div className="flex items-center gap-2 rounded-md bg-emerald/14 px-3 py-2 text-body-sm font-medium text-emerald">
                <CircleCheck className="h-4 w-4 shrink-0" />
                <span>{t("update_dialog.ready")}</span>
              </div>
            ) : (
              <>
                <div className="mb-1.5 flex items-center justify-between gap-3 text-meta text-ink/55">
                  <span>{t("update_dialog.updating")}</span>
                  {progress !== null && <span className="tabular-nums">{progress}%</span>}
                </div>
                <div className="h-2 overflow-hidden rounded-full bg-ink/10">
                  <div
                    className={
                      "h-full rounded-full bg-blue shadow-[0_0_10px_rgb(var(--color-blue)/0.32)] " +
                      (progress === null ? "update-progress-indeterminate" : "transition-[width] duration-150")
                    }
                    style={progress === null ? undefined : { width: `${progress}%` }}
                  />
                </div>
              </>
            )}
          </div>
        )}
        <div className="flex items-center justify-end gap-2 border-t border-ink/10 bg-ink/[0.035] px-5 py-3">
          <button
            type="button"
            disabled={installing}
            onClick={onCancel}
            className="rounded-md px-3 py-1.5 text-body-sm font-medium text-ink/72 transition hover:bg-ink/5 hover:text-ink disabled:opacity-45"
          >
            {t(updateReady ? "update_dialog.later" : "update_dialog.cancel")}
          </button>
          <button
            type="button"
            disabled={installing}
            onClick={onConfirm}
            className={confirmButtonClass}
          >
            {updateReady ? (
              <RotateCcw className="h-4 w-4" />
            ) : (
              <Download className="h-4 w-4" />
            )}
            <span>
              {installing
                ? t("update_dialog.updating")
                : updateReady
                  ? t("update_dialog.restart_now")
                  : canInstall
                    ? t("update_dialog.update_now")
                    : t("update_dialog.download_now")}
            </span>
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
