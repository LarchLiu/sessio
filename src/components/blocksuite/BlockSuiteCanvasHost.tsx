import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { addImages, addNoteAtPoint, EdgelessRootService, ExportManager, NoteDisplayMode, type GroupElementModel } from "@blocksuite/blocks";
import { Bound } from "@blocksuite/global/utils";
import { createPortal } from "react-dom";
import { ArrowUpRight, Camera, Check, FileImage, FilePlus2, FolderOpen, Layers3, LoaderCircle, MessageCircleQuestionMark, MessagesSquare, RefreshCcw, Save, StickyNote, Workflow, X } from "lucide-react";
import type { ComposerAttachment } from "../ComposerAttachments";
import PopupMenu, { type PopupMenuOption } from "../PopupMenu";
import ScrollArea from "../ScrollArea";
import type {
  CanvasBlockKind,
  CanvasBlockRecord,
  CanvasContextOption,
  CanvasDocumentState,
} from "../../canvasTypes";
import {
  createAstraRun,
  createCanvasAnchor,
  createCanvasContextFile,
  getThreadWorkSnapshot,
  readLocalImageDataUrl,
  readWorkspaceTextFile,
  saveCanvasDraft,
  saveCanvasRevision,
  savePastedAttachment,
  updateCanvasBlocks,
  type Agent,
} from "../../api";
import type { ChatComposerController } from "../../hooks/useChatComposer";
import {
  createBlockSuiteDoc,
  createEdgelessEditorWithSpecs,
  ensureEdgelessRoot,
  exportDocSnapshot,
  importDocSnapshot,
} from "./bootstrap";
import { PortalHost } from "./portalHost";
import { useReactToLitBridge } from "../../lib/blocksuite/reactToLit";
import { setBlockSuitePortalBridge } from "../../lib/blocksuite/portalBridge";
import {
  canvasBlockRecordToContextRef,
  canvasInteropModelToCanvasBlock,
  renderSelectionSummaryMarkdown,
  renderWorkflowSummaryMarkdown,
  surfaceElementToCanvasBlock,
  tryParseJson,
  workflowSnapshotToMarkdown,
} from "../../lib/blocksuite/persistence";
import type { MarkdownPreviewBlockModel } from "../../lib/blocksuite/blocks/markdown-preview";
import type { FileCardBlockModel } from "../../lib/blocksuite/blocks/file-card";
import type { WorkflowCardBlockModel } from "../../lib/blocksuite/blocks/workflow-card";

const CANVAS_ADD_FILES_EVENT = "sessio:canvas-add-files";
const AUTOSAVE_DEBOUNCE_MS = 900;

type CanvasSelectionRef = {
  blockId: string;
  title: string;
  sourcePath: string | null;
  blockKind: CanvasBlockKind;
  meta: Record<string, unknown> | null;
};

type BlockSuiteEditor = HTMLElement & {
  std?: {
    get?: <T>(token: unknown) => T;
    getService?: (flavour: string) => EdgelessRootService | null;
    host?: HTMLElement & {
      view?: {
        getBlock?: (blockId: string) => HTMLElement | null;
      };
    };
  };
};

type SelectionElementLike = {
  id: string;
  flavour?: string;
  type?: string;
  xywh?: string;
  title?: string;
  childIds?: string[];
};

export interface BlockSuiteCanvasHostProps {
  sessionId: string;
  sessionAgent: Agent;
  workspacePath: string | null;
  sessionThreadId?: string | null;
  editedFiles?: string[];
  selectedFileRequest?: {
    paths: string[];
    requestId: number;
  } | null;
  initialState: CanvasDocumentState;
  initialSnapshot: string | null;
  composer: ChatComposerController;
  onStateLoaded: (state: CanvasDocumentState) => void;
  onError: (message: string) => void;
  onOpenThreadMultiSessionChat?: (threadId: string) => void;
}

function snapshotToJson(doc: ReturnType<typeof createBlockSuiteDoc>["doc"]) {
  const snapshot = exportDocSnapshot(doc);
  return snapshot ? JSON.stringify(snapshot) : null;
}

