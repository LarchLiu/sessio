import { ChevronRight } from "lucide-react";
import { ReactNode, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
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
  kind?: "item" | "label";
  children?: PopupMenuOption<T>[];
}

type PopupMenuPosition = {
  top: number;
  left: number;
};

export default function PopupMenu<T extends string = string>({
  anchor,
  options,
  placement = "top",
  onSelect,
  onClose,
  className = "",
  overlayClassName = "",
}: {
  anchor: HTMLElement;
  options: PopupMenuOption<T>[];
  placement?: PopupMenuPlacement;
  onSelect: (key: T) => boolean | void;
  onClose: () => void;
  className?: string;
  overlayClassName?: string;
}) {
  const [pos, setPos] = useState<PopupMenuPosition | null>(null);
  const [submenuKey, setSubmenuKey] = useState<T | null>(null);
  const [submenuPos, setSubmenuPos] = useState<PopupMenuPosition | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const submenuRef = useRef<HTMLDivElement>(null);
  const optionRefs = useRef(new Map<string, HTMLButtonElement>());
  const activeSubmenuOption = useMemo(
    () =>
      submenuKey
        ? options.find((option) => option.key === submenuKey && (option.children?.length ?? 0) > 0) ?? null
        : null,
    [options, submenuKey],
  );

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
  }, [anchor, options, placement]);

  const updateSubmenuPosition = useCallback(() => {
    const option = activeSubmenuOption;
    if (!option) {
      setSubmenuPos(null);
      return;
    }
    const anchorNode = optionRefs.current.get(option.key);
    if (!anchorNode) {
      setSubmenuPos(null);
      return;
    }
    const rect = anchorNode.getBoundingClientRect();
    const submenuWidth = submenuRef.current?.offsetWidth ?? 224;
    const submenuHeight = submenuRef.current?.offsetHeight ?? 8 + (option.children?.length ?? 0) * 40;
    const gap = 6;
    const margin = 8;
    let top = rect.top - 4;
    let left = rect.right + gap;

    if (left + submenuWidth > window.innerWidth - margin) {
      left = rect.left - submenuWidth - gap;
    }

    top = Math.round(Math.max(margin, Math.min(top, window.innerHeight - submenuHeight - margin)));
    left = Math.round(Math.max(margin, Math.min(left, window.innerWidth - submenuWidth - margin)));
    setSubmenuPos({ top, left });
  }, [activeSubmenuOption]);

  useLayoutEffect(() => {
    updatePosition();
  }, [updatePosition]);

  useLayoutEffect(() => {
    updateSubmenuPosition();
  }, [updateSubmenuPosition]);

  useEffect(() => {
    const reposition = () => {
      updatePosition();
      updateSubmenuPosition();
    };
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
  }, [onClose, updatePosition, updateSubmenuPosition]);

  useEffect(() => {
    if (activeSubmenuOption) return;
    setSubmenuPos(null);
  }, [activeSubmenuOption]);

  useEffect(() => {
    if (
      submenuKey &&
      !options.some((option) => option.key === submenuKey && (option.children?.length ?? 0) > 0)
    ) {
      setSubmenuKey(null);
    }
  }, [options, submenuKey]);

  const select = (key: T, disabled?: boolean) => {
    if (disabled) return;
    const shouldClose = onSelect(key);
    if (shouldClose !== false) {
      onClose();
    }
  };

  const renderOption = (
    option: PopupMenuOption<T>,
    inSubmenu = false,
  ) => {
    if (option.kind === "label") {
      return (
        <div
          key={option.key}
          className="px-3 pt-2 pb-1 text-[11px] font-medium uppercase tracking-[0.12em] text-ink/32"
        >
          {option.label}
        </div>
      );
    }

    const hasChildren = !inSubmenu && (option.children?.length ?? 0) > 0;
    const isSubmenuOpen = activeSubmenuOption?.key === option.key;
    return (
      <button
        key={option.key}
        ref={(node) => {
          if (node) {
            optionRefs.current.set(option.key, node);
          } else {
            optionRefs.current.delete(option.key);
          }
        }}
        type="button"
        disabled={option.disabled}
        className={
          "flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-body-sm text-ink/72 transition hover:bg-ink/[0.08] hover:text-ink disabled:cursor-not-allowed disabled:opacity-45 " +
          (isSubmenuOpen ? "bg-ink/[0.08] text-ink" : "")
        }
        role="menuitem"
        onMouseEnter={() => {
          if (hasChildren) {
            setSubmenuKey(option.key);
            return;
          }
          if (!inSubmenu) setSubmenuKey(null);
        }}
        onFocus={() => {
          if (hasChildren) {
            setSubmenuKey(option.key);
            return;
          }
          if (!inSubmenu) setSubmenuKey(null);
        }}
        onClick={() => {
          if (hasChildren) {
            setSubmenuKey(option.key);
            return;
          }
          select(option.key, option.disabled);
        }}
      >
        {option.icon && <span className="shrink-0 text-ink/55">{option.icon}</span>}
        <span className="min-w-0 flex-1 truncate">{option.label}</span>
        {hasChildren && <ChevronRight className="h-4 w-4 shrink-0 text-ink/38" />}
      </button>
    );
  };

  return createPortal(
    <>
      <div
        className={`fixed inset-0 z-[39] bg-transparent ${overlayClassName}`.trim()}
        onMouseDown={onClose}
      />
      <div
        ref={menuRef}
        className={`fixed z-40 min-w-[192px] rounded-xl border border-ink/10 bg-surface-panel p-1.5 shadow-[0_20px_60px_rgba(0,0,0,0.22)] ${className}`.trim()}
        style={{
          top: pos?.top ?? -9999,
          left: pos?.left ?? -9999,
          visibility: pos ? "visible" : "hidden",
        }}
        role="menu"
      >
        {options.map((option) => renderOption(option))}
      </div>
      {activeSubmenuOption && activeSubmenuOption.children && (
        <div
          ref={submenuRef}
          className="fixed z-[41] min-w-[224px] rounded-xl border border-ink/10 bg-surface-panel p-1.5 shadow-[0_20px_60px_rgba(0,0,0,0.22)]"
          style={{
            top: submenuPos?.top ?? -9999,
            left: submenuPos?.left ?? -9999,
            visibility: submenuPos ? "visible" : "hidden",
          }}
          role="menu"
        >
          {activeSubmenuOption.children.map((option) => renderOption(option, true))}
        </div>
      )}
    </>,
    document.body,
  );
}
