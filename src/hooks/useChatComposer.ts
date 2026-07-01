import {
  useCallback,
  createElement,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type ReactNode,
  type RefObject,
  type SetStateAction,
} from "react";
import type {
  Agent,
  RuntimeAgentMetadata,
  RuntimeAgentSelection,
  SetRuntimeAgentSelectionRequest,
} from "../api";
import {
  setComputerUseSessionApproval,
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
  ComposerAttachmentPreviewList,
  type ComposerAttachmentDraft,
  type ComposerImageAttachmentPreview,
  useComposerAttachments,
} from "../components/ComposerAttachments";
import { RuntimeEffortControl, runtimePermissionModeOptions } from "../components/RuntimeMenuSelect";
import { buildSessioAssistantPromptBlock } from "../historyMerge";
import type { PendingNewChatSession } from "../navigation";
import { dispatchSessionStartedFallback, type LiveRuntimeAction, type LiveRuntimeState } from "../runtimeChat";
import {
  runtimeAgentForSelection,
  selectionEffort,
  selectionModel,
  selectionPermissionMode,
} from "../runtimeAgents";
import { useComputerUseFeatureEnabled } from "./useComputerUseFeatureEnabled";

type PendingSessionExtras = Omit<
  Partial<PendingNewChatSession>,
  "sessioRuntimeSessionId" | "agent" | "projectPath" | "projectName" | "prompt" | "timestamp"
>;

export interface ChatComposerStartOptions {
  workspacePath: string;
  projectName: string;
  extraContext?: string | null;
  assistantPrompt?: string | null;
  clearComposerOnSuccess?: boolean;
  pendingSession?: PendingSessionExtras;
  onPendingCreated?: (session: PendingNewChatSession) => void;
}

export interface ChatComposerController {
  text: string;
  setText: Dispatch<SetStateAction<string>>;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  attachmentButtonRef: RefObject<HTMLButtonElement | null>;
  attachmentMenuOpen: boolean;
  setAttachmentMenuOpen: Dispatch<SetStateAction<boolean>>;
  attachmentPreview: ReactNode;
  attachments: ReturnType<typeof useComposerAttachments>["attachments"];
  supportsAttachments: boolean;
  supportsImageAttachments: boolean;
  supportsEmbeddedContext: boolean;
  appendAttachments: (items: ComposerAttachmentDraft[]) => Promise<void>;
  removeAttachment: ReturnType<typeof useComposerAttachments>["removeAttachment"];
  pickAttachments: ReturnType<typeof useComposerAttachments>["pickAttachments"];
  pasteAttachments: ReturnType<typeof useComposerAttachments>["pasteAttachments"];
  sending: boolean;
  composerError: string | null;
  setComposerError: Dispatch<SetStateAction<string | null>>;
  canSend: boolean;
  canSendWithWorkspace: (workspacePath: string | null | undefined) => boolean;
  sendWithContext: (
    prompt: string,
    options?: {
      clearComposer?: boolean;
      attachments?: ReturnType<typeof useComposerAttachments>["attachments"];
      runtimeOptions?: Record<string, unknown>;
    },
  ) => Promise<{ ok: boolean; turnId: string | null }>;
  selectedAgent: Agent | null;
  selectedRuntimeAgent: RuntimeAgentMetadata | null;
  selectedModel: string;
  selectedEffort: string;
  selectedAgentModelValue: string;
  permissionMode: string;
  computerUseEnabled: boolean;
  computerUseActive: boolean;
  setComputerUseEnabled: Dispatch<SetStateAction<boolean>>;
  handleComputerUseToggle: () => void | Promise<void>;
  computerUseEligible: boolean;
  agentModelOptions: ReturnType<typeof agentModelSelectOptions>;
  permissionOptions: ReturnType<typeof runtimePermissionModeOptions>;
  handleAgentModelChange: (nextValue: string) => Promise<void>;
  handlePermissionModeChange: (nextValue: string) => Promise<void>;
  applyAgentSelection: (selection: {
    agent: Agent;
    model?: string;
    effort?: string;
    permissionMode?: string;
  }) => void;
  runStartSession: (prompt: string, options: ChatComposerStartOptions) => Promise<boolean>;
}

