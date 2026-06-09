import { Clapperboard, FilePenLine, GitBranch, ListChecks, Palette, Scissors, SpellCheck, Check, Circle, CircleAlert, CircleDot, CircleGauge, CircleUserRound, CircleCheck, MinusCircle, type LucideIcon } from "lucide-react";
import type { ReactElement } from "react";
import type { ProjectStageInfo, StageInfo, StageStatus, StageType } from "../api";
import IconifyIcon, { type IconifyIconClassName } from "../components/IconifyIcon";

export const STAGE_TYPE_ICONS: Record<StageType, LucideIcon> = {
  research: CircleGauge,
  plan: ListChecks,
  develop: GitBranch,
  build: GitBranch,
  writing: FilePenLine,
  editing: Scissors,
  review: CircleDot,
  proofreading: SpellCheck,
  screenplay: FilePenLine,
  storyboard: Clapperboard,
  design: Palette,
  production: Clapperboard,
  human: CircleUserRound,
  done: CircleCheck,
};

type IconComponent = (props: { className?: string }) => ReactElement;

function iconifyStageIcon(iconClassName: IconifyIconClassName): IconComponent {
  return ({ className }) => <IconifyIcon iconClassName={iconClassName} className={className} />;
}

export const STAGE_ICON_SET: Record<string, IconComponent> = {
  "material-symbols:search-rounded": iconifyStageIcon("icon-[material-symbols--search-rounded]"),
  "material-symbols:checklist-rounded": iconifyStageIcon("icon-[material-symbols--checklist-rounded]"),
  "material-symbols:code-rounded": iconifyStageIcon("icon-[material-symbols--code-rounded]"),
  "material-symbols:construction-rounded": iconifyStageIcon("icon-[material-symbols--construction-rounded]"),
  "material-symbols:edit-note-outline-rounded": iconifyStageIcon("icon-[material-symbols--edit-note-outline-rounded]"),
  "material-symbols:rate-review-outline-rounded": iconifyStageIcon("icon-[material-symbols--rate-review-outline-rounded]"),
  "material-symbols:spellcheck-rounded": iconifyStageIcon("icon-[material-symbols--spellcheck-rounded]"),
  "material-symbols:movie-outline-rounded": iconifyStageIcon("icon-[material-symbols--movie-outline-rounded]"),
  "material-symbols:dashboard-customize-outline-rounded": iconifyStageIcon("icon-[material-symbols--dashboard-customize-outline-rounded]"),
  "material-symbols:design-services-outline-rounded": iconifyStageIcon("icon-[material-symbols--design-services-outline-rounded]"),
  "material-symbols:video-library-outline-rounded": iconifyStageIcon("icon-[material-symbols--video-library-outline-rounded]"),
  "material-symbols:person-outline-rounded": iconifyStageIcon("icon-[material-symbols--person-outline-rounded]"),
  "material-symbols:task-alt-outline": iconifyStageIcon("icon-[material-symbols--task-alt-outline]"),
  "material-symbols:science-outline": iconifyStageIcon("icon-[material-symbols--science-outline]"),
  "material-symbols:route-outline": iconifyStageIcon("icon-[material-symbols--route-outline]"),
  "material-symbols:terminal-rounded": iconifyStageIcon("icon-[material-symbols--terminal-rounded]"),
  "material-symbols:build-outline-rounded": iconifyStageIcon("icon-[material-symbols--build-outline-rounded]"),
  "material-symbols:draw-outline-rounded": iconifyStageIcon("icon-[material-symbols--draw-outline-rounded]"),
  "material-symbols:fact-check-outline-rounded": iconifyStageIcon("icon-[material-symbols--fact-check-outline-rounded]"),
  "material-symbols:done-all-rounded": iconifyStageIcon("icon-[material-symbols--done-all-rounded]"),
  "material-symbols:account-tree-outline-rounded": iconifyStageIcon("icon-[material-symbols--account-tree-outline-rounded]"),
  "material-symbols:auto-awesome-outline-rounded": iconifyStageIcon("icon-[material-symbols--auto-awesome-outline-rounded]"),
  "material-symbols:bolt-outline-rounded": iconifyStageIcon("icon-[material-symbols--bolt-outline-rounded]"),
  "material-symbols:psychology-outline-rounded": iconifyStageIcon("icon-[material-symbols--psychology-outline-rounded]"),
  "material-symbols:rocket-launch-outline-rounded": iconifyStageIcon("icon-[material-symbols--rocket-launch-outline-rounded]"),
  "material-symbols:schema-outline-rounded": iconifyStageIcon("icon-[material-symbols--schema-outline-rounded]"),
};

