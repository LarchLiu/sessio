import { useEffect, useState, type ReactNode } from "react";
import {
  CircleCheck,
  CircleDashed,
  CircleDot,
  CircleGauge,
  CircleSlash,
  CircleUserRound,
  Folder,
  GitBranch,
  Kanban,
  type LucideIcon,
} from "lucide-react";
import type {
  KanbanItem,
  KanbanStatus,
  RuntimeAgentMetadata,
  RuntimeAgentSelection,
  SetRuntimeAgentSelectionRequest,
} from "../api";
import { listKanbanItems } from "../api";
import ChatComposer, { NewChatMenuButton, ScrambledProjectName } from "../components/ChatComposer";
import type { InlineMenuSelectOption } from "../components/InlineMenuSelect";
import InlineMenuSelect from "../components/InlineMenuSelect";
import { RuntimeMenuSelect } from "../components/RuntimeMenuSelect";
import { useChatComposer } from "../hooks/useChatComposer";
import { useI18n } from "../i18n";
import type { PendingNewChatSession, ProjectGroup } from "../navigation";
import type { LiveRuntimeAction, LiveRuntimeState } from "../runtimeChat";

const KANBAN_STATUSES: KanbanStatus[] = [
  "todo",
  "in_progress",
  "agent_review",
  "human_review",
  "done",
  "canceled",
];

const KANBAN_STATUS_ICONS: Record<KanbanStatus, LucideIcon> = {
  todo: CircleDashed,
  in_progress: CircleDot,
  canceled: CircleSlash,
  agent_review: CircleGauge,
  human_review: CircleUserRound,
  done: CircleCheck,
};

interface NewChatPageProps {
  projects: ProjectGroup[];
  initialProjectKey: string | null;
  runtimeAgents: RuntimeAgentMetadata[];
  lastRuntimeAgentSelection: RuntimeAgentSelection | null;
  rememberRuntimeAgentSelection: (selection: SetRuntimeAgentSelectionRequest) => Promise<void>;
  liveState: LiveRuntimeState;
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onError: (error: string | null) => void;
  onPendingSession: (session: PendingNewChatSession) => void;
}

export default function NewChatPage({
  projects,
  initialProjectKey,
  runtimeAgents,
  lastRuntimeAgentSelection,
  rememberRuntimeAgentSelection,
  liveState,
  dispatchLiveEvent,
  onError,
  onPendingSession,
}: NewChatPageProps) {
  const { t } = useI18n();
  const [projectKeyValue, setProjectKeyValue] = useState(() => initialProjectKey ?? projects[0]?.key ?? "");
  const [kanbanItems, setKanbanItems] = useState<KanbanItem[]>([]);
  const [selectedKanbanItemId, setSelectedKanbanItemId] = useState("");
  const project = projects.find((p) => p.key === projectKeyValue) ?? projects[0] ?? null;
  const workspacePath = project?.path ?? null;
  const projectId = project?.project.id ?? null;
  const composer = useChatComposer({
    runtimeAgents,
    lastRuntimeAgentSelection,
    rememberRuntimeAgentSelection,
    liveState,
    dispatchLiveEvent,
    onError,
    onPendingSession,
  });
  const selectedKanbanItem =
    kanbanItems.find((item) => item.id === selectedKanbanItemId) ?? null;
  const kanbanItemOptions: InlineMenuSelectOption[] = [
    {
      value: "",
      label: t("kanban.no_item"),
      icon: <Kanban className="h-4 w-4 text-ink/45" />,
    },
    ...KANBAN_STATUSES.flatMap((status) => {
      const items = kanbanItems.filter((item) => item.status === status);
      if (items.length === 0) return [];
      const StatusIcon = KANBAN_STATUS_ICONS[status];
      return items.map((item) => ({
        value: item.id,
        label: item.title,
        group: {
          value: status,
          label: kanbanStatusLabel(status, t),
          icon: <StatusIcon className="h-4 w-4 text-ink/55" />,
        },
      }));
    }),
  ];

  useEffect(() => {
    if (initialProjectKey && projects.some((p) => p.key === initialProjectKey)) {
      setProjectKeyValue(initialProjectKey);
      return;
    }
    if (projectKeyValue && projects.some((p) => p.key === projectKeyValue)) return;
    setProjectKeyValue(projects[0]?.key ?? "");
  }, [initialProjectKey, projectKeyValue, projects]);

  useEffect(() => {
    let cancelled = false;
    setKanbanItems([]);
    setSelectedKanbanItemId("");
    if (!projectId) return;
    listKanbanItems(projectId)
      .then((items) => {
        if (cancelled) return;
        setKanbanItems(items);
      })
      .catch((err) => {
        if (!cancelled) onError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [onError, projectId]);

  useEffect(() => {
    if (!selectedKanbanItemId) return;
    if (kanbanItems.some((item) => item.id === selectedKanbanItemId)) return;
    setSelectedKanbanItemId("");
  }, [kanbanItems, selectedKanbanItemId]);

  const handleSend = async () => {
    const prompt = composer.text.trim();
    if (!prompt) return;
    if (!workspacePath || !project) {
      composer.setComposerError(t("new_chat.no_project"));
      return;
    }
    const sent = await composer.runStartSession(prompt, {
      workspacePath,
      projectName: project.label,
      pendingSession: {
        kanbanItemId: selectedKanbanItem?.id,
        kanbanItemStatus: selectedKanbanItem?.status,
      },
    });
    if (sent) setSelectedKanbanItemId("");
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface-panel">
      <div className="flex min-h-0 flex-1 items-center justify-center px-6 pb-16">
        <ChatComposer
          composer={composer}
          title={<>What should we build in <ScrambledProjectName name={project?.label ?? "sessio"} />?</>}
          canSend={composer.canSendWithWorkspace(workspacePath)}
          onSend={() => void handleSend()}
          bottomRow={
            <BottomRow>
              <RuntimeMenuSelect
                ariaLabel={t("new_chat.project")}
                value={projectKeyValue}
                onChange={setProjectKeyValue}
                disabled={projects.length === 0}
                options={projects.map((p) => ({
                  value: p.key,
                  label: p.label,
                  icon: <Folder className="h-4 w-4 text-ink/55" />,
                }))}
              />
              <div className="flex min-w-0 max-w-[260px] items-center rounded-md text-ink/55 transition hover:bg-ink/8 hover:text-ink">
                <InlineMenuSelect
                  value={selectedKanbanItemId}
                  options={kanbanItemOptions}
                  onChange={setSelectedKanbanItemId}
                  menuAlign="trigger"
                  placeholder={t("kanban.select_item")}
                  ariaLabel={t("kanban.select_item")}
                  className="h-7 max-w-[260px] border-r-0 px-1.5 py-1 text-ink/60 hover:text-ink"
                  menuClassName="bg-surface-panel"
                  minMenuWidth={220}
                  emptyContent={t("kanban.no_items")}
                />
              </div>
              <NewChatMenuButton icon={GitBranch} label="main" text />
            </BottomRow>
          }
        />
      </div>
    </div>
  );
}

export function BottomRow({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-10 items-center gap-2 px-3 text-body-sm text-ink/55">
      {children}
    </div>
  );
}

function kanbanStatusLabel(status: KanbanStatus, t: (key: string) => string): string {
  return t(`kanban.status.${status}`);
}
