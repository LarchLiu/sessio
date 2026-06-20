import "tldraw/tldraw.css";

import {
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  AssetRecordType,
  Tldraw,
  createShapeId,
  createShapesForAssets,
  getMediaAssetInfoPartial,
  parseTldrawJsonFile,
  serializeTldrawJson,
  toRichText,
  type Editor,
} from "tldraw";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { AlertCircle, FileImage, FilePlus2, FolderOpen, Layers3, Save, StickyNote, Workflow } from "lucide-react";
import type { CanvasDocumentState, CanvasNodeKind, CanvasSourceType } from "../canvasTypes";
import {
  type Agent,
  getThreadWorkSnapshot,
  listProjectFiles,
  saveCanvasDraft,
  saveCanvasRevision,
  updateCanvasShapeRefs,
} from "../api";
import type { ChatComposerController } from "../hooks/useChatComposer";
import PopupMenu, { type PopupMenuOption } from "./PopupMenu";

export interface TldrawCanvasHostProps {
  sessionId: string;
  sessionAgent: Agent;
  workspacePath: string | null;
  editedFiles?: string[];
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
  sessionAgent,
  workspacePath,
  editedFiles = [],
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
  const addMenuButtonRef = useRef<HTMLButtonElement>(null);
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved" | "failed">("idle");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [selectionCount, setSelectionCount] = useState(0);
  const [projectFiles, setProjectFiles] = useState<string[]>([]);
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  const [selectedShapeId, setSelectedShapeId] = useState<string | null>(null);
  const [selectedShapeMeta, setSelectedShapeMeta] = useState<Record<string, unknown> | null>(null);
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

