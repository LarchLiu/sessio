import {
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import type { Placement } from "./Tooltip";
import { useI18n } from "../i18n";

const VIEWPORT_MARGIN = 8;

export interface ConfirmTooltipOptions {
  title: ReactNode;
  body?: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  placement?: Placement;
  onConfirm: () => void | Promise<void>;
}

export type ConfirmTooltipTrigger = (
  event: React.MouseEvent<HTMLElement>,
  options: ConfirmTooltipOptions,
) => void;

export default function ConfirmTooltip({
  children,
  offset = 8,
}: {
  children: (confirm: ConfirmTooltipTrigger) => ReactNode;
  offset?: number;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const [options, setOptions] = useState<ConfirmTooltipOptions | null>(null);
  const anchorRef = useRef<HTMLElement | null>(null);
  const tipRef = useRef<HTMLDivElement>(null);

  const updatePosition = useCallback(() => {
    const anchor = anchorRef.current;
    const tip = tipRef.current;
    if (!anchor || !tip || !options) return;
    const ar = anchor.getBoundingClientRect();
    const tw = tip.offsetWidth;
    const th = tip.offsetHeight;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const placement = options.placement ?? "top";

    let top = 0;
    let left = 0;
    switch (placement) {
      case "top":
        top = ar.top - th - offset;
        left = ar.left + ar.width / 2 - tw / 2;
        break;
      case "bottom":
        top = ar.bottom + offset;
        left = ar.left + ar.width / 2 - tw / 2;
        break;
      case "left":
        top = ar.top + ar.height / 2 - th / 2;
        left = ar.left - tw - offset;
        break;
      case "right":
        top = ar.top + ar.height / 2 - th / 2;
        left = ar.right + offset;
        break;
    }

    if (left < VIEWPORT_MARGIN) left = VIEWPORT_MARGIN;
    if (left + tw > vw - VIEWPORT_MARGIN) left = vw - VIEWPORT_MARGIN - tw;
    if (top < VIEWPORT_MARGIN) top = VIEWPORT_MARGIN;
    if (top + th > vh - VIEWPORT_MARGIN) top = vh - VIEWPORT_MARGIN - th;
    setPos({ top, left });
  }, [offset, options]);

  useLayoutEffect(() => {
    if (open) updatePosition();
  }, [open, options, updatePosition]);

  useEffect(() => {
    if (!open) return;
    const update = () => updatePosition();
    const onMouseDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (anchorRef.current?.contains(target) || tipRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("scroll", update, true);
    window.addEventListener("resize", update);
    window.addEventListener("mousedown", onMouseDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("scroll", update, true);
      window.removeEventListener("resize", update);
      window.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open, updatePosition]);

  const requestConfirm: ConfirmTooltipTrigger = (event, nextOptions) => {
    event.preventDefault();
    event.stopPropagation();
    anchorRef.current = event.currentTarget;
    setOptions(nextOptions);
    setPos(null);
    setOpen(true);
  };

  const confirm = async () => {
    if (!options) return;
    setConfirming(true);
    try {
      await options.onConfirm();
      setOpen(false);
    } finally {
      setConfirming(false);
    }
  };

  return (
    <>
      {children(requestConfirm)}
      {open &&
        options &&
        createPortal(
          <div
            ref={tipRef}
            style={{
              position: "fixed",
              top: pos?.top ?? -9999,
              left: pos?.left ?? -9999,
              visibility: pos ? "visible" : "hidden",
            }}
            className="z-50 w-[260px] max-w-[calc(100vw-16px)] rounded-md border border-ink/10 bg-tooltip-bg px-2.5 py-2 text-tooltip-fg shadow-lg"
          >
            <div className="text-body-sm font-medium leading-snug">{options.title}</div>
            {options.body && <div className="mt-1 text-caption leading-snug text-tooltip-fg/70">{options.body}</div>}
            <div className="mt-2 flex items-center justify-end gap-1.5">
              <button
                type="button"
                onClick={() => setOpen(false)}
                className="rounded px-2 py-1 text-caption text-tooltip-fg/65 transition hover:bg-white/10 hover:text-tooltip-fg"
              >
                {options.cancelLabel ?? t("delete.cancel")}
              </button>
              <button
                type="button"
                disabled={confirming}
                onClick={() => void confirm()}
                className="rounded bg-tooltip-fg px-2 py-1 text-caption font-medium text-tooltip-bg transition hover:opacity-90 disabled:opacity-50"
              >
                {options.confirmLabel ?? t("confirm.ok")}
              </button>
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}
