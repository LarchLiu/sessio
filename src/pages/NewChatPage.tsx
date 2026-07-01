import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  Brain,
  Check,
  Folder,
  GripVertical,
  MessageSquare,
  Swords,
  Trash2,
  Workflow,
} from "lucide-react";
import { DragDropProvider, type DragEndEvent } from "@dnd-kit/react";
import { isSortable, useSortable } from "@dnd-kit/react/sortable";
import type {
  Agent,
  AssistantInfo,
  ProjectStageInfo,
  RuntimeAgentMetadata,
  RuntimeAgentSelection,
  SetRuntimeAgentSelectionRequest,
  StageInfo,
  ThreadAgentInfo,
  ThreadInfo,
  ThreadKind,
} from "../api";
import {
  AGENT_LABEL,
  addThreadStage,
  createAstraRun,
  createThread,
  getRuntimeAgentSessionConfig,
  isAgent,
  listAssistants,
  listProjectStages,
} from "../api";
import { AgentGlyph } from "../components/AgentIcon";
import ChatComposer, {
  AssistantModeChip,
  SlashCommandModeChip,
  ScrambledProjectName,
  resizeTextareaToContent,
} from "../components/ChatComposer";
import ComposerCommandMenu from "../components/ComposerCommandMenu";
import type { InlineMenuSelectOption } from "../components/InlineMenuSelect";
import InlineMenuSelect from "../components/InlineMenuSelect";
import { ImagePreviewOverlay, type MarkdownImage } from "./ChatPage";
import { RuntimeMenuSelect } from "../components/RuntimeMenuSelect";
import StageSelectChip from "../components/StageSelectChip";
import Tooltip from "../components/Tooltip";
import { PeopleTeam24RegularIcon, Robot3LineIcon } from "../components/IconifyIcon";
import {
  filterAssistantCommandItems,
  filterComposerSlashCommands,
  normalizeAssistantAgent,
  parseComposerCommandTrigger,
  slashCommandItems,
  useComposerCommandMenuState,
} from "../composerCommands";
import { useChatComposer } from "../hooks/useChatComposer";
import { useI18n } from "../i18n";
import type { PendingNewChatSession, ProjectGroup } from "../navigation";
import { useAppshotComposerRegistration } from "../appshot";
import {
  formatChatSlashCommandText,
  parseRuntimeSessionAvailableCommands,
  parseSelectedSlashCommandName,
} from "../chatSlashCommands";
import type { AcpAvailableCommand, LiveRuntimeAction, LiveRuntimeState } from "../runtimeChat";

type NewChatMode = "chat" | ThreadKind;
type ParticipantDraft = {
  draftId: string;
  agent: Agent;
  model: string;
  effort: string;
  permissionMode: string;
};

const THREAD_MODES: ThreadKind[] = ["teamwork", "process", "brainstorm", "debate"];
const AGENT_PARTICIPANT_MODES = new Set<NewChatMode>(["brainstorm", "debate"]);
const ASTRA_THREAD_MODES = new Set<NewChatMode>(["teamwork", "process", "brainstorm", "debate"]);

