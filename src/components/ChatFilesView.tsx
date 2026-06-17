import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ChevronDown, FolderTree } from "lucide-react";
import type { FileEditItem } from "../acpRenderItems";
import { fileEditKey } from "../acpRenderItems";
import { useFileContent, languageFromPath } from "../hooks/useFileContent";
import { useI18n } from "../i18n";
import FileViewer from "./FileViewer";
import ScrollArea from "./ScrollArea";

export type ChatFilesSubview = "code" | "plain";

export interface ChatFilesViewProps {
  edits: FileEditItem[];
  workspacePath: string | null;
  /** "code" enables syntax highlighting; "plain" renders raw text. */
  subview: ChatFilesSubview;
}

export default function ChatFilesView({ edits, workspacePath, subview }: ChatFilesViewProps) {
  const { t } = useI18n();
  const [selectedKey, setSelectedKey] = useState<string | null>(() =>
    edits[0] ? fileEditKey(edits[0]) : null,
  );
  const [pickerOpen, setPickerOpen] = useState(false);
  const pickerAnchorRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (edits.length === 0) {
      setSelectedKey(null);
      return;
    }
    setSelectedKey((current) => {
      if (current && edits.some((edit) => fileEditKey(edit) === current)) return current;
      return fileEditKey(edits[0]);
    });
  }, [edits]);

  const selected = useMemo(
    () =>
      selectedKey
        ? edits.find((edit) => fileEditKey(edit) === selectedKey) ?? null
        : null,
    [edits, selectedKey],
  );

  const fileContent = useFileContent(selected, workspacePath);

  if (edits.length === 0) {
    return (
      <div className="flex h-full min-h-0 flex-1 items-center justify-center px-6 text-body-sm text-ink/45">
        {t("chat.files.empty")}
      </div>
    );
  }

  const selectedLabel = selected
    ? selected.displayPath || selected.path || ""
    : "";

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-ink/5 px-3">
        <button
          ref={pickerAnchorRef}
          type="button"
          aria-haspopup="listbox"
          aria-expanded={pickerOpen}
          aria-label={t("chat.files.choose_file")}
          onClick={() => setPickerOpen((open) => !open)}
          className={
            "inline-flex h-6 shrink-0 items-center gap-1 rounded-md px-1.5 text-ink/55 transition-colors hover:bg-ink/[0.05] hover:text-ink/80 " +
            (pickerOpen ? "bg-ink/[0.07] text-ink/85" : "")
          }
        >
          <FolderTree className="h-3.5 w-3.5" />
          <ChevronDown className="h-3 w-3 opacity-70" />
        </button>
        <div className="min-w-0 flex-1 truncate font-mono text-caption text-ink/72">
          {selectedLabel}
        </div>
      </div>
      <div className="relative flex min-h-0 flex-1 flex-col">
        {fileContent.loading && (
          <div className="absolute inset-0 z-10 flex items-center justify-center bg-surface-panel/40 text-body-sm text-ink/45">
            {t("chat.files.loading")}
          </div>
        )}
        {fileContent.error && !fileContent.loading && (
          <div className="m-3 rounded-md border border-status-warn/30 bg-status-warn/[0.08] p-3 text-body-sm text-status-warn">
            {t("chat.files.unavailable")}
            <div className="mt-1 font-mono text-caption opacity-70">{fileContent.error}</div>
          </div>
        )}
        {!fileContent.loading && !fileContent.error && fileContent.text !== null && selected && (
          <FileViewer
            text={fileContent.text}
            language={languageFromPath(selected.displayPath || selected.path || "")}
            mode={subview}
          />
        )}
      </div>
      {pickerOpen && pickerAnchorRef.current && (
        <FilePickerPopover
          anchor={pickerAnchorRef.current}
          edits={edits}
          selectedKey={selectedKey}
          onSelect={(key) => {
            setSelectedKey(key);
            setPickerOpen(false);
          }}
          onClose={() => setPickerOpen(false)}
        />
      )}
    </div>
  );
}

function FilePickerPopover({
  anchor,
  edits,
  selectedKey,
  onSelect,
  onClose,
}: {
  anchor: HTMLElement;
  edits: FileEditItem[];
  selectedKey: string | null;
  onSelect: (key: string) => void;
  onClose: () => void;
}) {
  const popoverRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number; maxHeight: number } | null>(null);

  useLayoutEffect(() => {
    const update = () => {
      const rect = anchor.getBoundingClientRect();
      const gap = 6;
      const margin = 8;
      const maxHeight = Math.max(160, window.innerHeight - rect.bottom - gap - margin);
      const top = rect.bottom + gap;
      const left = Math.max(margin, rect.left);
      setPos({ top, left, maxHeight });
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [anchor]);

  useEffect(() => {
    const onPointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (!target) return;
      if (popoverRef.current?.contains(target)) return;
      if (anchor.contains(target)) return;
      onClose();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [anchor, onClose]);

  if (!pos) return null;

  const node = (
    <div
      ref={popoverRef}
      role="listbox"
      style={{ top: pos.top, left: pos.left, maxHeight: pos.maxHeight, width: 360 }}
      className="fixed z-50 overflow-hidden rounded-md border border-ink/[0.10] bg-surface-panel shadow-[0_8px_24px_rgba(0,0,0,0.18)]"
    >
      <ScrollArea
        className="max-h-full"
        viewportClassName="py-1"
        persistScrollbars
      >
        <ul className="flex flex-col">
          {edits.map((edit) => {
            const key = fileEditKey(edit);
            const label = edit.displayPath || edit.path || "(unknown file)";
            const active = key === selectedKey;
            return (
              <li key={key}>
                <button
                  type="button"
                  role="option"
                  aria-selected={active}
                  onClick={() => onSelect(key)}
                  title={label}
                  className={
                    "grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-2.5 py-1.5 text-left text-body-sm transition-colors " +
                    (active
                      ? "bg-ink/[0.07] text-ink/90"
                      : "text-ink/72 hover:bg-ink/[0.04] hover:text-ink/90")
                  }
                >
                  <span className="min-w-0 truncate font-mono">{label}</span>
                  <span className="shrink-0 font-mono text-caption">
                    <span className="text-[rgb(var(--color-emerald))]">+{edit.additions ?? 0}</span>
                    <span className="text-ink/25"> </span>
                    <span className="text-status-error">-{edit.deletions ?? 0}</span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </ScrollArea>
    </div>
  );

  return createPortal(node, document.body);
}
