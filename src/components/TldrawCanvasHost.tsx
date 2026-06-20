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
import { AlertCircle, ArrowUpRight, Camera, FileImage, FilePlus2, FolderOpen, Layers3, MessageCircleQuestionMark, MessagesSquare, Save, StickyNote, Workflow } from "lucide-react";
import type {
  CanvasContextOption,
  CanvasContextRef,
  CanvasDocumentState,
  CanvasNodeKind,
  CanvasSourceType,
} from "../canvasTypes";
import {
  type Agent,
  createAstraRun,
  createCanvasAnchor,
  createCanvasContextFile,
  getThreadWorkSnapshot,
  listProjectFiles,
  savePastedAttachment,
  saveCanvasDraft,
  saveCanvasRevision,
  updateCanvasShapeRefs,
} from "../api";
import type { ComposerAttachment } from "./ComposerAttachments";
import type { ChatComposerController } from "../hooks/useChatComposer";
import PopupMenu, { type PopupMenuOption } from "./PopupMenu";

export interface TldrawCanvasHostProps {
  sessionId: string;
  sessionAgent: Agent;
  workspacePath: string | null;
  sessionThreadId?: string | null;
  editedFiles?: string[];
  initialState: CanvasDocumentState;
  initialSnapshot: string | null;
  composer: ChatComposerController;
  onStateLoaded: (state: CanvasDocumentState) => void;
  onError: (message: string) => void;
  onOpenProjectFile?: (path: string) => void;
  onOpenThreadMultiSessionChat?: (threadId: string) => void;
}

const AUTOSAVE_DEBOUNCE_MS = 900;

