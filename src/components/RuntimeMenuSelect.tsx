import { Hand, ScrollText, ShieldAlert, ShieldEllipsis, SquarePen } from "lucide-react";
import type { Agent } from "../api";
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
      />
    </div>
  );
}

export function runtimePermissionModeOptions(
  options: { value: string; label: string }[],
  selected: string,
  agent?: Agent | null,
): InlineMenuSelectOption[] {
  const rows = options
    .filter((option) => option.value.trim().length > 0)
    .map((option) => ({
      value: option.value,
      label: option.label || option.value,
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

function runtimePermissionModeIcon(agent: Agent | null | undefined, value: string) {
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
        return <SquarePen className={className} />;
      case "plan":
        return <ScrollText className={className} />;
      case "dontAsk":
        return <ShieldAlert className={className} />;
    }
  }
  return <Hand className={className} />;
}
