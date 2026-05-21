import { type CSSProperties, type ComponentType } from "react";
import { Claude, Gemini, OpenAI } from "@lobehub/icons";
import { AGENT_ACCENT, type Agent } from "../api";

const AGENT_ICON: Record<Agent, ComponentType<{ className?: string; style?: CSSProperties }>> = {
  codex: OpenAI,
  claude: Claude.Color,
  gemini: Gemini.Color,
};

export function AgentGlyph({
  agent,
  className,
  style,
}: {
  agent: Agent;
  className?: string;
  style?: CSSProperties;
}) {
  const Icon = AGENT_ICON[agent];
  return <Icon className={className} style={{ color: AGENT_ACCENT[agent], ...style }} />;
}

export function AgentBadge({
  agent,
  className,
}: {
  agent: Agent;
  className?: string;
}) {
  return (
    <span className={"inline-flex shrink-0 items-center justify-center " + (className ?? "")}>
      <AgentGlyph agent={agent} className={className ?? ""} />
    </span>
  );
}
