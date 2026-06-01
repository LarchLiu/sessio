import { STAGE_ICON_OPTIONS } from "../utils/stageDisplay";

export default function StageIconPicker({
  value,
  onChange,
}: {
  value: string | null;
  onChange: (value: string | null) => void;
}) {
  return (
    <div className="grid grid-cols-[repeat(13,2rem)] gap-1.5">
      {STAGE_ICON_OPTIONS.map(({ id, Icon }) => {
        const selected = value === id;
        return (
          <button
            key={id}
            type="button"
            onClick={() => onChange(selected ? null : id)}
            className={
              "flex h-8 w-8 items-center justify-center rounded-md border transition " +
              (selected
                ? "border-card-border/[0.28] bg-surface text-card-fg/90 shadow-[inset_0_0_0_1px_rgb(var(--color-card-fg)/0.08)]"
                : "border-card-border/[0.10] bg-card-chip/[0.045] text-card-muted/55 hover:border-card-border/[0.18] hover:bg-card-chip/[0.08] hover:text-card-fg/80")
            }
          >
            <Icon className="h-4 w-4" />
          </button>
        );
      })}
    </div>
  );
}
