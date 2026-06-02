import SearchRoundedIcon from "@iconify-react/material-symbols/search-rounded";
import ChecklistRoundedIcon from "@iconify-react/material-symbols/checklist-rounded";
import CodeRoundedIcon from "@iconify-react/material-symbols/code-rounded";
import ConstructionRoundedIcon from "@iconify-react/material-symbols/construction-rounded";
import EditNoteOutlineRoundedIcon from "@iconify-react/material-symbols/edit-note-outline-rounded";
import RateReviewOutlineRoundedIcon from "@iconify-react/material-symbols/rate-review-outline-rounded";
import SpellcheckRoundedIcon from "@iconify-react/material-symbols/spellcheck-rounded";
import MovieOutlineRoundedIcon from "@iconify-react/material-symbols/movie-outline-rounded";
import DashboardCustomizeOutlineRoundedIcon from "@iconify-react/material-symbols/dashboard-customize-outline-rounded";
import DesignServicesOutlineRoundedIcon from "@iconify-react/material-symbols/design-services-outline-rounded";
import VideoLibraryOutlineRoundedIcon from "@iconify-react/material-symbols/video-library-outline-rounded";
import PersonOutlineRoundedIcon from "@iconify-react/material-symbols/person-outline-rounded";
import TaskAltOutlineIcon from "@iconify-react/material-symbols/task-alt-outline";
import ScienceOutlineIcon from "@iconify-react/material-symbols/science-outline";
import RouteOutlineIcon from "@iconify-react/material-symbols/route-outline";
import TerminalRoundedIcon from "@iconify-react/material-symbols/terminal-rounded";
import BuildOutlineRoundedIcon from "@iconify-react/material-symbols/build-outline-rounded";
import DrawOutlineRoundedIcon from "@iconify-react/material-symbols/draw-outline-rounded";
import FactCheckOutlineRoundedIcon from "@iconify-react/material-symbols/fact-check-outline-rounded";
import DoneAllRoundedIcon from "@iconify-react/material-symbols/done-all-rounded";
import AccountTreeOutlineRoundedIcon from "@iconify-react/material-symbols/account-tree-outline-rounded";
import AutoAwesomeOutlineRoundedIcon from "@iconify-react/material-symbols/auto-awesome-outline-rounded";
import BoltOutlineRoundedIcon from "@iconify-react/material-symbols/bolt-outline-rounded";
import PsychologyOutlineRoundedIcon from "@iconify-react/material-symbols/psychology-outline-rounded";
import RocketLaunchOutlineRoundedIcon from "@iconify-react/material-symbols/rocket-launch-outline-rounded";
import SchemaOutlineRoundedIcon from "@iconify-react/material-symbols/schema-outline-rounded";
import { Clapperboard, FilePenLine, GitBranch, ListChecks, Palette, Scissors, SpellCheck, Check, Circle, CircleAlert, CircleDot, CircleGauge, CircleUserRound, CircleCheck, MinusCircle, type LucideIcon } from "lucide-react";
import type { ComponentType } from "react";
import type { ProjectStageInfo, StageInfo, StageStatus, StageType } from "../api";

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

type IconComponent = ComponentType<{ className?: string }>;

export const STAGE_ICON_SET: Record<string, IconComponent> = {
  "material-symbols:search-rounded": SearchRoundedIcon,
  "material-symbols:checklist-rounded": ChecklistRoundedIcon,
  "material-symbols:code-rounded": CodeRoundedIcon,
  "material-symbols:construction-rounded": ConstructionRoundedIcon,
  "material-symbols:edit-note-outline-rounded": EditNoteOutlineRoundedIcon,
  "material-symbols:rate-review-outline-rounded": RateReviewOutlineRoundedIcon,
  "material-symbols:spellcheck-rounded": SpellcheckRoundedIcon,
  "material-symbols:movie-outline-rounded": MovieOutlineRoundedIcon,
  "material-symbols:dashboard-customize-outline-rounded": DashboardCustomizeOutlineRoundedIcon,
  "material-symbols:design-services-outline-rounded": DesignServicesOutlineRoundedIcon,
  "material-symbols:video-library-outline-rounded": VideoLibraryOutlineRoundedIcon,
  "material-symbols:person-outline-rounded": PersonOutlineRoundedIcon,
  "material-symbols:task-alt-outline": TaskAltOutlineIcon,
  "material-symbols:science-outline": ScienceOutlineIcon,
  "material-symbols:route-outline": RouteOutlineIcon,
  "material-symbols:terminal-rounded": TerminalRoundedIcon,
  "material-symbols:build-outline-rounded": BuildOutlineRoundedIcon,
  "material-symbols:draw-outline-rounded": DrawOutlineRoundedIcon,
  "material-symbols:fact-check-outline-rounded": FactCheckOutlineRoundedIcon,
  "material-symbols:done-all-rounded": DoneAllRoundedIcon,
  "material-symbols:account-tree-outline-rounded": AccountTreeOutlineRoundedIcon,
  "material-symbols:auto-awesome-outline-rounded": AutoAwesomeOutlineRoundedIcon,
  "material-symbols:bolt-outline-rounded": BoltOutlineRoundedIcon,
  "material-symbols:psychology-outline-rounded": PsychologyOutlineRoundedIcon,
  "material-symbols:rocket-launch-outline-rounded": RocketLaunchOutlineRoundedIcon,
  "material-symbols:schema-outline-rounded": SchemaOutlineRoundedIcon,
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
        icon: CircleDot,
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