export function useChatComposer({
  runtimeAgents,
  lastRuntimeAgentSelection,
  rememberRuntimeAgentSelection,
  liveState,
  dispatchLiveEvent,
  onError,
  onPendingSession,
  onPreviewImageAttachment,
}: {
  runtimeAgents: RuntimeAgentMetadata[];
  lastRuntimeAgentSelection: RuntimeAgentSelection | null;
  rememberRuntimeAgentSelection: (selection: SetRuntimeAgentSelectionRequest) => Promise<void>;
  liveState: LiveRuntimeState;
  dispatchLiveEvent: Dispatch<LiveRuntimeAction>;
  onError: (error: string | null) => void;
  onPendingSession: (session: PendingNewChatSession) => void;
  onPreviewImageAttachment?: (image: ComposerImageAttachmentPreview) => void;
}): ChatComposerController {
  const initialRuntimeAgent =
    runtimeAgentForSelection(runtimeAgents, lastRuntimeAgentSelection) ?? runtimeAgents[0] ?? null;
  const [text, setText] = useState("");
  const [agent, setAgent] = useState<Agent | "">(() => initialRuntimeAgent?.agent ?? "");
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
  const [computerUseEnabled, setComputerUseEnabled] = useState(false);
  const [attachmentMenuOpen, setAttachmentMenuOpen] = useState(false);
  const [sending, setSending] = useState(false);
  const [composerError, setComposerError] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const attachmentButtonRef = useRef<HTMLButtonElement>(null);
  const fallbackRuntimeSequenceRef = useRef(0);
  const selectedAgentModelValue = agent ? agentModelSelectValue(agent, model) : "";
  const selectedRuntimeAgent =
    agent ? runtimeAgents.find((runtimeAgent) => runtimeAgent.agent === agent) ?? null : null;
  const computerUseFeatureEnabled = useComputerUseFeatureEnabled();
  const computerUseEligible = Boolean(
    selectedRuntimeAgent?.computerUseEligible && computerUseFeatureEnabled,
  );

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
            createElement(RuntimeEffortControl, {
              value: runtimeAgent.agent === agent ? effort : initialRuntimeEffort(runtimeAgent),
              options: runtimeEffortOptions(runtimeAgent),
              onChange: (value: string) => void handleEffortChange(runtimeAgent.agent, value),
              disabled: sending,
            }),
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
    addAttachments,
    removeAttachment,
    clearAttachments,
    pickAttachments,
    pasteAttachments,
  } = useComposerAttachments({
    capabilities: selectedRuntimeAgent?.capabilities,
    onError: (message) => {
      setComposerError(message);
      onError(message);
    },
  });

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
    if (computerUseEligible) return;
    setComputerUseEnabled(false);
  }, [computerUseEligible]);

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

  const applyAgentSelection = (selection: {
    agent: Agent;
    model?: string;
    effort?: string;
    permissionMode?: string;
  }) => {
    const targetRuntimeAgent =
      runtimeAgents.find((runtimeAgent) => runtimeAgent.agent === selection.agent) ?? null;
    if (!targetRuntimeAgent) return;
    const nextModel =
      selection.model && targetRuntimeAgent.models.some((option) => option.value === selection.model && option.enabled)
        ? selection.model
        : initialRuntimeModel(targetRuntimeAgent);
    const nextEffort =
      selection.effort && targetRuntimeAgent.efforts.some((option) => option.value === selection.effort)
        ? selection.effort
        : initialRuntimeEffort(targetRuntimeAgent);
    const nextPermissionMode =
      selection.permissionMode &&
      targetRuntimeAgent.permissionModes.some((option) => option.value === selection.permissionMode)
        ? selection.permissionMode
        : initialRuntimePermission(targetRuntimeAgent);
    setAgent(selection.agent);
    setModel(nextModel);
    setEffort(nextEffort);
    setPermissionMode(nextPermissionMode);
  };

  const runStartSession = async (
    promptValue: string,
    options: ChatComposerStartOptions,
  ): Promise<boolean> => {
    const prompt = promptValue.trim();
    if (!prompt || sending) return false;
    if (!agentModelOptions.some((option) => option.value === selectedAgentModelValue) || !agent) {
      const message = "No configured runtime agent available";
      setComposerError(message);
      onError(message);
      return false;
    }
    setSending(true);
    setComposerError(null);
    onError(null);
    try {
      const handle = await startAgentSession({
        agent,
        workspacePath: options.workspacePath,
        options: runtimeSessionOptions(model, permissionMode, effort, computerUseEnabled),
      });
      if (computerUseEnabled) {
        await setComputerUseSessionApproval(handle.sessioRuntimeSessionId, true);
      }
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
      const pendingSession: PendingNewChatSession = {
        ...(options.pendingSession ?? {}),
        sessioRuntimeSessionId: handle.sessioRuntimeSessionId,
        agent: handle.agent,
        projectPath: options.workspacePath,
        projectName: options.projectName,
        prompt,
        timestamp,
      };
      onPendingSession(pendingSession);
      options.onPendingCreated?.(pendingSession);
      const assistantPrompt = options.assistantPrompt?.trim()
        ? buildSessioAssistantPromptBlock(options.assistantPrompt, {
          source: "assistant",
        })
        : "";
      const visibleContext = options.extraContext?.trim() || "";
      const contextBlocks = [assistantPrompt, visibleContext].filter(Boolean);
      const inputText = contextBlocks.length > 0
        ? `${contextBlocks.join("\n\n")}\n\n---\n\n${prompt}`
        : prompt;
      await sendAgentInput(handle.sessioRuntimeSessionId, {
        text: inputText,
        attachments: attachments.map(({ path, mimeType, kind, displayName }) => ({
          path,
          mimeType,
          kind,
          displayName,
        })),
      });
      if (options.clearComposerOnSuccess ?? true) {
        setText("");
        clearAttachments();
      }
      return true;
    } catch (err) {
      const message = String(err);
      setComposerError(message);
      onError(message);
      return false;
    } finally {
      setSending(false);
    }
  };

  return {
    text,
    setText,
    textareaRef,
    attachmentButtonRef,
    attachmentMenuOpen,
    setAttachmentMenuOpen,
    attachmentPreview: createElement(ComposerAttachmentPreviewList, {
      attachments,
      onRemove: removeAttachment,
      onPreviewImage: onPreviewImageAttachment,
    }),
    attachments,
    supportsAttachments,
    supportsImageAttachments,
    supportsEmbeddedContext,
    appendAttachments: addAttachments,
    removeAttachment,
    pickAttachments,
    pasteAttachments,
    sending,
    composerError,
    setComposerError,
    canSend: text.trim().length > 0 && agentModelOptions.length > 0 && !sending,
    canSendWithWorkspace: (workspacePath) =>
      text.trim().length > 0 && Boolean(workspacePath) && agentModelOptions.length > 0 && !sending,
    sendWithContext: async () => ({ ok: false, turnId: null }),
    selectedAgent: agent || null,
    selectedRuntimeAgent,
    selectedModel: model,
    selectedEffort: effort,
    selectedAgentModelValue,
    permissionMode,
    computerUseEnabled,
    computerUseActive: computerUseEnabled,
    setComputerUseEnabled,
    handleComputerUseToggle: () => setComputerUseEnabled((enabled) => !enabled),
    computerUseEligible,
    agentModelOptions,
    permissionOptions,
    handleAgentModelChange,
    handlePermissionModeChange,
    applyAgentSelection,
    runStartSession,
  };
}

function initialRuntimePermission(agent: RuntimeAgentMetadata | null): string {
  return agent?.permissionMode ?? agent?.permissionModes[0]?.value ?? "";
}

function initialRuntimeModel(agent: RuntimeAgentMetadata | null): string {
  return agent?.model ?? agent?.models.find((option) => option.enabled)?.value ?? "";
}

export function runtimeSessionOptions(
  model: string,
  permissionMode: string,
  effort = "",
  computerUse = false,
): Record<string, unknown> {
  return {
    ...(model ? { model } : {}),
    ...(effort ? { effort } : {}),
    ...(permissionMode ? { permissionMode } : {}),
    ...(computerUse ? { computerUse: true } : {}),
  };
}
