import type { Agent, RuntimeAgentMetadata, RuntimeAgentOptionMetadata } from "../api";
import { AGENT_SHORT_LABEL, isAgent } from "../api";
import { AgentGlyph } from "./AgentIcon";
import type { InlineMenuSelectGroup, InlineMenuSelectOption } from "./InlineMenuSelect";

export function agentSelectOptions(
  runtimeAgents: RuntimeAgentMetadata[],
): InlineMenuSelectOption[] {
  return runtimeAgents.map((runtimeAgent) => ({
    value: runtimeAgent.agent,
    label: agentLabel(runtimeAgent.agent),
    icon: <AgentGlyph agent={runtimeAgent.agent} className="h-3.5 w-3.5" />,
  }));
}

export function agentModelSelectOptions(
  runtimeAgents: RuntimeAgentMetadata[],
  effortControls: Partial<Record<Agent, InlineMenuSelectGroup["control"]>> = {},
  selectedEfforts: Partial<Record<Agent, string>> = {},
): InlineMenuSelectOption[] {
  return runtimeAgents.flatMap((runtimeAgent) => {
    const group: InlineMenuSelectGroup = {
      value: runtimeAgent.agent,
      label: agentLabel(runtimeAgent.agent),
      icon: <AgentGlyph agent={runtimeAgent.agent} className="h-3.5 w-3.5" />,
      control: effortControls[runtimeAgent.agent],
    };
    const models =
      runtimeAgent.models.length > 0
        ? runtimeAgent.models
        : [{ value: runtimeAgent.model ?? "", label: runtimeAgent.model ?? "Default", displayName: runtimeAgent.model ?? "Default", enabled: true, order: 0 }];
    return models
      .filter((model) => model.value.trim().length > 0 && model.enabled)
      .map((model) => ({
        value: agentModelSelectValue(runtimeAgent.agent, model.value),
        label: model.displayName || model.label || model.value,
        suffix: effortLabel(runtimeAgent, selectedEfforts[runtimeAgent.agent]),
        icon: <AgentGlyph agent={runtimeAgent.agent} className="h-3.5 w-3.5" />,
        menuIcon: null,
        group,
      }));
  });
}

export function agentModelSelectValue(agent: Agent, model: string): string {
  return JSON.stringify({ agent, model });
}

export function parseAgentModelSelectValue(
  value: string,
): { agent: Agent; model: string } | null {
  try {
    const parsed = JSON.parse(value) as { agent?: unknown; model?: unknown };
    if (!isAgent(parsed.agent)) return null;
    return {
      agent: parsed.agent,
      model: typeof parsed.model === "string" ? parsed.model : "",
    };
  } catch {
    if (isAgent(value)) return { agent: value, model: "" };
    return null;
  }
}

export function initialRuntimeEffort(agent: RuntimeAgentMetadata | null): string {
  return agent?.effort ?? agent?.efforts[0]?.value ?? "";
}

export function runtimeEffortOptions(agent: RuntimeAgentMetadata | null): RuntimeAgentOptionMetadata[] {
  if (!agent) return [];
  if (agent.efforts.length > 0) return agent.efforts;
  return agent.effort ? [{ value: agent.effort, label: agent.effort, displayName: agent.effort, enabled: true, order: 0 }] : [];
}

function effortLabel(agent: RuntimeAgentMetadata, selectedEffort: string | undefined): string | undefined {
  const effort = selectedEffort ?? initialRuntimeEffort(agent);
  if (!effort) return undefined;
  const options = runtimeEffortOptions(agent);
  if (options.length <= 1) return undefined;
  const option = options.find((option) => option.value === effort);
  return option?.displayName || option?.label || effort;
}

function agentLabel(agent: Agent): string {
  return AGENT_SHORT_LABEL[agent] ?? agent;
}
