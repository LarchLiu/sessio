import {
  memo,
  type RefObject,
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { Check, Copy } from "lucide-react";
import type { SessionInfo } from "../api";
import { AGENT_LABEL } from "../api";
import { localeTag, useI18n } from "../i18n";
import ScrollArea from "./ScrollArea";

interface Props {
  session: SessionInfo;
}

interface MetaRow {
  label: string;
  value: string | null;
  copyable?: boolean;
  clampLines?: number;
}

interface MemoryAnchor {
  id: string;
  offset: number;
}

interface MemoryCacheEntry {
  scrollTop: number;
  anchor: MemoryAnchor | null;
}

const memoryViewCache = new Map<string, MemoryCacheEntry>();

function memorySourceKey(session: SessionInfo): string {
  return `${session.agent}:${session.id}:${session.filePath}`;
}

function SessionMemory({ session }: Props) {
  const sourceKey = memorySourceKey(session);
  const viewportRef = useRef<HTMLDivElement>(null);
  const anchorRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const pendingInitialPositionRef = useRef<"top" | "restore" | null>(
    memoryViewCache.has(sourceKey) ? "restore" : "top",
  );
  const initialPositionAppliedRef = useRef(false);

  useLayoutEffect(() => {
    pendingInitialPositionRef.current = memoryViewCache.has(sourceKey)
      ? "restore"
      : "top";
    initialPositionAppliedRef.current = false;
  }, [sourceKey]);

  const saveScrollSnapshot = useCallback(
    (vp: HTMLDivElement | null = viewportRef.current) => {
      if (!vp || !initialPositionAppliedRef.current) return;
      const vpRect = vp.getBoundingClientRect();
      let anchor: MemoryAnchor | null = null;
      let bestOffset = Number.NEGATIVE_INFINITY;
      for (const [id, el] of anchorRefs.current) {
        const offset = el.getBoundingClientRect().top - vpRect.top;
        if (offset <= 0 && offset > bestOffset) {
          bestOffset = offset;
          anchor = { id, offset };
        }
      }
      memoryViewCache.set(sourceKey, {
        scrollTop: vp.scrollTop,
        anchor,
      });
    },
    [sourceKey],
  );

  useLayoutEffect(() => {
    const vp = viewportRef.current;
    const mode = pendingInitialPositionRef.current;
    if (!vp || mode === null) return;
    const snapshot = memoryViewCache.get(sourceKey);
    if (mode === "restore" && snapshot?.anchor) {
      const el = anchorRefs.current.get(snapshot.anchor.id);
      if (el) {
        const vpRect = vp.getBoundingClientRect();
        const top = el.getBoundingClientRect().top - vpRect.top + vp.scrollTop;
        vp.scrollTop = Math.max(0, top - snapshot.anchor.offset);
      } else {
        vp.scrollTop = Math.max(
          0,
          Math.min(snapshot.scrollTop, vp.scrollHeight - vp.clientHeight),
        );
      }
    } else if (mode === "restore" && snapshot) {
      vp.scrollTop = Math.max(
        0,
        Math.min(snapshot.scrollTop, vp.scrollHeight - vp.clientHeight),
      );
    } else {
      vp.scrollTop = 0;
    }
    pendingInitialPositionRef.current = null;
    initialPositionAppliedRef.current = true;
  }, [sourceKey]);

  useLayoutEffect(() => {
    return () => {
      if (initialPositionAppliedRef.current) saveScrollSnapshot();
    };
  }, [saveScrollSnapshot]);

  return (
    <ScrollArea
      ref={viewportRef}
      className="flex-1 min-h-0 bg-surface-panel"
      onScroll={saveScrollSnapshot}
    >
      <div className="mx-auto w-full max-w-3xl px-10 py-8">
        <div
          className="mb-5"
          ref={(el) => {
            if (el) anchorRefs.current.set("header", el);
            else anchorRefs.current.delete("header");
          }}
        >
          <div className="text-subtitle font-medium text-ink/85">Memory</div>
          <div className="text-body-sm text-ink/45">Session metadata</div>
        </div>
        <SessionMetaList session={session} anchorRefs={anchorRefs} />
      </div>
    </ScrollArea>
  );
}

export default memo(SessionMemory);

export function SessionMetaList({
  session,
  anchorRefs,
}: Props & {
  anchorRefs?: RefObject<Map<string, HTMLDivElement>>;
}) {
  const { lang, t } = useI18n();
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const rows: MetaRow[] = [
    { label: t("meta.title"), value: session.title, clampLines: 2 },
    { label: t("meta.agent"), value: AGENT_LABEL[session.agent] },
    { label: t("meta.session_id"), value: session.id, copyable: true },
    {
      label: t("meta.project"),
      value: session.projectPath ?? session.projectName,
      copyable: Boolean(session.projectPath ?? session.projectName),
    },
    { label: t("meta.started"), value: formatDate(session.startedAt, lang) },
    { label: t("meta.updated"), value: formatDate(session.updatedAt, lang) },
    {
      label: t("meta.messages"),
      value: `${session.partial ? "~" : ""}${session.messageCount}`,
    },
    { label: t("meta.file"), value: session.filePath, copyable: true },
    { label: t("meta.file_size"), value: formatBytes(session.fileSize) },
    { label: t("meta.archived"), value: session.archived ? "Yes" : "No" },
    { label: t("meta.subagents"), value: String(session.subagents.length) },
  ];

  const copyValue = async (label: string, value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedKey(label);
      window.setTimeout(() => setCopiedKey(null), 1200);
    } catch (err) {
      console.error("copy metadata value failed", err);
    }
  };

  return (
    <div className="overflow-hidden rounded-b-xl border-x border-b border-ink/10 bg-surface-panel">
      {rows.map((row) => (
        <SessionMetaRow
          key={row.label}
          id={row.label}
          row={row}
          copied={copiedKey === row.label}
          onCopy={copyValue}
          anchorRefs={anchorRefs}
        />
      ))}
    </div>
  );
}

