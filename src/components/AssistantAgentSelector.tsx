import { type ReactNode, useMemo } from "react";
import type { Agent, AgentInfo, AssistantAgentInfo, RuntimeAgentMetadata, RuntimeAgentOptionMetadata } from "../api";
import { AGENT_LABEL, isAgent } from "../api";
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
  return agent?.model ?? agent?.models.find((model) => model.enabled)?.value ?? "";
}

function optionValue(options: RuntimeAgentOptionMetadata[], fallback: string) {
  return options.find((option) => option.enabled)?.value ?? options[0]?.value ?? fallback;
}

export function dbAgentsAsRuntimeAgents(agents: AgentInfo[]): RuntimeAgentMetadata[] {
  return agents
    .filter((agent) => agent.enabled && isAgent(agent.id))
    .map((agent) => ({
      agent: agent.id as Agent,
      enabled: agent.enabled,
      configured: agent.enabled,
      order: agent.order,
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
  const agentKey: Agent = isAgent(agent.id) ? agent.id : "codex";
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
    <div className="inline-flex min-w-0 max-w-full items-center gap-0.5">
      <RuntimeMenuSelect ariaLabel={t("agent.title")} value={agentModelValue} options={agentModelOptions} onChange={selectAgentModel} minMenuWidth={220} maxWidthClassName={compact ? "max-w-[210px]" : "max-w-[260px]"} />
      <RuntimeMenuSelect ariaLabel={t("assistant.permission_mode")} value={agent.mode} options={permissionOptions} onChange={(mode) => onChange({ ...agent, mode })} minMenuWidth={180} maxWidthClassName={compact ? "max-w-[150px]" : "max-w-[190px]"} />
    </div>
  );
}
