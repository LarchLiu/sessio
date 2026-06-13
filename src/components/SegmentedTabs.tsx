import type { LucideIcon } from "lucide-react";
import type { ComponentType, ReactNode } from "react";

export interface SegmentedTabItem<T extends string> {
  value: T;
  label: string;
  icon?: LucideIcon | ComponentType<{ className?: string }>;
  badge?: string | number;
}

interface SegmentedTabsProps<T extends string> {
  items: readonly SegmentedTabItem<T>[];
  value: T;
  onChange: (value: T) => void;
  itemWidth?: number;
  itemHeight?: number;
  padding?: number;
  className?: string;
  variant?: "segmented" | "underline";
  endAdornment?: ReactNode;
  fullWidth?: boolean;
}

export default function SegmentedTabs<T extends string>({
  items,
  value,
  onChange,
  itemWidth = 96,
  itemHeight = 26,
  padding = 2,
  className = "",
  variant = "segmented",
  endAdornment,
  fullWidth = false,
}: SegmentedTabsProps<T>) {
  if (items.length === 0) return null;

  const activeIndex = Math.max(
    0,
    items.findIndex((item) => item.value === value),
  );

  if (variant === "underline") {
    return (
      <div className={"flex items-center border-b border-card-border/[0.12] " + className}>
        <div className="flex min-w-0 items-center">
          {items.map(({ value: itemValue, label, icon: Icon, badge }) => {
            const active = itemValue === value;
            return (
              <button
                key={itemValue}
                type="button"
                onClick={() => onChange(itemValue)}
                style={{ width: itemWidth ? `${itemWidth}px` : undefined, height: `${itemHeight}px` }}
                className={
                  "relative inline-flex min-w-0 items-center justify-center gap-2 px-2 text-body-sm leading-none transition-colors duration-150 after:absolute after:bottom-[-1px] after:left-0 after:right-0 after:h-px " +
                  (active
                    ? "text-card-fg/90 after:bg-brand"
                    : "text-card-muted/55 after:bg-transparent hover:text-card-fg/80")
                }
              >
                {Icon ? <Icon className="h-3.5 w-3.5 shrink-0" /> : null}
                <span className="min-w-0 truncate">{label}</span>
                {badge !== undefined && (
                  <span className="shrink-0 rounded-full bg-card-chip/[0.10] px-1.5 py-0.5 text-[11px] font-semibold leading-none text-card-chip-fg/65">
                    {badge}
                  </span>
                )}
              </button>
            );
          })}
        </div>
        {endAdornment ? <div className="ml-auto flex items-center">{endAdornment}</div> : null}
      </div>
    );
  }

  return (
    <div
      className={
        "relative inline-flex items-center rounded-md bg-ink/[0.14] " +
        (fullWidth ? "w-full " : "") +
        className
      }
      style={{ padding }}
    >
      <div
        aria-hidden
        className="absolute rounded bg-surface shadow-[0_1px_2px_rgba(0,0,0,0.18)] transition-transform duration-300 ease-out"
        style={{
          width: fullWidth ? `calc((100% - ${padding * 2}px) / ${items.length})` : `${itemWidth}px`,
          height: `${itemHeight}px`,
          left: `${padding}px`,
          top: `${padding}px`,
          transform: fullWidth ? `translateX(${activeIndex * 100}%)` : `translateX(${activeIndex * itemWidth}px)`,
        }}
      />
      {items.map(({ value: itemValue, label, icon: Icon }) => {
        const active = itemValue === value;
        return (
          <button
            key={itemValue}
            type="button"
            onClick={() => onChange(itemValue)}
            style={{ width: fullWidth ? undefined : `${itemWidth}px`, height: `${itemHeight}px` }}
            className={
              "relative z-10 inline-flex min-w-0 items-center justify-center gap-1.5 rounded px-2 text-body-sm leading-none transition-colors duration-150 " +
              (fullWidth ? "flex-1 " : "") +
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
