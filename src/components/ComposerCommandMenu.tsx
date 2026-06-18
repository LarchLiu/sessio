import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AtSign, Hash, SquareTerminal } from "lucide-react";

export interface ComposerCommandItem {
  key: string;
  label: string;
  description?: string;
  icon?: ReactNode;
  iconKey?: "slash" | "assistant" | "thread";
}

export default function ComposerCommandMenu({
  anchor,
  items,
  activeIndex,
  header,
  emptyText,
  onActiveIndexChange,
  onSelect,
  onClose,
}: {
  anchor: HTMLElement;
  items: ComposerCommandItem[];
  activeIndex: number;
  header?: string;
  emptyText?: string;
  onActiveIndexChange: (index: number) => void;
  onSelect: (key: string) => void;
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; bottom: number; width: number } | null>(null);

  useLayoutEffect(() => {
    const update = () => {
      const rect = anchor.getBoundingClientRect();
      setPos({
        left: Math.round(rect.left),
        bottom: Math.round(window.innerHeight - rect.top + 8),
        width: Math.round(Math.max(rect.width, 240)),
      });
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [anchor]);

  useLayoutEffect(() => {
    const el = menuRef.current?.querySelector(`[data-index="${activeIndex}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, items]);

  return createPortal(
    <>
      <div className="fixed inset-0 z-[59] bg-transparent" onMouseDown={onClose} />
      <div
        ref={menuRef}
        className="fixed z-[60] max-h-[260px] overflow-auto rounded-xl border border-ink/10 bg-surface-panel p-1 shadow-[0_20px_60px_rgba(0,0,0,0.22)]"
        style={{
          left: pos?.left ?? -9999,
          bottom: pos?.bottom ?? -9999,
          width: pos?.width,
          visibility: pos ? "visible" : "hidden",
        }}
        role="listbox"
      >
        {header && (
          <div className="px-2.5 pb-1 pt-1.5 text-[11px] font-medium uppercase tracking-normal text-ink/45">
            {header}
          </div>
        )}
        {items.length === 0 ? (
          <div className="px-2.5 py-2 text-body-sm text-ink/45">{emptyText}</div>
        ) : (
          items.map((item, index) => (
            <button
              key={item.key}
              type="button"
              data-index={index}
              role="option"
              aria-selected={index === activeIndex}
              onMouseEnter={() => onActiveIndexChange(index)}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => onSelect(item.key)}
              className={
                "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-body-sm transition " +
                (index === activeIndex ? "bg-ink/[0.08] text-ink" : "text-ink/72 hover:text-ink")
              }
            >
              {(item.icon || item.iconKey) && (
                <span className="flex h-4 w-4 shrink-0 items-center justify-center text-ink/55">
                  {item.icon ?? defaultCommandItemIcon(item.iconKey)}
                </span>
              )}
              <span className="min-w-0 flex-1">
                <span className="block truncate">{item.label}</span>
                {item.description && (
                  <span className="mt-0.5 block truncate text-caption text-ink/40">
                    {item.description}
                  </span>
                )}
              </span>
            </button>
          ))
        )}
      </div>
    </>,
    document.body,
  );
}

function defaultCommandItemIcon(kind: ComposerCommandItem["iconKey"]): ReactNode {
  if (kind === "assistant") return <AtSign className="h-4 w-4" />;
  if (kind === "thread") return <Hash className="h-4 w-4" />;
  return <SquareTerminal className="h-4 w-4" />;
}
