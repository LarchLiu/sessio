import { FileCodeCorner, ListChevronsDownUp, ListChevronsUpDown, MessageSquare, PanelBottomClose, PanelBottomOpen, PanelLeftOpen, PanelRightClose, Presentation, type LucideIcon } from "lucide-react";
import type { ProjectInfo, SessionInfo } from "../api";
import type { ChatView } from "../navigation";
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
  rightSidebarOpen?: boolean;
  terminalDockOpen?: boolean;
  terminalDockVisible?: boolean;
  chatView?: ChatView;
  chatViewVisible?: boolean;
  appChatVisible?: boolean;
  onToggleAppChat?: () => void;
  onOpenSidebar: () => void;
  onToggleMetaPopover: () => void;
  onToggleTerminalDock?: () => void;
  onToggleRightSidebar?: () => void;
  onChatViewChange?: (view: ChatView) => void;
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
  rightSidebarOpen,
  terminalDockOpen = false,
  terminalDockVisible = false,
  chatView = "chat",
  chatViewVisible = false,
  appChatVisible = true,
  onToggleAppChat,
  onOpenSidebar,
  onToggleMetaPopover,
  onToggleTerminalDock,
  onToggleRightSidebar,
  onChatViewChange,
}: AppHeaderProps) {
  const { t } = useI18n();

  const rightPanelLabel = rightSidebarOpen
    ? t("sidebar.right_close")
    : t("sidebar.right_open");
  const terminalDockLabel = terminalDockOpen
    ? t("terminal_dock.hide")
    : t("terminal_dock.show");

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
      <div className="flex h-full items-center justify-self-end gap-2" data-tauri-drag-region="false">
        {onToggleAppChat && (
          <Tooltip content={t(appChatVisible ? "apps.chat_hide" : "apps.chat_show")} placement="bottom">
            <button
              type="button"
              aria-label={t(appChatVisible ? "apps.chat_hide" : "apps.chat_show")}
              aria-pressed={appChatVisible}
              aria-controls="app-chat"
              data-tauri-drag-region="false"
              onClick={onToggleAppChat}
              className={
                "rounded-md p-1 transition-colors hover:bg-ink/5 hover:text-ink " +
                (appChatVisible ? "bg-ink/5 text-ink" : "text-ink/55")
              }
            >
              <MessageSquare className="h-4 w-4" />
            </button>
          </Tooltip>
        )}
        {chatViewVisible && onChatViewChange && (
          <ChatViewToggle value={chatView} onChange={onChatViewChange} />
        )}
        {terminalDockVisible && onToggleTerminalDock && (
          <Tooltip content={terminalDockLabel} placement="bottom">
            <button
              type="button"
              aria-label={terminalDockLabel}
              aria-pressed={terminalDockOpen}
              data-tauri-drag-region="false"
              onClick={onToggleTerminalDock}
              className="rounded-md p-1 text-ink/55 transition-colors hover:bg-ink/5 hover:text-ink"
            >
              {terminalDockOpen ? (
                <PanelBottomClose className="h-4 w-4" />
              ) : (
                <PanelBottomOpen className="h-4 w-4" />
              )}
            </button>
          </Tooltip>
        )}
        {onToggleRightSidebar && !rightSidebarOpen && (
          <div className={isMac ? "" : "mr-[138px]"}>
            <Tooltip content={rightPanelLabel} placement="bottom">
              <button
                type="button"
                aria-label={rightPanelLabel}
                aria-pressed={rightSidebarOpen ?? false}
                data-tauri-drag-region="false"
                onClick={onToggleRightSidebar}
                className="rounded-md p-1 text-ink/55 transition-opacity duration-300 hover:bg-ink/5 hover:text-ink"
              >
                <PanelRightClose className="h-4 w-4" />
              </button>
            </Tooltip>
          </div>
        )}
      </div>
      <div className="absolute top-0 right-0 z-20">
        <WindowControls />
      </div>
    </div>
  );
}

function ChatViewToggle({
  value,
  onChange,
}: {
  value: ChatView;
  onChange: (next: ChatView) => void;
}) {
  const { t } = useI18n();
  const items: { value: ChatView; icon: LucideIcon; label: string }[] = [
    { value: "chat", icon: MessageSquare, label: t("header.view_chat") },
    { value: "file", icon: FileCodeCorner, label: t("header.view_file") },
    { value: "canvas", icon: Presentation, label: t("header.view_canvas") },
  ];
  return (
    <div
      role="tablist"
      aria-label={t("header.view_label")}
      className="inline-flex items-center rounded-md bg-ink/[0.07] p-0.5"
    >
      {items.map(({ value: itemValue, icon: Icon, label }) => {
        const active = itemValue === value;
        return (
          <Tooltip key={itemValue} content={label} placement="bottom">
            <button
              type="button"
              role="tab"
              aria-selected={active}
              aria-label={label}
              onClick={() => onChange(itemValue)}
              className={
                "flex h-6 w-7 items-center justify-center rounded transition-colors " +
                (active
                  ? "bg-surface text-ink/85 shadow-[0_1px_2px_rgba(0,0,0,0.18)]"
                  : "text-ink/55 hover:text-ink/80")
              }
            >
              <Icon className="h-3.5 w-3.5" />
            </button>
          </Tooltip>
        );
      })}
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
