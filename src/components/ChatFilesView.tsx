import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ChevronDown, Files } from "lucide-react";
import type { FileEditItem } from "../acpRenderItems";
import { fileEditKey, fileEditMatchesPath } from "../acpRenderItems";
import { useFileContent, languageFromPath } from "../hooks/useFileContent";
import { useFileGitDiff } from "../hooks/useFileGitDiff";
import { useI18n } from "../i18n";
import FileViewer from "./FileViewer";
import ScrollArea from "./ScrollArea";

export type ChatFilesSubview = "code" | "plain";

export interface ChatFilesViewProps {
  edits: FileEditItem[];
  workspacePath: string | null;
  /** "code" enables syntax highlighting; "plain" renders raw text. */
  subview: ChatFilesSubview;
  editingLocked?: boolean;
  requestedSelection?: {
    key: string;
    requestId: number;
  } | null;
  reloadKey?: number;
}

export default function ChatFilesView({
  edits,
  workspacePath,
  subview,
  editingLocked = false,
  requestedSelection = null,
  reloadKey = 0,
}: ChatFilesViewProps) {
  const { t } = useI18n();
  const [selectedKey, setSelectedKey] = useState<string | null>(() =>
    edits[0] ? fileEditKey(edits[0]) : null,
  );
  const [pickerOpen, setPickerOpen] = useState(false);
  const pickerAnchorRef = useRef<HTMLButtonElement>(null);
  const scrollPositionsRef = useRef<Record<string, number>>({});
  const plainEditorLeaveCheckRef = useRef<(() => Promise<boolean>) | null>(null);
  const handledSelectionRequestRef = useRef<string | null>(null);

  const selectFile = useCallback(async (key: string): Promise<boolean> => {
    if (key === selectedKey) return true;
    if (subview === "plain" && plainEditorLeaveCheckRef.current) {
      const canLeave = await plainEditorLeaveCheckRef.current();
      if (!canLeave) return false;
    }
    setSelectedKey(key);
    return true;
  }, [selectedKey, subview]);

  useEffect(() => {
    if (edits.length === 0) {
      setSelectedKey(null);
      return;
    }
    if (selectedKey && edits.some((edit) => fileEditKey(edit) === selectedKey)) return;
    void selectFile(fileEditKey(edits[0]));
  }, [edits, selectedKey, selectFile]);

  useEffect(() => {
    if (!requestedSelection) return;
    const requestKey = `${requestedSelection.requestId}:${requestedSelection.key}`;
    if (handledSelectionRequestRef.current === requestKey) return;
    const matchedEdit = edits.find((edit) =>
      fileEditMatchesPath(edit, requestedSelection.key),
    );
    if (!matchedEdit) return;
    handledSelectionRequestRef.current = requestKey;
    void selectFile(fileEditKey(matchedEdit));
  }, [edits, requestedSelection?.key, requestedSelection?.requestId, selectFile]);

  const selected = useMemo(
    () =>
      selectedKey
        ? edits.find((edit) => fileEditKey(edit) === selectedKey) ?? null
        : null,
    [edits, selectedKey],
  );

  const fileContent = useFileContent(selected, workspacePath, reloadKey);
  const fileGitDiff = useFileGitDiff(selected, workspacePath, reloadKey);

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
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-ink/5 px-10">
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
          <Files className="h-3.5 w-3.5" />
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
            fileKey={`${selectedKey ?? ""}:${fileContent.mtimeMs ?? "unknown"}`}
            text={fileContent.text}
            language={languageFromPath(selected.displayPath || selected.path || "")}
            mode={subview}
            workspacePath={workspacePath}
            path={fileContent.path}
            mtimeMs={fileContent.mtimeMs}
            contentVersion={fileContent.contentVersion}
            editingLocked={editingLocked}
            onSaved={fileContent.applyLocalSave}
            onPlainEditorLeaveCheckChange={(handle) => {
              plainEditorLeaveCheckRef.current = handle;
            }}
            gitDiff={fileGitDiff.diff}
            savedScrollTop={
              selectedKey ? (scrollPositionsRef.current[selectedKey] ?? 0) : 0
            }
            onScrollTopChange={(nextTop) => {
              if (!selectedKey) return;
              scrollPositionsRef.current[selectedKey] = nextTop;
            }}
          />
        )}
      </div>
      {pickerOpen && pickerAnchorRef.current && (
        <FilePickerPopover
          anchor={pickerAnchorRef.current}
          edits={edits}
          selectedKey={selectedKey}
          onSelect={(key) => {
            void selectFile(key).then((selectedNext) => {
              if (selectedNext) setPickerOpen(false);
            });
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
  const [pos, setPos] = useState<{
    top: number;
    left: number;
    maxHeight: number;
    width: number;
  } | null>(null);

  const longestLabel = useMemo(
    () =>
      edits.reduce((max, edit) => {
        const label = edit.displayPath || edit.path || "(unknown file)";
        return Math.max(max, label.length);
      }, 0),
    [edits],
  );

  useLayoutEffect(() => {
    const update = () => {
      const rect = anchor.getBoundingClientRect();
      const gap = 6;
      const margin = 8;
      const maxHeight = Math.max(160, window.innerHeight - rect.bottom - gap - margin);
      const maxWidth = Math.max(320, window.innerWidth - margin * 2);
      const estimatedWidth = Math.max(360, Math.round(longestLabel * 7.4) + 132);
      const width = Math.min(maxWidth, estimatedWidth);
      const top = rect.bottom + gap;
      const left = Math.min(
        Math.max(margin, rect.left),
        window.innerWidth - margin - width,
      );
      setPos({ top, left, maxHeight, width });
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [anchor, longestLabel]);

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
      style={{
        top: pos.top,
        left: pos.left,
        maxHeight: pos.maxHeight,
        width: pos.width,
      }}
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
            const hasStats = edit.additions != null || edit.deletions != null;
            return (
              <li key={key}>
                <button
                  type="button"
                  role="option"
                  aria-selected={active}
                  onClick={() => onSelect(key)}
                  title={label}
                  className={
                    "grid w-full grid-cols-[minmax(0,1fr)_auto] items-start gap-3 px-2.5 py-1.5 text-left text-body-sm transition-colors " +
                    (active
                      ? "bg-ink/[0.07] text-ink/90"
                      : "text-ink/72 hover:bg-ink/[0.04] hover:text-ink/90")
                  }
                >
                  <span className="min-w-0 whitespace-pre-wrap break-all font-mono leading-relaxed">
                    {label}
                  </span>
                  {hasStats ? (
                    <span className="shrink-0 pt-0.5 font-mono text-caption">
                      <span className="text-[rgb(var(--color-emerald))]">+{edit.additions ?? 0}</span>
                      <span className="text-ink/25"> </span>
                      <span className="text-status-error">-{edit.deletions ?? 0}</span>
                    </span>
                  ) : null}
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
