import { createPortal } from "react-dom";
import { useI18n } from "../i18n";

export default function ConfirmPopover({
  title,
  body,
  pos,
  onCancel,
  onConfirm,
}: {
  title: string;
  body: string;
  pos: { x: number; y: number };
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useI18n();
  const POPOVER_W = 288;
  const POPOVER_H = 118;
  const GAP = 6;
  const MARGIN = 8;
  const RAISE = 8;
  const left = Math.min(
    Math.max(pos.x - POPOVER_W / 2, MARGIN),
    window.innerWidth - POPOVER_W - MARGIN,
  );
  const top =
    pos.y >= POPOVER_H + GAP + MARGIN
      ? Math.max(MARGIN, pos.y - POPOVER_H - GAP - RAISE)
      : Math.min(
          window.innerHeight - POPOVER_H - MARGIN,
          Math.max(MARGIN, pos.y + GAP - RAISE),
        );

  return createPortal(
    <div className="fixed inset-0 z-[70]" onClick={onCancel}>
      <div
        className="fixed w-72 rounded-md border border-ink/10 bg-surface px-3 py-3 shadow-[0_12px_30px_rgba(0,0,0,0.18)]"
        style={{ left, top }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="text-body-sm font-medium text-ink">{title}</div>
        <div className="mt-1.5 text-body-sm text-ink/60 leading-snug">{body}</div>
        <div className="mt-3 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="px-2.5 py-1 rounded-md text-body-sm text-ink/70 hover:text-ink hover:bg-ink/5 transition"
          >
            {t("delete.cancel")}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            className="px-2.5 py-1 rounded-md text-body-sm text-white bg-status-error hover:bg-status-error/90 transition"
          >
            {t("delete.confirm")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
