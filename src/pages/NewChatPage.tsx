import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  ArrowUp,
  ChevronDown,
  CircleCheck,
  CircleDashed,
  CircleDot,
  CircleGauge,
  CircleSlash,
  CircleUserRound,
  Folder,
  GitBranch,
  Kanban,
  Mic,
  Plus,
  type LucideIcon,
} from "lucide-react";
import {
  type Agent,
  type KanbanItem,
  type KanbanStatus,
  type RuntimeAgentMetadata,
  type RuntimeAgentSelection,
  type SetRuntimeAgentSelectionRequest,
  listKanbanItems,
  sendAgentInput,
  startAgentSession,
  updateRuntimeAgentPreferences,
} from "../api";
import {
  agentModelSelectOptions,
  agentModelSelectValue,
  initialRuntimeEffort,
  parseAgentModelSelectValue,
  runtimeEffortOptions,
} from "../components/AgentSelect";
import {
  attachmentMenuOptions,
  ComposerAttachmentMenu,
  ComposerAttachmentPreviewList,
  useComposerAttachments,
} from "../components/ComposerAttachments";
import type { InlineMenuSelectOption } from "../components/InlineMenuSelect";
import InlineMenuSelect from "../components/InlineMenuSelect";
import { RuntimeEffortControl, RuntimeMenuSelect, runtimePermissionModeOptions } from "../components/RuntimeMenuSelect";
import Tooltip from "../components/Tooltip";
import { useI18n } from "../i18n";
import type { PendingNewChatSession, ProjectGroup } from "../navigation";
import { dispatchSessionStartedFallback, type LiveRuntimeAction, type LiveRuntimeState } from "../runtimeChat";
import {
  runtimeAgentForSelection,
  selectionEffort,
  selectionModel,
  selectionPermissionMode,
} from "../runtimeAgents";

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

