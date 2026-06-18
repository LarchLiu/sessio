import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ChevronDown, Code2, Eye, Files, Pencil } from "lucide-react";
import type { FileEditItem } from "../acpRenderItems";
import { fileEditKey, fileEditMatchesPath } from "../acpRenderItems";
import {
  isPlainEditorEditableDocumentPath,
  isPlainEditorMarkdownDocumentPath,
} from "../hooks/plainEditorFileTypes";
import { useFileContent, languageFromPath } from "../hooks/useFileContent";
import { useFileGitDiff } from "../hooks/useFileGitDiff";
import { useI18n } from "../i18n";
import FileViewer from "./FileViewer";
import type { PlainEditorMode } from "./PlainEditorView";
import ScrollArea from "./ScrollArea";
import Tooltip from "./Tooltip";
import "./plain-editor-theme.css";

export type ChatFilesSubview = "code" | "plain";
type ChatFilesDisplayMode = "code" | PlainEditorMode;

export interface ChatFilesViewProps {
  edits: FileEditItem[];
  workspacePath: string | null;
  /** "code" enables syntax highlighting; "plain" renders raw text. */
  subview: ChatFilesSubview;
  onSubviewChange?: (subview: ChatFilesSubview) => void;
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
  onSubviewChange,
  editingLocked = false,
  requestedSelection = null,
  reloadKey = 0,
}: ChatFilesViewProps) {
  const { t } = useI18n();
  const [selectedKey, setSelectedKey] = useState<string | null>(() =>
    edits[0] ? fileEditKey(edits[0]) : null,
  );
  const [pickerOpen, setPickerOpen] = useState(false);
  const [plainEditorMode, setPlainEditorMode] = useState<PlainEditorMode>("edit");
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
  const selectedLabel = selected
    ? selected.displayPath || selected.path || ""
    : "";
  const selectedPath = fileContent.path ?? selected?.displayPath ?? selected?.path ?? null;
  const documentFile = isPlainEditorEditableDocumentPath(selectedPath);
  const previewDocument = isPlainEditorMarkdownDocumentPath(selectedPath);
  const effectiveSubview: ChatFilesSubview = documentFile ? subview : "code";
  const effectivePlainEditorMode: PlainEditorMode =
    previewDocument ? plainEditorMode : "edit";
  const displayMode: ChatFilesDisplayMode =
    effectiveSubview === "plain" ? effectivePlainEditorMode : "code";
  const showDocumentControls = documentFile;

  useEffect(() => {
    if (effectiveSubview !== "plain" && plainEditorMode !== "edit") {
      setPlainEditorMode("edit");
    }
  }, [effectiveSubview, plainEditorMode]);

  useEffect(() => {
    if (!previewDocument && plainEditorMode === "preview") {
      setPlainEditorMode("edit");
    }
  }, [plainEditorMode, previewDocument]);

  const handleDisplayModeChange = useCallback(async (nextMode: ChatFilesDisplayMode) => {
    if (nextMode === displayMode) return;
    if (nextMode === "preview" && !previewDocument) return;

    if (nextMode === "code") {
      if (effectiveSubview === "plain" && plainEditorLeaveCheckRef.current) {
        const canLeave = await plainEditorLeaveCheckRef.current();
        if (!canLeave) return;
      }
      setPlainEditorMode("edit");
      onSubviewChange?.("code");
      return;
    }

    setPlainEditorMode(nextMode);
    if (effectiveSubview !== "plain") {
      onSubviewChange?.("plain");
    }
  }, [displayMode, effectiveSubview, onSubviewChange, previewDocument]);

  if (edits.length === 0) {
    return (
      <div className="flex h-full min-h-0 flex-1 items-center justify-center px-6 text-body-sm text-ink/45">
        {t("chat.files.empty")}
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-2 px-10">
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
        <div className="flex shrink-0 items-center gap-1">
          {showDocumentControls && (
            <FileDisplayModeToggle
              value={displayMode}
              previewAvailable={previewDocument}
              disabled={!onSubviewChange}
              onChange={(next) => {
                void handleDisplayModeChange(next);
              }}
            />
          )}
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
            mode={effectiveSubview}
            workspacePath={workspacePath}
            path={fileContent.path}
            mtimeMs={fileContent.mtimeMs}
            contentVersion={fileContent.contentVersion}
            editingLocked={editingLocked}
            plainEditorMode={effectivePlainEditorMode}
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

function FileDisplayModeToggle({
  value,
  previewAvailable,
  disabled = false,
  onChange,
}: {
  value: ChatFilesDisplayMode;
  previewAvailable: boolean;
  disabled?: boolean;
  onChange: (value: ChatFilesDisplayMode) => void;
}) {
  const { t } = useI18n();
  const options: Array<{
    value: ChatFilesDisplayMode;
    label: string;
    Icon: typeof Code2;
  }> = [
    { value: "code", label: t("header.view_code"), Icon: Code2 },
    { value: "edit", label: t("chat.files.editor_edit_mode"), Icon: Pencil },
  ];
  if (previewAvailable) {
    options.push({ value: "preview", label: t("chat.files.editor_preview_mode"), Icon: Eye });
  }

  return (
    <div className="sessio-file-row-toggle" role="group" aria-label={t("chat.files.file_mode")}>
      {options.map(({ value: optionValue, label, Icon }) => (
        <Tooltip key={optionValue} content={label} placement="bottom">
          <button
            type="button"
            aria-label={label}
            aria-pressed={value === optionValue}
            disabled={disabled}
            className="sessio-file-row-toggle-button"
            onClick={() => onChange(optionValue)}
          >
            <Icon aria-hidden="true" className="h-3.5 w-3.5" />
          </button>
        </Tooltip>
      ))}
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
        width: pos.width,
      }}
      className="fixed z-50 overflow-hidden rounded-md border border-ink/[0.10] bg-surface-panel shadow-[0_8px_24px_rgba(0,0,0,0.18)]"
    >
      <ScrollArea
        className="overscroll-contain"
        style={{ maxHeight: Math.min(pos.maxHeight, 360) }}
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
                    "block w-full px-2.5 py-1.5 text-left text-body-sm transition-colors " +
                    (active
                      ? "bg-ink/[0.07] text-ink/90"
                      : "text-ink/72 hover:bg-ink/[0.04] hover:text-ink/90")
                  }
                >
                  <span className="min-w-0 whitespace-pre-wrap break-all font-mono leading-relaxed">
                    {label}
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