interface NewChatPageProps {
  projects: ProjectGroup[];
  initialProjectKey: string | null;
  runtimeAgents: RuntimeAgentMetadata[];
  lastRuntimeAgentSelection: RuntimeAgentSelection | null;
  rememberRuntimeAgentSelection: (selection: SetRuntimeAgentSelectionRequest) => Promise<void>;
  liveState: LiveRuntimeState;
  dispatchLiveEvent: React.Dispatch<LiveRuntimeAction>;
  onError: (error: string | null) => void;
  onProjectChange?: (projectKey: string | null) => void;
  onPendingSession: (session: PendingNewChatSession) => void;
  onThreadCreated: (project: ProjectGroup, thread: ThreadInfo) => void;
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
  onProjectChange,
  onPendingSession,
  onThreadCreated,
}: NewChatPageProps) {
  const { t } = useI18n();
  const [projectKeyValue, setProjectKeyValue] = useState(() => initialProjectKey ?? projects[0]?.key ?? "");
  const lastAppliedInitialProjectKeyRef = useRef<string | null>(null);
  const [mode, setMode] = useState<NewChatMode>("chat");
  const [projectStages, setProjectStages] = useState<ProjectStageInfo[]>([]);
  const [stageOrder, setStageOrder] = useState<string[]>([]);
  const [selectedStageIds, setSelectedStageIds] = useState<string[]>([]);
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);
  const [selectedAssistantIds, setSelectedAssistantIds] = useState<string[]>([]);
  const [participantDrafts, setParticipantDrafts] = useState<ParticipantDraft[]>([]);
  const [threadSending, setThreadSending] = useState(false);
  const [chatTransitionPending, setChatTransitionPending] = useState(false);
  const [selectedAssistant, setSelectedAssistant] = useState<AssistantInfo | null>(null);
  const [selectedSlashCommand, setSelectedSlashCommand] = useState<AcpAvailableCommand | null>(null);
  const [cachedAvailableCommands, setCachedAvailableCommands] = useState<AcpAvailableCommand[]>([]);
  const [previewImage, setPreviewImage] = useState<MarkdownImage | null>(null);
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
    onPreviewImageAttachment: setPreviewImage,
  });
  useAppshotComposerRegistration(composer, true);
  const threadMode = mode !== "chat";
  const composerBusy = composer.sending || threadSending || chatTransitionPending;
  const threadValidationError = threadMode
    ? validateThreadMode({
      mode,
      selectedStageIds,
      selectedAssistantIds,
      participantDrafts,
      t,
    })
    : null;
  const threadCanSend =
    threadMode &&
    composer.text.trim().length > 0 &&
    Boolean(workspacePath) &&
    !threadSending &&
    !threadValidationError;

  const threadKindOptions: InlineMenuSelectOption[] = useMemo(
    () => [
      {
        value: "chat",
        label: t("new_chat.mode.chat"),
        icon: <MessageSquare className="h-4 w-4 text-ink/55" />,
      },
      ...THREAD_MODES.map((kind) => ({
        value: kind,
        label: t(`thread.kind.${kind}`),
        icon: threadKindIcon(kind, "h-4 w-4 text-ink/55"),
      })),
    ],
    [t],
  );
  const selectableStages = useMemo(
    () => projectStages.filter((stage) => stage.enabled && stageAllowsThreadAddition(stage)),
    [projectStages],
  );
  const orderedStages = useMemo(() => {
    const byId = new Map(projectStages.map((stage) => [stage.id, stage]));
    return stageOrder
      .map((id) => byId.get(id))
      .filter((stage): stage is ProjectStageInfo => Boolean(stage));
  }, [projectStages, stageOrder]);
  const assistantOptions = useMemo(
    () =>
      assistants
        .filter((assistant) => assistant.projectId === projectId && assistant.enabled)
        .map((assistant) => ({
          value: assistant.id,
          label: assistant.name,
          icon: assistantRobotIcon(assistant.color),
        })),
    [assistants, projectId],
  );
  const participantOptions = useMemo(
    () => {
      const selectedParticipants = new Set(
        participantDrafts.map((participant) =>
          participantDraftValue(participant.agent, participant.model),
        ),
      );
      return runtimeAgents
        .flatMap((runtimeAgent) => {
          const models =
            runtimeAgent.models.length > 0
              ? runtimeAgent.models.filter((model) => model.enabled && model.value.trim().length > 0)
              : runtimeAgent.model
                ? [{ value: runtimeAgent.model, label: runtimeAgent.model, displayName: runtimeAgent.model, enabled: true, order: 0 }]
                : [];
          return models.map((model) => {
            const value = participantDraftValue(runtimeAgent.agent, model.value);
            return {
              value,
              label: `${AGENT_LABEL[runtimeAgent.agent]} · ${model.displayName || model.label || model.value}`,
              icon: <AgentGlyph agent={runtimeAgent.agent} className="h-3.5 w-3.5" />,
            };
          });
        })
        .filter((option) => !selectedParticipants.has(option.value));
    },
    [participantDrafts, runtimeAgents],
  );

  const command = useMemo(
    () => parseComposerCommandTrigger(composer.text, ["slash", "assistant", "thread"]),
    [composer.text],
  );
  const commandItems = useMemo(() => {
    if (!command) return [];
    if (command.kind === "slash") {
      return slashCommandItems(filterComposerSlashCommands(cachedAvailableCommands, command.query));
    }
    if (command.kind === "thread") {
      return THREAD_MODES.filter((kind) => kind.startsWith(command.query.toLowerCase())).map((kind) => ({
        key: kind,
        label: t(`thread.kind.${kind}`),
        description: t(`new_chat.command.thread.${kind}`),
        icon: threadKindIcon(kind, "h-4 w-4 text-ink/55"),
      }));
    }
    return filterAssistantCommandItems(assistants, projectId, command.query);
  }, [assistants, cachedAvailableCommands, command, projectId, t]);
  const commandMenu = useComposerCommandMenuState({
    trigger: command,
    items: commandItems,
    disabled: composerBusy,
  });

  const projectKeys = useMemo(() => new Set(projects.map((p) => p.key)), [projects]);
  const handleProjectChange = useCallback((nextProjectKey: string) => {
    setProjectKeyValue(nextProjectKey);
    onProjectChange?.(nextProjectKey || null);
  }, [onProjectChange]);

  useEffect(() => {
    if (!initialProjectKey) {
      lastAppliedInitialProjectKeyRef.current = null;
      return;
    }
    if (!projectKeys.has(initialProjectKey)) return;
    if (initialProjectKey === lastAppliedInitialProjectKeyRef.current) return;
    lastAppliedInitialProjectKeyRef.current = initialProjectKey;
    setProjectKeyValue(initialProjectKey);
  }, [initialProjectKey, projectKeys]);

  useEffect(() => {
    if (projectKeyValue && projectKeys.has(projectKeyValue)) return;
    const fallbackProjectKey = projects[0]?.key ?? "";
    setProjectKeyValue(fallbackProjectKey);
    onProjectChange?.(fallbackProjectKey || null);
  }, [onProjectChange, projectKeyValue, projectKeys, projects]);

  useEffect(() => {
    let cancelled = false;
    setProjectStages([]);
    setAssistants([]);
    setSelectedAssistantIds([]);
    setSelectedStageIds([]);
    setStageOrder([]);
    if (!projectId) return;
    Promise.all([listProjectStages(projectId), listAssistants(projectId)])
      .then(([stages, nextAssistants]) => {
        if (cancelled) return;
        const sortedStages = stages.slice().sort((a, b) => a.order - b.order);
        const allowed = sortedStages.filter((stage) => stage.enabled && stageAllowsThreadAddition(stage));
        setProjectStages(sortedStages);
        setStageOrder(sortedStages.map((stage) => stage.id));
        setSelectedStageIds(allowed.map((stage) => stage.id));
        setAssistants(nextAssistants);
        setSelectedAssistantIds(
          nextAssistants
            .filter((assistant) => assistant.projectId === projectId && assistant.enabled)
            .map((assistant) => assistant.id),
        );
      })
      .catch((err) => {
        if (!cancelled) onError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [onError, projectId]);

  useEffect(() => {
    setSelectedAssistantIds((current) => {
      const valid = new Set(assistantOptions.map((option) => option.value));
      return current.filter((id) => valid.has(id));
    });
  }, [assistantOptions]);

  useEffect(() => {
    let cancelled = false;
    getRuntimeAgentSessionConfig(composer.selectedAgent ?? "codex")
      .then((config) => {
        if (cancelled) return;
        setCachedAvailableCommands(parseRuntimeSessionAvailableCommands(config));
      })
      .catch(() => {
        if (!cancelled) setCachedAvailableCommands([]);
      });
    return () => {
      cancelled = true;
    };
  }, [composer.selectedAgent]);

  useEffect(() => {
    setStageOrder((current) => {
      const ids = projectStages.map((stage) => stage.id);
      return [
        ...current.filter((id) => ids.includes(id)),
        ...ids.filter((id) => !current.includes(id)),
      ];
    });
    setSelectedStageIds((current) => {
      const valid = new Set(selectableStages.map((stage) => stage.id));
      return current.filter((id) => valid.has(id));
    });
  }, [projectStages, selectableStages]);

  useEffect(() => {
    setParticipantDrafts((current) => {
      const existing = current.filter((participant) => runtimeAgentSupports(runtimeAgents, participant));
      return mode === "debate" ? existing.slice(0, 2) : existing;
    });
  }, [mode, runtimeAgents]);

  const handleSend = async () => {
    const prompt = buildPromptText(selectedSlashCommand, composer.text).trim();
    if (!prompt || threadSending || chatTransitionPending) return;
    const slashName = parseSelectedSlashCommandName(prompt);
    if (slashName) {
      const slashCommand = cachedAvailableCommands.find((item) => item.name === slashName);
      if (slashCommand && (slashCommand.commandType ?? "agent_builtin") !== "agent_builtin") {
        composer.setComposerError(`Unsupported app command: ${slashCommand.name}`);
        return;
      }
    }
    if (!workspacePath || !project) {
      composer.setComposerError(t("new_chat.no_project"));
      return;
    }
    if (mode === "chat") {
      const sent = await composer.runStartSession(prompt, {
        workspacePath,
        projectName: project.label,
        assistantPrompt: selectedAssistant?.systemPrompt?.trim() || undefined,
        clearComposerOnSuccess: false,
        pendingSession: {
          origin: "new_chat",
        },
      });
      if (sent) {
        setChatTransitionPending(true);
      }
      return;
    }
    const validationError = validateThreadMode({
      mode,
      selectedStageIds,
      selectedAssistantIds,
      participantDrafts,
      t,
    });
    if (validationError) {
      composer.setComposerError(validationError);
      return;
    }
    const threadDraft = parseThreadDraft(prompt);

    setThreadSending(true);
    try {
      const agentParticipants = AGENT_PARTICIPANT_MODES.has(mode)
        ? participantDraftsToThreadAgents(participantDrafts)
        : [];
      let thread = await createThread(
        project.project.id,
        threadDraft.goal,
        threadDraft.description,
        mode,
        mode === "teamwork" ? selectedAssistantIds : [],
        agentParticipants,
      );
      if (mode === "process") {
        const selected = new Set(selectedStageIds);
        const stages: StageInfo[] = [];
        for (const stageId of stageOrder.filter((id) => selected.has(id))) {
          stages.push(await addThreadStage(thread.id, stageId, []));
        }
        thread = {
          ...thread,
          stages: stages.sort((a, b) => a.order - b.order),
          updatedAt: Math.max(thread.updatedAt, ...stages.map((stage) => stage.updatedAt), thread.updatedAt),
        };
      }

      if (ASTRA_THREAD_MODES.has(mode)) {
        await createAstraRun(thread.id, null);
      }
      composer.setText("");
      setSelectedSlashCommand(null);
      setSelectedAssistant(null);
      onThreadCreated(project, thread);
    } catch (err) {
      const message = String(err);
      composer.setComposerError(message);
      onError(message);
    } finally {
      setThreadSending(false);
    }
  };

  const handleCommandSelect = (key: string) => {
    if (!command) return;
    if (command.kind === "slash") {
      const selected = cachedAvailableCommands.find((item) => item.name === key);
      if (!selected) return;
      setSelectedSlashCommand(selected);
      composer.setText(command.rest);
    } else if (command.kind === "thread") {
      const kind = key as ThreadKind;
      // 选择 thread kind 后清掉 #slug，由底部 chat mode 选择器展示
      setMode(kind);
      setSelectedAssistant(null);
      composer.setText(command.rest);
    } else {
      const assistant = assistants.find((item) => item.id === key);
      if (!assistant) return;
      setMode("chat");
      setSelectedAssistant(assistant);
      // 选中 assistant 后聊天框 agent 直接切到 assistant 绑定的 agent
      composer.applyAgentSelection({
        agent: normalizeAssistantAgent(assistant.agent.id),
        model: assistant.agent.model,
        effort: assistant.agent.effort,
        permissionMode: assistant.agent.mode,
      });
      composer.setText(command.rest);
    }
    commandMenu.resetDismissed();
    window.requestAnimationFrame(() => {
      const el = composer.textareaRef.current;
      if (!el) return;
      el.focus();
      resizeTextareaToContent(el);
      const pos = el.value.length;
      el.setSelectionRange(pos, pos);
    });
  };

  const handleTextareaKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>): boolean => {
    return commandMenu.handleKeyDown(event, handleCommandSelect);
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface-panel">
      <div className="flex min-h-0 flex-1 items-center justify-center px-6 pb-16">
        <div className="relative w-full max-w-[730px]">
          <ChatComposer
            composer={composer}
            variant="chat"
            title={<>What should we build in <ScrambledProjectName name={project?.label ?? "sessio"} />?</>}
            canSend={
              threadMode
                ? threadCanSend
                : composer.canSendWithWorkspace(workspacePath) && !threadSending && !chatTransitionPending
            }
            onSend={() => void handleSend()}
            onTextareaKeyDown={handleTextareaKeyDown}
            sendButtonVariant={threadMode ? "astra" : "chat"}
            sendButtonLabel={threadMode ? threadValidationError ?? createThreadLabel(mode, t) : undefined}
            sendButtonBusy={threadMode ? threadSending || composer.sending : composer.sending || chatTransitionPending}
            runtimeControlsDisabled={threadMode || chatTransitionPending}
            placeholder={threadMode ? t("new_chat.thread_placeholder") : undefined}
            modeActions={
              !threadMode ? (
                <>
                  {selectedSlashCommand ? (
                    <SlashCommandModeChip
                      name={selectedSlashCommand.name}
                      onRemove={() => setSelectedSlashCommand(null)}
                    />
                  ) : null}
                  {selectedAssistant ? (
                    <AssistantModeChip
                      icon={assistantRobotIcon(selectedAssistant.color)}
                      name={selectedAssistant.name}
                      onRemove={() => setSelectedAssistant(null)}
                    />
                  ) : null}
                </>
              ) : undefined
            }
            bottomRow={
              <BottomRow>
                <RuntimeMenuSelect
                  ariaLabel={t("new_chat.project")}
                  value={projectKeyValue}
                  onChange={handleProjectChange}
                  disabled={projects.length === 0}
                  options={projects.map((p) => ({
                    value: p.key,
                    label: p.label,
                    icon: <Folder className="h-4 w-4 text-ink/55" />,
                  }))}
                />
                <div className="flex min-w-0 max-w-[220px] items-center rounded-md text-ink/55 transition hover:bg-ink/8 hover:text-ink">
                  <InlineMenuSelect
                    value={mode}
                    options={threadKindOptions}
                    onChange={(value) => setMode(value as NewChatMode)}
                    menuAlign="trigger"
                    placeholder={t("new_chat.thread_kind")}
                    ariaLabel={t("new_chat.thread_kind")}
                    className="h-7 max-w-[220px] border-r-0 px-1.5 py-1 text-ink/60 hover:text-ink"
                    menuClassName="bg-surface-panel"
                    minMenuWidth={180}
                  />
                </div>
              </BottomRow>
            }
          />
          {threadMode && (
            <div className="absolute left-1/2 top-full z-20 w-[calc(100%-32px)] -translate-x-1/2 overflow-hidden rounded-b-xl border border-t-0 border-ink/[0.07] bg-ink/[0.03] px-3 py-2 shadow-[inset_0_1px_0_rgb(var(--color-ink)/0.02)]">
              <ThreadSetupPanel
                mode={mode}
                orderedStages={orderedStages}
                selectableStageIds={new Set(selectableStages.map((stage) => stage.id))}
                selectedStageIds={selectedStageIds}
                onToggleStage={(stageId) => {
                  if (!selectableStages.some((stage) => stage.id === stageId)) return;
                  setSelectedStageIds((current) =>
                    current.includes(stageId)
                      ? current.filter((id) => id !== stageId)
                      : [...current, stageId],
                  );
                }}
                onStageDragEnd={(event) => {
                  if (event.canceled) return;
                  const { source } = event.operation;
                  if (!isSortable(source)) return;
                  const from = source.initialIndex;
                  const to = source.index;
                  if (from === to) return;
                  setStageOrder((current) => {
                    const next = [...current];
                    const [id] = next.splice(from, 1);
                    if (!id) return current;
                    next.splice(to, 0, id);
                    return next;
                  });
                }}
                validationError={threadValidationError}
                assistantOptions={assistantOptions}
                selectedAssistantIds={selectedAssistantIds}
                onAssistantIdsChange={setSelectedAssistantIds}
                participantDrafts={participantDrafts}
                participantOptions={participantOptions}
                runtimeAgents={runtimeAgents}
                onAddParticipant={(value) => {
                  const draft = participantDraftFromValue(value, runtimeAgents);
                  if (!draft) return;
                  setParticipantDrafts((current) => {
                    if (mode === "debate" && current.length >= 2) return current;
                    if (current.some((participant) => participant.agent === draft.agent && participant.model === draft.model)) {
                      return current;
                    }
                    return [...current, { ...draft, draftId: stableDraftId(draft, current.length) }];
                  });
                }}
                onRemoveParticipant={(draftId) => {
                  setParticipantDrafts((current) => current.filter((participant) => participant.draftId !== draftId));
                }}
              />
            </div>
          )}
          {commandMenu.open && command && composer.textareaRef.current && (
            <ComposerCommandMenu
              anchor={composer.textareaRef.current}
              items={commandItems}
              activeIndex={commandMenu.activeIndex}
              header={
                command.kind === "slash"
                  ? t("chat.command.header")
                  : command.kind === "thread"
                    ? t("new_chat.command.thread_header")
                    : t("new_chat.command.assistant_header")
              }
              emptyText={
                command.kind === "slash"
                  ? t("chat.command.empty")
                  : command.kind === "thread"
                    ? t("new_chat.command.no_thread")
                    : t("new_chat.command.no_assistant")
              }
              onActiveIndexChange={commandMenu.setActiveIndex}
              onSelect={handleCommandSelect}
              onClose={() => commandMenu.setDismissedFor(command.raw)}
            />
          )}
        </div>
      </div>
      {previewImage && (
        <ImagePreviewOverlay
          image={previewImage}
          onClose={() => setPreviewImage(null)}
        />
      )}
    </div>
  );
}

function ThreadSetupPanel({
  mode,
  orderedStages,
  selectableStageIds,
  selectedStageIds,
  onToggleStage,
  onStageDragEnd,
  assistantOptions,
  selectedAssistantIds,
  onAssistantIdsChange,
  participantDrafts,
  participantOptions,
  runtimeAgents,
  validationError,
  onAddParticipant,
  onRemoveParticipant,
}: {
  mode: NewChatMode;
  orderedStages: ProjectStageInfo[];
  selectableStageIds: Set<string>;
  selectedStageIds: string[];
  onToggleStage: (stageId: string) => void;
  onStageDragEnd: (event: DragEndEvent) => void;
  assistantOptions: { value: string; label: string; icon?: ReactNode }[];
  selectedAssistantIds: string[];
  onAssistantIdsChange: (ids: string[]) => void;
  participantDrafts: ParticipantDraft[];
  participantOptions: InlineMenuSelectOption[];
  runtimeAgents: RuntimeAgentMetadata[];
  validationError: string | null;
  onAddParticipant: (value: string) => void;
  onRemoveParticipant: (draftId: string) => void;
}) {
  const { t } = useI18n();
  const withValidation = (content: ReactNode) => (
    <div className="flex min-w-0 flex-col gap-1.5">
      {content}
      {validationError && (
        <span className="text-caption leading-tight text-status-error/75" aria-live="polite">
          {validationError}
        </span>
      )}
    </div>
  );
  if (mode === "process") {
    return withValidation(
      <DragDropProvider onDragEnd={onStageDragEnd}>
        <div className="flex flex-wrap items-center gap-1.5">
          {orderedStages.length === 0 ? (
            <span className="text-caption text-ink/38">{t("new_chat.no_stages")}</span>
          ) : (
            orderedStages.map((stage, index) => (
              <SetupStageChip
                key={stage.id}
                stage={stage}
                index={index}
                selected={selectedStageIds.includes(stage.id)}
                selectable={selectableStageIds.has(stage.id)}
                onToggle={onToggleStage}
              />
            ))
          )}
        </div>
      </DragDropProvider>,
    );
  }
  if (mode === "teamwork") {
    return withValidation(
      assistantOptions.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          {assistantOptions.map((assistant) => (
            <AssistantSelectChip
              key={assistant.value}
              assistant={assistant}
              selected={selectedAssistantIds.includes(assistant.value)}
              onToggle={(assistantId) => {
                onAssistantIdsChange(
                  selectedAssistantIds.includes(assistantId)
                    ? selectedAssistantIds.filter((id) => id !== assistantId)
                    : [...selectedAssistantIds, assistantId],
                );
              }}
            />
          ))}
        </div>
      ) : (
        <span className="text-caption text-ink/38">{t("thread.no_assistants")}</span>
      ),
    );
  }
  if (mode === "brainstorm" || mode === "debate") {
    return withValidation(
      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        {participantDrafts.map((participant, index) => (
          <ParticipantChip
            key={participant.draftId}
            participant={participant}
            index={index}
            runtimeAgent={runtimeAgents.find((agent) => agent.agent === participant.agent) ?? null}
            canRemove
            onRemove={() => onRemoveParticipant(participant.draftId)}
          />
        ))}
        <ParticipantAddMenu
          options={participantOptions}
          onAdd={onAddParticipant}
          disabled={participantOptions.length === 0 || (mode === "debate" && participantDrafts.length >= 2)}
        />
      </div>,
    );
  }
  return null;
}

