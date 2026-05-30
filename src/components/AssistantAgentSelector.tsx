import { type ReactNode, useMemo } from "react";
import type { Agent, AgentInfo, AssistantAgentInfo, RuntimeAgentMetadata, RuntimeAgentOptionMetadata } from "../api";
import { AGENT_LABEL } from "../api";
import {
  agentModelSelectOptions,
  agentModelSelectValue,
  initialRuntimeEffort,
  parseAgentModelSelectValue,
  runtimeEffortOptions,
} from "./AgentSelect";
import { RuntimeEffortControl, RuntimeMenuSelect, runtimePermissionModeOptions } from "./RuntimeMenuSelect";
import { useI18n } from "../i18n";

function initialRuntimeModel(agent: RuntimeAgentMetadata | null): string {
  return agent?.model ?? agent?.models[0]?.value ?? "";
}

function optionValue(options: RuntimeAgentOptionMetadata[], fallback: string) {
  return options[0]?.value ?? fallback;
}

export function dbAgentsAsRuntimeAgents(agents: AgentInfo[]): RuntimeAgentMetadata[] {
  return agents
    .filter((agent) => agent.enabled && (agent.id === "codex" || agent.id === "claude" || agent.id === "gemini"))
    .map((agent) => ({
      agent: agent.id as Agent,
      enabled: agent.enabled,
      configured: agent.commands.session.length > 0,
      transport: agent.transport,
      model: agent.model,
      models: agent.models,
      effort: agent.effort,
      efforts: agent.efforts,
      permissionMode: agent.permissionMode,
      permissionModes: agent.permissionModes,
      sessionCommand: agent.commands.session[0] ?? null,
      versionCommand: agent.commands.version[0] ?? null,
      detectedVersion: null,
      capabilities: null,
      updatedAt: agent.updatedAt,
    }));
}

export function assistantAgentPayloadFromRuntime(
  runtimeAgent: RuntimeAgentMetadata | null | undefined,
  model: string,
  mode: string,
  effort: string,
): AssistantAgentInfo {
  return {
    id: runtimeAgent?.agent ?? "",
    name: runtimeAgent ? AGENT_LABEL[runtimeAgent.agent] : "",
    model,
    mode,
    effort,
  };
}

export function defaultAssistantAgent(runtimeAgent: RuntimeAgentMetadata | null | undefined): AssistantAgentInfo {
  return assistantAgentPayloadFromRuntime(
    runtimeAgent,
    initialRuntimeModel(runtimeAgent ?? null),
    runtimeAgent?.permissionMode ?? runtimeAgent?.permissionModes[0]?.value ?? "",
    initialRuntimeEffort(runtimeAgent ?? null),
  );
}

export default function AssistantAgentSelector({
  agent,
  agents,
  onChange,
  compact = false,
}: {
  agent: AssistantAgentInfo;
  agents: AgentInfo[];
  onChange: (agent: AssistantAgentInfo) => void;
  compact?: boolean;
}) {
  const { t } = useI18n();
  const runtimeAgents = useMemo(() => dbAgentsAsRuntimeAgents(agents), [agents]);
  const agentKey = (agent.id === "claude" || agent.id === "gemini" ? agent.id : "codex") as Agent;
  const selectedAgent = runtimeAgents.find((runtimeAgent) => runtimeAgent.agent === agent.id) ?? null;
  const agentModelValue = agentModelSelectValue(agentKey, agent.model);
  const agentModelOptions = agentModelSelectOptions(
    runtimeAgents,
    Object.fromEntries(
      runtimeAgents.map((runtimeAgent) => [
        runtimeAgent.agent,
        <RuntimeEffortControl
          value={runtimeAgent.agent === agent.id ? agent.effort : initialRuntimeEffort(runtimeAgent)}
          options={runtimeEffortOptions(runtimeAgent)}
          onChange={(effort) => {
            if (runtimeAgent.agent !== agent.id) return;
            onChange({ ...agent, effort });
          }}
        />,
      ]),
    ) as Partial<Record<Agent, ReactNode>>,
    { [agentKey]: agent.effort },
  );
  const permissionOptions = runtimePermissionModeOptions(
    selectedAgent?.permissionModes ?? [],
    agent.mode,
    selectedAgent?.agent,
  );

  const selectAgentModel = (value: string) => {
    const parsed = parseAgentModelSelectValue(value);
    if (!parsed) return;
    const next = runtimeAgents.find((agent) => agent.agent === parsed.agent);
    if (!next) return;
    onChange(
      assistantAgentPayloadFromRuntime(
        next,
        parsed.model || optionValue(next.models, agent.model),
        optionValue(next.permissionModes, agent.mode),
        next.agent === agent.id ? agent.effort : initialRuntimeEffort(next),
      ),
    );
  };

  return (
    <div className={compact ? "grid grid-cols-2 gap-1" : "grid grid-cols-[minmax(0,1fr)_minmax(0,0.72fr)] gap-2"}>
      <RuntimeMenuSelect ariaLabel={t("agent.title")} value={agentModelValue} options={agentModelOptions} onChange={selectAgentModel} minMenuWidth={220} maxWidthClassName="max-w-none" />
      <RuntimeMenuSelect ariaLabel={t("assistant.permission_mode")} value={agent.mode} options={permissionOptions} onChange={(mode) => onChange({ ...agent, mode })} minMenuWidth={180} maxWidthClassName="max-w-none" />
    </div>
  );
}
