import {
  ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown } from "lucide-react";
import ScrollArea from "./ScrollArea";

const MENU_GAP = 6;
const MENU_MARGIN = 8;
const MENU_MAX_HEIGHT = 260;

type MenuPlacement = "auto" | "bottom" | "top" | "left" | "right";
type MenuPosition = { top: number; left: number; width: number };

export interface InlineMenuSelectOption {
  value: string;
  label: string;
  icon?: ReactNode;
  disabled?: boolean;
}

export interface InlineMenuSelectProps {
  value: string;
  options: InlineMenuSelectOption[];
  onChange: (value: string) => void;
  menuAlign?: "trigger" | "parent";
  menuPlacement?: MenuPlacement;
  placeholder?: string;
  ariaLabel?: string;
  className?: string;
  menuClassName?: string;
  minMenuWidth?: number;
  emptyContent?: ReactNode;
}

export default function InlineMenuSelect({
  value,
  options,
  onChange,
  menuAlign = "trigger",
  menuPlacement = "auto",
  placeholder,
  ariaLabel,
  className = "",
  menuClassName = "",
  minMenuWidth = 180,
  emptyContent,
}: InlineMenuSelectProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<MenuPosition | null>(null);
  const anchorRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const selectedItemRef = useRef<HTMLButtonElement>(null);
  const selected = options.find((option) => option.value === value);

  const applyPosition = useCallback((next: MenuPosition) => {
    setPos((current) =>
      current &&
      current.top === next.top &&
      current.left === next.left &&
      current.width === next.width
        ? current
        : next,
    );
  }, []);

  const updatePosition = useCallback(() => {
    if (!open) return;
    const anchor = anchorRef.current;
    if (!anchor) return;
    const rect = anchor.getBoundingClientRect();
    const alignRect =
      menuAlign === "parent"
        ? anchor.parentElement?.getBoundingClientRect() ?? rect
        : rect;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const viewportMaxWidth = Math.max(120, vw - MENU_MARGIN * 2);
    const width = Math.round(
      Math.min(
        Math.max(rect.width, minMenuWidth),
        viewportMaxWidth,
      ),
    );
    const maxLeft = Math.max(MENU_MARGIN, vw - width - MENU_MARGIN);
    const measuredMenuHeight = menuRef.current?.offsetHeight ?? 0;
    const estimatedMenuHeight = options.length > 0 ? options.length * 32 + 8 : 40;
    const menuHeight = Math.min(
      MENU_MAX_HEIGHT,
      Math.max(32, measuredMenuHeight || estimatedMenuHeight),
    );
    const maxTop = Math.max(MENU_MARGIN, vh - menuHeight - MENU_MARGIN);
    const roomBelow = vh - rect.bottom - MENU_MARGIN;
    const resolvedPlacement =
      menuPlacement === "auto"
        ? roomBelow >= menuHeight
          ? "bottom"
          : "top"
        : menuPlacement;

    let left = alignRect.left;
    let top = rect.bottom + MENU_GAP;

    if (resolvedPlacement === "top") {
      top = rect.top - menuHeight - MENU_GAP;
    } else if (resolvedPlacement === "left") {
      left = rect.left - width - MENU_GAP;
      top = alignRect.top;
    } else if (resolvedPlacement === "right") {
      left = rect.right + MENU_GAP;
      top = alignRect.top;
    }

    applyPosition({
      top: Math.round(Math.max(MENU_MARGIN, Math.min(top, maxTop))),
      left: Math.round(Math.max(MENU_MARGIN, Math.min(left, maxLeft))),
      width,
    });
  }, [applyPosition, open, options.length, minMenuWidth, menuAlign, menuPlacement]);

  useLayoutEffect(() => {
    updatePosition();
  }, [updatePosition]);

  useLayoutEffect(() => {
    if (!open || !pos) return;
    updatePosition();
  }, [open, pos, updatePosition]);

  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    const onMouseDown = (e: MouseEvent) => {
      const target = e.target as Node | null;
      if (anchorRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      close();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("mousedown", onMouseDown);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open, updatePosition]);

  useLayoutEffect(() => {
    if (!open || !pos) return;
    selectedItemRef.current?.scrollIntoView({ block: "center" });
  }, [open, pos, value]);

  const select = (nextValue: string) => {
    onChange(nextValue);
    setOpen(false);
  };

  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        aria-label={ariaLabel}
        onClick={() => setOpen((v) => !v)}
        className={
          "inline-flex h-7 max-w-[128px] shrink-0 items-center gap-1 border-r border-ink/10 pr-2 text-body-sm text-ink/70 outline-none hover:text-ink transition " +
          className
        }
      >
        {selected?.icon && <span className="shrink-0">{selected.icon}</span>}
        <span className="min-w-0 flex-1 truncate">{selected?.label ?? placeholder ?? ""}</span>
        <ChevronDown className="w-3.5 h-3.5 shrink-0" />
      </button>
      {open &&
        pos &&
        createPortal(
          <div
            className="fixed inset-0 z-[79] bg-transparent"
            onMouseDown={() => setOpen(false)}
          />,
          document.body,
        )}
      {open &&
        pos &&
        createPortal(
          <div
            ref={menuRef}
            onWheel={(e) => e.stopPropagation()}
            onScroll={(e) => e.stopPropagation()}
            className={
              "fixed z-[80] rounded-md border border-ink/10 bg-surface-panel shadow-[0_20px_60px_rgba(0,0,0,0.22)] overflow-hidden " +
              menuClassName
            }
            style={{
              top: pos.top,
              left: pos.left,
              width: pos.width,
              maxHeight: MENU_MAX_HEIGHT,
            }}
          >
            <ScrollArea
              className="max-h-[260px] overscroll-contain"
              viewportClassName="py-1"
            >
              {options.length > 0 ? (
                options.map((option) => (
                  <button
                    key={option.value}
                    ref={option.value === value ? selectedItemRef : undefined}
                    type="button"
                    disabled={option.disabled}
                    onClick={() => select(option.value)}
                    className={
                      "flex w-full items-center gap-2 px-3 py-1.5 text-left text-body-sm transition " +
                      (option.value === value
                        ? "bg-ink/8 text-ink"
                        : "text-ink/70 hover:bg-ink/5 hover:text-ink") +
                      (option.disabled ? " opacity-40 pointer-events-none" : "")
                    }
                  >
                    {option.icon && <span className="shrink-0">{option.icon}</span>}
                    <span className="min-w-0 flex-1 truncate">{option.label}</span>
                    {option.value === value && (
                      <Check className="h-3.5 w-3.5 shrink-0 text-ink/65" />
                    )}
                  </button>
                ))
              ) : (
                <div className="px-3 py-2 text-body-sm text-ink/45">
                  {emptyContent}
                </div>
              )}
            </ScrollArea>
          </div>,
          document.body,
        )}
    </>
  );
}
