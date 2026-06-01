import AssistantBotIcon from "./AssistantBotIcon";

const ASSISTANT_COLOR_OPTIONS = [
  "#0ea5e9",
  "#8b5cf6",
  "#22c55e",
  "#f59e0b",
  "#ec4899",
  "#f97316",
  "#ef4444",
  "#14b8a6",
  "#6366f1",
  "#a855f7",
  "#06b6d4",
  "#84cc16",
];

export default function AssistantColorPicker({
  value,
  onChange,
}: {
  value: string | null;
  onChange: (value: string | null) => void;
}) {
  const customSelected = Boolean(value && !ASSISTANT_COLOR_OPTIONS.includes(value));

  return (
    <div className="grid grid-cols-[repeat(13,2rem)] gap-1.5">
      {ASSISTANT_COLOR_OPTIONS.map((color) => {
        const selected = value === color;
        return (
          <button
            key={color}
            type="button"
            onClick={() => onChange(color)}
            className={
              "flex h-8 w-8 items-center justify-center rounded-md border transition " +
              (selected
                ? "border-card-border/[0.28] bg-surface shadow-[inset_0_0_0_1px_rgb(var(--color-card-fg)/0.08)]"
                : "border-card-border/[0.10] bg-card-chip/[0.045] hover:border-card-border/[0.18] hover:bg-card-chip/[0.08]")
            }
          >
            <AssistantBotIcon color={color} className="h-4 w-4 shrink-0" />
          </button>
        );
      })}
      <label
        className={
          "relative flex h-8 w-8 cursor-pointer items-center justify-center overflow-hidden rounded-md border transition " +
          (customSelected
            ? "border-card-border/[0.28] bg-surface shadow-[inset_0_0_0_1px_rgb(var(--color-card-fg)/0.08)]"
            : "border-card-border/[0.10] bg-card-chip/[0.045] hover:border-card-border/[0.18] hover:bg-card-chip/[0.08]")
        }
      >
        <span
          className="absolute inset-1.5 rounded-full"
          style={{
            background: value && customSelected
              ? value
              : "conic-gradient(from 90deg, #ef4444, #f59e0b, #22c55e, #06b6d4, #6366f1, #ec4899, #ef4444)",
          }}
        />
        <input
          type="color"
          value={value ?? "#0ea5e9"}
          onChange={(event) => onChange(event.target.value)}
          className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
        />
      </label>
    </div>
  );
}