export default function TldrawCanvasHost({
  sessionId,
  sessionAgent,
  workspacePath,
  sessionThreadId = null,
  editedFiles = [],
  initialState,
  initialSnapshot,
  composer,
  onStateLoaded,
  onError,
  onOpenProjectFile,
  onOpenThreadMultiSessionChat,
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
  const [bridgeBusy, setBridgeBusy] = useState<null | "ask" | "snapshot" | "workflow">(null);
  const [workflowRunState, setWorkflowRunState] = useState<null | { status: string; runId: string; threadId: string | null }>(null);
  const [anchors, setAnchors] = useState(initialState.anchors);
  const lastSavedRevision = initialState.savedRevision?.revision ?? null;

  useEffect(() => {
    currentSnapshotRef.current = initialSnapshot;
    hydratedRef.current = false;
  }, [initialSnapshot, sessionId]);

  useEffect(() => {
    setAnchors(initialState.anchors);
  }, [initialState.anchors]);

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
    { key: "file", label: "Choose files", icon: <FolderOpen className="h-4 w-4" /> },
    { key: "image", label: "Add image", icon: <FileImage className="h-4 w-4" /> },
    { key: "workflow", label: "Add workflow", icon: <Workflow className="h-4 w-4" /> },
    { key: "note", label: "Add note", icon: <StickyNote className="h-4 w-4" /> },
  ], []);

  const patchShapeMeta = (shapeId: string, patch: Record<string, unknown>) => {
    const editor = editorRef.current;
    if (!editor) return;
    const shape = editor.getShape(shapeId as never);
    if (!shape) return;
    const nextMeta = {
      ...(shape.meta ?? {}),
      ...patch,
    };
    editor.updateShapes([
        {
          id: shape.id,
          type: shape.type,
          meta: nextMeta as never,
        },
      ]);
    if (selectedShapeId === shapeId) {
      setSelectedShapeMeta(nextMeta);
    }
  };

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

  const restoreSavedRevision = async () => {
    const editor = editorRef.current;
    const snapshotText = initialState.savedSnapshot;
    if (!editor || !snapshotText) return;
    try {
      const result = parseTldrawJsonFile({
        json: snapshotText,
        schema: editor.store.schema,
      });
      if (!result.ok) {
        throw new Error(result.error.type);
      }
      editor.loadSnapshot(result.value.getStoreSnapshot());
      currentSnapshotRef.current = snapshotText;
      setSaveState("saved");
      setSaveError(null);
      onStateLoaded({
        ...initialState,
        draftSnapshot: snapshotText,
        savedSnapshot: snapshotText,
      });
      await persistShapeRefs(editor);
    } catch (error) {
      const message = `Failed to restore the last saved revision: ${String(error)}`;
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

  const addFileNodes = (paths: string[]) => {
    const editor = editorRef.current;
    const nextPaths = paths.filter((path) => path.trim());
    if (!editor || nextPaths.length === 0) return;
    const point = placePoint(editor);
    const shapes = nextPaths.map((path, index) => {
      const id = createShapeId("file");
      const fileName = path.split(/[/\\]/).pop() ?? path;
      const sourceType = resolveFileSourceType(path, workspacePath);
      const column = index % 3;
      const row = Math.floor(index / 3);
      return {
        id,
        type: "geo" as const,
        x: point.x + column * 348,
        y: point.y + row * 138,
        props: {
          geo: "rectangle" as const,
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
      };
    });
    editor.createShapes(shapes);
    editor.select(...shapes.map((shape) => shape.id));
  };

  const addFileNode = (path: string) => {
    addFileNodes([path]);
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
    const mirrorLabel = snapshot?.snapshot.goal?.trim() || null;
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
          mirrorSource: snapshot?.threadId
            ? {
                kind: "thread_workflow",
                label: mirrorLabel,
              }
            : null,
          execution: {
            enabled: Boolean(snapshot?.threadId),
            driver: snapshot?.threadId ? "astra" : null,
            lastRunId: null,
            lastStatus: "idle",
          },
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
      try {
        const selection = await open({
          multiple: true,
          directory: false,
        });
        if (!selection) return;
        addFileNodes(Array.isArray(selection) ? selection : [selection]);
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

  const getSelectedShapeRefs = () => {
    const editor = editorRef.current;
    if (!editor) return [];
    const selectedIds = editor.getSelectedShapeIds();
    const selectedSet = new Set(selectedIds);
    const refsById = new Map(initialState.shapeRefs.map((ref) => [ref.shapeId, ref]));
    return editor
      .getCurrentPageShapes()
      .filter((shape) => selectedSet.has(shape.id))
      .map((shape) => {
        const ref = refsById.get(shape.id);
        const meta = (shape.meta ?? {}) as Record<string, unknown>;
        const title =
          typeof meta.title === "string" && meta.title.trim()
            ? meta.title
            : ref?.sourceKey
              ? ref.sourceKey
              : shape.type;
        const sourcePath =
          typeof meta.sourcePath === "string" && meta.sourcePath.trim()
            ? meta.sourcePath
            : ref?.sourcePath ?? null;
        const sourceKey =
          typeof meta.sourceKey === "string" && meta.sourceKey.trim()
            ? meta.sourceKey
            : ref?.sourceKey ?? null;
        const sourceType =
          typeof meta.sourceType === "string" && meta.sourceType.trim()
            ? meta.sourceType
            : ref?.sourceType ?? shape.type;
        const kind =
          typeof meta.kind === "string" && meta.kind.trim()
            ? meta.kind
            : ref?.kind ?? shape.type;
        const summary = buildShapeSummary(kind, title, sourcePath, meta);
        const contextRef: CanvasContextRef = {
          shapeId: shape.id,
          kind: normalizeNodeKind(kind),
          sourceType,
          sourcePath,
          sourceKey,
          summary,
        };
        return {
          shape,
          meta,
          title,
          sourcePath,
          kind: normalizeNodeKind(kind),
          contextRef,
        };
      });
  };

  const exportSelectionSnapshot = async () => {
    const editor = editorRef.current;
    if (!editor) return null;
    const ids = editor.getSelectedShapeIds();
    if (ids.length === 0) return null;
    const result = await editor.toImage(ids, {
      format: "png",
      background: true,
      scale: 1,
      padding: 24,
    });
    const blob = result?.blob ?? null;
    if (!blob) return null;
    const path = await saveBlobAsAttachment(blob, `canvas-selection-${Date.now()}.png`);
    return {
      path,
      attachment: {
        path,
        kind: "image",
        mimeType: "image/png",
        previewDataUrl: null,
        displayName: "Canvas selection",
        name: "Canvas selection",
      } satisfies ComposerAttachment,
    };
  };

  const buildSelectionContext = async () => {
    const refs = getSelectedShapeRefs();
    if (refs.length === 0) return null;
    const selectionSummary = renderSelectionSummaryMarkdown(refs);
    const summaryPath = await createCanvasContextFile({
      sessionId,
      kind: "selection",
      fileNamePrefix: "canvas-selection",
      content: selectionSummary,
    });
    const attachments: ComposerAttachment[] = [
      {
        path: summaryPath,
        kind: "file",
        mimeType: "text/markdown",
        previewDataUrl: null,
        displayName: "Canvas selection summary",
        name: "Canvas selection summary",
      },
    ];
    const workflowRefs = refs.filter((ref) => ref.kind === "workflow");
    for (const workflow of workflowRefs) {
      const workflowMarkdown = renderWorkflowSummaryMarkdown(workflow.meta, workflow.title);
      const workflowPath = await createCanvasContextFile({
        sessionId,
        kind: "workflow",
        fileNamePrefix: safeFilePrefix(workflow.title),
        content: workflowMarkdown,
      });
      attachments.push({
        path: workflowPath,
        kind: "file",
        mimeType: "text/markdown",
        previewDataUrl: null,
        displayName: `${workflow.title} workflow summary`,
        name: `${workflow.title} workflow summary`,
      });
    }
    const snapshot = await exportSelectionSnapshot();
    if (snapshot) attachments.push(snapshot.attachment);
    const canvasContext: CanvasContextOption = {
      canvasId: initialState.document.id,
      scope: "selection",
      shapeIds: refs.map((ref) => ref.shape.id),
      snapshotAttachmentPath: snapshot?.path ?? null,
      refs: refs.map((ref) => ref.contextRef),
    };
    return {
      refs,
      attachments,
      canvasContext,
      summaryText: selectionSummary,
    };
  };

  const attachSelectionSnapshot = async () => {
    if (!composer.supportsImageAttachments) {
      onError("The selected agent does not support image attachments.");
      return;
    }
    setBridgeBusy("snapshot");
    try {
      const snapshot = await exportSelectionSnapshot();
      if (!snapshot) {
        onError("Select one or more shapes before attaching a snapshot.");
        return;
      }
      await composer.appendAttachments([
        {
          path: snapshot.path,
          kind: "image",
          mimeType: "image/png",
          displayName: snapshot.attachment.displayName,
          name: snapshot.attachment.name,
        },
      ]);
    } catch (error) {
      onError(`Failed to attach selection snapshot: ${String(error)}`);
    } finally {
      setBridgeBusy(null);
    }
  };

  const askSelection = async () => {
    setBridgeBusy("ask");
    try {
      const payload = await buildSelectionContext();
      if (!payload) {
        onError("Select one or more shapes before asking about the canvas.");
        return;
      }
      const prompt =
        composer.text.trim() ||
        `Help me reason about these ${payload.refs.length} selected canvas item${payload.refs.length === 1 ? "" : "s"}.`;
      const sent = await composer.sendWithContext(prompt, {
        clearComposer: true,
        attachments: [...composer.attachments, ...payload.attachments],
        runtimeOptions: {
          canvasContext: payload.canvasContext,
        },
      });
      if (!sent.ok) {
        onError("Failed to send the canvas selection to the agent.");
        return;
      }
      const turnId = sent.turnId ?? `canvas-selection:${Date.now()}`;
      const anchor = await createCanvasAnchor({
        sessionId,
        anchorShapeId: payload.refs[0]?.shape.id ?? null,
        selectionShapeIdsJson: JSON.stringify(payload.refs.map((ref) => ref.shape.id)),
        turnId,
        summary: payload.refs.map((ref) => ref.title).join(", ").slice(0, 180),
      });
      setAnchors((current) => [anchor, ...current]);
    } catch (error) {
      onError(`Failed to ask about the current selection: ${String(error)}`);
    } finally {
      setBridgeBusy(null);
    }
  };

  const runWorkflowSelection = async () => {
    if (!selectedShapeId || !selectedShapeMeta) return;
    const threadId =
      typeof selectedShapeMeta.threadId === "string" && selectedShapeMeta.threadId.trim()
        ? selectedShapeMeta.threadId
        : null;
    if (!threadId) {
      onError("This workflow node is not linked to a thread workflow.");
      return;
    }
    setBridgeBusy("workflow");
    patchShapeMeta(selectedShapeId, {
      execution: {
        ...parseWorkflowExecution(selectedShapeMeta),
        enabled: true,
        driver: "astra",
        lastStatus: "running",
      },
    });
    try {
      const run = await createAstraRun(threadId, composer.text.trim() || null);
      const nextState = {
        status: run.status,
        runId: run.runId,
        threadId,
      };
      setWorkflowRunState(nextState);
      patchShapeMeta(selectedShapeId, {
        execution: {
          ...parseWorkflowExecution(selectedShapeMeta),
          enabled: true,
          driver: "astra",
          lastRunId: run.runId,
          lastStatus: run.status,
        },
      });
    } catch (error) {
      patchShapeMeta(selectedShapeId, {
        execution: {
          ...parseWorkflowExecution(selectedShapeMeta),
          enabled: true,
          driver: "astra",
          lastStatus: "failed",
        },
      });
      onError(`Failed to start workflow run: ${String(error)}`);
    } finally {
      setBridgeBusy(null);
    }
  };

  const openWorkflowThread = () => {
    const selectedThreadId =
      selectedShapeMeta && typeof selectedShapeMeta.threadId === "string" && selectedShapeMeta.threadId.trim()
        ? selectedShapeMeta.threadId
        : sessionThreadId;
    if (!selectedThreadId) {
      onError("This canvas item is not linked to a thread workflow yet.");
      return;
    }
    if (!onOpenThreadMultiSessionChat) {
      onError("Thread multi-session chat is not available from this view.");
      return;
    }
    onOpenThreadMultiSessionChat(selectedThreadId);
  };

  const canOpenWorkflowThread = Boolean(
    onOpenThreadMultiSessionChat && (
      (selectedShapeMeta && typeof selectedShapeMeta.threadId === "string" && selectedShapeMeta.threadId.trim()) ||
      sessionThreadId
    ),
  );

  return (
    <div className="canvas-tldraw-host absolute inset-0 flex min-h-0 flex-col">
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
          {suggestionFiles.length === 0 && (
            <div className="rounded-full border border-dashed border-ink/10 px-3 py-1.5 text-ink/40">
              Add files, notes, workflow mirrors, or screenshots to seed the canvas.
            </div>
          )}
        </div>
        <div className="flex items-center gap-2 text-caption">
          <span className="rounded-full border border-ink/10 px-2.5 py-1 text-ink/55">
            {selectionCount > 0 ? `${selectionCount} selected` : "Canvas"}
          </span>
          <button
            type="button"
            onClick={() => void askSelection()}
            disabled={selectionCount === 0 || bridgeBusy !== null}
            className="inline-flex items-center gap-1.5 rounded-full border border-ink/10 px-2.5 py-1 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <MessageCircleQuestionMark className="h-3.5 w-3.5" />
            Ask selection
          </button>
          <button
            type="button"
            onClick={() => void attachSelectionSnapshot()}
            disabled={selectionCount === 0 || bridgeBusy !== null || !composer.supportsImageAttachments}
            className="inline-flex items-center gap-1.5 rounded-full border border-ink/10 px-2.5 py-1 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <Camera className="h-3.5 w-3.5" />
            Attach snapshot
          </button>
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
          <div>
            {saveError}
            {lastSavedRevision !== null ? ` Last saved revision: ${lastSavedRevision}.` : ""}
          </div>
          {initialState.savedSnapshot && (
            <button
              type="button"
              onClick={() => void restoreSavedRevision()}
              className="mt-2 inline-flex items-center gap-1.5 rounded-full border border-status-error/20 px-3 py-1 text-[11px] transition hover:bg-status-error/10"
            >
              <Save className="h-3.5 w-3.5" />
              Restore saved revision
            </button>
          )}
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
              const nextMeta = (currentSelected?.meta ?? null) as Record<string, unknown> | null;
              setSelectedShapeMeta(nextMeta);
              setWorkflowRunState(readWorkflowRunState(nextMeta));
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
      {(selectedShapeMeta || anchors.length > 0) && (
        <aside className="absolute right-4 top-16 z-20 w-[280px] rounded-2xl border border-ink/10 bg-surface-panel/95 p-3 shadow-lg backdrop-blur">
          <div className="text-caption uppercase tracking-wide text-ink/38">Inspector</div>
          {selectedShapeMeta ? (
            <div className="mt-3 space-y-2 text-body-sm text-ink/72">
              <div>
                <div className="text-caption text-ink/40">Type</div>
                <div>{String(selectedShapeMeta.kind ?? "note")}</div>
              </div>
              <div>
                <div className="text-caption text-ink/40">Title</div>
                <div>{String(selectedShapeMeta.title ?? "Untitled")}</div>
              </div>
              {typeof selectedShapeMeta.sourcePath === "string" && (
                <div>
                  <div className="text-caption text-ink/40">Source</div>
                  <div className="break-all">{selectedShapeMeta.sourcePath}</div>
                </div>
              )}
              {typeof selectedShapeMeta.threadId === "string" && (
                <div>
                  <div className="text-caption text-ink/40">Thread link</div>
                  <div className="break-all">{selectedShapeMeta.threadId}</div>
                </div>
              )}
              {typeof selectedShapeMeta.threadStageId === "string" && (
                <div>
                  <div className="text-caption text-ink/40">Stage link</div>
                  <div className="break-all">{selectedShapeMeta.threadStageId}</div>
                </div>
              )}
              {selectedShapeMeta.kind === "workflow" && (
                <div className="rounded-xl border border-ink/8 bg-ink/[0.03] px-2.5 py-2 text-caption text-ink/58">
                  Canvas keeps only the definition and latest run pointer here. Thread lanes, replay, and execution details stay in multi-session chat.
                </div>
              )}
              {selectedShapeMeta.kind === "workflow" && (
                <div className="flex flex-wrap gap-2 pt-1">
                  <button
                    type="button"
                    onClick={() => void runWorkflowSelection()}
                    disabled={bridgeBusy !== null || typeof selectedShapeMeta.threadId !== "string"}
                    className="inline-flex items-center gap-1.5 rounded-full border border-ink/10 px-3 py-1.5 text-caption text-ink/70 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    <Workflow className="h-3.5 w-3.5" />
                    Run workflow
                  </button>
                  <button
                    type="button"
                    onClick={openWorkflowThread}
                    disabled={bridgeBusy !== null || !canOpenWorkflowThread}
                    className="inline-flex items-center gap-1.5 rounded-full border border-ink/10 px-3 py-1.5 text-caption text-ink/70 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    <MessagesSquare className="h-3.5 w-3.5" />
                    Open thread chat
                  </button>
                </div>
              )}
              {workflowRunState && (
                <div>
                  <div className="text-caption text-ink/40">Latest run</div>
                  <div>{workflowRunState.status}</div>
                  <div className="break-all text-caption text-ink/45">{workflowRunState.runId}</div>
                  {workflowRunState.threadId && (
                    <button
                      type="button"
                      onClick={openWorkflowThread}
                      className="mt-2 inline-flex items-center gap-1 text-caption text-ink/55 transition hover:text-ink/72"
                    >
                      <ArrowUpRight className="h-3.5 w-3.5" />
                      Inspect full thread activity
                    </button>
                  )}
                </div>
              )}
            </div>
          ) : (
            <div className="mt-3 text-body-sm text-ink/48">
              Select a shape to inspect its source metadata.
            </div>
          )}
          {anchors.length > 0 && (
            <div className="mt-4 border-t border-ink/8 pt-3">
              <div className="text-caption uppercase tracking-wide text-ink/38">Recent anchors</div>
              <div className="mt-2 space-y-2">
                {anchors.slice(0, 4).map((anchor) => (
                  <div
                    key={anchor.id}
                    className={
                      "rounded-xl border px-2.5 py-2 text-caption " +
                      (anchor.anchorShapeId && anchor.anchorShapeId === selectedShapeId
                        ? "border-ink/16 bg-ink/[0.05] text-ink/72"
                        : "border-ink/8 bg-ink/[0.03] text-ink/65")
                    }
                  >
                    <div className="font-medium text-ink/72">{anchor.summary ?? "Canvas anchor"}</div>
                    <div className="mt-1 break-all text-ink/45">Turn: {anchor.turnId}</div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </aside>
      )}
      {!selectedShapeMeta && anchors.length === 0 && (
        <div className="pointer-events-none absolute right-4 top-16 z-10 w-[260px] rounded-2xl border border-dashed border-ink/10 bg-surface-panel/70 p-3 text-caption text-ink/42 backdrop-blur">
          Select a shape to inspect metadata, trigger workflow actions, or review anchors after sending a canvas selection.
        </div>
      )}
      {addMenuOpen && addMenuButtonRef.current && (
        <PopupMenu
          anchor={addMenuButtonRef.current}
          options={addMenuOptions}
          placement="bottom-start"
          className="z-[1010]"
          overlayClassName="z-[1009]"
          onSelect={(key) => void handleAddMenuSelect(key)}
          onClose={() => setAddMenuOpen(false)}
        />
      )}
      <div className="sr-only" aria-hidden>
        {composer.selectedAgent}
      </div>
      <style>{`
        .canvas-tldraw-host .tlui-layout__top {
          top: 56px;
        }

        .canvas-tldraw-host .tlui-style-panel__wrapper {
          top: 112px;
        }
      `}</style>
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

function normalizeNodeKind(value: string): CanvasNodeKind {
  switch (value) {
    case "file":
    case "image":
    case "video":
    case "workflow":
    case "note":
    case "group":
      return value;
    default:
      return "note";
  }
}

function buildShapeSummary(
  kind: string,
  title: string,
  sourcePath: string | null,
  meta: Record<string, unknown>,
): string {
  if (kind === "workflow") {
    return `${title}${sourcePath ? ` (${sourcePath})` : ""}`;
  }
  if (kind === "image") {
    return `${title}${sourcePath ? ` from ${sourcePath}` : ""}`;
  }
  if (kind === "file") {
    return `${title}${sourcePath ? ` at ${sourcePath}` : ""}`;
  }
  if (kind === "note") {
    const note = typeof meta.title === "string" ? meta.title : "Canvas note";
    return note;
  }
  return title;
}

function renderSelectionSummaryMarkdown(
  refs: Array<{
    title: string;
    sourcePath: string | null;
    kind: CanvasNodeKind;
    contextRef: CanvasContextRef;
  }>,
): string {
  const lines = [
    "# Canvas selection",
    "",
    ...refs.map((ref, index) =>
      `${index + 1}. ${ref.kind} - ${ref.title}${ref.sourcePath ? ` (${ref.sourcePath})` : ""}`,
    ),
    "",
    "Use the attached canvas snapshot and workflow summaries when helpful.",
  ];
  return lines.join("\n");
}

function renderWorkflowSummaryMarkdown(meta: Record<string, unknown>, title: string): string {
  const snapshotJson =
    typeof meta.workflowSnapshotJson === "string" && meta.workflowSnapshotJson.trim()
      ? meta.workflowSnapshotJson
      : null;
  const snapshot = snapshotJson ? tryParseJson(snapshotJson) : null;
  const stages = Array.isArray(snapshot?.stages) ? snapshot.stages : [];
  const lines = [
    `# ${title}`,
    "",
    typeof snapshot?.goal === "string" && snapshot.goal.trim() ? snapshot.goal.trim() : "Workflow summary",
    "",
  ];
  if (stages.length > 0) {
    lines.push("## Stages", "");
    for (const stage of stages.slice(0, 8)) {
      if (!stage || typeof stage !== "object") continue;
      const stageRecord = stage as Record<string, unknown>;
      const name =
        typeof stageRecord.name === "string" && stageRecord.name.trim()
          ? stageRecord.name.trim()
          : "Stage";
      const status =
        typeof stageRecord.status === "string" && stageRecord.status.trim()
          ? stageRecord.status.trim()
          : "unknown";
      lines.push(`- ${name}: ${status}`);
    }
    lines.push("");
  }
  return lines.join("\n");
}

function safeFilePrefix(value: string): string {
  const cleaned = value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  return cleaned || "canvas";
}

function parseWorkflowExecution(meta: Record<string, unknown> | null): {
  enabled: boolean;
  driver: string | null;
  lastRunId: string | null;
  lastStatus: string | null;
} {
  const execution = meta?.execution;
  if (!execution || typeof execution !== "object" || Array.isArray(execution)) {
    return {
      enabled: false,
      driver: null,
      lastRunId: null,
      lastStatus: null,
    };
  }
  const record = execution as Record<string, unknown>;
  return {
    enabled: Boolean(record.enabled),
    driver: typeof record.driver === "string" ? record.driver : null,
    lastRunId: typeof record.lastRunId === "string" ? record.lastRunId : null,
    lastStatus: typeof record.lastStatus === "string" ? record.lastStatus : null,
  };
}

function readWorkflowRunState(
  meta: Record<string, unknown> | null,
): { status: string; runId: string; threadId: string | null } | null {
  if (!meta || meta.kind !== "workflow") return null;
  const execution = parseWorkflowExecution(meta);
  if (!execution.lastStatus) return null;
  return {
    status: execution.lastStatus,
    runId: execution.lastRunId ?? "pending",
    threadId: typeof meta.threadId === "string" ? meta.threadId : null,
  };
}

function tryParseJson(value: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(value);
    return parsed && typeof parsed === "object" ? parsed as Record<string, unknown> : null;
  } catch {
    return null;
  }
}

async function saveBlobAsAttachment(blob: Blob, fileName: string): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = "";
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunkSize));
  }
  const { path } = await savePastedAttachment({
    fileName,
    mimeType: blob.type || "image/png",
    dataBase64: btoa(binary),
  });
  return path;
}
