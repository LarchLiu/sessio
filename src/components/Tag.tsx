import { CSSProperties, ReactNode } from "react";

interface TagProps {
  label: string;
  icon?: ReactNode;
  // CSS RGB triplet (space-separated channels), e.g. "var(--color-agent-codex)"
  // or "167 139 250". Used to derive text, background, and border colors.
  color?: string;
  className?: string;
  style?: CSSProperties;
  title?: string;
}

export default function Tag({
  label,
  icon,
  color,
  className,
  style,
  title,
}: TagProps) {
  const derived: CSSProperties | undefined = color
    ? icon
      ? { color: `rgb(${color})` }
      : {
          color: `rgb(${color})`,
          background: `rgb(${color} / 0.13)`,
          border: `1px solid rgb(${color} / 0.25)`,
        }
    : undefined;
  return (
    <span
      title={title}
      style={{ ...derived, ...style }}
      className={
        "shrink-0 inline-flex items-center gap-1.5 text-caption uppercase font-medium rounded px-2 py-[3px] leading-none " +
        (className ?? "")
      }
    >
      {icon}
      {icon ? null : label}
    </span>
  );
}
