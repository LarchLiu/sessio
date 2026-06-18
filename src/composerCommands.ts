import { createElement, useCallback, useEffect, useMemo, useState, type KeyboardEvent } from "react";
import type { ComposerCommandItem } from "./components/ComposerCommandMenu";
import type { Agent, AssistantInfo } from "./api";
import { AGENT_LABEL, isAgent } from "./api";
import type { AcpAvailableCommand } from "./runtimeChat";
import AssistantBotIcon from "./components/AssistantBotIcon";

export type ComposerCommandTriggerKind = "slash" | "assistant" | "thread";

export interface ComposerCommandTrigger {
  kind: ComposerCommandTriggerKind;
  query: string;
  rest: string;
  raw: string;
}

export function parseComposerCommandTrigger(
  text: string,
  kinds: ComposerCommandTriggerKind[],
): ComposerCommandTrigger | null {
  const trigger = text[0];
  const kind = triggerKindFromChar(trigger);
  if (!kind || !kinds.includes(kind)) return null;
  if (kind === "slash" && /\s/.test(text)) return null;
  const spaceIndex = text.indexOf(" ");
  const newlineIndex = text.indexOf("\n");
  const separators = [spaceIndex, newlineIndex].filter((index) => index >= 0);
  if (separators.length === 0) {
    return { kind, query: text.slice(1), rest: "", raw: text };
  }
  const splitIndex = Math.min(...separators);
  return {
    kind,
    query: text.slice(1, splitIndex),
    rest: text.slice(splitIndex + 1),
    raw: text,
  };
}

function triggerKindFromChar(value: string): ComposerCommandTriggerKind | null {
  if (value === "/") return "slash";
  if (value === "@") return "assistant";
  if (value === "#") return "thread";
  return null;
}

export function filterComposerSlashCommands(
  commands: AcpAvailableCommand[],
  query: string,
): AcpAvailableCommand[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return commands;
  return commands.filter((command) => command.name.toLowerCase().startsWith(normalized));
}

export function slashCommandItems(commands: AcpAvailableCommand[]): ComposerCommandItem[] {
  return commands.map((command) => ({
    key: command.name,
    label: `/${command.name}`,
    description: command.description || command.input?.hint || undefined,
    iconKey: "slash",
  }));
}

export function filterAssistantCommandItems(
  assistants: AssistantInfo[],
  projectId: string | null,
  query: string,
): ComposerCommandItem[] {
  const normalized = query.trim().toLowerCase();
  return assistants
    .filter((assistant) => (!projectId || assistant.projectId === projectId) && assistant.enabled)
    .filter((assistant) => assistant.name.toLowerCase().includes(normalized))
    .map((assistant) => ({
      key: assistant.id,
      label: assistant.name,
      description: AGENT_LABEL[normalizeAssistantAgent(assistant.agent.id)],
      icon: createElement(AssistantBotIcon, { color: assistant.color, className: "h-4 w-4" }),
    }));
}

export function normalizeAssistantAgent(id: string): Agent {
  return isAgent(id) ? id : "codex";
}

export function formatSlashCommandText(
  command: Pick<AcpAvailableCommand, "name" | "input">,
): string {
  return `/${command.name}${command.input?.kind === "unstructured" ? " " : ""}`;
}

export function useComposerCommandMenuState({
  trigger,
  items,
  disabled = false,
}: {
  trigger: ComposerCommandTrigger | null;
  items: ComposerCommandItem[];
  disabled?: boolean;
}) {
  const [activeIndex, setActiveIndex] = useState(0);
  const [dismissedFor, setDismissedFor] = useState<string | null>(null);

  const open = useMemo(
    () =>
      !disabled &&
      trigger !== null &&
      dismissedFor !== trigger.raw &&
      (items.length > 0 || trigger.query.length === 0),
    [disabled, dismissedFor, items.length, trigger],
  );

  useEffect(() => {
    setActiveIndex(0);
  }, [trigger?.kind, trigger?.query]);

  useEffect(() => {
    if (activeIndex >= items.length) setActiveIndex(0);
  }, [activeIndex, items.length]);

  useEffect(() => {
    if (!trigger) setDismissedFor(null);
  }, [trigger]);

  const handleKeyDown = useCallback(
    (
      event: KeyboardEvent<HTMLTextAreaElement>,
      onSelect: (key: string) => void,
      fallback?: () => boolean,
    ): boolean => {
      if (!open || items.length === 0) return fallback?.() ?? false;
      if (event.nativeEvent.isComposing) return fallback?.() ?? false;
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setActiveIndex((index) => (index + 1) % items.length);
        return true;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setActiveIndex((index) => (index - 1 + items.length) % items.length);
        return true;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        const item = items[activeIndex];
        if (!item) return fallback?.() ?? false;
        event.preventDefault();
        onSelect(item.key);
        return true;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setDismissedFor(trigger?.raw ?? null);
        return true;
      }
      return fallback?.() ?? false;
    },
    [activeIndex, items, open, trigger?.raw],
  );

  const resetDismissed = useCallback(() => {
    setDismissedFor(null);
  }, []);

  return {
    activeIndex,
    open,
    dismissedFor,
    setActiveIndex,
    setDismissedFor,
    resetDismissed,
    handleKeyDown,
  };
}
