import { useEffect, useMemo, useState } from "react";
import type { ComponentType } from "react";
import { Files, Info, PanelRightOpen, Workflow, type LucideIcon } from "lucide-react";
import type { ProjectInfo, SessionInfo } from "../api";
import { useI18n } from "../i18n";
import ThreadPage from "../pages/ThreadPage";
import { ProjectWorkbenchPage, type ProjectView } from "../pages/ProjectPage";
import { BitcoinHashesOutlineIcon, HashIcon, Robot3LineIcon } from "./IconifyIcon";
import Tooltip from "./Tooltip";

type IconComponent = LucideIcon | ComponentType<{ className?: string }>;
type RightTab =
  | { kind: "thread" }
  | { kind: "project"; view: ProjectView };

interface AppRightSidebarProps {
  // Context that decides what fills the panel.
  selectedThread: { projectId: string; threadId: string; goal: string } | null;
  selectedSessionProject: ProjectInfo | null;
  selectedThreadProject: ProjectInfo | null;
  // Actions wired from App.tsx so navigation from these pages keeps working.
  onSelectThreadChatSession: (session: SessionInfo) => void;
  onOpenThreadMultiSessionChat: () => void;
  onClose: () => void;
  onError: (message: string | null) => void;
}

function sameTab(a: RightTab, b: RightTab) {
  if (a.kind !== b.kind) return false;
  if (a.kind === "project" && b.kind === "project") return a.view === b.view;
  return true;
}

export default function AppRightSidebar({
  selectedThread,
  selectedSessionProject,
  selectedThreadProject,
  onSelectThreadChatSession,
  onOpenThreadMultiSessionChat,
  onClose,
  onError,
}: AppRightSidebarProps) {
  const { t } = useI18n();

  const threadProject = selectedThreadProject;
  const threadId = selectedThread?.threadId ?? null;
  const hasThread = Boolean(threadProject && threadId);
  // Prefer the thread's project (when a thread is selected) so the project
  // tabs operate on the same context. Fall back to the session's project.
  const project = threadProject ?? selectedSessionProject ?? null;
  const hasProject = Boolean(project);

  const tabs = useMemo(() => {
    const items: { id: string; label: string; icon: IconComponent; tab: RightTab }[] = [];
    if (hasProject) {
      items.push(
        {
          id: "files",
          label: t("project.files"),
          icon: Files,
          tab: { kind: "project", view: "files" },
        },
        {
          id: "threads",
          label: t("thread.title"),
          icon: BitcoinHashesOutlineIcon,
          tab: { kind: "project", view: "threads" },
        },
        {
          id: "workflows",
          label: t("project.processTemplateId"),
          icon: Workflow,
          tab: { kind: "project", view: "stages" },
        },
        {
          id: "assistants",
          label: t("assistant.title"),
          icon: Robot3LineIcon,
          tab: { kind: "project", view: "assistants" },
        },
      );
    }
    if (hasThread) {
      items.push({
        id: "thread",
        label: t("sidebar.right_tab_thread"),
        icon: HashIcon,
        tab: { kind: "thread" },
      });
    }
    return items;
  }, [hasProject, hasThread, t]);

  const defaultTab: RightTab | null = useMemo(() => {
    if (hasProject) return { kind: "project", view: "files" };
    if (hasThread) return { kind: "thread" };
    return null;
  }, [hasProject, hasThread]);

  const [activeTab, setActiveTab] = useState<RightTab | null>(defaultTab);

  // Keep `activeTab` valid as the available tab set changes (e.g. user
  // switches between selecting a session and selecting a thread).
  useEffect(() => {
    if (!defaultTab) {
      if (activeTab !== null) setActiveTab(null);
      return;
    }
    if (!activeTab || !tabs.some((item) => sameTab(item.tab, activeTab))) {
      setActiveTab(defaultTab);
    }
  }, [activeTab, defaultTab, tabs]);

  return (
    <div className="flex h-full min-h-0 w-full flex-col">
      <div
        data-tauri-drag-region
        className="relative flex h-12 shrink-0 items-center justify-between gap-2 border-b border-ink/10 px-3 select-none"
      >
        <div className="flex min-w-0 items-center gap-1">
          {tabs.map(({ id, label, icon: Icon, tab }) => {
            const active = activeTab !== null && sameTab(tab, activeTab);
            return (
              <Tooltip key={id} content={label} placement="bottom">
                <button
                  type="button"
                  aria-label={label}
                  aria-pressed={active}
                  data-tauri-drag-region="false"
                  onClick={() => setActiveTab(tab)}
                  className={
                    "inline-flex h-8 w-8 items-center justify-center rounded-md transition " +
                    (active
                      ? "bg-ink/[0.08] text-ink"
                      : "text-ink/55 hover:bg-ink/5 hover:text-ink/85")
                  }
                >
                  <Icon className="h-4 w-4" />
                </button>
              </Tooltip>
            );
          })}
        </div>
        <Tooltip content={t("sidebar.right_close")} placement="bottom">
          <button
            type="button"
            aria-label={t("sidebar.right_close")}
            data-tauri-drag-region="false"
            onClick={onClose}
            className="rounded-md p-1 text-ink/55 transition hover:bg-ink/5 hover:text-ink"
          >
            <PanelRightOpen className="h-4 w-4" />
          </button>
        </Tooltip>
      </div>
      <div className="flex flex-1 min-h-0 flex-col overflow-hidden">
        {activeTab?.kind === "thread" && threadProject && threadId ? (
          <ThreadPage
            project={threadProject}
            threadId={threadId}
            transparent
            onSelectSession={onSelectThreadChatSession}
            onOpenMultiSessionChat={onOpenThreadMultiSessionChat}
            onError={onError}
          />
        ) : activeTab?.kind === "project" && project ? (
          <ProjectWorkbenchPage
            project={project}
            view={activeTab.view}
            hideTabs
            onSelectThreadChatSession={onSelectThreadChatSession}
            onError={onError}
          />
        ) : (
          <div className="flex flex-1 min-h-0 flex-col items-center justify-center gap-2 px-6 text-center text-ink/45">
            <Info className="h-5 w-5 text-ink/35" />
            <p className="text-body-sm leading-snug">
              {t("sidebar.right_empty_hint")}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