  const suggestionFiles = useMemo(() => {
    const merged = [...editedFiles, ...projectFiles].filter(Boolean);
    return Array.from(new Set(merged)).slice(0, 4);
  }, [editedFiles, projectFiles]);
  const addMenuOptions = useMemo<PopupMenuOption<string>[]>(() => [
    { key: "file", label: "Add file", icon: <FolderOpen className="h-4 w-4" /> },
    { key: "image", label: "Add image", icon: <FileImage className="h-4 w-4" /> },
    { key: "workflow", label: "Add workflow", icon: <Workflow className="h-4 w-4" /> },
    { key: "note", label: "Add note", icon: <StickyNote className="h-4 w-4" /> },
  ], []);

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
        const sourceKey =
          typeof meta.sourceKey === "string" && meta.sourceKey.trim()
            ? meta.sourceKey
            : null;
        const metaKind = typeof meta.kind === "string" ? meta.kind : null;
        const metaSourceType = typeof meta.sourceType === "string" ? meta.sourceType : null;
        const shapeType: CanvasNodeKind =
          metaKind === "file" ||
          metaKind === "image" ||
          metaKind === "workflow" ||
          metaKind === "note" ||
          metaKind === "group" ||
          metaKind === "video"
            ? metaKind
            : shape.type === "image"
              ? "image"
              : shape.type === "group"
                ? "group"
                : "note";
        const sourceType: CanvasSourceType =
          metaSourceType === "workspace_file" ||
          metaSourceType === "edited_file" ||
          metaSourceType === "attachment_file" ||
          metaSourceType === "attachment_image" ||
          metaSourceType === "video_file" ||
          metaSourceType === "workflow_definition" ||
          metaSourceType === "note" ||
          metaSourceType === "group"
            ? metaSourceType
            : shape.type === "image"
              ? "attachment_image"
              : shape.type === "group"
                ? "group"
                : "note";
        return {
          shapeId: shape.id,
          kind: shapeType,
          sourceType,
          sourceKey,
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

  const placePoint = (editor: Editor) => {
    const viewport = editor.getViewportPageBounds();
    const center = viewport.center;
    return {
      x: Math.round(center.x - 120 + Math.random() * 24),
      y: Math.round(center.y - 60 + Math.random() * 24),
    };
  };

  const addNoteNode = () => {
    const editor = editorRef.current;
    if (!editor) return;
    const point = placePoint(editor);
    const id = createShapeId("note");
    editor.createShapes([
      {
        id,
        type: "note",
        x: point.x,
        y: point.y,
        props: {
          richText: toRichText("New note"),
        },
        meta: {
          kind: "note",
          sourceType: "note",
          title: "New note",
        },
      },
    ]);
    editor.select(id);
  };

  const addFileNode = (path: string) => {
    const editor = editorRef.current;
    if (!editor) return;
    const point = placePoint(editor);
    const id = createShapeId("file");
    const fileName = path.split(/[/\\]/).pop() ?? path;
    const sourceType = resolveFileSourceType(path, workspacePath);
    editor.createShapes([
      {
        id,
        type: "geo",
        x: point.x,
        y: point.y,
        props: {
          geo: "rectangle",
          w: 320,
          h: 110,
          richText: toRichText(`${fileName}\n${path}`),
        },
        meta: {
          kind: "file",
          sourceType,
          sourcePath: path,
          sourceKey: fileName,
          title: fileName,
        },
      },
    ]);
    editor.select(id);
  };

  const addWorkflowNode = async () => {
    const editor = editorRef.current;
    if (!editor) return;
    const point = placePoint(editor);
    const snapshot = await getThreadWorkSnapshot(sessionAgent, sessionId).catch(() => null);
    const title = snapshot?.snapshot.goal || "Workflow";
    const body = snapshot?.snapshot.stages?.slice(0, 5).map((stage) => `- ${stage.name}: ${stage.status}`).join("\n")
      ?? "- Define the next step\n- Run the first slice\n- Review the output";
    const workflowMeta = snapshot?.snapshot ? serializeJsonValue(snapshot.snapshot) : null;
    const id = createShapeId("workflow");
    editor.createShapes([
      {
        id,
        type: "geo",
        x: point.x,
        y: point.y,
        props: {
          geo: "diamond",
          w: 300,
          h: 180,
          richText: toRichText(`${title}\n\n${body}`),
        },
        meta: {
          kind: "workflow",
          sourceType: "workflow_definition",
          title,
          sourceKey: snapshot?.threadId ?? title,
          threadId: snapshot?.threadId ?? null,
          threadStageId: snapshot?.stageId ?? null,
          workflowSnapshotJson: workflowMeta,
        },
      },
    ]);
    editor.select(id);
  };

  const addImageNode = async () => {
    const editor = editorRef.current;
    if (!editor) return;
    try {
      const selection = await open({
        multiple: false,
        directory: false,
        filters: [
          {
            name: "Images",
            extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "heif"],
          },
        ],
      });
      if (!selection || Array.isArray(selection)) return;
      const path = selection;
      const point = placePoint(editor);
      const response = await fetch(convertFileSrc(path));
      const blob = await response.blob();
      const file = new File([blob], path.split(/[/\\]/).pop() ?? "image", {
        type: blob.type || "image/png",
      });
      const assetId = AssetRecordType.createId(path);
      const asset = await getMediaAssetInfoPartial(file, assetId, true, false);
      asset.props.src = convertFileSrc(path);
      asset.meta = {
        sourcePath: path,
      };
      await createShapesForAssets(editor, [asset], point);
      const createdIds = editor.getSelectedShapeIds();
      if (createdIds[0]) {
        const shape = editor.getShape(createdIds[0]);
        if (shape) {
          editor.updateShapes([
            {
              id: shape.id,
              type: shape.type,
              meta: {
                ...(shape.meta ?? {}),
                kind: "image",
                sourceType: "attachment_image",
                sourcePath: path,
                title: file.name,
              },
            },
          ]);
        }
      }
    } catch (error) {
      onError(`Failed to add image node: ${String(error)}`);
    }
  };

  const saveRevision = async () => {
    const editor = editorRef.current;
    if (!editor) return;
    try {
      const snapshotJson = await serializeTldrawJson(editor);
      setSaveState("saving");
      setSaveError(null);
      const saved = await saveCanvasRevision({
        sessionId,
        title: initialState.document.title,
        snapshotJson,
        source: "manual",
      });
      currentSnapshotRef.current = snapshotJson;
      setSaveState("saved");
      onStateLoaded({
        ...initialState,
        document: saved.document,
        draftSnapshot: snapshotJson,
        savedRevision: saved.revision,
        savedSnapshot: snapshotJson,
      });
      await persistShapeRefs(editor);
    } catch (error) {
      const message = `Canvas save failed: ${String(error)}`;
      setSaveState("failed");
      setSaveError(message);
      onError(message);
    }
  };

