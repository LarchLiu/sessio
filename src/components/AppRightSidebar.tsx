import { PanelRightOpen, Info } from "lucide-react";
import type { ProjectInfo, SessionInfo } from "../api";
import { useI18n } from "../i18n";
import ThreadPage from "../pages/ThreadPage";
import { ProjectWorkbenchPage } from "../pages/ProjectPage";
import Tooltip from "./Tooltip";

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
  const sessionProject = !selectedThread ? selectedSessionProject : null;

  const hasThread = Boolean(threadProject && threadId);
  const hasProject = !hasThread && Boolean(sessionProject);

  return (
    <div className="flex h-full min-h-0 w-full flex-col">
      <div
        data-tauri-drag-region
        className="relative flex h-12 shrink-0 items-center justify-between border-b border-ink/10 px-5 select-none"
      >
        <span
          data-tauri-drag-region
          className="truncate text-body-sm font-medium leading-none text-ink/72"
        >
          {hasThread
            ? selectedThread?.goal ?? t("sidebar.right_empty_title")
            : hasProject
              ? sessionProject?.name ?? t("sidebar.right_empty_title")
              : t("sidebar.right_empty_title")}
        </span>
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
        {hasThread && threadProject && threadId ? (
          <ThreadPage
            project={threadProject}
            threadId={threadId}
            onSelectSession={onSelectThreadChatSession}
            onOpenMultiSessionChat={onOpenThreadMultiSessionChat}
            onError={onError}
          />
        ) : hasProject && sessionProject ? (
          <ProjectWorkbenchPage
            project={sessionProject}
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
