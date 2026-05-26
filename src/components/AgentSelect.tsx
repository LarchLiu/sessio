import type { RuntimeAgentMetadata } from "../api";
import { AgentGlyph } from "./AgentIcon";
import type { InlineMenuSelectOption } from "./InlineMenuSelect";

export function agentSelectOptions(
  runtimeAgents: RuntimeAgentMetadata[],
): InlineMenuSelectOption[] {
  return runtimeAgents.map((runtimeAgent) => ({
    value: runtimeAgent.agent,
    label:
      runtimeAgent.agent === "codex"
        ? "Codex"
        : runtimeAgent.agent === "claude"
          ? "Claude"
          : "Gemini",
    icon: <AgentGlyph agent={runtimeAgent.agent} className="h-3.5 w-3.5" />,
  }));
}
