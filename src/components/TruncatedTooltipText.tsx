import { type HTMLAttributes } from "react";
import Tooltip from "./Tooltip";

interface TruncatedTooltipTextProps extends HTMLAttributes<HTMLDivElement> {
  text: string;
}

export default function TruncatedTooltipText({
  text,
  className,
  ...props
}: TruncatedTooltipTextProps) {
  return (
    <Tooltip content={text} placement="top" delayMs={600}>
      <div className={className} {...props}>
        {text}
      </div>
    </Tooltip>
  );
}