const PROJECT_NAME_SCRAMBLE_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

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
  const initialRuntimeAgent =
    runtimeAgentForSelection(runtimeAgents, lastRuntimeAgentSelection) ?? runtimeAgents[0] ?? null;
  const [text, setText] = useState("");
  const [projectKeyValue, setProjectKeyValue] = useState(() => initialProjectKey ?? projects[0]?.key ?? "");
  const [agent, setAgent] = useState<Agent | "">(
    () => initialRuntimeAgent?.agent ?? "",
  );
  const [model, setModel] = useState(() =>
    initialRuntimeAgent ? selectionModel(initialRuntimeAgent, lastRuntimeAgentSelection) : "",
  );
  const [effort, setEffort] = useState(() =>
    initialRuntimeAgent
      ? selectionEffort(initialRuntimeAgent, lastRuntimeAgentSelection, initialRuntimeEffort)
      : "",
  );
  const [permissionMode, setPermissionMode] = useState(() =>
    initialRuntimeAgent ? selectionPermissionMode(initialRuntimeAgent, lastRuntimeAgentSelection) : "",
  );
  const [attachmentMenuOpen, setAttachmentMenuOpen] = useState(false);
  const [sending, setSending] = useState(false);
  const [composerError, setComposerError] = useState<string | null>(null);
  const [kanbanItems, setKanbanItems] = useState<KanbanItem[]>([]);
  const [selectedKanbanItemId, setSelectedKanbanItemId] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const attachmentButtonRef = useRef<HTMLButtonElement>(null);
  const fallbackRuntimeSequenceRef = useRef(0);
  const project = projects.find((p) => p.key === projectKeyValue) ?? projects[0] ?? null;
  const workspacePath = project?.path ?? null;
  const projectId = project?.project.id ?? null;
  const selectedAgentModelValue = agent ? agentModelSelectValue(agent, model) : "";
  const selectedRuntimeAgent =
    agent ? runtimeAgents.find((runtimeAgent) => runtimeAgent.agent === agent) ?? null : null;
  const handleEffortChange = useCallback(async (targetAgent: Agent, nextValue: string) => {
    if (targetAgent === agent) setEffort(nextValue);
    try {
      await updateRuntimeAgentPreferences({ agent: targetAgent, effort: nextValue });
      if (targetAgent === agent && selectedRuntimeAgent) {
        await rememberRuntimeAgentSelection({
          agent: targetAgent,
          model,
          effort: nextValue,
          permissionMode,
        });
      }
    } catch (err) {
      const message = String(err);
      setComposerError(message);
      onError(message);
    }
  }, [agent, model, onError, permissionMode, rememberRuntimeAgentSelection, selectedRuntimeAgent]);
  const agentModelOptions = useMemo(
    () =>
      agentModelSelectOptions(
        runtimeAgents,
        Object.fromEntries(
          runtimeAgents.map((runtimeAgent) => [
            runtimeAgent.agent,
            <RuntimeEffortControl
              value={runtimeAgent.agent === agent ? effort : initialRuntimeEffort(runtimeAgent)}
              options={runtimeEffortOptions(runtimeAgent)}
              onChange={(value) => void handleEffortChange(runtimeAgent.agent, value)}
              disabled={sending}
            />,
          ]),
        ) as Partial<Record<Agent, ReactNode>>,
        agent ? { [agent]: effort } : {},
      ),
    [agent, effort, handleEffortChange, runtimeAgents, sending],
  );
  const permissionOptions = runtimePermissionModeOptions(
    selectedRuntimeAgent?.permissionModes ?? [],
    permissionMode,
    selectedRuntimeAgent?.agent,
  );
  const {
    attachments,
    supportsAttachments,
    supportsImageAttachments,
    supportsEmbeddedContext,
    removeAttachment,
    clearAttachments,
    pickAttachments,
  } = useComposerAttachments({
    capabilities: selectedRuntimeAgent?.capabilities,
    onError: (message) => {
      setComposerError(message);
      onError(message);
    },
  });
  const canSend =
    text.trim().length > 0 &&
    Boolean(workspacePath) &&
    agentModelOptions.length > 0 &&
    !sending;
  const attachmentOptions = attachmentMenuOptions({
    supportsImageAttachments,
    supportsEmbeddedContext,
    imageLabel: t("new_chat.add_images"),
    fileLabel: t("new_chat.add_files"),
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
    if (agentModelOptions.some((option) => option.value === selectedAgentModelValue)) return;
    const current = agent ? runtimeAgents.find((item) => item.agent === agent) ?? null : null;
    const next =
      current ??
      runtimeAgentForSelection(runtimeAgents, lastRuntimeAgentSelection) ??
      runtimeAgents[0] ??
      null;
    if (!next) return;
    setAgent(next.agent);
    setModel(selectionModel(next, lastRuntimeAgentSelection));
    setEffort(selectionEffort(next, lastRuntimeAgentSelection, initialRuntimeEffort));
    setPermissionMode(selectionPermissionMode(next, lastRuntimeAgentSelection));
  }, [agent, agentModelOptions, lastRuntimeAgentSelection, runtimeAgents, selectedAgentModelValue]);

  useEffect(() => {
    if (!selectedRuntimeAgent) return;
    if (
      permissionMode &&
      selectedRuntimeAgent.permissionModes.some((option) => option.value === permissionMode)
    ) {
      return;
    }
    setPermissionMode(initialRuntimePermission(selectedRuntimeAgent));
  }, [
    permissionMode,
    selectedRuntimeAgent?.agent,
    selectedRuntimeAgent?.permissionMode,
    selectedRuntimeAgent?.permissionModes,
  ]);

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

  useEffect(() => {
    if (!supportsAttachments) {
      setAttachmentMenuOpen(false);
    }
  }, [supportsAttachments]);

  useEffect(() => {
    window.requestAnimationFrame(() => textareaRef.current?.focus());
  }, []);

  useEffect(() => {
    if (!attachmentMenuOpen) return;
    const close = () => setAttachmentMenuOpen(false);
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("resize", close);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [attachmentMenuOpen]);

  const handleAgentModelChange = async (nextValue: string) => {
    const parsed = parseAgentModelSelectValue(nextValue);
    if (!parsed) return;
    const targetRuntimeAgent =
      runtimeAgents.find((runtimeAgent) => runtimeAgent.agent === parsed.agent) ?? null;
    if (!targetRuntimeAgent) return;
    setAgent(parsed.agent);
    setModel(parsed.model);
    setEffort(initialRuntimeEffort(targetRuntimeAgent));
    setPermissionMode(initialRuntimePermission(targetRuntimeAgent));
    try {
      await updateRuntimeAgentPreferences({ agent: parsed.agent, model: parsed.model });
      await rememberRuntimeAgentSelection({
        agent: parsed.agent,
        model: parsed.model,
        effort: initialRuntimeEffort(targetRuntimeAgent),
        permissionMode: initialRuntimePermission(targetRuntimeAgent),
      });
    } catch (err) {
      const message = String(err);
      setComposerError(message);
      onError(message);
    }
  };

  const handlePermissionModeChange = async (nextValue: string) => {
    if (!selectedRuntimeAgent) return;
    setPermissionMode(nextValue);
    try {
      await updateRuntimeAgentPreferences({ agent: selectedRuntimeAgent.agent, permissionMode: nextValue });
      await rememberRuntimeAgentSelection({
        agent: selectedRuntimeAgent.agent,
        model,
        effort,
        permissionMode: nextValue,
      });
    } catch (err) {
      const message = String(err);
      setComposerError(message);
      onError(message);
    }
  };

  const handleSend = async () => {
    const prompt = text.trim();
    if (!prompt || sending) return;
    if (!workspacePath || !project) {
      setComposerError(t("new_chat.no_project"));
      return;
    }
    if (!agentModelOptions.some((option) => option.value === selectedAgentModelValue)) {
      setComposerError("No configured runtime agent available");
      return;
    }
    if (!agent) {
      setComposerError("No configured runtime agent available");
      return;
    }
    setSending(true);
    setComposerError(null);
    onError(null);
    try {
      const handle = await startAgentSession({
        agent,
        workspacePath,
        options: runtimeSessionOptions(model, permissionMode, effort),
      });
      await rememberRuntimeAgentSelection({
        agent,
        model,
        effort,
        permissionMode,
      });
      const timestamp = Date.now();
      dispatchSessionStartedFallback({
        dispatch: dispatchLiveEvent,
        handle,
        liveState,
        sequenceRef: fallbackRuntimeSequenceRef,
        timestamp,
      });
      onPendingSession({
        sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
        agent: handle.agent,
        projectPath: workspacePath,
        projectName: project.label,
        prompt,
        timestamp,
        kanbanItemId: selectedKanbanItem?.id,
        kanbanItemStatus: selectedKanbanItem?.status,
      });
      await sendAgentInput(handle.sessioRuntimeSessionId, {
        text: prompt,
        attachments: attachments.map(({ path, mimeType, kind }) => ({ path, mimeType, kind })),
      });
      setText("");
      setSelectedKanbanItemId("");
      clearAttachments();
    } catch (err) {
      const message = String(err);
      setComposerError(message);
      onError(message);
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="flex flex-1 min-h-0 flex-col bg-surface-panel">
      <div className="flex flex-1 min-h-0 items-center justify-center px-6 pb-16">
        <div className="w-full max-w-[730px]">
          <h1 className="mb-11 text-center text-[28px] font-medium leading-tight tracking-normal text-ink/92">
            What should we build in <ScrambledProjectName name={project?.label ?? "sessio"} />?
          </h1>
          {composerError && (
            <div className="mb-2 rounded-md border border-status-error/25 bg-status-error/10 px-3 py-2 text-body-sm text-status-error">
              {composerError}
            </div>
          )}
          <div
            className={
              "overflow-hidden rounded-2xl bg-ink/[0.055] shadow-[inset_0_0_0_1px_rgb(var(--color-ink)/0.08)] transition-shadow " +
              (composerError
                ? "shadow-[inset_0_0_0_1px_rgb(var(--color-status-error)/0.35)]"
                : "focus-within:shadow-[inset_0_0_0_1px_rgb(var(--color-ink)/0.20)]")
            }
          >
            <ComposerAttachmentPreviewList
              attachments={attachments}
              onRemove={removeAttachment}
            />
            <textarea
              ref={textareaRef}
              value={text}
              placeholder={t("new_chat.placeholder")}
              rows={2}
              onChange={(event) => {
                resizeTextareaToContent(event.currentTarget);
                setText(event.target.value);
              }}
              onInput={(event) => resizeTextareaToContent(event.currentTarget)}
              onKeyDown={(event) => {
                if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) {
                  return;
                }
                event.preventDefault();
                if (canSend) void handleSend();
              }}
              className="chat-composer-textarea block w-full resize-none bg-transparent px-3.5 py-3.5 text-body leading-5 text-ink/88 placeholder:text-ink/38 outline-none"
            />
            <div className="flex h-12 items-center justify-between gap-3 border-b border-ink/5 px-3 pb-2">
              <div className="flex min-w-0 items-center gap-3">
                {supportsAttachments && (
                  <Tooltip content={t("new_chat.add_context")} placement="top">
                    <button
                      ref={attachmentButtonRef}
                      type="button"
                      onClick={() => setAttachmentMenuOpen((open) => !open)}
                      className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-ink/55 transition hover:bg-ink/8 hover:text-ink"
                      aria-label={t("new_chat.add_context")}
                      aria-expanded={attachmentMenuOpen}
                      aria-haspopup="menu"
                    >
                      <Plus className="h-5 w-5" />
                    </button>
                  </Tooltip>
                )}
                <RuntimeMenuSelect
                  ariaLabel="Default permissions"
                  value={permissionMode}
                  onChange={(value) => void handlePermissionModeChange(value)}
                  disabled={!selectedRuntimeAgent}
                  options={permissionOptions}
                />
              </div>
              <div className="flex shrink-0 items-center gap-2.5">
                <RuntimeMenuSelect
                  ariaLabel={t("new_chat.agent")}
                  value={selectedAgentModelValue}
                  onChange={(value) => void handleAgentModelChange(value)}
                  disabled={agentModelOptions.length === 0}
                  options={agentModelOptions}
                />
                <NewChatMenuButton icon={Mic} label={t("new_chat.voice")} />
                <Tooltip content={sending ? t("new_chat.sending") : t("new_chat.send")} placement="top">
                  <button
                    type="button"
                    disabled={!canSend}
                    onClick={() => void handleSend()}
                    className="flex h-7 w-7 items-center justify-center rounded-full bg-ink/70 text-[rgb(var(--color-bg-panel))] transition hover:bg-ink disabled:cursor-not-allowed disabled:bg-ink/25 disabled:text-[rgb(var(--color-bg-panel)/0.7)]"
                    aria-label={sending ? t("new_chat.sending") : t("new_chat.send")}
                  >
                    <ArrowUp className="h-5 w-5" />
                  </button>
                </Tooltip>
              </div>
            </div>
            <div className="flex h-10 items-center gap-2 px-3 text-body-sm text-ink/55">
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
            </div>
          </div>
          {attachmentMenuOpen && attachmentButtonRef.current && (
            <ComposerAttachmentMenu
              anchor={attachmentButtonRef.current}
              options={attachmentOptions}
              onClose={() => setAttachmentMenuOpen(false)}
              onSelect={(key) => {
                void pickAttachments(key);
              }}
            />
          )}
        </div>
      </div>
    </div>
  );
}

