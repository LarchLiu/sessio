import { Hand, ScrollText, ShieldAlert, ShieldEllipsis } from "lucide-react";
import CodeXmlIcon from '@iconify-react/material-symbols/code-xml';
import type { Agent, RuntimeAgentOptionMetadata } from "../api";
import InlineMenuSelect, { type InlineMenuSelectOption } from "./InlineMenuSelect";

export interface RuntimeMenuSelectProps {
  ariaLabel: string;
  value: string;
  options: InlineMenuSelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
  menuPlacement?: "auto" | "bottom" | "top" | "left" | "right";
  minMenuWidth?: number;
  maxWidthClassName?: string;
  portalZIndex?: number;
}

export function RuntimeMenuSelect({
  ariaLabel,
  value,
  options,
  onChange,
  disabled = false,
  menuPlacement = "auto",
  minMenuWidth = 180,
  maxWidthClassName = "max-w-[220px]",
  portalZIndex,
}: RuntimeMenuSelectProps) {
  return (
    <div className={`flex min-w-0 ${maxWidthClassName} items-center rounded-md text-ink/55 transition hover:bg-ink/8 hover:text-ink`}>
      <InlineMenuSelect
        value={value}
        options={disabled ? options.map((option) => ({ ...option, disabled: true })) : options}
        onChange={onChange}
        menuAlign="trigger"
        menuPlacement={menuPlacement}
        placeholder={ariaLabel}
        ariaLabel={ariaLabel}
        className={`h-7 ${maxWidthClassName} border-r-0 px-1.5 py-1 text-ink/60 hover:text-ink`}
        menuClassName="bg-surface-panel"
        minMenuWidth={minMenuWidth}
        emptyContent={ariaLabel}
        portalZIndex={portalZIndex}
      />
    </div>
  );
}

export function runtimePermissionModeOptions(
  options: RuntimeAgentOptionMetadata[],
  selected: string,
  agent?: Agent | null,
): InlineMenuSelectOption[] {
  const rows = options
    .filter((option) => option.value.trim().length > 0)
    .map((option) => ({
      value: option.value,
      label: option.displayName || option.label || option.value,
      icon: runtimePermissionModeIcon(agent, option.value),
    }));
  if (selected && !rows.some((option) => option.value === selected)) {
    rows.unshift({
      value: selected,
      label: selected,
      icon: runtimePermissionModeIcon(agent, selected),
    });
  }
  return rows;
}

export function runtimePermissionModeIcon(agent: Agent | null | undefined, value: string) {
  const className = "h-4 w-4 text-ink/55";
  if (agent === "codex") {
    switch (value) {
      case "read-only":
        return <Hand className={className} />;
      case "auto":
        return <ShieldEllipsis className={className} />;
      case "full-access":
        return <ShieldAlert className={className} />;
    }
  }
  if (agent === "claude") {
    switch (value) {
      case "default":
        return <Hand className={className} />;
      case "acceptEdits":
        return <CodeXmlIcon className={className} />;
      case "plan":
        return <ScrollText className={className} />;
      case "dontAsk":
        return <ShieldAlert className={className} />;
    }
  }
  return <Hand className={className} />;
}

export function RuntimeEffortControl({
  value,
  options,
  onChange,
  disabled = false,
}: {
  value: string;
  options: RuntimeAgentOptionMetadata[];
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  const visible = options.filter((option) => option.value.trim().length > 0);
  if (visible.length <= 1) return null;
  const selectedIndex = Math.max(0, visible.findIndex((option) => option.value === value));
  return (
    <div
      className={
        "relative flex h-4 items-center rounded-full bg-ink/10 px-0.5 " +
        (disabled ? "opacity-45" : "")
      }
      role="radiogroup"
      aria-label="Reasoning effort"
      style={{ width: Math.max(34, visible.length * 11) }}
    >
      <span
        className="absolute top-0.5 h-3 rounded-full bg-ink/70 transition-all"
        style={{
          width: `${100 / visible.length}%`,
          left: `${(selectedIndex * 100) / visible.length}%`,
        }}
      />
      {visible.map((option) => (
        <button
          key={option.value}
          type="button"
          disabled={disabled}
          role="radio"
          aria-checked={option.value === value}
          aria-label={option.displayName || option.label || option.value}
          title={option.displayName || option.label || option.value}
          onClick={() => onChange(option.value)}
          className="relative z-10 flex h-4 flex-1 items-center justify-center rounded-full disabled:cursor-not-allowed"
        >
          <span
            className={
              "h-1 w-1 rounded-full transition " +
              (option.value === value ? "bg-bg-panel/80" : "bg-ink/30")
            }
          />
        </button>
      ))}
    </div>
  );
}