function SetupStageChip({
  stage,
  index,
  selected,
  selectable,
  onToggle,
}: {
  stage: ProjectStageInfo;
  index: number;
  selected: boolean;
  selectable: boolean;
  onToggle: (stageId: string) => void;
}) {
  const { handleRef, isDragSource, isDropTarget, ref } = useSortable({
    id: stage.id,
    index,
    group: "new-chat-thread-stages",
    transition: {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
      idle: true,
    },
  });
  return (
    <StageSelectChip
      ref={ref}
      stage={stage}
      selected={selected}
      selectable={selectable}
      onToggle={onToggle}
      state={
        isDragSource
          ? "dragging"
          : isDropTarget
            ? "drop-target"
            : "idle"
      }
      dragHandle={
        <button
          ref={handleRef}
          type="button"
          className="cursor-grab touch-none rounded p-0.5 text-current/50 hover:bg-ink/5 active:cursor-grabbing"
        >
          <GripVertical className="h-3.5 w-3.5" />
        </button>
      }
    />
  );
}

function ParticipantChip({
  participant,
  index,
  runtimeAgent,
  canRemove,
  onRemove,
}: {
  participant: ParticipantDraft;
  index: number;
  runtimeAgent: RuntimeAgentMetadata | null;
  canRemove: boolean;
  onRemove: () => void;
}) {
  const modelLabel = modelDisplayName(runtimeAgent, participant.model);
  return (
    <span className="inline-flex h-7 max-w-[260px] items-center gap-1.5 rounded-md border border-ink/[0.08] bg-surface-panel px-1.5 text-caption text-ink/65">
      <span className="text-ink/35 tabular-nums">{index + 1}</span>
      <AgentGlyph agent={participant.agent} className="h-3.5 w-3.5 shrink-0" />
      <span className="min-w-0 truncate">
        {AGENT_LABEL[participant.agent]}
        {modelLabel ? <span className="text-ink/40"> · {modelLabel}</span> : null}
      </span>
      {canRemove && (
        <Tooltip content="Remove" placement="top">
          <button
            type="button"
            onClick={onRemove}
            className="shrink-0 rounded p-0.5 text-ink/35 transition hover:bg-ink/6 hover:text-ink/70"
            aria-label="Remove participant"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </Tooltip>
      )}
    </span>
  );
}

