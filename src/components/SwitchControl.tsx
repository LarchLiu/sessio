import Tooltip from "./Tooltip";

export default function SwitchControl({
  checked,
  tooltip,
  onToggle,
  className = "",
}: {
  checked: boolean;
  tooltip?: string;
  onToggle: () => void;
  className?: string;
}) {
  const control = (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={onToggle}
      className={
        "relative h-5 w-9 rounded-full border transition " +
        (checked
          ? "border-brand/45 bg-brand/20"
          : "border-card-border/[0.12] bg-card-panel") +
        (className ? ` ${className}` : "")
      }
    >
      <span
        className={
          "absolute top-1/2 h-3.5 w-3.5 -translate-y-1/2 rounded-full transition " +
          (checked
            ? "left-[18px] bg-brand"
            : "left-1 bg-card-subtle/45")
        }
      />
    </button>
  );

  if (!tooltip) return control;
  return (
    <Tooltip content={tooltip} placement="top">
      {control}
    </Tooltip>
  );
}
