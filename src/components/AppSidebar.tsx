import { type MouseEvent, type ReactNode, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { createPortal } from "react-dom";
import {
  Brain,
  CalendarClock,
  ChevronDown,
  CircleAlert,
  Download,
  Folder,
  FolderOpen,
  FolderPlus,
  Key,
  Kanban,
  LoaderCircle,
  MailPlus,
  MessagesSquare,
  PanelLeftClose,
  Settings,
  SquarePen,
  Swords,
  Workflow,
  X,
} from "lucide-react";
import {
  ProjectInfo,
  SessionInfo,
  ProcessTemplateInfo,
  addExistingProject,
  createDefaultProject,
  listProcessTemplateStages,
  listProcessTemplates,
  type ProjectStageInfo,
  type ThreadIndexItemInfo,
  type ThreadKind,
} from "../api";
import { useI18n } from "../i18n";
import type { ProjectGroup } from "../navigation";
import {
  aggregateLiveSessionActivity,
  liveSessionActivity,
  type LiveRuntimeState,
} from "../runtimeChat";
import { sessionDisplayTitle } from "../appUtils";
import { useUpdateCheck } from "../updater";
import { AgentGlyph } from "./AgentIcon";
import IconifyIcon, { HashIcon, PeopleTeam24RegularIcon } from "./IconifyIcon";
import PopupMenu, { type PopupMenuOption } from "./PopupMenu";
import { RuntimeMenuSelect } from "./RuntimeMenuSelect";
import ScrollArea from "./ScrollArea";
import StageSelectChip from "./StageSelectChip";
import Tooltip from "./Tooltip";

const SIDEBAR_SESSION_PREVIEW_LIMIT = 5;

type SidebarSessionEntry = {
  kind: "session";
  session: SessionInfo;
  time: number;
};

type SidebarThreadChatEntry = {
  kind: "thread";
  thread: ThreadIndexItemInfo;
  time: number;
};

type SidebarListEntry = SidebarSessionEntry | SidebarThreadChatEntry;

type SidebarThreadRef = {
  id: string;
  goal: string;
  kind: ThreadKind;
  createdAt: number;
  updatedAt: number;
};

type AppSidebarProps = {
  isMac: boolean;
  projectSectionExpanded: boolean;
  projectGroups: ProjectGroup[];
  expandedProjects: Set<string>;
  expandedProjectSessions: Set<string>;
  threadIndexItems: ThreadIndexItemInfo[];
  selectedKey: string | null;
  selectedIdentityKey: string | null;
  selectedProjectId: string | null;
  selectedThreadId: string | null;
  hasActiveSelection: boolean;
  liveState: LiveRuntimeState;
  runtimeSessionAliases: Record<string, string>;
  unreadSessionIds: Set<string>;
  update: ReturnType<typeof useUpdateCheck>;
  onCloseSidebar: () => void;
  onNewChat: () => void;
  onToggleProjectSection: () => void;
  onProjectAdded: (project: ProjectInfo) => void;
  onToggleProjectExpanded: (projectKey: string) => void;
  onOpenKanban: (project: ProjectGroup) => void;
  onNewProjectChat: (project: ProjectGroup) => void;
  onSelectSession: (project: ProjectGroup, session: SessionInfo) => void;
  onSelectThread: (
    project: ProjectGroup,
    thread: SidebarThreadRef,
    source: "thread" | "threadChat",
  ) => void;
  onToggleProjectSessions: (projectKey: string) => void;
  onProjectContextMenu: (project: ProjectGroup, e: MouseEvent) => void;
  onSessionContextMenu: (
    session: SessionInfo,
    pos: { x: number; y: number },
  ) => void;
  onOpenSettings: () => void;
  onOpenAutoTasks: () => void;
  autoTasksActive: boolean;
  onInstallUpdate: () => void;
  onError: (error: string | null) => void;
};

export default function AppSidebar({
  isMac,
  projectSectionExpanded,
  projectGroups,
  expandedProjects,
  expandedProjectSessions,
  threadIndexItems,
  selectedKey,
  selectedIdentityKey,
  selectedProjectId,
  selectedThreadId,
  hasActiveSelection,
  liveState,
  runtimeSessionAliases,
  unreadSessionIds,
  update,
  onCloseSidebar,
  onNewChat,
  onToggleProjectSection,
  onProjectAdded,
  onToggleProjectExpanded,
  onOpenKanban,
  onNewProjectChat,
  onSelectSession,
  onSelectThread,
  onToggleProjectSessions,
  onProjectContextMenu,
  onSessionContextMenu,
  onOpenSettings,
  onOpenAutoTasks,
  autoTasksActive,
  onInstallUpdate,
  onError,
}: AppSidebarProps) {
  const { t } = useI18n();
  const threadIndexByProject = useMemo(() => {
    const grouped = new Map<string, ThreadIndexItemInfo[]>();
    for (const item of threadIndexItems) {
      const current = grouped.get(item.projectId);
      if (current) current.push(item);
      else grouped.set(item.projectId, [item]);
    }
    return grouped;
  }, [threadIndexItems]);

  return (
    <aside
      className="flex h-full min-w-0 flex-col overflow-hidden border-r border-ink/5 bg-surface-sidebar"
    >
      <div data-tauri-drag-region className="relative h-12 w-full shrink-0">
        {!isMac && (
          <span className="absolute left-3 top-1/2 -translate-y-1/2 text-title font-semibold text-ink/85 pointer-events-none select-none">
            Sessio
          </span>
        )}
        <Tooltip content={t("sidebar.close")} placement="bottom">
          <button
            type="button"
            aria-label={t("sidebar.close")}
            data-tauri-drag-region="false"
            onClick={onCloseSidebar}
            className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-ink/55 hover:text-ink transition rounded-md"
          >
            <PanelLeftClose className="w-4 h-4" />
          </button>
        </Tooltip>
      </div>

      <nav className="flex-1 min-h-0 w-full p-2 pb-0 flex flex-col gap-0.5">
        <button
          type="button"
          onClick={onNewChat}
          className={
            "mb-2 flex h-8 w-full items-center gap-2 rounded-md px-2.5 text-left text-body-sm font-medium transition " +
            (!hasActiveSelection && !autoTasksActive
              ? "bg-ink/10 text-ink"
              : "text-ink/72 hover:bg-ink/5 hover:text-ink")
          }
        >
          <SquarePen className="h-4 w-4 shrink-0" />
          <span className="truncate">{t("sidebar.new_chat")}</span>
        </button>
        <button
          type="button"
          onClick={onOpenAutoTasks}
          className={
            "mb-2 flex h-8 w-full items-center gap-2 rounded-md px-2.5 text-left text-body-sm font-medium transition " +
            (autoTasksActive
              ? "bg-ink/10 text-ink"
              : "text-ink/72 hover:bg-ink/5 hover:text-ink")
          }
        >
          <CalendarClock className="h-4 w-4 shrink-0" />
          <span className="truncate">{t("sidebar.auto_tasks")}</span>
        </button>
        <div className="shrink-0 flex flex-col gap-0.5">
          <div className="flex items-center gap-1">
            <SectionHeader
              label={t("sidebar.by_project")}
              collapsed={!projectSectionExpanded}
              onToggle={onToggleProjectSection}
              action={
                <ProjectActionsButton
                  onProjectAdded={onProjectAdded}
                  onError={onError}
                />
              }
            />
          </div>
        </div>

        {projectSectionExpanded && (
          <ScrollArea
            className="flex-1 min-h-0 -mr-2"
            viewportClassName="pr-3 flex flex-col gap-1"
          >
            {projectGroups.map((project) => (
              <ProjectSidebarGroup
                key={project.key}
                project={project}
                expanded={expandedProjects.has(project.key)}
                sessionsExpanded={expandedProjectSessions.has(project.key)}
                selectedKey={selectedKey}
                selectedIdentityKey={selectedIdentityKey}
                selectedProjectId={selectedProjectId}
                selectedThreadId={selectedThreadId}
                liveState={liveState}
                runtimeSessionAliases={runtimeSessionAliases}
                unreadSessionIds={unreadSessionIds}
                threadIndexItems={threadIndexByProject.get(project.project.id) ?? []}
                onSelectProject={() => onToggleProjectExpanded(project.key)}
                onOpenKanban={() => onOpenKanban(project)}
                onNewChat={() => onNewProjectChat(project)}
                onSelectSession={(session) => onSelectSession(project, session)}
                onSelectThread={(thread, source) => onSelectThread(project, thread, source)}
                onToggleSessionLimit={() => onToggleProjectSessions(project.key)}
                onProjectContextMenu={(event) => onProjectContextMenu(project, event)}
                onSessionContextMenu={onSessionContextMenu}
              />
            ))}
          </ScrollArea>
        )}
      </nav>

      <SidebarFooter
        update={update}
        onOpenSettings={onOpenSettings}
        onInstallUpdate={onInstallUpdate}
      />
    </aside>
  );
}

function SidebarFooter({
  update,
  onOpenSettings,
  onInstallUpdate,
}: {
  update: ReturnType<typeof useUpdateCheck>;
  onOpenSettings: () => void;
  onInstallUpdate: () => void;
}) {
  const { t } = useI18n();

  return (
    <div className="relative w-full border-t border-ink/10">
      <div className="px-3 py-2 flex items-center justify-between gap-2">
        <div className="shrink-0 flex items-center gap-1">
          <Tooltip content={t("sidebar.settings")} placement="top">
            <button
              type="button"
              aria-label={t("sidebar.settings")}
              onClick={onOpenSettings}
              className="p-1 text-ink/55 hover:text-ink transition rounded-md"
            >
              <Settings className="w-4 h-4" />
            </button>
          </Tooltip>
        </div>
        <div className="shrink-0 flex items-center justify-end gap-1">
          {update.hasUpdate && update.latestVersion && (
            <Tooltip
              content={
                update.installing
                  ? updateProgressLabel(update, t)
                  : update.canInstall
                    ? t("sidebar.update_available_install", { version: update.latestVersion })
                    : t("sidebar.update_available_download", { version: update.latestVersion })
              }
              placement="top"
            >
              <button
                type="button"
                aria-label={
                  update.installing
                    ? updateProgressLabel(update, t)
                    : update.canInstall
                      ? t("sidebar.update_available_install", { version: update.latestVersion })
                      : t("sidebar.update_available_download", { version: update.latestVersion })
                }
                disabled={update.installing}
                onClick={onInstallUpdate}
                className="relative p-1 text-ink/55 transition rounded-md hover:text-ink disabled:cursor-default disabled:opacity-70"
              >
                {update.installing ? (
                  <LoaderCircle className="w-4 h-4 animate-spin" />
                ) : (
                  <Download className="w-4 h-4" />
                )}
              </button>
            </Tooltip>
          )}
        </div>
      </div>
    </div>
  );
}

function updateProgressLabel(
  update: ReturnType<typeof useUpdateCheck>,
  t: (key: string, vars?: Record<string, string | number>) => string,
): string {
  if (update.totalBytes && update.totalBytes > 0) {
    const pct = Math.max(
      0,
      Math.min(99, Math.round((update.downloadedBytes / update.totalBytes) * 100)),
    );
    return t("sidebar.update_installing_progress", { progress: pct });
  }
  return t("sidebar.update_installing");
}

function SectionHeader({
  label,
  collapsed,
  onToggle,
  action,
}: {
  label: string;
  collapsed: boolean;
  onToggle: () => void;
  action?: ReactNode;
}) {
  return (
    <div className="group flex w-full items-center px-2 mt-3 mb-1 text-caption text-ink/55 transition hover:text-ink/85">
      <button
        type="button"
        onClick={onToggle}
        className="flex min-w-0 flex-1 items-center justify-between text-left text-body"
      >
        <span>{label}</span>
      </button>
      {action && (
        <span className="mr-1 opacity-0 transition-opacity group-hover:opacity-100">
          {action}
        </span>
      )}
      <button type="button" onClick={onToggle} className="rounded p-0.5 text-ink/45 hover:text-ink/80">
        <Chevron collapsed={collapsed} />
      </button>
    </div>
  );
}

function Chevron({ collapsed }: { collapsed: boolean }) {
  return (
    <ChevronDown
      className={
        "w-3.5 h-3.5 transition-transform duration-150 " +
        (collapsed ? "-rotate-90" : "")
      }
    />
  );
}

function processOptions(processTemplates: ProcessTemplateInfo[]) {
  return processTemplates.map((process) => ({
    value: process.id,
    label: process.name,
  }));
}

function ProjectActionsButton({
  onProjectAdded,
  onError,
}: {
  onProjectAdded: (project: ProjectInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [openMenu, setOpenMenu] = useState(false);
  const [form, setForm] = useState<null | { mode: "existing" | "new"; basePath: string | null; name: string; processTemplateId: string; enabledStageIds: string[] }>(null);
  const [processTemplates, setProcessTemplates] = useState<ProcessTemplateInfo[]>([]);
  const [processStages, setProcessTemplateStages] = useState<Record<string, ProjectStageInfo[]>>({});
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuOptions: PopupMenuOption<"existing" | "new">[] = [
    {
      key: "existing",
      label: t("project.add_existing"),
      icon: <Folder className="h-4 w-4" />,
    },
    {
      key: "new",
      label: t("project.create_new"),
      icon: <FolderPlus className="h-4 w-4" />,
    },
  ];

  const defaultEnabledStageIds = (stages: ProjectStageInfo[]) =>
    stages.filter((stage) => stage.type === "builtin" && stage.enabled).map((stage) => stage.id);

  const loadProcessTemplateStages = async (processTemplateId: string) => {
    if (processStages[processTemplateId]) return processStages[processTemplateId];
    const stages = await listProcessTemplateStages(processTemplateId);
    setProcessTemplateStages((prev) => ({ ...prev, [processTemplateId]: stages }));
    return stages;
  };

  const updateProcessTemplate = async (processTemplateId: string) => {
    const stages = await loadProcessTemplateStages(processTemplateId);
    setForm((current) =>
      current
        ? { ...current, processTemplateId, enabledStageIds: defaultEnabledStageIds(stages) }
        : current,
    );
  };

  const toggleStage = (stageId: string) => {
    setForm((current) => {
      if (!current) return current;
      const selected = new Set(current.enabledStageIds);
      if (selected.has(stageId)) selected.delete(stageId);
      else selected.add(stageId);
      return { ...current, enabledStageIds: Array.from(selected) };
    });
  };

  const pickExistingProject = async () => {
    setOpenMenu(false);
    setFormError(null);
    try {
      const selection = await open({ directory: true, multiple: false });
      if (typeof selection !== "string") return;
      const processRows = processTemplates.length > 0 ? processTemplates : await listProcessTemplates();
      setProcessTemplates(processRows);
      const processTemplateId = processRows[0]?.id ?? "code";
      const stages = await loadProcessTemplateStages(processTemplateId);
      const defaultName = selection.split(/[\\/]/).filter(Boolean).pop() ?? "";
      setForm({ mode: "existing", basePath: selection, name: defaultName, processTemplateId, enabledStageIds: defaultEnabledStageIds(stages) });
    } catch (err) {
      onError(String(err));
    }
  };

  const openNewProjectForm = async () => {
    setOpenMenu(false);
    setFormError(null);
    try {
      const processRows = processTemplates.length > 0 ? processTemplates : await listProcessTemplates();
      setProcessTemplates(processRows);
      const processTemplateId = processRows[0]?.id ?? "code";
      const stages = await loadProcessTemplateStages(processTemplateId);
      setForm({ mode: "new", basePath: null, name: "", processTemplateId, enabledStageIds: defaultEnabledStageIds(stages) });
    } catch (err) {
      onError(String(err));
    }
  };

  const save = async () => {
    if (!form || saving) return;
    const name = form.name.trim();
    if (!name) {
      setFormError(t("project.name_required"));
      return;
    }
    setSaving(true);
    onError(null);
    setFormError(null);
    try {
      const project =
        form.mode === "existing"
          ? await addExistingProject(form.basePath ?? "", name, form.processTemplateId, form.enabledStageIds)
          : await createDefaultProject(name, form.processTemplateId, form.enabledStageIds);
      onProjectAdded(project);
      setForm(null);
    } catch (err) {
      setFormError(String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <Tooltip content={t("project.add")} placement="top">
        <button
          ref={buttonRef}
          type="button"
          onClick={() => setOpenMenu((value) => !value)}
          className="rounded-md p-0.5 text-ink/45 transition hover:bg-ink/5 hover:text-ink"
          aria-label={t("project.add")}
        >
          <FolderPlus className="h-4 w-4" />
        </button>
      </Tooltip>
      {openMenu && buttonRef.current && (
        <PopupMenu
          anchor={buttonRef.current}
          options={menuOptions}
          placement="right"
          onClose={() => setOpenMenu(false)}
          onSelect={(key) => {
            if (key === "existing") void pickExistingProject();
            else void openNewProjectForm();
          }}
        />
      )}
      {form && createPortal(
        <div className="fixed inset-0 z-[90] flex items-center justify-center bg-black/35 px-4">
          <div className="w-full max-w-[420px] rounded-xl border border-ink/10 bg-surface-panel p-4 shadow-2xl">
            <div className="mb-3 flex items-center justify-between gap-3">
              <div>
                <div className="text-body font-medium text-ink">
                  {form.mode === "existing" ? t("project.add_existing") : t("project.create_new")}
                </div>
                <div className="mt-1 truncate text-caption text-ink/45">
                  {form.mode === "existing" ? form.basePath : "~/.sessio/projects"}
                </div>
              </div>
              <button type="button" onClick={() => setForm(null)} className="rounded-md p-1 text-ink/45 hover:bg-ink/5 hover:text-ink">
                <X className="h-4 w-4" />
              </button>
            </div>
            <label className="mb-3 block">
              <span className="mb-1 block text-caption uppercase tracking-wide text-ink/45">{t("project.name")}</span>
              <input
                value={form.name}
                onChange={(event) => setForm({ ...form, name: event.target.value })}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void save();
                  if (event.key === "Escape") setForm(null);
                }}
                autoFocus
                className="w-full rounded-md border border-ink/10 bg-ink/5 px-3 py-2 text-body text-ink outline-none focus:border-ink/25"
              />
            </label>
            {formError && (
              <div className="mb-3 rounded-md border border-status-error/25 bg-status-error/10 px-3 py-2 text-body-sm text-status-error">
                {formError}
              </div>
            )}
            <div className="mb-4 flex items-center justify-between gap-3">
              <span className="text-body-sm text-ink/55">{t("project.processTemplateId")}</span>
              <RuntimeMenuSelect
                ariaLabel={t("project.processTemplateId")}
                value={form.processTemplateId}
                options={processOptions(processTemplates)}
                onChange={(value) => void updateProcessTemplate(value)}
                portalZIndex={100}
              />
            </div>
            <div className="mb-4">
              <div className="mb-2 text-body-sm text-ink/55">{t("stage.project_stages")}</div>
              <div className="flex flex-wrap gap-1.5">
                {(processStages[form.processTemplateId] ?? []).filter((stage) => stage.type === "builtin").map((stage) => {
                  const selected = form.enabledStageIds.includes(stage.id);
                  return (
                    <StageSelectChip
                      key={stage.id}
                      stage={stage}
                      selected={selected}
                      onToggle={toggleStage}
                    />
                  );
                })}
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setForm(null)} className="rounded-md px-3 py-1.5 text-body-sm text-ink/60 hover:bg-ink/5 hover:text-ink">
                {t("delete.cancel")}
              </button>
              <button type="button" disabled={saving} onClick={() => void save()} className="rounded-md bg-ink px-3 py-1.5 text-body-sm text-[rgb(var(--color-bg-panel))] disabled:opacity-50">
                {saving ? t("new_chat.sending") : t("project.save")}
              </button>
            </div>
          </div>
        </div>,
        document.body,
      )}
    </>
  );
}

function ProjectSidebarGroup({
  project,
  expanded,
  sessionsExpanded,
  selectedKey,
  selectedIdentityKey,
  selectedProjectId,
  selectedThreadId,
  liveState,
  runtimeSessionAliases,
  unreadSessionIds,
  threadIndexItems,
  onSelectProject,
  onOpenKanban,
  onNewChat,
  onSelectSession,
  onSelectThread,
  onToggleSessionLimit,
  onProjectContextMenu,
  onSessionContextMenu,
}: {
  project: ProjectGroup;
  expanded: boolean;
  sessionsExpanded: boolean;
  selectedKey: string | null;
  selectedIdentityKey: string | null;
  selectedProjectId: string | null;
  selectedThreadId: string | null;
  liveState: LiveRuntimeState;
  runtimeSessionAliases: Record<string, string>;
  unreadSessionIds: Set<string>;
  threadIndexItems: ThreadIndexItemInfo[];
  onSelectProject: () => void;
  onOpenKanban: () => void;
  onNewChat: () => void;
  onSelectSession: (session: SessionInfo) => void;
  onSelectThread: (thread: SidebarThreadRef, source: "thread" | "threadChat") => void;
  onToggleSessionLimit: () => void;
  onProjectContextMenu: (e: MouseEvent) => void;
  onSessionContextMenu: (
    session: SessionInfo,
    pos: { x: number; y: number },
  ) => void;
}) {
  const { t } = useI18n();
  const projectButtonRef = useRef<HTMLButtonElement>(null);
  const FolderIcon = expanded ? FolderOpen : Folder;
  const sidebarEntries = useMemo<SidebarListEntry[]>(() => {
    const linkedSessionKeys = new Set<string>();
    const threadEntries: SidebarThreadChatEntry[] = [];
    for (const item of threadIndexItems) {
      for (const key of item.sessionKeys) linkedSessionKeys.add(key);
      threadEntries.push({
        kind: "thread",
        thread: item,
        time: item.time,
      });
    }

    const byKey = new Map<string, SidebarSessionEntry>();
    for (const session of project.sessions) {
      const key = sessionIdentityKey(session);
      if (linkedSessionKeys.has(key)) continue;
      const time = sessionTime(session);
      const current = byKey.get(key);
      if (!current || time > current.time) {
        byKey.set(key, { kind: "session", session, time });
      }
    }

    return [...Array.from(byKey.values()), ...threadEntries].sort((a, b) => b.time - a.time);
  }, [project.sessions, threadIndexItems]);
  const visibleEntries = sessionsExpanded
    ? sidebarEntries
    : sidebarEntries.slice(0, SIDEBAR_SESSION_PREVIEW_LIMIT);
  const canToggleSessionLimit =
    sidebarEntries.length > SIDEBAR_SESSION_PREVIEW_LIMIT;
  const displayCount = sidebarEntries.length;
  const projectActive = selectedProjectId === project.project.id;
  const toggleSessionLimit = () => {
    const collapsing = sessionsExpanded;
    onToggleSessionLimit();
    if (collapsing) {
      requestAnimationFrame(() => {
        projectButtonRef.current?.scrollIntoView({ block: "nearest" });
      });
    }
  };
  return (
    <div>
      <button
        ref={projectButtonRef}
        type="button"
        onClick={onSelectProject}
        className={
          "group flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition " +
          (projectActive ? "bg-ink/10 text-ink" : "text-ink/70 hover:bg-ink/5 hover:text-ink")
        }
        onContextMenu={onProjectContextMenu}
      >
        <FolderIcon
          className={
            "w-3.5 h-3.5 shrink-0 text-ink/55 transition-transform duration-200 " +
            (expanded ? "scale-105" : "scale-100")
          }
        />
        <span className="min-w-0 flex-1 truncate text-body" title={project.path ?? project.label}>{project.label}</span>
        <span className="text-meta text-ink/40 tabular-nums group-hover:hidden">
          {displayCount}
        </span>
        <span className="ml-auto hidden shrink-0 items-center gap-0.5 group-hover:flex">
          <Tooltip content={t("project.workbench")} placement="top">
            <span
              role="button"
              tabIndex={0}
              aria-label={t("project.workbench")}
              onClick={(event) => {
                event.stopPropagation();
                onOpenKanban();
              }}
              onKeyDown={(event) => {
                if (event.key !== "Enter" && event.key !== " ") return;
                event.preventDefault();
                event.stopPropagation();
                onOpenKanban();
              }}
              className="rounded p-0.5 text-ink/35 transition hover:bg-ink/[0.08] hover:text-ink/75"
            >
              <Kanban className="h-4 w-4" />
            </span>
          </Tooltip>
          <Tooltip content={t("sidebar.new_chat")} placement="top">
            <span
              role="button"
              tabIndex={0}
              aria-label={t("sidebar.new_chat")}
              onClick={(event) => {
                event.stopPropagation();
                onNewChat();
              }}
              onKeyDown={(event) => {
                if (event.key !== "Enter" && event.key !== " ") return;
                event.preventDefault();
                event.stopPropagation();
                onNewChat();
              }}
              className="rounded p-0.5 text-ink/35 transition hover:bg-ink/[0.08] hover:text-ink/75"
            >
              <SquarePen className="h-4 w-4" />
            </span>
          </Tooltip>
        </span>
      </button>
      <div
        className={
          "grid overflow-hidden transition-[grid-template-rows,opacity] duration-200 ease-out " +
          (expanded ? "grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0")
        }
      >
        <div className="min-h-0 overflow-hidden">
          <div
            className={
              "mt-0.5 flex flex-col gap-0.5 transition-transform duration-200 ease-out " +
              (expanded ? "translate-y-0" : "-translate-y-1")
            }
          >
            {visibleEntries.map((entry) => {
              if (entry.kind === "thread") {
                const threadLiveSessions = entry.thread.sessionKeys.map((sessionIdKey) => {
                  const runtimeId = runtimeSessionAliases[sessionIdKey];
                  return runtimeId ? liveState.sessions[runtimeId] : undefined;
                });
                const threadActivity = aggregateLiveSessionActivity(threadLiveSessions);
                const threadUnread = entry.thread.sessionKeys.some((sessionIdKey) => {
                  if (unreadSessionIds.has(sessionIdKey)) return true;
                  const runtimeId = runtimeSessionAliases[sessionIdKey];
                  return runtimeId ? unreadSessionIds.has(runtimeId) : false;
                });
                return (
                  <SidebarThreadItem
                    key={entry.thread.threadId}
                    thread={threadRefFromIndexItem(entry.thread)}
                    time={entry.time}
                    active={selectedThreadId === entry.thread.threadId}
                    liveActivity={threadActivity}
                    unread={threadUnread}
                    onSelect={() => onSelectThread(threadRefFromIndexItem(entry.thread), "threadChat")}
                    onOpenThread={() => onSelectThread(threadRefFromIndexItem(entry.thread), "thread")}
                  />
                );
              }
              const { session } = entry;
              const key = sessionKey(session);
              const unreadKeys = sessionUnreadKeys(session, runtimeSessionAliases);
              const runtimeSessionId = runtimeSessionAliases[sessionIdentityKey(session)] ?? session.id;
              const liveActivity = liveSessionActivity(liveState.sessions[runtimeSessionId]);
              return (
                <SidebarSessionItem
                  key={key}
                  item={session}
                  source="project"
                  active={selectedKey === key || selectedIdentityKey === sessionIdentityKey(session)}
                  liveActivity={liveActivity}
                  unread={intersectsSet(unreadKeys, unreadSessionIds)}
                  onSelect={() => onSelectSession(session)}
                  onContextMenu={(event) => {
                    event.preventDefault();
                    onSessionContextMenu(session, { x: event.clientX, y: event.clientY });
                  }}
                />
              );
            })}
          </div>
          {canToggleSessionLimit && (
            <button
              type="button"
              onClick={toggleSessionLimit}
              className="mt-0.5 ml-7 px-1 py-1 text-left text-body-sm text-ink/40 transition hover:text-ink/65"
            >
              {t(sessionsExpanded ? "list.show_less" : "list.show_more")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function SidebarSessionItem({
  item,
  source,
  active,
  liveActivity,
  unread,
  onSelect,
  onContextMenu,
}: {
  item: SessionInfo;
  source: "project" | "thread";
  active: boolean;
  liveActivity: ReturnType<typeof liveSessionActivity>;
  unread: boolean;
  onSelect: () => void;
  onContextMenu: (e: MouseEvent) => void;
}) {
  const { t } = useI18n();
  const title = sessionDisplayTitle(item) ?? t("list.no_user_message");
  const channelLabel = channelPlatformLabel(item.channel?.platform);
  const channelIconClass = channelPlatformIconClass(item.channel?.platform);
  const relativeTime = formatShortRelativeTime(item.updatedAt ?? item.startedAt, t);
  return (
    <button
      type="button"
      onClick={onSelect}
      onContextMenu={onContextMenu}
      title={channelLabel ? `${title} · ${channelLabel}` : title}
      className={
        "group relative flex w-full items-center gap-2 rounded-md py-1.5 pl-7 pr-2 text-left transition " +
        (active
          ? "bg-ink/10 text-ink"
          : "text-ink/65 hover:bg-ink/5 hover:text-ink")
      }
    >
      <SidebarSessionStatus activity={liveActivity} unread={unread} />
      <span className="relative flex h-4 w-4 shrink-0 items-center justify-center text-ink">
        {source === "thread" && (
          <HashIcon className="absolute -left-1 top-1/2 h-3 w-3 -translate-y-1/2 text-ink/45" />
        )}
        {channelIconClass ? (
          <IconifyIcon
            iconClassName={channelIconClass}
            className={
              "h-3.5 w-3.5 " +
              (source === "thread" ? "relative z-10 translate-x-1" : "")
            }
          />
        ) : (
          <AgentGlyph
            agent={item.agent}
            className={
              "h-3.5 w-3.5 " +
              (source === "thread" ? "relative z-10 translate-x-1" : "")
            }
          />
        )}
      </span>
      <span
        className={
          "min-w-0 flex-1 truncate text-body-sm leading-snug " +
          (item.archived ? "text-ink/45" : "text-inherit")
        }
      >
        {sessionDisplayTitle(item) ?? (
          <span className="text-ink/30">{t("list.no_user_message")}</span>
        )}
      </span>
      <span className="shrink-0 text-meta tabular-nums text-ink/35">
        {relativeTime}
      </span>
    </button>
  );
}

function channelPlatformLabel(platform: string | null | undefined): string | null {
  switch (platform) {
    case "telegram":
      return "Telegram";
    case "discord":
      return "Discord";
    case "feishu":
      return "Lark";
    case "lark":
      return "Lark";
    case "wework":
      return "WeWork";
    case "wechat":
      return "WeChat";
    default:
      return null;
  }
}

function channelPlatformIconClass(platform: string | null | undefined): string | null {
  switch (platform) {
    case "telegram":
      return "icon-[streamline-logos--telegram-logo-2-solid]";
    case "discord":
      return "icon-[streamline-logos--discord-logo-2-solid]";
    case "feishu":
    case "lark":
      return "icon-[icon-park-outline--new-lark]";
    case "wechat":
      return "icon-[streamline-logos--wechat-logo-solid]";
    default:
      return null;
  }
}

function SidebarThreadItem({
  thread,
  time,
  active,
  liveActivity,
  unread,
  onSelect,
  onOpenThread,
}: {
  thread: SidebarThreadRef;
  time?: number;
  active: boolean;
  liveActivity: ReturnType<typeof liveSessionActivity>;
  unread: boolean;
  onSelect: () => void;
  onOpenThread?: () => void;
}) {
  const { t } = useI18n();
  const relativeTime = formatShortRelativeTime(time ?? thread.updatedAt ?? thread.createdAt, t);
  const showThreadPageButton = Boolean(onOpenThread);
  return (
    <button
      type="button"
      onClick={onSelect}
      title={thread.goal}
      className={
        "group relative flex w-full items-center gap-2 rounded-md py-1.5 pl-7 pr-2 text-left transition " +
        (active ? "bg-ink/10 text-ink" : "text-ink/65 hover:bg-ink/5 hover:text-ink")
      }
    >
      <SidebarSessionStatus activity={liveActivity} unread={unread} />
      <SidebarThreadStatusIcon kind={thread.kind} />
      <span className="min-w-0 flex-1 truncate text-body-sm leading-snug">
        {thread.goal || <span className="text-ink/30">{t("thread.goal_placeholder")}</span>}
      </span>
      {showThreadPageButton && onOpenThread ? (
        <>
          <span className="shrink-0 text-meta tabular-nums text-ink/35 group-hover:hidden">
            {relativeTime}
          </span>
          <span className="ml-auto hidden shrink-0 items-center gap-0.5 group-hover:flex">
            <Tooltip content={t("thread.open_detail_page")} placement="top">
              <span
                role="button"
                tabIndex={0}
                aria-label={t("thread.open_detail_page")}
                onClick={(event) => {
                  event.stopPropagation();
                  onOpenThread();
                }}
                onKeyDown={(event) => {
                  if (event.key !== "Enter" && event.key !== " ") return;
                  event.preventDefault();
                  event.stopPropagation();
                  onOpenThread();
                }}
                className="flex h-4 w-4 items-center justify-center rounded text-ink/35 transition hover:bg-ink/[0.08] hover:text-ink/75"
              >
                <HashIcon className="h-3 w-3" />
              </span>
            </Tooltip>
          </span>
        </>
      ) : (
        <span className="shrink-0 text-meta tabular-nums text-ink/35">
          {relativeTime}
        </span>
      )}
    </button>
  );
}

function threadRefFromIndexItem(item: ThreadIndexItemInfo): SidebarThreadRef {
  return {
    id: item.threadId,
    goal: item.goal,
    kind: item.kind,
    createdAt: item.createdAt,
    updatedAt: item.updatedAt,
  };
}

function SidebarThreadStatusIcon({ kind }: { kind: ThreadKind }) {
  const Icon = threadKindSidebarIcon(kind);
  return (
    <span className="flex h-4 w-4 shrink-0 items-center justify-center text-ink">
      <Icon className="h-3.5 w-3.5" />
    </span>
  );
}

function threadKindSidebarIcon(kind: ThreadKind) {
  switch (kind) {
    case "process":
      return Workflow;
    case "teamwork":
      return PeopleTeam24RegularIcon;
    case "brainstorm":
      return Brain;
    case "debate":
      return Swords;
    default:
      return MessagesSquare;
  }
}

function SidebarSessionStatus({
  activity,
  unread,
}: {
  activity: ReturnType<typeof liveSessionActivity>;
  unread: boolean;
}) {
  if (activity === "permission") {
    return (
      <span className="pointer-events-none absolute left-2 top-1/2 flex h-3.5 w-3.5 -translate-y-1/2 items-center justify-center text-status-warn">
        <Key className="h-3 w-3" />
      </span>
    );
  }
  if (activity === "failed") {
    return (
      <span className="pointer-events-none absolute left-2 top-1/2 flex h-3.5 w-3.5 -translate-y-1/2 items-center justify-center text-status-error">
        <CircleAlert className="h-3 w-3" />
      </span>
    );
  }
  if (activity === "running") {
    return (
      <span className="pointer-events-none absolute left-2 top-1/2 flex h-3.5 w-3.5 -translate-y-1/2 items-center justify-center text-brand">
        <LoaderCircle className="h-3 w-3 animate-spin" />
      </span>
    );
  }
  if (!unread) return null;
  return (
    <span className="pointer-events-none absolute left-2 top-1/2 flex h-3.5 w-3.5 -translate-y-1/2 items-center justify-center text-brand">
      <MailPlus className="h-3 w-3" />
    </span>
  );
}

function sessionKey(s: SessionInfo): string {
  return `${s.agent}:${s.filePath}:${s.id}`;
}

function sessionIdentityKey(s: SessionInfo): string {
  return `${s.agent}:${s.id}`;
}

function sessionTime(session: SessionInfo): number {
  return session.updatedAt ?? session.startedAt ?? 0;
}

function sessionUnreadKeys(
  session: SessionInfo,
  runtimeSessionAliases: Record<string, string>,
): string[] {
  const keys = new Set<string>([session.id, sessionIdentityKey(session)]);
  const runtimeSessionId = runtimeSessionAliases[sessionIdentityKey(session)];
  if (runtimeSessionId) keys.add(runtimeSessionId);
  return Array.from(keys);
}

function intersectsSet(keys: Iterable<string>, lookup: Set<string>): boolean {
  for (const key of keys) {
    if (lookup.has(key)) return true;
  }
  return false;
}

function formatShortRelativeTime(ts: number | null, t: (key: string, vars?: Record<string, string | number>) => string): string {
  if (!ts) return "";
  const diffMs = Math.max(0, Date.now() - ts);
  const minute = 60 * 1000;
  const hour = 60 * minute;
  const day = 24 * hour;
  const week = 7 * day;
  const month = 30 * day;
  if (diffMs < hour) {
    return t("time.minute", { count: Math.max(1, Math.floor(diffMs / minute)) });
  }
  if (diffMs < day) return t("time.hour", { count: Math.floor(diffMs / hour) });
  if (diffMs < week) return t("time.day", { count: Math.floor(diffMs / day) });
  if (diffMs < month) return t("time.week", { count: Math.floor(diffMs / week) });
  return t("time.month", { count: Math.floor(diffMs / month) });
}
