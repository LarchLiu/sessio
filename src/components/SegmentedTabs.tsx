import type { LucideIcon } from "lucide-react";
import type { ComponentType } from "react";

export interface SegmentedTabItem<T extends string> {
  value: T;
  label: string;
  icon?: LucideIcon | ComponentType<{ className?: string }>;
}

interface SegmentedTabsProps<T extends string> {
  items: readonly SegmentedTabItem<T>[];
  value: T;
  onChange: (value: T) => void;
  itemWidth?: number;
  itemHeight?: number;
  padding?: number;
  className?: string;
}

export default function SegmentedTabs<T extends string>({
  items,
  value,
  onChange,
  itemWidth = 96,
  itemHeight = 26,
  padding = 2,
  className = "",
}: SegmentedTabsProps<T>) {
  if (items.length === 0) return null;

  const activeIndex = Math.max(
    0,
    items.findIndex((item) => item.value === value),
  );

  return (
    <div
      className={"relative inline-flex items-center rounded-md bg-ink/[0.14] " + className}
      style={{ padding }}
    >
      <div
        aria-hidden
        className="absolute rounded bg-surface shadow-[0_1px_2px_rgba(0,0,0,0.18)] transition-transform duration-300 ease-out"
        style={{
          width: `${itemWidth}px`,
          height: `${itemHeight}px`,
          left: `${padding}px`,
          top: `${padding}px`,
          transform: `translateX(${activeIndex * itemWidth}px)`,
        }}
      />
      {items.map(({ value: itemValue, label, icon: Icon }) => {
        const active = itemValue === value;
        return (
          <button
            key={itemValue}
            type="button"
            onClick={() => onChange(itemValue)}
            style={{ width: `${itemWidth}px`, height: `${itemHeight}px` }}
            className={
              "relative z-10 inline-flex min-w-0 items-center justify-center gap-1.5 rounded px-2 text-body-sm leading-none transition-colors duration-150 " +
              (active ? "text-ink" : "text-ink/55 hover:text-ink/85")
            }
          >
            {Icon ? <Icon className="h-3.5 w-3.5 shrink-0" /> : null}
            <span className="min-w-0 truncate">{label}</span>
          </button>
        );
      })}
    </div>
  );
}
