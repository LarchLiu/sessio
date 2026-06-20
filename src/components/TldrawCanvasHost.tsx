import "tldraw/tldraw.css";

import {
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Tldraw,
  parseTldrawJsonFile,
  serializeTldrawJson,
  type Editor,
} from "tldraw";
import { AlertCircle, FilePlus2, Save } from "lucide-react";
import type { CanvasDocumentState, CanvasNodeKind, CanvasSourceType } from "../canvasTypes";
import {
  listProjectFiles,
  saveCanvasDraft,
  updateCanvasShapeRefs,
} from "../api";
import type { ChatComposerController } from "../hooks/useChatComposer";

export interface TldrawCanvasHostProps {
  sessionId: string;
  workspacePath: string | null;
  initialState: CanvasDocumentState;
  initialSnapshot: string | null;
  composer: ChatComposerController;
  onStateLoaded: (state: CanvasDocumentState) => void;
  onError: (message: string) => void;
  onOpenProjectFile?: (path: string) => void;
}

const AUTOSAVE_DEBOUNCE_MS = 900;

export default function TldrawCanvasHost({
  sessionId,
  workspacePath,
  initialState,
  initialSnapshot,
  composer,
  onStateLoaded,
  onError,
  onOpenProjectFile,
}: TldrawCanvasHostProps) {
  const editorRef = useRef<Editor | null>(null);
  const autosaveTimerRef = useRef<number | null>(null);
  const inflightSaveRef = useRef(false);
  const queuedSaveRef = useRef<string | null>(null);
  const currentSnapshotRef = useRef(initialSnapshot);
  const hydratedRef = useRef(false);
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved" | "failed">("idle");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [selectionCount, setSelectionCount] = useState(0);
  const [projectFiles, setProjectFiles] = useState<string[]>([]);
  const lastSavedRevision = initialState.savedRevision?.revision ?? null;

  useEffect(() => {
    currentSnapshotRef.current = initialSnapshot;
    hydratedRef.current = false;
  }, [initialSnapshot, sessionId]);

  useEffect(() => {
    if (!workspacePath) {
      setProjectFiles([]);
      return;
    }
    let cancelled = false;
    listProjectFiles(workspacePath)
      .then((files) => {
        if (cancelled) return;
        setProjectFiles(files.slice(0, 8));
      })
      .catch((error) => {
        if (!cancelled) onError(String(error));
      });
    return () => {
      cancelled = true;
    };
  }, [onError, workspacePath]);

  useEffect(() => {
    return () => {
      if (autosaveTimerRef.current !== null) {
        window.clearTimeout(autosaveTimerRef.current);
      }
    };
  }, []);

  const suggestionFiles = useMemo(
    () => projectFiles.filter(Boolean).slice(0, 3),
    [projectFiles],
  );

  const hydrateSnapshot = async (editor: Editor, snapshotText: string | null) => {
    if (!snapshotText || hydratedRef.current) return;
    try {
      const result = parseTldrawJsonFile({
        json: snapshotText,
        schema: editor.store.schema,
      });
      if (!result.ok) {
        throw new Error(result.error.type);
      }
      editor.loadSnapshot(result.value.getStoreSnapshot());
      hydratedRef.current = true;
    } catch (error) {
      const message = `Failed to load saved canvas state: ${String(error)}`;
      setSaveState("failed");
      setSaveError(message);
      onError(message);
    }
  };

  const persistShapeRefs = async (editor: Editor) => {
    const shapes = editor
      .getCurrentPageShapes()
      .map((shape) => {
        const meta = (shape.meta ?? {}) as Record<string, unknown>;
        const sourcePath =
          typeof meta.sourcePath === "string" && meta.sourcePath.trim()
            ? meta.sourcePath
            : null;
        const shapeType: CanvasNodeKind =
          shape.type === "image" ? "image" : shape.type === "group" ? "group" : "note";
        const sourceType: CanvasSourceType =
          shape.type === "image"
            ? "attachment_image"
            : shape.type === "group"
              ? "group"
              : "note";
        return {
          shapeId: shape.id,
          kind: shapeType,
          sourceType,
          sourcePath,
          metadataJson: JSON.stringify(meta),
        };
      });
    try {
      await updateCanvasShapeRefs({
        sessionId,
        refs: shapes,
      });
    } catch (error) {
      const message = `Failed to sync canvas refs: ${String(error)}`;
      setSaveState("failed");
      setSaveError(message);
      onError(message);
    }
  };

  const flushSave = async (snapshotJson: string) => {
    inflightSaveRef.current = true;
    setSaveState("saving");
    setSaveError(null);
    try {
      const saved = await saveCanvasDraft({
        sessionId,
        title: initialState.document.title,
        snapshotJson,
      });
      currentSnapshotRef.current = snapshotJson;
      setSaveState("saved");
      onStateLoaded({
        ...initialState,
        document: saved.document,
        draftSnapshot: snapshotJson,
      });
      if (editorRef.current) {
        void persistShapeRefs(editorRef.current);
      }
    } catch (error) {
      const message = `Canvas draft save failed: ${String(error)}`;
      setSaveState("failed");
      setSaveError(message);
      onError(message);
    } finally {
      inflightSaveRef.current = false;
      const queued = queuedSaveRef.current;
      queuedSaveRef.current = null;
      if (queued && queued !== currentSnapshotRef.current) {
        void flushSave(queued);
      }
    }
  };

  const scheduleSave = async (editor: Editor) => {
    const snapshotJson = await serializeTldrawJson(editor);
    if (snapshotJson === currentSnapshotRef.current) return;
    if (autosaveTimerRef.current !== null) {
      window.clearTimeout(autosaveTimerRef.current);
    }
    autosaveTimerRef.current = window.setTimeout(() => {
      if (inflightSaveRef.current) {
        queuedSaveRef.current = snapshotJson;
        return;
      }
      void flushSave(snapshotJson);
    }, AUTOSAVE_DEBOUNCE_MS);
  };

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col">
      <div className="flex items-center justify-between gap-3 border-b border-ink/8 bg-surface-panel/90 px-4 py-2">
        <div className="flex items-center gap-3 text-caption text-ink/50">
          {suggestionFiles.map((file) => (
            <button
              key={file}
              type="button"
              onClick={() => onOpenProjectFile?.(file)}
              className="inline-flex items-center gap-1.5 rounded-full border border-ink/10 px-3 py-1.5 transition hover:bg-ink/5"
            >
              <FilePlus2 className="h-3.5 w-3.5" />
              <span className="max-w-[180px] truncate">{file.split(/[/\\]/).pop() ?? file}</span>
            </button>
          ))}
        </div>
        <div className="flex items-center gap-2 text-caption">
          <span className="rounded-full border border-ink/10 px-2.5 py-1 text-ink/55">
            {selectionCount > 0 ? `${selectionCount} selected` : "Canvas"}
          </span>
          <span
            className={
              "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 " +
              (saveState === "failed"
                ? "bg-status-error/10 text-status-error"
                : saveState === "saving"
                  ? "bg-ink/8 text-ink/65"
                  : "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300")
            }
          >
            {saveState === "failed" ? (
              <AlertCircle className="h-3.5 w-3.5" />
            ) : (
              <Save className="h-3.5 w-3.5" />
            )}
            <span>
              {saveState === "failed"
                ? "Save failed"
                : saveState === "saving"
                  ? "Saving…"
                  : saveState === "saved"
                    ? "Saved"
                    : "Draft autosave"}
            </span>
          </span>
        </div>
      </div>
      {saveError && (
        <div className="mx-4 mt-3 rounded-xl border border-status-error/20 bg-status-error/8 px-3 py-2 text-caption text-status-error">
          {saveError}
          {lastSavedRevision !== null ? ` Last saved revision: ${lastSavedRevision}.` : ""}
        </div>
      )}
      <div className="flex-1 min-h-0">
        <Tldraw
          persistenceKey={undefined}
          hideUi={false}
          onMount={(editor) => {
            editorRef.current = editor;
            void hydrateSnapshot(editor, currentSnapshotRef.current);
            const dispose = editor.store.listen(() => {
              setSelectionCount(editor.getSelectedShapeIds().length);
              void scheduleSave(editor);
            });
            setSelectionCount(editor.getSelectedShapeIds().length);
            return () => {
              dispose();
              editorRef.current = null;
            };
          }}
        />
      </div>
      {!workspacePath && (
        <div className="border-t border-ink/8 px-4 py-2 text-caption text-status-warn">
          This session has no workspace path, so file suggestions and source-opening are limited.
        </div>
      )}
      {workspacePath && onOpenProjectFile && (
        <div className="border-t border-ink/8 px-4 py-2 text-caption text-ink/50">
          Open a suggested file to inspect it in the file view, then switch back to keep sketching on the canvas.
        </div>
      )}
      <div className="sr-only" aria-hidden>
        {composer.selectedAgent}
      </div>
    </div>
  );
}
