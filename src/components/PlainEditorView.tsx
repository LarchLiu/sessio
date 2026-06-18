import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { Check, Save } from "lucide-react";
import { Compartment, EditorState, Transaction, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { defaultHighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { writeWorkspaceTextFile } from "../api";
import {
  isPlainEditorEditableDocumentPath,
  isPlainEditorMarkdownDocumentPath,
} from "../hooks/plainEditorFileTypes";
import { useI18n } from "../i18n";
import PlainMarkdownPreview from "./PlainMarkdownPreview";
import { useEffectiveThemeType } from "./shikiHighlight";
import Tooltip from "./Tooltip";
import "./plain-editor-theme.css";

export interface PlainEditorViewProps {
  fileKey: string;
  text: string;
  workspacePath: string | null;
  path: string | null;
  mtimeMs: number | null;
  contentVersion: string;
  editingLocked?: boolean;
  editorMode?: PlainEditorMode;
  onSaved: (content: string, mtimeMs: number) => void;
  onPlainEditorLeaveCheckChange?: (handle: (() => Promise<boolean>) | null) => void;
  onEditorModeAvailabilityChange?: (available: boolean) => void;
}

type PlainEditorSaveStatus =
  | "clean"
  | "dirty"
  | "saving"
  | "saved"
  | "readonly"
  | "conflict"
  | "error";

export type PlainEditorMode = "edit" | "preview";

const languageCompartment = new Compartment();
const editableCompartment = new Compartment();

const plainEditorTheme = EditorView.theme({
  "&": {
    height: "100%",
    backgroundColor: "transparent",
    color: "var(--plain-editor-text)",
    fontFamily: "var(--plain-editor-font-family)",
    fontSize: "var(--plain-editor-font-size)",
  },
  ".cm-scroller": {
    overflow: "auto",
    lineHeight: "var(--plain-editor-line-height)",
    fontFamily: "inherit",
    scrollbarWidth: "thin",
    scrollbarColor: "rgb(var(--color-fg) / 0.28) transparent",
  },
  ".cm-scroller::-webkit-scrollbar": {
    width: "8px",
    height: "8px",
  },
  ".cm-scroller::-webkit-scrollbar-track": {
    backgroundColor: "transparent",
  },
  ".cm-scroller::-webkit-scrollbar-thumb": {
    borderRadius: "999px",
    backgroundColor: "rgb(var(--color-fg) / 0.28)",
  },
  ".cm-scroller::-webkit-scrollbar-thumb:hover": {
    backgroundColor: "rgb(var(--color-fg) / 0.42)",
  },
  ".cm-content": {
    boxSizing: "border-box",
    minHeight: "100%",
    width: "min(var(--plain-editor-max-width), 100%)",
    margin: "0 auto",
    padding:
      "var(--plain-editor-padding-y) var(--plain-editor-padding-x) calc(var(--plain-editor-padding-y) + 40px)",
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
    caretColor: "var(--plain-editor-heading)",
  },
  ".cm-line": {
    padding: "0",
  },
  ".cm-gutters": {
    display: "none",
  },
  ".cm-activeLine, .cm-activeLineGutter": {
    backgroundColor: "transparent",
  },
  ".cm-selectionBackground, ::selection": {
    backgroundColor: "var(--plain-editor-selection) !important",
  },
  ".cm-focused": {
    outline: "none",
  },
  ".cm-cursor, .cm-dropCursor": {
    borderLeftColor: "var(--plain-editor-heading)",
  },
}, { dark: false });

function plainLanguageExtension(path: string | null): Extension {
  const lower = (path ?? "").toLowerCase();
  if (
    lower.endsWith(".md") ||
    lower.endsWith(".markdown") ||
    lower.endsWith(".mdx") ||
    lower.endsWith(".mkd") ||
    lower.endsWith(".mdown") ||
    lower.endsWith(".qmd")
  ) {
    return markdown({ base: markdownLanguage });
  }
  return [];
}

function normalizeEditorText(text: string): string {
  return text.replace(/\r\n/g, "\n");
}

function changedOnDiskMessage(detail: string): boolean {
  return detail.toLowerCase().includes("changed on disk");
}

export default function PlainEditorView({
  fileKey,
  text,
  workspacePath,
  path,
  mtimeMs,
  contentVersion,
  editingLocked = false,
  editorMode = "edit",
  onSaved,
  onPlainEditorLeaveCheckChange,
  onEditorModeAvailabilityChange,
}: PlainEditorViewProps) {
  const { t } = useI18n();
  const themeType = useEffectiveThemeType();
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const loadedVersionRef = useRef("");
  const baselineContentRef = useRef(normalizeEditorText(text));
  const expectedMtimeRef = useRef<number | null>(mtimeMs);
  const statusRef = useRef<PlainEditorSaveStatus>("clean");
  const dirtyRef = useRef(false);
  const savingRef = useRef<Promise<boolean> | null>(null);
  const saveNowRef = useRef<() => Promise<boolean>>(async () => true);
  const editableDocument = useMemo(
    () => isPlainEditorEditableDocumentPath(path),
    [path],
  );
  const [dirty, setDirtyState] = useState(false);
  const [status, setStatusState] = useState<PlainEditorSaveStatus>("clean");
  const [messageKey, setMessageKey] = useState<string | null>(null);
  const [messageDetail, setMessageDetail] = useState<string | null>(null);
  const [previewText, setPreviewText] = useState(() => normalizeEditorText(text));

  const effectiveVersion = contentVersion || fileKey;
  const previewableDocument = useMemo(
    () => isPlainEditorMarkdownDocumentPath(path),
    [path],
  );
  const saveable =
    editableDocument &&
    workspacePath !== null &&
    path !== null &&
    mtimeMs !== null &&
    status !== "readonly" &&
    status !== "conflict" &&
    status !== "error";
  const editable = saveable && !editingLocked;

  const setDirty = useCallback((nextDirty: boolean) => {
    dirtyRef.current = nextDirty;
    setDirtyState(nextDirty);
  }, []);

  const setStatus = useCallback(
    (
      nextStatus: PlainEditorSaveStatus,
      nextMessageKey: string | null = null,
      detail: string | null = null,
    ) => {
      statusRef.current = nextStatus;
      setStatusState(nextStatus);
      setMessageKey(nextMessageKey);
      setMessageDetail(detail);
    },
    [],
  );

  const hasPendingChanges = useCallback(
    () => dirtyRef.current || savingRef.current !== null,
    [],
  );

  const currentEditorText = useCallback(() => {
    return normalizeEditorText(viewRef.current?.state.doc.toString() ?? "");
  }, []);

  const replaceDocument = useCallback((nextText: string) => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current === nextText) return;
    const selection = view.state.selection.main;
    const nextLength = nextText.length;
    view.dispatch({
      changes: { from: 0, to: current.length, insert: nextText },
      selection: {
        anchor: Math.min(selection.anchor, nextLength),
        head: Math.min(selection.head, nextLength),
      },
      annotations: Transaction.remote.of(true),
    });
  }, []);

  const loadDocument = useCallback(
    (nextText: string, nextVersion: string, nextMtimeMs: number | null) => {
      const normalized = normalizeEditorText(nextText);
      replaceDocument(normalized);
      baselineContentRef.current = normalized;
      setPreviewText(normalized);
      expectedMtimeRef.current = nextMtimeMs;
      loadedVersionRef.current = nextVersion;
      setDirty(false);
      if (!editableDocument) {
        setStatus("readonly", "chat.files.editor_readonly_document_only");
        return;
      }
      setStatus("clean");
    },
    [editableDocument, replaceDocument, setDirty, setStatus],
  );

  const saveNow = useCallback(async () => {
    if (savingRef.current) return savingRef.current;
    if (!dirtyRef.current) {
      return statusRef.current !== "conflict" && statusRef.current !== "error";
    }
    const serialized = currentEditorText();
    setPreviewText(serialized);
    if (serialized === baselineContentRef.current) {
      setDirty(false);
      setStatus("clean");
      return true;
    }
    if (!editableDocument) {
      setStatus("readonly", "chat.files.editor_readonly_document_only");
      return false;
    }
    if (!workspacePath || !path || expectedMtimeRef.current === null) {
      setStatus("error", "chat.files.editor_save_missing_path");
      return false;
    }

    const expectedMtimeMs = expectedMtimeRef.current;
    const savePromise = writeWorkspaceTextFile(
      workspacePath,
      path,
      serialized,
      expectedMtimeMs,
    )
      .then((result) => {
        expectedMtimeRef.current = result.mtimeMs;
        baselineContentRef.current = serialized;
        onSaved(serialized, result.mtimeMs);
        const latest = normalizeEditorText(viewRef.current?.state.doc.toString() ?? "");
        if (latest !== serialized) {
          setDirty(true);
          setStatus("dirty", "chat.files.editor_unsaved");
          return false;
        }
        setDirty(false);
        setStatus("saved", "chat.files.editor_saved");
        return true;
      })
      .catch((err) => {
        const detail = String(err);
        if (changedOnDiskMessage(detail)) {
          setStatus("conflict", "chat.files.editor_mtime_conflict", detail);
        } else {
          setStatus("error", "chat.files.editor_save_failed", detail);
        }
        return false;
      })
      .finally(() => {
        savingRef.current = null;
      });

    savingRef.current = savePromise;
    setStatus("saving", "chat.files.editor_saving");
    return savePromise;
  }, [
    currentEditorText,
    editableDocument,
    onSaved,
    path,
    setDirty,
    setStatus,
    workspacePath,
  ]);

  const canLeaveDocument = useCallback(async () => {
    if (savingRef.current) return savingRef.current;
    if (!dirtyRef.current) {
      return statusRef.current !== "conflict" && statusRef.current !== "error";
    }
    setStatus("dirty", "chat.files.editor_leave_blocked_unsaved");
    return false;
  }, [setStatus]);

  useEffect(() => {
    saveNowRef.current = saveNow;
  }, [saveNow]);

  useEffect(() => {
    onPlainEditorLeaveCheckChange?.(canLeaveDocument);
    return () => onPlainEditorLeaveCheckChange?.(null);
  }, [canLeaveDocument, onPlainEditorLeaveCheckChange]);

  useEffect(() => {
    if (!hostRef.current) return;
    const state = EditorState.create({
      doc: normalizeEditorText(text),
      extensions: [
        history(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        EditorView.lineWrapping,
        plainEditorTheme,
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        languageCompartment.of(plainLanguageExtension(path)),
        editableCompartment.of([
          EditorState.readOnly.of(!editable),
          EditorView.editable.of(editable),
        ]),
        EditorView.updateListener.of((update) => {
          if (!update.docChanged) return;
          if (update.transactions.some((tr) => tr.annotation(Transaction.remote))) return;
          const current = normalizeEditorText(update.state.doc.toString());
          setPreviewText(current);
          if (current === baselineContentRef.current) {
            setDirty(false);
            if (statusRef.current === "dirty") setStatus("clean");
            return;
          }
          setDirty(true);
          setStatus("dirty", "chat.files.editor_unsaved");
        }),
      ],
    });
    const view = new EditorView({ state, parent: hostRef.current });
    viewRef.current = view;
    loadedVersionRef.current = effectiveVersion;
    baselineContentRef.current = normalizeEditorText(text);
    setPreviewText(normalizeEditorText(text));
    expectedMtimeRef.current = mtimeMs;
    if (!editableDocument) {
      setStatus("readonly", "chat.files.editor_readonly_document_only");
    }
    return () => {
      view.destroy();
      viewRef.current = null;
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps -- CodeMirror instance is persistent; prop changes are applied by effects below.

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: languageCompartment.reconfigure(plainLanguageExtension(path)),
    });
  }, [path]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: editableCompartment.reconfigure([
        EditorState.readOnly.of(!editable),
        EditorView.editable.of(editable),
      ]),
    });
  }, [editable]);

  useEffect(() => {
    onEditorModeAvailabilityChange?.(previewableDocument);
    return () => onEditorModeAvailabilityChange?.(false);
  }, [onEditorModeAvailabilityChange, previewableDocument]);

  useEffect(() => {
    if (editorMode === "preview") {
      setPreviewText(currentEditorText());
      return;
    }
    requestAnimationFrame(() => viewRef.current?.focus());
  }, [currentEditorText, editorMode]);

  useEffect(() => {
    if (!effectiveVersion) return;
    if (loadedVersionRef.current === effectiveVersion) {
      expectedMtimeRef.current = mtimeMs;
      return;
    }
    if (hasPendingChanges()) {
      setStatus("conflict", "chat.files.editor_external_change_pending");
      return;
    }
    loadDocument(text, effectiveVersion, mtimeMs);
  }, [effectiveVersion, hasPendingChanges, loadDocument, mtimeMs, setStatus, text]);

  const handleSave = useCallback(() => {
    void saveNow();
  }, [saveNow]);

  const handleKeyDownCapture = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void saveNowRef.current();
      }
    },
    [],
  );

  const lockMessage = editingLocked && editableDocument && !hasPendingChanges()
    ? "chat.files.editor_locked_agent"
    : null;
  const visibleMessageKey = lockMessage ?? messageKey;
  const statusLabel = visibleMessageKey ? t(visibleMessageKey) : null;
  const canSave = saveable && status !== "saving" && dirty;

  return (
    <div
      className="sessio-plain-editor-view flex h-full min-h-0 min-w-0 flex-col"
      data-theme-type={themeType}
      onKeyDownCapture={handleKeyDownCapture}
    >
      {(visibleMessageKey || dirty) && (
        <div
          className={
            "sessio-plain-editor-status mx-auto mt-4 flex items-center gap-2 rounded-md border px-2 py-1.5 text-body-sm " +
            (status === "error" || status === "conflict"
              ? "border-status-warn/30 bg-status-warn/[0.08] text-status-warn"
              : "border-ink/10 bg-ink/[0.04] text-ink/60")
          }
        >
          <div className="min-w-0 flex-1">
            {statusLabel && <div>{statusLabel}</div>}
            {messageDetail && (
              <div className="mt-1 truncate font-mono text-caption opacity-70">{messageDetail}</div>
            )}
          </div>
          {saveable && dirty && (
            <Tooltip content={t("chat.files.editor_save")} placement="bottom">
              <button
                type="button"
                aria-label={t("chat.files.editor_save")}
                className="sessio-plain-editor-save-button"
                disabled={!canSave}
                onClick={handleSave}
              >
                {status === "saved" ? (
                  <Check aria-hidden="true" className="h-3.5 w-3.5" />
                ) : (
                  <Save aria-hidden="true" className="h-3.5 w-3.5" />
                )}
              </button>
            </Tooltip>
          )}
        </div>
      )}
      <div
        ref={hostRef}
        className={
          "sessio-plain-editor-host min-h-0 flex-1 " +
          (editorMode === "preview" ? "hidden" : "")
        }
      />
      {editorMode === "preview" && <PlainMarkdownPreview text={previewText} />}
    </div>
  );
}
