import { Robot3FillIcon } from "./IconifyIcon";

export default function AssistantBotIcon({
  color,
  className = "h-4 w-4 shrink-0",
}: {
  color?: string | null;
  className?: string;
}) {
  return <Robot3FillIcon className={className} style={color ? { color } : undefined} />;
}