  const groupSelection = () => {
    const editor = editorRef.current;
    if (!editor) return;
    const ids = editor.getSelectedShapeIds();
    if (ids.length < 2) return;
    editor.groupShapes(ids);
  };

  const ungroupSelection = () => {
    const editor = editorRef.current;
    if (!editor) return;
    const ids = editor.getSelectedShapeIds();
    if (ids.length === 0) return;
    editor.ungroupShapes(ids);
  };

  const handleAddMenuSelect = async (key: string) => {
    setAddMenuOpen(false);
    if (key === "note") {
      addNoteNode();
      return;
    }
    if (key === "workflow") {
      await addWorkflowNode();
      return;
    }
    if (key === "image") {
      await addImageNode();
      return;
    }
    if (key === "file") {
      const next = suggestionFiles[0];
      if (next) {
        addFileNode(next);
        return;
      }
      try {
        const selection = await open({
          multiple: false,
          directory: false,
        });
        if (!selection || Array.isArray(selection)) return;
        addFileNode(selection);
      } catch (error) {
        onError(`Failed to add file node: ${String(error)}`);
      }
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
          <button
            ref={addMenuButtonRef}
            type="button"
            onClick={() => setAddMenuOpen((value) => !value)}
            className="inline-flex items-center gap-1.5 rounded-full border border-ink/12 px-3 py-1.5 text-ink/72 transition hover:bg-ink/5"
          >
            <Layers3 className="h-3.5 w-3.5" />
            Add to canvas
          </button>
          {suggestionFiles.map((file) => (
            <button
              key={file}
              type="button"
              onClick={() => addFileNode(file)}
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
          <button
            type="button"
            onClick={() => groupSelection()}
            disabled={selectionCount < 2}
            className="rounded-full border border-ink/10 px-2.5 py-1 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Group
          </button>
          <button
            type="button"
            onClick={() => ungroupSelection()}
            disabled={selectionCount === 0}
            className="rounded-full border border-ink/10 px-2.5 py-1 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Ungroup
          </button>
          <button
            type="button"
            onClick={() => void saveRevision()}
            className="rounded-full border border-ink/10 px-2.5 py-1 text-ink/70 transition hover:bg-ink/5"
          >
            Save canvas
          </button>
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
              const selectedIds = editor.getSelectedShapeIds();
              setSelectionCount(selectedIds.length);
              const currentSelected = selectedIds[0] ? editor.getShape(selectedIds[0]) : null;
              setSelectedShapeId(currentSelected?.id ?? null);
              setSelectedShapeMeta((currentSelected?.meta ?? null) as Record<string, unknown> | null);
              void scheduleSave(editor);
            });
            const selectedIds = editor.getSelectedShapeIds();
            setSelectionCount(selectedIds.length);
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
          {selectedShapeId && selectedShapeMeta && typeof selectedShapeMeta.sourcePath === "string" ? (
            <button
              type="button"
              onClick={() => onOpenProjectFile(selectedShapeMeta.sourcePath as string)}
              className="inline-flex items-center gap-1.5 rounded-full border border-ink/10 px-3 py-1.5 text-ink/70 transition hover:bg-ink/5"
            >
              <FolderOpen className="h-3.5 w-3.5" />
              Open source file
            </button>
          ) : (
            "Open a suggested file to inspect it in the file view, then switch back to keep sketching on the canvas."
          )}
        </div>
      )}
      {addMenuOpen && addMenuButtonRef.current && (
        <PopupMenu
          anchor={addMenuButtonRef.current}
          options={addMenuOptions}
          placement="bottom-start"
          onSelect={(key) => void handleAddMenuSelect(key)}
          onClose={() => setAddMenuOpen(false)}
        />
      )}
      <div className="sr-only" aria-hidden>
        {composer.selectedAgent}
      </div>
    </div>
  );
}

function resolveFileSourceType(path: string, workspacePath: string | null): CanvasSourceType {
  if (!workspacePath) return "attachment_file";
  const normalizedWorkspace = normalizePathSegment(workspacePath);
  const normalizedPath = normalizePathSegment(path);
  if (
    normalizedPath === normalizedWorkspace ||
    normalizedPath.startsWith(`${normalizedWorkspace}/`)
  ) {
    return "workspace_file";
  }
  return "attachment_file";
}

function normalizePathSegment(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+$/, "");
}

function serializeJsonValue(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return "null";
  }
}
