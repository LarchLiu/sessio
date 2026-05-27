import { ReactNode, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

export type PopupMenuPlacement =
  | "top"
  | "top-start"
  | "top-end"
  | "bottom"
  | "bottom-start"
  | "bottom-end"
  | "left"
  | "right";

export interface PopupMenuOption<T extends string = string> {
  key: T;
  label: string;
  icon?: ReactNode;
  disabled?: boolean;
}

export default function PopupMenu<T extends string = string>({
  anchor,
  options,
  placement = "top",
  onSelect,
  onClose,
  className = "",
}: {
  anchor: HTMLElement;
  options: PopupMenuOption<T>[];
  placement?: PopupMenuPlacement;
  onSelect: (key: T) => void;
  onClose: () => void;
  className?: string;
}) {
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const updatePosition = useCallback(() => {
    const rect = anchor.getBoundingClientRect();
    const menuWidth = menuRef.current?.offsetWidth ?? 192;
    const menuHeight = menuRef.current?.offsetHeight ?? 8 + options.length * 40;
    const gap = 10;
    const margin = 8;
    let top = rect.bottom + gap;
    let left = rect.left + rect.width / 2 - menuWidth / 2;

    if (placement.startsWith("top")) top = rect.top - menuHeight - gap;
    if (placement.startsWith("bottom")) top = rect.bottom + gap;
    if (placement === "left") {
      top = rect.top + rect.height / 2 - menuHeight / 2;
      left = rect.left - menuWidth - gap;
    }
    if (placement === "right") {
      top = rect.top + rect.height / 2 - menuHeight / 2;
      left = rect.right + gap;
    }
    if (placement.endsWith("-start")) left = rect.left;
    if (placement.endsWith("-end")) left = rect.right - menuWidth;

    top = Math.round(Math.max(margin, Math.min(top, window.innerHeight - menuHeight - margin)));
    left = Math.round(Math.max(margin, Math.min(left, window.innerWidth - menuWidth - margin)));
    setPos({ top, left });
  }, [anchor, options.length, placement]);

  useLayoutEffect(() => {
    updatePosition();
  }, [updatePosition]);

  useEffect(() => {
    const reposition = () => updatePosition();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("scroll", reposition, true);
    window.addEventListener("resize", reposition);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("scroll", reposition, true);
      window.removeEventListener("resize", reposition);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [onClose, updatePosition]);

  return createPortal(
    <>
      <div className="fixed inset-0 z-[39] bg-transparent" onMouseDown={onClose} />
      <div
        ref={menuRef}
        className={
          "fixed z-40 min-w-[192px] rounded-xl border border-ink/10 bg-surface-panel p-1.5 shadow-[0_20px_60px_rgba(0,0,0,0.22)] " +
          className
        }
        style={{
          top: pos?.top ?? -9999,
          left: pos?.left ?? -9999,
          visibility: pos ? "visible" : "hidden",
        }}
        role="menu"
      >
        {options.map((option) => (
          <button
            key={option.key}
            type="button"
            disabled={option.disabled}
            className="flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-body-sm text-ink/72 transition hover:bg-ink/[0.08] hover:text-ink disabled:cursor-not-allowed disabled:opacity-45"
            role="menuitem"
            onClick={() => {
              if (option.disabled) return;
              onSelect(option.key);
              onClose();
            }}
          >
            {option.icon && <span className="shrink-0 text-ink/55">{option.icon}</span>}
            <span>{option.label}</span>
          </button>
        ))}
      </div>
    </>,
    document.body,
  );
}