function SessionMetaRow({
  id,
  row,
  copied,
  onCopy,
  anchorRefs,
}: {
  id: string;
  row: MetaRow;
  copied: boolean;
  onCopy: (label: string, value: string) => void;
  anchorRefs?: RefObject<Map<string, HTMLDivElement>>;
}) {
  const copyValue = row.copyable && row.value ? row.value : null;
  return (
    <div
      ref={(el) => {
        if (!anchorRefs) return;
        if (el) anchorRefs.current.set(id, el);
        else anchorRefs.current.delete(id);
      }}
      className="grid grid-cols-[140px_minmax(0,1fr)] items-center gap-4 border-b border-ink/[0.06] px-4 py-2.5 last:border-b-0"
    >
      <div className="text-caption uppercase text-ink/35">{row.label}</div>
      <div className="flex min-w-0 items-center gap-2 text-body-sm text-ink/75">
        <span
          className="min-w-0 flex-1 break-words"
          style={
            row.clampLines
              ? {
                  display: "-webkit-box",
                  WebkitLineClamp: row.clampLines,
                  WebkitBoxOrient: "vertical",
                  overflow: "hidden",
                }
              : undefined
          }
        >
          {row.value || <span className="text-ink/30">-</span>}
        </span>
        {copyValue && (
          <button
            type="button"
            className="shrink-0 rounded p-1 text-ink/35 transition-colors hover:bg-ink/[0.06] hover:text-ink/70"
            onClick={() => onCopy(row.label, copyValue)}
            aria-label={`Copy ${row.label}`}
          >
            {copied ? (
              <Check className="h-3.5 w-3.5" />
            ) : (
              <Copy className="h-3.5 w-3.5" />
            )}
          </button>
        )}
      </div>
    </div>
  );
}

function formatDate(ts: number | null, lang: "en" | "zh"): string | null {
  if (!ts) return null;
  return new Date(ts).toLocaleString(localeTag(lang), {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let size = bytes;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}
