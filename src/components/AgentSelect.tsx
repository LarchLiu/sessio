import type { Agent, RuntimeAgentMetadata } from "../api";
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
): InlineMenuSelectOption[] {
  return runtimeAgents.flatMap((runtimeAgent) => {
    const group: InlineMenuSelectGroup = {
      value: runtimeAgent.agent,
      label: agentLabel(runtimeAgent.agent),
      icon: <AgentGlyph agent={runtimeAgent.agent} className="h-3.5 w-3.5" />,
    };
    const models =
      runtimeAgent.models.length > 0
        ? runtimeAgent.models
        : [{ value: runtimeAgent.model ?? "", label: runtimeAgent.model ?? "Default" }];
    return models
      .filter((model) => model.value.trim().length > 0)
      .map((model) => ({
        value: agentModelSelectValue(runtimeAgent.agent, model.value),
        label: model.label || model.value,
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
    if (parsed.agent !== "codex" && parsed.agent !== "claude" && parsed.agent !== "gemini") {
      return null;
    }
    return {
      agent: parsed.agent,
      model: typeof parsed.model === "string" ? parsed.model : "",
    };
  } catch {
    if (value === "codex" || value === "claude" || value === "gemini") {
      return { agent: value, model: "" };
    }
    return null;
  }
}

function agentLabel(agent: Agent): string {
  if (agent === "codex") return "Codex";
  if (agent === "claude") return "Claude";
  return "Gemini";
}