function initialRuntimePermission(agent: RuntimeAgentMetadata | null): string {
  return agent?.permissionMode ?? agent?.permissionModes[0]?.value ?? "";
}

function runtimeSessionOptions(model: string, permissionMode: string, effort = ""): Record<string, unknown> {
  return {
    transport: "acp",
    ...(model ? { model } : {}),
    ...(effort ? { effort } : {}),
    ...(permissionMode ? { permissionMode } : {}),
  };
}

function kanbanStatusLabel(status: KanbanStatus, t: (key: string) => string): string {
  return t(`kanban.status.${status}`);
}

function resizeTextareaToContent(el: HTMLTextAreaElement) {
  el.style.height = "auto";
  const lineHeight = parseFloat(getComputedStyle(el).lineHeight) || 20;
  const minHeight = lineHeight * 2;
  const maxHeight = lineHeight * 6;
  const nextHeight = Math.min(Math.max(el.scrollHeight, minHeight), maxHeight);
  el.style.height = `${nextHeight}px`;
  el.style.overflowY = el.scrollHeight > maxHeight ? "auto" : "hidden";
}

function ScrambledProjectName({ name }: { name: string }) {
  const [display, setDisplay] = useState(name);
  const previousNameRef = useRef(name);

  useEffect(() => {
    const previousName = previousNameRef.current;
    previousNameRef.current = name;
    if (previousName === name) {
      setDisplay(name);
      return;
    }
    if (
      typeof window === "undefined" ||
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ) {
      setDisplay(name);
      return;
    }

    let frame = 0;
    let raf = 0;
    const maxLength = Math.max(previousName.length, name.length);
    const frames = Math.min(24, Math.max(12, maxLength + 8));

    const tick = () => {
      frame += 1;
      const settled = Math.floor((frame / frames) * maxLength);
      let next = "";
      for (let index = 0; index < maxLength; index += 1) {
        const target = name[index] ?? "";
        if (index < settled || frame >= frames) {
          next += target;
        } else if (target) {
          next += PROJECT_NAME_SCRAMBLE_CHARS[
            Math.floor(Math.random() * PROJECT_NAME_SCRAMBLE_CHARS.length)
          ];
        }
      }
      setDisplay(next || name);
      if (frame < frames) {
        raf = window.requestAnimationFrame(tick);
      }
    };

    raf = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(raf);
  }, [name]);

  return (
    <span className="inline-block min-w-[3ch] font-mono tabular-nums text-ink">
      {display}
    </span>
  );
}

function NewChatMenuButton({
  icon: Icon,
  label,
  text,
}: {
  icon: LucideIcon;
  label: string;
  text?: boolean;
}) {
  return (
    <button
      type="button"
      className={
        "flex min-w-0 items-center gap-1.5 rounded-md py-1 text-body-sm text-ink/55 transition hover:bg-ink/8 hover:text-ink " +
        (text ? "max-w-[220px] px-1.5" : "h-7 w-7 justify-center px-0")
      }
      aria-label={label}
    >
      <Icon className="h-4 w-4 shrink-0" />
      {text && <span className="truncate">{label}</span>}
      {text && <ChevronDown className="h-3.5 w-3.5 shrink-0" />}
    </button>
  );
}
