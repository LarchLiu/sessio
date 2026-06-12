import { ListChevronsDownUp, ListChevronsUpDown, PanelLeftOpen, type LucideIcon } from "lucide-react";
import type { ProjectInfo, SessionInfo } from "../api";
import { useI18n } from "../i18n";
import { AgentGlyph } from "./AgentIcon";
import Tooltip from "./Tooltip";
import WindowControls from "./WindowControls";

interface AppHeaderProps {
  isMac: boolean;
  sidebarOpen: boolean;
  selected: SessionInfo | null;
  detailTitle: string;
  contextTitle: { label: string; icon: LucideIcon } | null;
  entityTitle: { kind: "thread" | "project"; title: string; icon: LucideIcon; pill?: string } | null;
  projectContext: Pick<ProjectInfo, "name"> | null;
  activeMessageMeta: {
    count: number;
    partial: boolean;
  } | null;
  metaPopoverOpen: boolean;
  onOpenSidebar: () => void;
  onToggleMetaPopover: () => void;
}

export default function AppHeader({
  isMac,
  sidebarOpen,
  selected,
  detailTitle,
  contextTitle,
  entityTitle,
  projectContext,
  activeMessageMeta,
  metaPopoverOpen,
  onOpenSidebar,
  onToggleMetaPopover,
}: AppHeaderProps) {
  const { t } = useI18n();

  return (
    <div
      data-tauri-drag-region
      className={
        "relative h-12 shrink-0 grid grid-cols-3 items-center px-5 bg-surface border-b border-ink/10 select-none " +
        (isMac ? "" : "pr-[138px]")
      }
    >
      <Tooltip content={t("sidebar.open")} placement="bottom">
        <button
          type="button"
          aria-label={t("sidebar.open")}
          data-tauri-drag-region="false"
          onClick={onOpenSidebar}
          className={
            "absolute top-1/2 -translate-y-1/2 p-1 text-ink/55 hover:text-ink rounded-md transition-opacity duration-300 " +
            (isMac ? "left-24 " : "left-2 ") +
            (sidebarOpen ? "opacity-0 pointer-events-none" : "opacity-100")
          }
        >
          <PanelLeftOpen className="w-4 h-4" />
        </button>
      </Tooltip>
      <div
        data-tauri-drag-region
        className={
          "flex items-center gap-2 min-w-0 " +
          (sidebarOpen ? "" : isMac ? "pl-[112px] " : "pl-9 ")
        }
      >
        {selected && activeMessageMeta && sidebarOpen && (
          <HeaderMessageMetaButton
            label={`${activeMessageMeta.partial ? "~" : ""}${t("header.messages_count", { count: activeMessageMeta.count })}`}
            open={metaPopoverOpen}
            onToggle={onToggleMetaPopover}
          />
        )}
        {selected && !sidebarOpen && (
          <>
            <span
              data-tauri-drag-region
              className="flex h-4 w-4 shrink-0"
            >
              <AgentGlyph
                agent={selected.agent}
                className="h-4 w-4 pointer-events-none"
              />
            </span>
            <div
              data-tauri-drag-region
              className="min-w-0 max-w-[min(42vw,520px)]"
            >
              <div
                data-tauri-drag-region
                className="truncate text-body-sm font-medium leading-tight text-ink/85"
              >
                {detailTitle}
              </div>
              {activeMessageMeta && (
                <HeaderMessageMetaButton
                  label={`${activeMessageMeta.partial ? "~" : ""}${t("header.messages_count", { count: activeMessageMeta.count })}`}
                  open={metaPopoverOpen}
                  onToggle={onToggleMetaPopover}
                  compact
                />
              )}
            </div>
          </>
        )}
        {!selected && entityTitle && entityTitle.kind !== "project" && !sidebarOpen && (
          <HeaderEntityTitle title={entityTitle} />
        )}
      </div>
      <div data-tauri-drag-region className="flex h-full items-center justify-self-center">
        {contextTitle ? (
          <HeaderContextTitle title={contextTitle} project={projectContext} />
        ) : null}
      </div>
      <div className="flex h-full items-center justify-self-end" data-tauri-drag-region="false" />
      <div className="absolute top-0 right-0 z-20">
        <WindowControls />
      </div>
    </div>
  );
}

function HeaderEntityTitle({
  title,
}: {
  title: { kind: "thread" | "project"; title: string; icon: LucideIcon; pill?: string };
}) {
  const Icon = title.icon;
  return (
    <div
      data-tauri-drag-region
      className="flex min-w-0 max-w-[min(42vw,620px)] items-center gap-2"
    >
      <Icon data-tauri-drag-region className="h-4 w-4 shrink-0 pointer-events-none text-ink/45" />
      <span data-tauri-drag-region className="min-w-0 truncate text-body font-medium leading-none text-ink/85">
        {title.title}
      </span>
      {title.pill && (
        <span data-tauri-drag-region className="shrink-0 rounded-full border border-card-border/[0.14] bg-card-chip/[0.08] px-2 py-0.5 text-meta uppercase leading-none text-card-fg/55">
          {title.pill}
        </span>
      )}
    </div>
  );
}

function HeaderContextTitle({
  title,
  project,
}: {
  title: { label: string; icon: LucideIcon };
  project: Pick<ProjectInfo, "name"> | null;
}) {
  const Icon = title.icon;
  return (
    <div
      data-tauri-drag-region
      className="inline-flex h-6 max-w-[min(52vw,760px)] items-center gap-2 text-body-sm font-medium leading-none text-ink/50"
    >
      <span data-tauri-drag-region className="inline-flex min-w-0 items-center gap-2 uppercase tracking-[0.12em]">
        <Icon data-tauri-drag-region className="h-4 w-4 shrink-0 pointer-events-none" />
        <span data-tauri-drag-region className="truncate">{title.label}</span>
      </span>
      {project && (
        <>
          <span data-tauri-drag-region className="shrink-0 text-ink/28">·</span>
          <span data-tauri-drag-region className="min-w-0 truncate text-body-sm uppercase leading-none tracking-normal text-ink/72">
            {project.name}
          </span>
        </>
      )}
    </div>
  );
}

function HeaderMessageMetaButton({
  label,
  open,
  onToggle,
  compact = false,
}: {
  label: string;
  open: boolean;
  onToggle: () => void;
  compact?: boolean;
}) {
  const Icon = open ? ListChevronsDownUp : ListChevronsUpDown;
  return (
    <div
      data-tauri-drag-region
      className={
        "inline-flex items-center gap-1.5 " +
        (compact
          ? "text-caption text-ink/40"
          : "text-body font-medium text-ink/45")
      }
    >
      <span data-tauri-drag-region className="tabular-nums leading-tight">
        {label}
      </span>
      <button
        type="button"
        data-tauri-drag-region="false"
        onClick={onToggle}
        className="group -m-1 rounded-md p-1 text-ink/35 transition-colors hover:bg-ink/[0.05] hover:text-ink/65"
      >
        <Icon
          className={
            "shrink-0 transition-[transform,opacity] duration-200 " +
            (compact ? "h-3.5 w-3.5" : "h-4 w-4") +
            (open ? " rotate-0 scale-110" : " rotate-0 scale-100")
          }
        />
      </button>
    </div>
  );
}
