import { ListChevronsDownUp, ListChevronsUpDown, PanelLeftOpen, Search } from "lucide-react";
import type { ComponentType } from "react";
import type { MemoryBackendStatus, SessionInfo } from "../api";
import { useI18n } from "../i18n";
import type { DetailMode } from "../navigation";
import { AgentGlyph } from "./AgentIcon";
import Tooltip from "./Tooltip";
import WindowControls from "./WindowControls";

interface AppHeaderProps {
  isMac: boolean;
  sidebarOpen: boolean;
  selected: SessionInfo | null;
  detailTitle: string;
  detailMode: DetailMode;
  showDetailTabs: boolean;
  activeMessageMeta: {
    count: number;
    partial: boolean;
  } | null;
  metaPopoverOpen: boolean;
  memoryBackendStatus: MemoryBackendStatus | null;
  memoryBackendMissing: boolean;
  projectCount: number;
  onOpenSidebar: () => void;
  onDetailModeChange: (mode: DetailMode) => void;
  onToggleMetaPopover: () => void;
  onOpenSearch: () => void;
  onRefreshMemoryBackend: () => Promise<void> | void;
  MemoryBackendMissingButton: ComponentType<{
    status: MemoryBackendStatus;
    placement: "bottom";
    onRefresh: () => Promise<void> | void;
  }>;
}

export default function AppHeader({
  isMac,
  sidebarOpen,
  selected,
  detailTitle,
  detailMode,
  showDetailTabs,
  activeMessageMeta,
  metaPopoverOpen,
  memoryBackendStatus,
  memoryBackendMissing,
  projectCount,
  onOpenSidebar,
  onDetailModeChange,
  onToggleMetaPopover,
  onOpenSearch,
  onRefreshMemoryBackend,
  MemoryBackendMissingButton,
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
              className="flex h-5 w-5 shrink-0"
            >
              <AgentGlyph
                agent={selected.agent}
                className="h-5 w-5 pointer-events-none"
              />
            </span>
            <div
              data-tauri-drag-region
              className="min-w-0 max-w-[min(42vw,520px)]"
            >
              <div
                data-tauri-drag-region
                className="truncate text-body font-medium leading-tight text-ink/85"
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
      </div>
      <div data-tauri-drag-region="false" className="justify-self-center">
        {showDetailTabs && (
          <HeaderModeTabs mode={detailMode} onChange={onDetailModeChange} />
        )}
      </div>
      <div className="justify-self-end" data-tauri-drag-region="false">
        {memoryBackendMissing && memoryBackendStatus ? (
          <MemoryBackendMissingButton
            status={memoryBackendStatus}
            placement="bottom"
            onRefresh={onRefreshMemoryBackend}
          />
        ) : (
          <Tooltip content={t("header.search")} placement="bottom">
            <button
              type="button"
              aria-label={t("header.search")}
              onClick={onOpenSearch}
              disabled={projectCount === 0}
              className="p-1 text-ink/55 hover:text-ink disabled:opacity-35 disabled:hover:text-ink/55 transition rounded-md"
            >
              <Search className="w-4 h-4" />
            </button>
          </Tooltip>
        )}
      </div>
      <div className="absolute top-0 right-0 z-20">
        <WindowControls />
      </div>
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

function HeaderModeTabs({
  mode,
  onChange,
}: {
  mode: DetailMode;
  onChange: (mode: DetailMode) => void;
}) {
  const items: { value: DetailMode; label: string }[] = [
    { value: "chat", label: "Chat" },
    { value: "project", label: "Project" },
  ];
  const activeIndex = Math.max(
    0,
    items.findIndex((item) => item.value === mode),
  );
  const BTN_W = 72;
  return (
    <div className="relative flex items-center rounded-md bg-ink/[0.14] p-0.5">
      <div
        aria-hidden
        className="absolute top-0.5 left-0.5 h-[26px] rounded bg-surface shadow-[0_1px_2px_rgba(0,0,0,0.18)] transition-transform duration-300 ease-out"
        style={{
          width: `${BTN_W}px`,
          transform: `translateX(${activeIndex * BTN_W}px)`,
        }}
      />
      {items.map(({ value, label }, index) => {
        const active = index === activeIndex;
        return (
          <button
            key={label}
            type="button"
            onClick={() => onChange(value)}
            style={{ width: `${BTN_W}px` }}
            className={
              "relative z-10 h-[26px] flex items-center justify-center rounded text-body-sm leading-none transition-colors duration-150 " +
              (active ? "text-ink" : "text-ink/55 hover:text-ink/85")
            }
          >
            {label}
          </button>
        );
      })}
    </div>
  );
}
