import { CSSProperties } from "react";

interface TagProps {
  label: string;
  className?: string;
  style?: CSSProperties;
  title?: string;
}

export default function Tag({ label, className, style, title }: TagProps) {
  return (
    <span
      title={title}
      style={style}
      className={
        "shrink-0 text-caption uppercase font-medium rounded px-2 py-[3px] " +
        (className ?? "")
      }
    >
      {label}
    </span>
  );
}