export default function BlockSuiteCanvasHost({
  sessionId,
  sessionAgent,
  workspacePath,
  sessionThreadId = null,
  editedFiles = [],
  selectedFileRequest = null,
  initialState,
  initialSnapshot,
  composer,
  onStateLoaded,
  onError,
  onOpenThreadMultiSessionChat,
}: BlockSuiteCanvasHostProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<BlockSuiteEditor | null>(null);
  const docRef = useRef<ReturnType<typeof createBlockSuiteDoc>["doc"] | null>(null);
  const latestStateRef = useRef(initialState);
  const blockUpdatedDisposeRef = useRef<{ dispose: () => void } | null>(null);
  const autosaveTimerRef = useRef<number | null>(null);
  const inflightSaveRef = useRef(false);
  const queuedSnapshotRef = useRef<string | null>(null);
  const currentSnapshotRef = useRef(initialSnapshot);
  const handledFileRequestRef = useRef<string | null>(null);
  const addMenuButtonRef = useRef<HTMLButtonElement>(null);
  const editedFilesButtonRef = useRef<HTMLButtonElement>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [status, setStatus] = useState("Initializing BlockSuite canvas…");
  const [isReady, setIsReady] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [selectionCount, setSelectionCount] = useState(0);
  const [selectedBlockId, setSelectedBlockId] = useState<string | null>(null);
  const [selectedBlockMeta, setSelectedBlockMeta] = useState<Record<string, unknown> | null>(null);
  const [anchors, setAnchors] = useState(initialState.anchors);
  const [blockRecords, setBlockRecords] = useState(initialState.blockRecords);
  const [bridgeBusy, setBridgeBusy] = useState<null | "ask" | "snapshot" | "workflow">(null);
  const [workflowRunState, setWorkflowRunState] = useState<null | { status: string; runId: string; threadId: string | null }>(null);
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  const [editedFilesPickerOpen, setEditedFilesPickerOpen] = useState(false);
  const [pendingEditedFiles, setPendingEditedFiles] = useState<string[]>([]);
  const [reactToLit, portals] = useReactToLitBridge();

  const changedFiles = useMemo(
    () => Array.from(new Set(editedFiles.map((path) => path.trim()).filter(Boolean))),
    [editedFiles],
  );
  const lastSavedRevision = initialState.savedRevision?.revision ?? null;

  const addMenuOptions = useMemo<PopupMenuOption<string>[]>(() => [
    { key: "file", label: "Choose files", icon: <FolderOpen className="h-4 w-4" /> },
    { key: "image", label: "Add image", icon: <FileImage className="h-4 w-4" /> },
    { key: "workflow", label: "Add workflow", icon: <Workflow className="h-4 w-4" /> },
    { key: "note", label: "Add note", icon: <StickyNote className="h-4 w-4" /> },
  ], []);

  useEffect(() => {
    latestStateRef.current = initialState;
    setAnchors(initialState.anchors);
    setBlockRecords(initialState.blockRecords);
  }, [initialState]);

  const getEditor = useCallback(() => editorRef.current, []);
  const getDoc = useCallback(() => docRef.current, []);

  const getRootService = useCallback(() => {
    return getEditor()?.std?.getService?.("affine:page") ?? null;
  }, [getEditor]);

  const updateSelectionState = useCallback(() => {
    const rootService = getRootService();
    if (!rootService) {
      setSelectionCount(0);
      setSelectedBlockId(null);
      setSelectedBlockMeta(null);
      setWorkflowRunState(null);
      return;
    }
    const selectedIds = rootService.selection.selectedIds ?? [];
    setSelectionCount(selectedIds.length);
    const selectedId = selectedIds[0] ?? null;
    setSelectedBlockId(selectedId);
    if (!selectedId) {
      setSelectedBlockMeta(null);
      setWorkflowRunState(null);
      return;
    }
    const nextMeta = readCanvasMeta(selectedId, getDoc(), blockRecords);
    setSelectedBlockMeta(nextMeta);
    setWorkflowRunState(readWorkflowRunState(nextMeta));
  }, [blockRecords, getDoc, getRootService]);

  const syncCanvasBlocks = useCallback(async (doc: NonNullable<ReturnType<typeof getDoc>>) => {
    const nextBlocks: CanvasBlockRecord["metadataJson"][] = [];
    const records = doc
      .getBlocks()
      .map((item) => canvasInteropModelToCanvasBlock(item as MarkdownPreviewBlockModel | FileCardBlockModel | WorkflowCardBlockModel))
      .filter((item): item is NonNullable<typeof item> => Boolean(item));

    const surface = doc.getBlocksByFlavour("affine:surface")[0]?.model as {
      elementModels?: unknown[];
    } | null;
    const surfaceRecords = Array.isArray(surface?.elementModels)
      ? surface.elementModels
          .map((item) => surfaceElementToCanvasBlock(item as { id: string; type?: string; title?: string; childIds?: string[] }))
          .filter((item): item is NonNullable<typeof item> => Boolean(item))
      : [];

    void nextBlocks;
    try {
      const saved = await updateCanvasBlocks({
        sessionId,
        blocks: [...records, ...surfaceRecords],
      });
      setBlockRecords(saved);
      latestStateRef.current = {
        ...latestStateRef.current,
        blockRecords: saved,
      };
      onStateLoaded({
        ...latestStateRef.current,
        blockRecords: saved,
      });
    } catch (error) {
      const message = `Failed to sync canvas blocks: ${String(error)}`;
      setSaveError(message);
      onError(message);
    } finally {
      updateSelectionState();
    }
  }, [onError, onStateLoaded, sessionId, updateSelectionState]);

  const scheduleSyncBlocks = useCallback(() => {
    const doc = getDoc();
    if (!doc) return;
    void syncCanvasBlocks(doc);
  }, [getDoc, syncCanvasBlocks]);

  const flushSaveRef = useRef<(snapshotJson: string) => Promise<void>>(async () => {});

  const attachDoc = useCallback((host: HTMLDivElement, doc: ReturnType<typeof createBlockSuiteDoc>["doc"]) => {
    ensureEdgelessRoot(doc);
    const editor = createEdgelessEditorWithSpecs(doc) as BlockSuiteEditor;
    editorRef.current?.remove();
    editorRef.current = editor;
    docRef.current = doc;
    host.replaceChildren(editor);
    blockUpdatedDisposeRef.current?.dispose();
    blockUpdatedDisposeRef.current = doc.slots.blockUpdated.on(() => {
      const snapshotJson = snapshotToJson(doc);
      if (!snapshotJson || snapshotJson === currentSnapshotRef.current) return;
      if (autosaveTimerRef.current !== null) {
        window.clearTimeout(autosaveTimerRef.current);
      }
      autosaveTimerRef.current = window.setTimeout(() => {
        if (inflightSaveRef.current) {
          queuedSnapshotRef.current = snapshotJson;
          return;
        }
        void flushSaveRef.current(snapshotJson);
      }, AUTOSAVE_DEBOUNCE_MS);
      updateSelectionState();
    });
    window.requestAnimationFrame(() => {
      updateSelectionState();
      scheduleSyncBlocks();
    });
  }, [scheduleSyncBlocks, updateSelectionState]);

  const openEditedFilesPicker = () => {
    if (changedFiles.length === 0) return;
    setPendingEditedFiles((current) => (
      current.length > 0
        ? current.filter((path) => changedFiles.includes(path))
        : changedFiles
    ));
    setEditedFilesPickerOpen(true);
  };

  const closeEditedFilesPicker = () => {
    setEditedFilesPickerOpen(false);
  };

  const togglePendingEditedFile = (path: string) => {
    setPendingEditedFiles((current) => (
      current.includes(path)
        ? current.filter((item) => item !== path)
        : [...current, path]
    ));
  };

  const resolveFileSourceType = useCallback((path: string) => {
    if (!workspacePath) return "inline_markdown";
    const normalizedWorkspace = normalizePathSegment(workspacePath);
    const normalizedPath = normalizePathSegment(resolveCanvasFilePath(path, workspacePath) ?? path);
    if (
      normalizedPath === normalizedWorkspace ||
      normalizedPath.startsWith(`${normalizedWorkspace}/`)
    ) {
      return changedFiles.includes(path) || changedFiles.includes(normalizedPath)
        ? "edited_file"
        : "workspace_file";
    }
    return "inline_markdown";
  }, [changedFiles, workspacePath]);

  const addFileCards = useCallback(async (paths: string[]) => {
    const doc = getDoc();
    const rootService = getRootService();
    if (!doc || !rootService || !workspacePath) return;
    const uniquePaths = Array.from(new Set(paths.map((path) => path.trim()).filter(Boolean)));
    const center = rootService.viewport.center;
    for (const [index, path] of uniquePaths.entries()) {
      try {
        const absolutePath = resolveCanvasFilePath(path, workspacePath);
        if (!absolutePath) throw new Error("file path is unavailable");
        const file = await readWorkspaceTextFile(workspacePath, absolutePath).catch(() => null);
        const title = absolutePath.split(/[/\\]/).pop() ?? absolutePath;
        const bound = Bound.fromCenter(
          [center.x + (index % 3) * 360, center.y + Math.floor(index / 3) * 156],
          340,
          144,
        );
        rootService.addBlock(
          "sessio:file-card",
          {
            title,
            sourcePath: absolutePath,
            sourceType: resolveFileSourceType(path),
            subtitle: absolutePath,
            summary: summarizeText(file?.content ?? "", 260),
            status: file ? "ready" : "unavailable",
            contentVersion: file ? `${absolutePath}:${file.mtimeMs}` : absolutePath,
            xywh: bound.serialize(),
          },
          doc.getBlocksByFlavour("affine:surface")[0]?.model ?? undefined,
        );
      } catch (error) {
        onError(`Failed to add file card for ${path}: ${String(error)}`);
      }
    }
    await syncCanvasBlocks(doc);
  }, [getDoc, getRootService, onError, resolveFileSourceType, syncCanvasBlocks, workspacePath]);

  const addPendingEditedFiles = () => {
    if (pendingEditedFiles.length === 0) return;
    void addFileCards(pendingEditedFiles);
    setEditedFilesPickerOpen(false);
  };

  const addNoteNode = useCallback(async () => {
    const rootService = getRootService();
    const editor = getEditor();
    if (!rootService || !editor) return;
    const [x, y] = rootService.viewport.toViewCoord(rootService.viewport.center.x, rootService.viewport.center.y);
    const noteId = addNoteAtPoint(editor.std as never, { x, y });
    const doc = getDoc();
    if (!doc) return;
    const note = doc.getBlockById(noteId);
    if (note) {
      doc.updateBlock(note, {
        displayMode: NoteDisplayMode.EdgelessOnly,
      });
    }
    scheduleSyncBlocks();
  }, [getDoc, getEditor, getRootService, scheduleSyncBlocks]);

  const addImageNode = useCallback(async () => {
    const editor = getEditor();
    if (!editor) return;
    try {
      const selection = await open({
        multiple: true,
        directory: false,
        filters: [
          {
            name: "Images",
            extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "heif"],
          },
        ],
      });
      if (!selection) return;
      const paths = Array.isArray(selection) ? selection : [selection];
      const files = await Promise.all(paths.map(async (path) => {
        const response = await fetch(convertFileSrc(path));
        const blob = await response.blob();
        return new File([blob], path.split(/[/\\]/).pop() ?? "image", {
          type: blob.type || "image/png",
        });
      }));
      await addImages(editor.std as never, files);
      scheduleSyncBlocks();
    } catch (error) {
      onError(`Failed to add image node: ${String(error)}`);
    }
  }, [getEditor, onError, scheduleSyncBlocks]);

  const addWorkflowCard = useCallback(async () => {
    const doc = getDoc();
    const rootService = getRootService();
    if (!doc || !rootService) return;
    const snapshotResult = await getThreadWorkSnapshot(sessionAgent, sessionId).catch(() => null);
    const snapshot = snapshotResult?.snapshot ?? null;
    const title = snapshot?.goal?.trim() || "Workflow";
    const summaryMarkdown = workflowSnapshotToMarkdown(snapshot);
    const center = rootService.viewport.center;
    const bound = Bound.fromCenter([center.x, center.y], 360, 196);
    rootService.addBlock(
      "sessio:workflow-card",
      {
        title,
        threadId: snapshotResult?.threadId ?? "",
        threadStageId: snapshotResult?.stageId ?? "",
        sourceType: "workflow_definition",
        workflowSummaryMarkdown: summaryMarkdown,
        executionState: "idle",
        lastRunId: "",
        workflowSnapshotJson: snapshot ? JSON.stringify(snapshot) : "",
        threadGoal: snapshot?.goal ?? "",
        status: "ready",
        xywh: bound.serialize(),
      },
      doc.getBlocksByFlavour("affine:surface")[0]?.model ?? undefined,
    );
    await syncCanvasBlocks(doc);
  }, [getDoc, getRootService, sessionAgent, sessionId, syncCanvasBlocks]);

  const groupSelection = useCallback(() => {
    const rootService = getRootService();
    if (!rootService || rootService.selection.selectedElements.length < 2) return;
    rootService.createGroupFromSelected();
    scheduleSyncBlocks();
  }, [getRootService, scheduleSyncBlocks]);

  const ungroupSelection = useCallback(() => {
    const rootService = getRootService();
    if (!rootService || rootService.selection.selectedElements.length !== 1) return;
    const selected = rootService.selection.selectedElements[0] as SelectionElementLike | undefined;
    if (!selected || selected.type !== "group") return;
    rootService.ungroup(selected as unknown as GroupElementModel);
    scheduleSyncBlocks();
  }, [getRootService, scheduleSyncBlocks]);

  const promoteFileCardToMarkdown = useCallback((blockId: string) => {
    const doc = getDoc();
    const rootService = getRootService();
    if (!doc || !rootService) return;
    const model = doc.getBlockById(blockId) as FileCardBlockModel | null;
    if (!model) return;
    const bound = Bound.deserialize(model.xywh);
    const nextWidth = Math.max(bound.w, 420);
    const nextHeight = Math.max(bound.h + 96, 260);
    rootService.addBlock(
      "sessio:markdown-preview",
      {
        title: model.title || "Markdown preview",
        sourcePath: model.sourcePath || "",
        sourceType: model.sourceType || "workspace_file",
        excerpt: model.summary || "",
        renderMode: "summary",
        collapsed: false,
        contentVersion: model.contentVersion || model.sourcePath || "",
        cachedContent: "",
        xywh: Bound.fromCenter([bound.center[0] + 28, bound.center[1] + 28], nextWidth, nextHeight).serialize(),
      },
      doc.getBlocksByFlavour("affine:surface")[0]?.model ?? undefined,
    );
    rootService.removeElement(blockId);
    scheduleSyncBlocks();
  }, [getDoc, getRootService, scheduleSyncBlocks]);

  const runWorkflowBlock = useCallback(async (blockId: string) => {
    const doc = getDoc();
    if (!doc) return;
    const model = doc.getBlockById(blockId) as WorkflowCardBlockModel | null;
    const threadId = model?.threadId?.trim() || "";
    if (!model || !threadId) {
      onError("This workflow card is not linked to a thread workflow.");
      return;
    }
    setBridgeBusy("workflow");
    try {
      doc.updateBlock(model, {
        executionState: "running",
        status: "running",
      });
      const run = await createAstraRun(threadId, composer.text.trim() || null);
      const nextRunState = {
        status: run.status,
        runId: run.runId,
        threadId,
      };
      setWorkflowRunState(nextRunState);
      doc.updateBlock(model, {
        executionState: run.status,
        lastRunId: run.runId,
        status: run.status,
      });
      await syncCanvasBlocks(doc);
      updateSelectionState();
    } catch (error) {
      doc.updateBlock(model, {
        executionState: "failed",
        status: "failed",
      });
      onError(`Failed to start workflow run: ${String(error)}`);
    } finally {
      setBridgeBusy(null);
    }
  }, [composer.text, getDoc, onError, syncCanvasBlocks, updateSelectionState]);

  const openWorkflowThread = useCallback((blockId: string) => {
    const doc = getDoc();
    const model = doc?.getBlockById(blockId) as WorkflowCardBlockModel | null;
    const targetThreadId = model?.threadId?.trim() || sessionThreadId || "";
    if (!targetThreadId) {
      onError("This workflow card is not linked to a thread workflow yet.");
      return;
    }
    if (!onOpenThreadMultiSessionChat) {
      onError("Thread multi-session chat is not available from this view.");
      return;
    }
    onOpenThreadMultiSessionChat(targetThreadId);
  }, [getDoc, onError, onOpenThreadMultiSessionChat, sessionThreadId]);

  useEffect(() => {
    setBlockSuitePortalBridge({
      reactToLit,
      workspacePath,
      updateBlock: (blockId, props) => {
        const doc = getDoc();
        const model = doc?.getBlockById(blockId) ?? null;
        if (!doc || !model) return;
        doc.updateBlock(model, props);
        scheduleSyncBlocks();
      },
      promoteFileCardToMarkdown,
      runWorkflowBlock,
      openWorkflowThread,
    });
    return () => {
      setBlockSuitePortalBridge(null);
    };
  }, [getDoc, openWorkflowThread, promoteFileCardToMarkdown, reactToLit, runWorkflowBlock, scheduleSyncBlocks, workspacePath]);

  useEffect(() => {
    currentSnapshotRef.current = initialSnapshot;
  }, [initialSnapshot, sessionId]);

  useEffect(() => {
    return () => {
      if (autosaveTimerRef.current !== null) {
        window.clearTimeout(autosaveTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    let activeDoc = createBlockSuiteDoc(initialState.document.id).doc;
    const documentTitle = initialState.document.title;

    const clearSaveError = () => {
      setSaveError(null);
    };

    const mountDoc = (doc: typeof activeDoc) => {
      activeDoc = doc;
      attachDoc(host, doc);
    };

    const flushSave = async (snapshotJson: string) => {
      inflightSaveRef.current = true;
      setIsSaving(true);
      try {
        const saved = await saveCanvasDraft({
          sessionId,
          title: documentTitle,
          snapshotJson,
        });
        currentSnapshotRef.current = snapshotJson;
        clearSaveError();
        setStatus("Draft saved.");
        latestStateRef.current = {
          ...latestStateRef.current,
          document: saved.document,
          draftSnapshot: snapshotJson,
        };
        onStateLoaded(latestStateRef.current);
        if (docRef.current) {
          void syncCanvasBlocks(docRef.current);
        }
      } catch (error) {
        const message = `Canvas draft save failed: ${String(error)}`;
        setSaveError(message);
        onError(message);
      } finally {
        inflightSaveRef.current = false;
        setIsSaving(false);
        const queued = queuedSnapshotRef.current;
        queuedSnapshotRef.current = null;
        if (queued && queued !== currentSnapshotRef.current) {
          void flushSave(queued);
        }
      }
    };
    flushSaveRef.current = flushSave;

    const restore = async () => {
      try {
        if (!initialSnapshot) {
          mountDoc(activeDoc);
          clearSaveError();
          setStatus("Opened a fresh BlockSuite canvas.");
          setIsReady(true);
          return;
        }
        const snapshot = JSON.parse(initialSnapshot);
        const restored = await importDocSnapshot(snapshot);
        mountDoc(restored ?? activeDoc);
        clearSaveError();
        setStatus("Restored canvas snapshot.");
        setIsReady(true);
      } catch (error) {
        mountDoc(activeDoc);
        const message = `Failed to restore saved canvas state: ${String(error)}`;
        setSaveError(message);
        onError(message);
        setStatus("Opened a fresh BlockSuite canvas after restore failure.");
        setIsReady(true);
      }
    };

    void restore();

    return () => {
      if (autosaveTimerRef.current !== null) {
        window.clearTimeout(autosaveTimerRef.current);
      }
      blockUpdatedDisposeRef.current?.dispose();
      blockUpdatedDisposeRef.current = null;
      editorRef.current?.remove();
      editorRef.current = null;
    };
  }, [
    attachDoc,
    initialSnapshot,
    initialState.document.id,
    initialState.document.title,
    onError,
    onStateLoaded,
    sessionId,
    syncCanvasBlocks,
  ]);

  const handleSaveRevision = async () => {
    const doc = getDoc();
    if (!doc) return;
    const snapshotJson = snapshotToJson(doc);
    if (!snapshotJson) {
      onError("Canvas snapshot export failed.");
      return;
    }
    setIsSaving(true);
    try {
      const saved = await saveCanvasRevision({
        sessionId,
        title: initialState.document.title,
        snapshotJson,
        source: "manual",
      });
      currentSnapshotRef.current = snapshotJson;
      setSaveError(null);
      setStatus(`Saved revision ${saved.revision.revision}.`);
      latestStateRef.current = {
        ...latestStateRef.current,
        document: saved.document,
        draftSnapshot: snapshotJson,
        savedRevision: saved.revision,
        savedSnapshot: snapshotJson,
      };
      onStateLoaded(latestStateRef.current);
      await syncCanvasBlocks(doc);
    } catch (error) {
      const message = `Canvas save failed: ${String(error)}`;
      setSaveError(message);
      onError(message);
    } finally {
      setIsSaving(false);
    }
  };

  const handleRestoreSavedRevision = async () => {
    const savedSnapshot = initialState.savedSnapshot;
    if (!savedSnapshot || !hostRef.current) return;
    try {
      const snapshot = JSON.parse(savedSnapshot);
      const restored = await importDocSnapshot(snapshot);
      if (!restored) {
        throw new Error("snapshot import returned null");
      }
      ensureEdgelessRoot(restored);
      attachDoc(hostRef.current, restored);
      currentSnapshotRef.current = savedSnapshot;
      setSaveError(null);
      setStatus("Restored the last saved revision.");
      latestStateRef.current = {
        ...latestStateRef.current,
        draftSnapshot: savedSnapshot,
      };
      onStateLoaded(latestStateRef.current);
      await syncCanvasBlocks(restored);
    } catch (error) {
      const message = `Failed to restore the last saved revision: ${String(error)}`;
      setSaveError(message);
      onError(message);
    }
  };

  const handleAddMenuSelect = async (key: string) => {
    setAddMenuOpen(false);
    if (key === "note") {
      await addNoteNode();
      return;
    }
    if (key === "workflow") {
      await addWorkflowCard();
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
        await addFileCards(Array.isArray(selection) ? selection : [selection]);
      } catch (error) {
        onError(`Failed to add file card: ${String(error)}`);
      }
    }
  };

  const exportSelectionSnapshot = useCallback(async () => {
    const editor = getEditor();
    const rootService = getRootService();
    if (!editor || !rootService) return null;
    const selected = rootService.selection.selectedElements;
    if (selected.length === 0) return null;
    const bounds = selected
      .map((item) => Bound.deserialize(item.xywh))
      .filter(Boolean);
    if (bounds.length === 0) return null;
    const common = bounds.reduce((current, next) => current.unite(next));
    const exportManager = editor.std?.get?.(ExportManager) as ExportManager | null;
    const rootHost = editor.std?.host?.querySelector?.("affine-edgeless-root") as {
      surface?: { renderer?: unknown };
    } | null;
    if (!exportManager || !rootHost?.surface?.renderer) return null;
    const blocks = selected.filter(item => "flavour" in item) as BlockSuite.EdgelessBlockModelType[];
    const shapes = selected.filter(item => !("flavour" in item)) as BlockSuite.SurfaceModel[];
    const canvas = await exportManager.edgelessToCanvas(rootHost.surface.renderer as never, common, undefined, blocks, shapes, {
      zoom: rootService.viewport.zoom,
    });
    if (!canvas) return null;
    const blob: Blob = await new Promise((resolve, reject) =>
      canvas.toBlob(
        next => (next ? resolve(next) : reject(new Error("Canvas can not export blob"))),
        "image/png",
      ),
    );
    const path = await saveBlobAsAttachment(blob, `canvas-selection-${Date.now()}.png`);
    const previewDataUrl = await readLocalImageDataUrl(path).catch(() => null);
    return {
      path,
      attachment: {
        path,
        kind: "image",
        mimeType: "image/png",
        previewDataUrl,
        displayName: "Canvas selection",
        name: "Canvas selection",
      } satisfies ComposerAttachment,
    };
  }, [getEditor, getRootService]);

  const getSelectedCanvasRefs = useCallback((): CanvasSelectionRef[] => {
    const rootService = getRootService();
    if (!rootService) return [];
    const selectedIds = rootService.selection.selectedIds ?? [];
    const selectedSet = new Set(selectedIds);
    const refsById = new Map(blockRecords.map((ref) => [ref.blockId, ref]));
    return selectedIds.map((id) => {
      const ref = refsById.get(id);
      const meta = readCanvasMeta(id, getDoc(), blockRecords);
      const title =
        (typeof meta?.title === "string" && meta.title.trim() ? meta.title.trim() : null)
        ?? ref?.sourceKey
        ?? fallbackTitle(meta?.kind, id);
      const kind = normalizeBlockKind(
        typeof meta?.kind === "string" ? meta.kind : ref?.blockKind ?? "note",
      );
      const sourcePath =
        typeof meta?.sourcePath === "string" && meta.sourcePath.trim()
          ? meta.sourcePath
          : ref?.sourcePath ?? null;
      if (!selectedSet.has(id)) return null;
      return {
        blockId: id,
        title,
        sourcePath,
        blockKind: kind,
        meta,
      };
    }).filter((item): item is CanvasSelectionRef => Boolean(item));
  }, [blockRecords, getDoc, getRootService]);

  const buildSelectionContext = useCallback(async () => {
    const refs = getSelectedCanvasRefs();
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
    const workflowRefs = refs.filter((ref) => ref.blockKind === "workflow_card");
    for (const workflow of workflowRefs) {
      const workflowMarkdown = renderWorkflowSummaryMarkdown(workflow.meta ?? {}, workflow.title);
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
      blockIds: refs.map((ref) => ref.blockId),
      elementIds: [],
      snapshotAttachmentPath: snapshot?.path ?? null,
      refs: refs.map((ref) => {
        const record = blockRecords.find((item) => item.blockId === ref.blockId);
        return record
          ? canvasBlockRecordToContextRef(record)
          : {
              blockId: ref.blockId,
              blockKind: ref.blockKind,
              sourceType: "note",
              sourcePath: ref.sourcePath,
              sourceKey: ref.title,
              summary: ref.title,
            };
      }),
    };
    return {
      refs,
      attachments,
      canvasContext,
    };
  }, [blockRecords, exportSelectionSnapshot, getSelectedCanvasRefs, initialState.document.id, sessionId]);

  const attachSelectionSnapshot = async () => {
    if (!composer.supportsImageAttachments) {
      onError("The selected agent does not support image attachments.");
      return;
    }
    setBridgeBusy("snapshot");
    try {
      const snapshot = await exportSelectionSnapshot();
      if (!snapshot) {
        onError("Select one or more blocks before attaching a snapshot.");
        return;
      }
      await composer.appendAttachments([snapshot.attachment]);
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
        onError("Select one or more canvas items before asking about the canvas.");
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
        anchorBlockId: payload.refs[0]?.blockId ?? null,
        selectionBlockIdsJson: JSON.stringify(payload.refs.map((ref) => ref.blockId)),
        selectionElementIdsJson: "[]",
        turnId,
        summary: payload.refs.map((ref) => ref.title).join(", ").slice(0, 180),
      });
      setAnchors((current) => [anchor, ...current]);
      latestStateRef.current = {
        ...latestStateRef.current,
        anchors: [anchor, ...latestStateRef.current.anchors],
      };
    } catch (error) {
      onError(`Failed to ask about the current selection: ${String(error)}`);
    } finally {
      setBridgeBusy(null);
    }
  };

  useEffect(() => {
    const requestId = selectedFileRequest?.requestId ?? null;
    if (!selectedFileRequest || requestId === null) return;
    const requestKey = `${sessionId}:${requestId}`;
    if (handledFileRequestRef.current === requestKey) return;
    handledFileRequestRef.current = requestKey;
    void addFileCards(selectedFileRequest.paths);
  }, [addFileCards, selectedFileRequest, sessionId, workspacePath]);

  useEffect(() => {
    const handleCanvasAddFiles = (event: Event) => {
      const detail = (event as CustomEvent<{ paths?: string[]; sessionId?: string | null }>).detail;
      if (!detail || detail.sessionId !== sessionId || !Array.isArray(detail.paths)) return;
      void addFileCards(detail.paths);
    };
    window.addEventListener(CANVAS_ADD_FILES_EVENT, handleCanvasAddFiles);
    return () => window.removeEventListener(CANVAS_ADD_FILES_EVENT, handleCanvasAddFiles);
  }, [addFileCards, sessionId]);

  const canOpenWorkflowThread = Boolean(
    onOpenThreadMultiSessionChat && (
      (selectedBlockMeta && typeof selectedBlockMeta.threadId === "string" && selectedBlockMeta.threadId.trim()) ||
      sessionThreadId
    ),
  );

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col">
      <div className="flex h-9 shrink-0 items-center justify-between gap-3 border-b border-ink/8 bg-surface-panel/90 px-4">
        <div className="flex items-center gap-3 text-caption text-ink/50">
          <button
            ref={addMenuButtonRef}
            type="button"
            onClick={() => setAddMenuOpen((value) => !value)}
            className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/12 px-3 text-ink/72 transition hover:bg-ink/5"
          >
            <Layers3 className="h-3.5 w-3.5" />
            Add to canvas
          </button>
          <button
            ref={editedFilesButtonRef}
            type="button"
            onClick={openEditedFilesPicker}
            disabled={changedFiles.length === 0}
            className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-3 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <FilePlus2 className="h-3.5 w-3.5" />
            Add edited files
          </button>
        </div>
        <div className="flex items-center gap-2 text-caption">
          <span className="inline-flex h-6 items-center rounded-md border border-ink/10 px-2.5 text-ink/55">
            {selectionCount > 0 ? `${selectionCount} selected` : isReady ? status : "Canvas"}
          </span>
          <button
            type="button"
            onClick={() => void askSelection()}
            disabled={selectionCount === 0 || bridgeBusy !== null}
            className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-2.5 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <MessageCircleQuestionMark className="h-3.5 w-3.5" />
            Ask selection
          </button>
          <button
            type="button"
            onClick={() => void attachSelectionSnapshot()}
            disabled={selectionCount === 0 || bridgeBusy !== null || !composer.supportsImageAttachments}
            className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-2.5 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <Camera className="h-3.5 w-3.5" />
            Attach snapshot
          </button>
          <button
            type="button"
            onClick={groupSelection}
            disabled={selectionCount < 2}
            className="inline-flex h-6 items-center rounded-md border border-ink/10 px-2.5 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Group
          </button>
          <button
            type="button"
            onClick={ungroupSelection}
            disabled={selectionCount !== 1 || selectedBlockMeta?.kind !== "group"}
            className="inline-flex h-6 items-center rounded-md border border-ink/10 px-2.5 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Ungroup
          </button>
          {initialState.savedSnapshot && (
            <button
              type="button"
              onClick={() => void handleRestoreSavedRevision()}
              className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-2.5 text-ink/65 transition hover:bg-ink/5"
            >
              <RefreshCcw className="h-3.5 w-3.5" />
              Restore
            </button>
          )}
          <button
            type="button"
            onClick={() => void handleSaveRevision()}
            disabled={!isReady || isSaving}
            className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-2.5 text-ink/70 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {isReady ? <Save className="h-3.5 w-3.5" /> : <LoaderCircle className="h-3.5 w-3.5 animate-spin" />}
            Save
          </button>
        </div>
      </div>
      {saveError && (
        <div className="mx-4 mt-3 rounded-xl border border-status-error/20 bg-status-error/8 px-3 py-2 text-caption text-status-error">
          <div>
            {saveError}
            {lastSavedRevision !== null ? ` Last saved revision: ${lastSavedRevision}.` : ""}
          </div>
        </div>
      )}
      <div ref={hostRef} className="min-h-0 flex-1" />
      {(selectedBlockMeta || anchors.length > 0) && (
        <aside className="absolute right-4 top-16 z-20 w-[280px] rounded-2xl border border-ink/10 bg-surface-panel/95 p-3 shadow-lg backdrop-blur">
          <div className="text-caption uppercase tracking-wide text-ink/38">Inspector</div>
          {selectedBlockMeta ? (
            <div className="mt-3 space-y-2 text-body-sm text-ink/72">
              <div>
                <div className="text-caption text-ink/40">Type</div>
                <div>{String(selectedBlockMeta.kind ?? "note")}</div>
              </div>
              <div>
                <div className="text-caption text-ink/40">Title</div>
                <div>{String(selectedBlockMeta.title ?? "Untitled")}</div>
              </div>
              {typeof selectedBlockMeta.sourcePath === "string" && (
                <div>
                  <div className="text-caption text-ink/40">Source</div>
                  <div className="break-all">{selectedBlockMeta.sourcePath}</div>
                </div>
              )}
              {typeof selectedBlockMeta.threadId === "string" && (
                <div>
                  <div className="text-caption text-ink/40">Thread link</div>
                  <div className="break-all">{selectedBlockMeta.threadId}</div>
                </div>
              )}
              {typeof selectedBlockMeta.threadStageId === "string" && (
                <div>
                  <div className="text-caption text-ink/40">Stage link</div>
                  <div className="break-all">{selectedBlockMeta.threadStageId}</div>
                </div>
              )}
              {selectedBlockMeta.kind === "workflow_card" && (
                <div className="rounded-xl border border-ink/8 bg-ink/[0.03] px-2.5 py-2 text-caption text-ink/58">
                  Canvas keeps the workflow mirror and latest run pointer here. Replay and execution details stay in multi-session chat.
                </div>
              )}
              {selectedBlockMeta.kind === "workflow_card" && selectedBlockId && (
                <div className="flex flex-wrap gap-2 pt-1">
                  <button
                    type="button"
                    onClick={() => void runWorkflowBlock(selectedBlockId)}
                    disabled={bridgeBusy !== null || typeof selectedBlockMeta.threadId !== "string"}
                    className="inline-flex items-center gap-1.5 rounded-md border border-ink/10 px-3 py-1.5 text-caption text-ink/70 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    <Workflow className="h-3.5 w-3.5" />
                    Run workflow
                  </button>
                  <button
                    type="button"
                    onClick={() => openWorkflowThread(selectedBlockId)}
                    disabled={bridgeBusy !== null || !canOpenWorkflowThread}
                    className="inline-flex items-center gap-1.5 rounded-md border border-ink/10 px-3 py-1.5 text-caption text-ink/70 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
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
                      onClick={() => {
                        if (selectedBlockId) openWorkflowThread(selectedBlockId);
                      }}
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
              Select a block to inspect its source metadata.
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
                      (anchor.anchorBlockId && anchor.anchorBlockId === selectedBlockId
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
      {!selectedBlockMeta && anchors.length === 0 && (
        <div className="pointer-events-none absolute right-4 top-16 z-10 w-[260px] rounded-2xl border border-dashed border-ink/10 bg-surface-panel/70 p-3 text-caption text-ink/42 backdrop-blur">
          Select a block to inspect metadata, trigger workflow actions, or review anchors after sending a canvas selection.
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
      {editedFilesPickerOpen && editedFilesButtonRef.current && (
        <EditedFilesPopover
          anchor={editedFilesButtonRef.current}
          files={changedFiles}
          selectedFiles={pendingEditedFiles}
          onToggleFile={togglePendingEditedFile}
          onAdd={addPendingEditedFiles}
          onClose={closeEditedFilesPicker}
        />
      )}
      <PortalHost portals={portals} />
    </div>
  );
}

function normalizePathSegment(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+$/, "");
}

function summarizeText(content: string, maxLength: number): string {
  return content
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .slice(0, 6)
    .join(" ")
    .slice(0, maxLength);
}

function resolveCanvasFilePath(path: string, workspacePath: string | null): string | null {
  if (!path) return null;
  if (/^([a-zA-Z]:[\\/]|\/)/.test(path)) return path;
  if (!workspacePath) return null;
  const separator = workspacePath.includes("\\") ? "\\" : "/";
  const trimmedRoot = workspacePath.replace(/[\\/]+$/, "");
  const trimmedPath = path.replace(/^[\\/]+/, "");
  return `${trimmedRoot}${separator}${trimmedPath}`;
}

function normalizeBlockKind(value: string): CanvasBlockKind {
  switch (value) {
    case "markdown_preview":
    case "file_card":
    case "workflow_card":
    case "note":
    case "group":
    case "image":
      return value;
    default:
      return "note";
  }
}

function fallbackTitle(kind: unknown, blockId: string): string {
  if (kind === "workflow_card") return "Workflow";
  if (kind === "file_card") return "File";
  if (kind === "markdown_preview") return "Markdown preview";
  if (kind === "image") return "Image";
  if (kind === "group") return "Group";
  return blockId ? `Block ${blockId.slice(0, 6)}` : "Canvas note";
}

function readWorkflowRunState(
  meta: Record<string, unknown> | null,
): { status: string; runId: string; threadId: string | null } | null {
  if (!meta || meta.kind !== "workflow_card") return null;
  const status = typeof meta.executionState === "string" ? meta.executionState : null;
  if (!status) return null;
  return {
    status,
    runId: typeof meta.lastRunId === "string" && meta.lastRunId.trim() ? meta.lastRunId : "pending",
    threadId: typeof meta.threadId === "string" ? meta.threadId : null,
  };
}

function readCanvasMeta(
  blockId: string,
  doc: ReturnType<typeof createBlockSuiteDoc>["doc"] | null,
  blockRecords: CanvasBlockRecord[],
): Record<string, unknown> | null {
  const model = doc?.getBlockById(blockId) as (Record<string, unknown> & { flavour?: string }) | null;
  if (model) {
    if (model.flavour === "sessio:markdown-preview") {
      return {
        kind: "markdown_preview",
        title: model.title,
        sourcePath: model.sourcePath,
        sourceType: model.sourceType,
        excerpt: model.excerpt,
        renderMode: model.renderMode,
      };
    }
    if (model.flavour === "sessio:file-card") {
      return {
        kind: "file_card",
        title: model.title,
        sourcePath: model.sourcePath,
        sourceType: model.sourceType,
        subtitle: model.subtitle,
        summary: model.summary,
        status: model.status,
      };
    }
    if (model.flavour === "sessio:workflow-card") {
      return {
        kind: "workflow_card",
        title: model.title,
        threadId: model.threadId,
        threadStageId: model.threadStageId,
        workflowSummaryMarkdown: model.workflowSummaryMarkdown,
        executionState: model.executionState,
        lastRunId: model.lastRunId,
        threadGoal: model.threadGoal,
        workflowSnapshotJson: model.workflowSnapshotJson,
      };
    }
    if (model.flavour === "affine:note") {
      return {
        kind: "note",
        title: blockRecords.find(item => item.blockId === blockId)?.sourceKey ?? "New note",
      };
    }
    if (model.flavour === "affine:image") {
      return {
        kind: "image",
        title: typeof model.caption === "string" && model.caption.trim() ? model.caption.trim() : "Image",
      };
    }
  }
  const record = blockRecords.find((item) => item.blockId === blockId);
  return tryParseJson(record?.metadataJson);
}

function safeFilePrefix(value: string): string {
  const cleaned = value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  return cleaned || "canvas";
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

function EditedFilesPopover({
  anchor,
  files,
  selectedFiles,
  onToggleFile,
  onAdd,
  onClose,
}: {
  anchor: HTMLElement;
  files: string[];
  selectedFiles: string[];
  onToggleFile: (path: string) => void;
  onAdd: () => void;
  onClose: () => void;
}) {
  const popoverRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{
    top: number;
    left: number;
    width: number;
    maxHeight: number;
  } | null>(null);

  useLayoutEffect(() => {
    const update = () => {
      const rect = anchor.getBoundingClientRect();
      const gap = 6;
      const margin = 8;
      const width = Math.min(420, Math.max(320, rect.width + 120));
      const top = rect.bottom + gap;
      const maxHeight = Math.max(200, window.innerHeight - top - margin);
      const left = Math.min(
        Math.max(margin, rect.left),
        window.innerWidth - margin - width,
      );
      setPos({
        top,
        left,
        width,
        maxHeight,
      });
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

  return createPortal(
    <div
      ref={popoverRef}
      className="fixed z-[1012] overflow-hidden rounded-xl border border-ink/10 bg-surface-panel shadow-[0_20px_60px_rgba(0,0,0,0.22)]"
      style={{
        top: pos.top,
        left: pos.left,
        width: pos.width,
      }}
    >
      <div className="flex items-center justify-between gap-3 border-b border-ink/8 px-3 py-2.5">
        <div className="min-w-0">
          <div className="text-body-sm font-medium text-ink/82">Edited files</div>
          <div className="text-caption text-ink/48">
            Select multiple files to add to canvas
          </div>
        </div>
        <button
          type="button"
          onClick={onClose}
          className="inline-flex h-7 w-7 items-center justify-center rounded-md text-ink/45 transition hover:bg-ink/5 hover:text-ink/72"
          aria-label="Close edited files picker"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
      <ScrollArea
        className="overscroll-contain"
        style={{ maxHeight: Math.min(pos.maxHeight, 320) }}
        viewportClassName="py-1"
        persistScrollbars
      >
        <ul className="flex flex-col px-1.5 py-1">
          {files.map((path) => {
            const checked = selectedFiles.includes(path);
            const fileName = path.split(/[/\\]/).pop() ?? path;
            return (
              <li key={path}>
                <button
                  type="button"
                  onClick={() => onToggleFile(path)}
                  className="flex w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-left transition hover:bg-ink/[0.05]"
                >
                  <span className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded border border-ink/15 bg-ink/[0.04]">
                    {checked && <Check className="h-3 w-3 text-ink/72" />}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-body-sm text-ink/82">{fileName}</span>
                    <span className="mt-0.5 block break-all font-mono text-caption text-ink/45">{path}</span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </ScrollArea>
      <div className="flex items-center justify-between gap-3 border-t border-ink/8 px-3 py-2.5">
        <div className="text-caption text-ink/48">
          {selectedFiles.length} selected
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={onClose}
            className="inline-flex items-center gap-1.5 rounded-md border border-ink/10 px-3 py-1.5 text-caption text-ink/58 transition hover:bg-ink/[0.05]"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onAdd}
            disabled={selectedFiles.length === 0}
            className="inline-flex items-center gap-1.5 rounded-md border border-ink/10 px-3 py-1.5 text-caption text-ink/72 transition hover:bg-ink/[0.05] disabled:cursor-not-allowed disabled:opacity-40"
          >
            <FilePlus2 className="h-3.5 w-3.5" />
            Add to canvas
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