export const STAGE_ICON_OPTIONS = Object.entries(STAGE_ICON_SET).map(([id, Icon]) => ({ id, Icon }));

export const DEFAULT_STAGE_TYPE_ICON_IDS: Partial<Record<StageType, string>> = {
  research: "material-symbols:search-rounded",
  plan: "material-symbols:checklist-rounded",
  develop: "material-symbols:code-rounded",
  build: "material-symbols:construction-rounded",
  writing: "material-symbols:edit-note-outline-rounded",
  editing: "material-symbols:rate-review-outline-rounded",
  review: "material-symbols:fact-check-outline-rounded",
  proofreading: "material-symbols:spellcheck-rounded",
  screenplay: "material-symbols:movie-outline-rounded",
  storyboard: "material-symbols:dashboard-customize-outline-rounded",
  design: "material-symbols:design-services-outline-rounded",
  production: "material-symbols:video-library-outline-rounded",
  human: "material-symbols:person-outline-rounded",
  done: "material-symbols:done-all-rounded",
};

export function stageTypeLabel(type: StageType, t: (key: string) => string): string {
  return t(`stage.type.${type}`);
}

export function projectStageLabel(stage: Pick<ProjectStageInfo | StageInfo, "type" | "kind" | "name">, t: (key: string) => string): string {
  return stage.type === "builtin" && stage.kind
    ? stageTypeLabel(stage.kind, t)
    : stage.name || t("stage.custom");
}

export function projectStageIconClass(stage: Pick<ProjectStageInfo | StageInfo, "kind">): LucideIcon {
  return stage.kind ? STAGE_TYPE_ICONS[stage.kind] : ListChecks;
}

export function projectStageIcon(stage: Pick<ProjectStageInfo | StageInfo, "kind" | "icon">, className = "h-3.5 w-3.5") {
  const iconId = stage.icon || (stage.kind ? DEFAULT_STAGE_TYPE_ICON_IDS[stage.kind] : null);
  if (iconId) {
    const PresetIcon = STAGE_ICON_SET[iconId];
    if (PresetIcon) return <PresetIcon className={className} />;
  }
  const Icon = projectStageIconClass(stage);
  return <Icon className={className} />;
}

export const STAGE_STATUS_ORDER: StageStatus[] = [
  "not_started",
  "in_progress",
  "needs_review",
  "blocked",
  "completed",
  "skipped",
];

export type StageStatusVisual = {
  icon: LucideIcon;
  markerClass: string;
  textClass: string;
};

export function stageStatusVisual(status: StageStatus): StageStatusVisual {
  switch (status) {
    case "completed":
      return {
        icon: Check,
        markerClass:
          "border-[rgb(var(--color-emerald)/0.80)] bg-[rgb(var(--color-emerald))] text-[rgb(var(--color-bg-panel))]",
        textClass: "text-[rgb(var(--color-emerald))]",
      };
    case "in_progress":
      return {
        icon: CircleDot,
        markerClass:
          "border-[rgb(var(--color-emerald)/0.80)] bg-surface-panel text-[rgb(var(--color-emerald))]",
        textClass: "text-[rgb(var(--color-emerald))]",
      };
    case "needs_review":
      return {
        icon: CircleGauge,
        markerClass: "border-sky-500/70 bg-surface-panel text-sky-500",
        textClass: "text-sky-500",
      };
    case "blocked":
      return {
        icon: CircleAlert,
        markerClass: "border-amber-500/70 bg-surface-panel text-amber-500",
        textClass: "text-amber-500",
      };
    case "skipped":
      return {
        icon: MinusCircle,
        markerClass: "border-ink/15 bg-surface-panel text-ink/30",
        textClass: "text-ink/40",
      };
    case "not_started":
    default:
      return {
        icon: Circle,
        markerClass: "border-ink/15 bg-surface-panel text-ink/30",
        textClass: "text-ink/45",
      };
  }
}
