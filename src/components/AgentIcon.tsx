import { type CSSProperties, type ReactNode } from "react";
import { Claude, Gemini, OpenAI } from "@lobehub/icons";
import { type Agent } from "../api";
import { PiIcon } from "./IconifyIcon";

const AGENT_ICON: Record<Agent, (props: { className?: string; style?: CSSProperties }) => ReactNode> = {
  "astra-pi": PiIcon,
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
  return <Icon className={className} style={{ color: "rgb(var(--color-fg))", ...style }} />;
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