function ParticipantAddMenu({
  options,
  onAdd,
  disabled,
}: {
  options: InlineMenuSelectOption[];
  onAdd: (value: string) => void;
  disabled: boolean;
}) {
  const { t } = useI18n();
  return (
    <div className="flex min-w-0 items-center rounded-md text-ink/55 transition hover:bg-ink/8 hover:text-ink">
      <InlineMenuSelect
        value=""
        options={options}
        onChange={onAdd}
        placeholder={t("new_chat.add_participant")}
        ariaLabel={t("new_chat.add_participant")}
        className={
          "h-7 max-w-[170px] border-r-0 px-1.5 py-1 text-ink/60 hover:text-ink " +
          (disabled ? "pointer-events-none opacity-40" : "")
        }
        menuClassName="bg-surface-panel"
        minMenuWidth={260}
        emptyContent={t("new_chat.no_participants")}
      />
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

function AssistantSelectChip({
  assistant,
  selected,
  onToggle,
}: {
  assistant: { value: string; label: string; icon?: ReactNode };
  selected: boolean;
  onToggle: (assistantId: string) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onToggle(assistant.value)}
      className={
        "inline-flex h-7 max-w-[180px] items-center gap-1.5 rounded-md border px-1.5 text-caption transition duration-150 " +
        (selected
          ? "border-ink/[0.14] bg-ink/[0.048] text-ink/70"
          : "border-ink/[0.08] bg-surface-panel text-ink/45 hover:bg-ink/[0.04] hover:text-ink/65")
      }
    >
      {assistant.icon}
      <span className="min-w-0 truncate">{assistant.label}</span>
      <span className="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border border-ink/[0.35] bg-ink/[0.04] text-ink/75">
        {selected && <Check className="h-3 w-3" />}
      </span>
    </button>
  );
}

function threadKindIcon(kind: ThreadKind, className: string) {
  switch (kind) {
    case "process":
      return <Workflow className={className} />;
    case "teamwork":
      return <PeopleTeam24RegularIcon className={className} />;
    case "brainstorm":
      return <Brain className={className} />;
    case "debate":
      return <Swords className={className} />;
  }
}

function createThreadLabel(mode: NewChatMode, t: (key: string, vars?: Record<string, string | number>) => string): string {
  return t("new_chat.create_thread", { kind: t(mode === "chat" ? "new_chat.mode.chat" : `thread.kind.${mode}`) });
}

function buildPromptText(
  slashCommand: Pick<AcpAvailableCommand, "name" | "input"> | null,
  text: string,
): string {
  const body = text.trim();
  if (!slashCommand) return body;
  const commandText = formatChatSlashCommandText(slashCommand).trimEnd();
  return body ? `${commandText} ${body}` : commandText;
}

function stageAllowsThreadAddition(stage: ProjectStageInfo): boolean {
  return stage.assistants.length > 0 || stage.allowEmptyAssistants;
}

function assistantRobotIcon(color: string | null | undefined) {
  return (
    <Robot3LineIcon
      className="h-3.5 w-3.5 shrink-0"
      style={{ color: color ?? "rgb(var(--color-brand))" }}
    />
  );
}

function validateThreadMode({
  mode,
  selectedStageIds,
  selectedAssistantIds,
  participantDrafts,
  t,
}: {
  mode: NewChatMode;
  selectedStageIds: string[];
  selectedAssistantIds: string[];
  participantDrafts: ParticipantDraft[];
  t: (key: string) => string;
}): string | null {
  if (mode === "process" && selectedStageIds.length === 0) return t("new_chat.thread_requires_stage");
  if (mode === "teamwork" && selectedAssistantIds.length === 0) return t("new_chat.thread_requires_assistant");
  if (mode === "brainstorm" && participantDrafts.length < 2) return t("new_chat.thread_requires_two_participants");
  if (mode === "debate" && participantDrafts.length !== 2) return t("new_chat.thread_requires_exactly_two_participants");
  return null;
}

function participantDraftsToThreadAgents(drafts: ParticipantDraft[]): ThreadAgentInfo[] {
  return drafts.filter((draft) => draft.model.trim().length > 0).map((draft, index) => ({
    participantId: "",
    agent: draft.agent,
    model: draft.model.trim(),
    effort: draft.effort.trim(),
    permissionMode: draft.permissionMode.trim(),
    order: index,
  }));
}

function runtimeAgentSupports(runtimeAgents: RuntimeAgentMetadata[], participant: ParticipantDraft): boolean {
  const runtimeAgent = runtimeAgents.find((agent) => agent.agent === participant.agent);
  if (!runtimeAgent) return false;
  if (!participant.model) return true;
  if (runtimeAgent.model === participant.model) return true;
  return runtimeAgent.models.some((model) => model.enabled && model.value === participant.model);
}

function parseThreadDraft(input: string): { goal: string; description: string | null; message: string } {
  const message = input.replace(/\r\n/g, "\n").trim();
  const [goalLine = "", ...descriptionLines] = message.split("\n");
  const description = descriptionLines.join("\n").trim();
  return {
    goal: goalLine.trim(),
    description: description.length > 0 ? description : null,
    message,
  };
}

function participantDraftValue(agent: Agent, model: string): string {
  return JSON.stringify({ agent, model });
}

function participantDraftFromValue(value: string, runtimeAgents: RuntimeAgentMetadata[]): Omit<ParticipantDraft, "draftId"> | null {
  try {
    const parsed = JSON.parse(value) as { agent?: unknown; model?: unknown };
    const agent = parsed.agent;
    const model = typeof parsed.model === "string" ? parsed.model : "";
    if (!isAgent(agent)) return null;
    const runtimeAgent = runtimeAgents.find((item) => item.agent === agent) ?? null;
    return {
      agent,
      model,
      effort: runtimeAgent?.effort ?? runtimeAgent?.efforts[0]?.value ?? "",
      permissionMode: runtimeAgent?.permissionMode ?? runtimeAgent?.permissionModes[0]?.value ?? "",
    };
  } catch {
    return null;
  }
}

function stableDraftId(
  draft: Pick<ParticipantDraft, "agent" | "model" | "effort" | "permissionMode">,
  index: number,
): string {
  return `${draft.agent}:${draft.model}:${draft.effort}:${draft.permissionMode}:${index}:${Date.now()}`;
}

function modelDisplayName(runtimeAgent: RuntimeAgentMetadata | null, model: string): string {
  if (!model) return "";
  return runtimeAgent?.models.find((option) => option.value === model)?.displayName
    ?? runtimeAgent?.models.find((option) => option.value === model)?.label
    ?? model;
}
