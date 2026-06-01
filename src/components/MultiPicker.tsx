import { type ReactNode, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown } from "lucide-react";
import ScrollArea from "./ScrollArea";

const MENU_GAP = 6;
const MENU_MARGIN = 8;
const MENU_MAX_HEIGHT = 260;

export interface MultiPickerOption {
  value: string;
  label: string;
  icon?: ReactNode;
}

export default function MultiPicker({
  selectedValues,
  options,
  onChange,
  placeholder,
  className = "",
}: {
  selectedValues: string[];
  options: MultiPickerOption[];
  onChange: (values: string[]) => void;
  placeholder: string;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number; width: number } | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const selected = new Set(selectedValues);
  const selectedOptions = selectedValues
    .map((value) => options.find((option) => option.value === value))
    .filter((option): option is MultiPickerOption => Boolean(option));

  const updatePosition = useCallback(() => {
    if (!open) return;
    const button = buttonRef.current;
    if (!button) return;
    const rect = button.getBoundingClientRect();
    const width = Math.max(180, rect.width);
    const maxLeft = Math.max(MENU_MARGIN, window.innerWidth - width - MENU_MARGIN);
    const estimatedHeight = Math.min(
      MENU_MAX_HEIGHT,
      Math.max(40, options.length * 32 + 12),
    );
    const roomBelow = window.innerHeight - rect.bottom - MENU_MARGIN;
    const top =
      roomBelow >= estimatedHeight
        ? rect.bottom + MENU_GAP
        : rect.top - estimatedHeight - MENU_GAP;
    const maxTop = Math.max(MENU_MARGIN, window.innerHeight - estimatedHeight - MENU_MARGIN);
    setPos({
      top: Math.round(Math.max(MENU_MARGIN, Math.min(top, maxTop))),
      left: Math.round(Math.max(MENU_MARGIN, Math.min(rect.left, maxLeft))),
      width,
    });
  }, [open, options.length]);

  useLayoutEffect(() => {
    updatePosition();
  }, [updatePosition]);

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (buttonRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onMouseDown);
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open, updatePosition]);

  const toggle = (value: string) => {
    if (selected.has(value)) {
      onChange(selectedValues.filter((item) => item !== value));
      return;
    }
    onChange([...selectedValues, value]);
  };

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        onClick={() => setOpen((value) => !value)}
        className={
          "inline-flex h-7 max-w-[260px] items-center gap-1.5 px-2 text-caption text-ink/65 outline-none hover:text-ink " +
          className
        }
      >
        <span className="flex min-w-0 items-center gap-1 overflow-hidden">
          {selectedOptions.length > 0 ? (
            selectedOptions.map((option) => (
              <span key={option.value} className="inline-flex min-w-0 shrink items-center gap-1">
                {option.icon}
                <span className="truncate">{option.label}</span>
              </span>
            ))
          ) : (
            <span className="min-w-0 flex-1 truncate text-left">{placeholder}</span>
          )}
        </span>
        <ChevronDown className="h-3.5 w-3.5 shrink-0 text-ink/40" />
      </button>
      {open &&
        pos &&
        createPortal(
          <div
            ref={menuRef}
            onWheel={(event) => event.stopPropagation()}
            className="fixed overflow-hidden rounded-lg border border-ink/10 bg-surface-panel p-1.5 shadow-lg"
            style={{
              top: pos.top,
              left: pos.left,
              width: pos.width,
              maxHeight: MENU_MAX_HEIGHT,
              zIndex: 90,
            }}
          >
            <ScrollArea className="max-h-[248px] overscroll-contain">
              {options.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  onClick={() => toggle(option.value)}
                  className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-caption text-ink/65 hover:bg-ink/5"
                >
                  <span className="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border border-ink/15 bg-ink/5">
                    {selected.has(option.value) && <Check className="h-3 w-3" />}
                  </span>
                  {option.icon}
                  <span className="min-w-0 flex-1 truncate">{option.label}</span>
                </button>
              ))}
            </ScrollArea>
          </div>,
          document.body,
        )}
    </>
  );
}
