import { Check } from "lucide-react";
import { forwardRef, type ReactNode } from "react";
import type { ProjectStageInfo, StageInfo } from "../api";
import { useI18n } from "../i18n";
import { projectStageIcon, projectStageLabel } from "../utils/stageDisplay";

type StageSelectChipStage = Pick<ProjectStageInfo | StageInfo, "id" | "type" | "kind" | "name" | "icon">;

export interface StageSelectChipProps {
  stage: StageSelectChipStage;
  selected: boolean;
  onToggle: (stageId: string) => void;
  selectable?: boolean;
  dragHandle?: ReactNode;
  className?: string;
  state?: "idle" | "dragging" | "drop-target";
}

const StageSelectChip = forwardRef<HTMLDivElement, StageSelectChipProps>(function StageSelectChip(
  {
    stage,
    selected,
    onToggle,
    selectable = true,
    dragHandle,
    className = "",
    state = "idle",
  },
  ref,
) {
  const { t } = useI18n();
  const label = projectStageLabel(stage, t);
  const stateClass =
    state === "dragging"
      ? "z-20 cursor-grabbing border-ink/30 bg-surface-panel shadow-lg"
      : state === "drop-target"
        ? "border-ink/35 bg-ink/12 shadow-[inset_2px_0_0_rgb(var(--color-fg)/0.28)]"
        : !selectable
          ? "cursor-not-allowed border-ink/10 bg-surface-panel text-ink/25 opacity-55"
          : selected
            ? "border-ink/25 bg-ink/10 text-ink/75"
            : "border-ink/10 bg-surface-panel text-ink/45 hover:bg-ink/5 hover:text-ink/65";

  return (
    <div
      ref={ref}
      className={
        "inline-flex h-7 items-center gap-1.5 rounded-md border px-1.5 text-caption transition duration-150 " +
        stateClass +
        (className ? ` ${className}` : "")
      }
    >
      {dragHandle}
      <button
        type="button"
        onClick={() => onToggle(stage.id)}
        disabled={!selectable}
        className="inline-flex min-w-0 items-center gap-1.5 disabled:cursor-not-allowed"
      >
        {projectStageIcon(stage)}
        <span className="max-w-[140px] truncate">{label}</span>
        <span className="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border border-ink/55 bg-ink/8 text-ink/80">
          {selected && <Check className="h-3 w-3" />}
        </span>
      </button>
    </div>
  );
});

export default StageSelectChip;
