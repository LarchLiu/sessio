import { useEffect, useMemo, useRef, useState } from "react";
import { FilePlus2, LoaderCircle, RefreshCcw, Save } from "lucide-react";
import { Bound } from "@blocksuite/global/utils";
import type { CanvasDocumentState } from "../../canvasTypes";
import { readWorkspaceTextFile, saveCanvasDraft, saveCanvasRevision, updateCanvasBlocks } from "../../api";
import type { Agent } from "../../api";
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
import { markdownPreviewModelToCanvasBlock } from "../../lib/blocksuite/persistence";
import type { MarkdownPreviewBlockModel } from "../../lib/blocksuite/blocks/markdown-preview";
import { EdgelessRootService } from "@blocksuite/blocks";
import { setBlockSuitePortalBridge } from "../../lib/blocksuite/portalBridge";

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

const AUTOSAVE_DEBOUNCE_MS = 900;

function snapshotToJson(doc: ReturnType<typeof createBlockSuiteDoc>["doc"]) {
  const snapshot = exportDocSnapshot(doc);
  return snapshot ? JSON.stringify(snapshot) : null;
}

export default function BlockSuiteCanvasHost({
  sessionId,
  workspacePath,
  editedFiles = [],
  selectedFileRequest = null,
  initialState,
  initialSnapshot,
  onStateLoaded,
  onError,
}: BlockSuiteCanvasHostProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<HTMLElement | null>(null);
  const docRef = useRef<ReturnType<typeof createBlockSuiteDoc>["doc"] | null>(null);
  const blockUpdatedDisposeRef = useRef<{ dispose: () => void } | null>(null);
  const autosaveTimerRef = useRef<number | null>(null);
  const inflightSaveRef = useRef(false);
  const queuedSnapshotRef = useRef<string | null>(null);
  const currentSnapshotRef = useRef(initialSnapshot);
  const handledFileRequestRef = useRef<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [status, setStatus] = useState("Initializing BlockSuite canvas…");
  const [isReady, setIsReady] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [reactToLit, portals] = useReactToLitBridge();

  const changedFiles = useMemo(
    () => Array.from(new Set(editedFiles.map((path) => path.trim()).filter(Boolean))),
    [editedFiles],
  );

  useEffect(() => {
    setBlockSuitePortalBridge({
      reactToLit,
      workspacePath,
      updateBlock: (blockId, props) => {
        const doc = docRef.current;
        const model = doc?.getBlockById(blockId) ?? null;
        if (!doc || !model) return;
        doc.updateBlock(model, props);
      },
    });
    return () => {
      setBlockSuitePortalBridge(null);
    };
  }, [reactToLit, workspacePath]);

  const lastSavedRevision = initialState.savedRevision?.revision ?? null;

  const attachDoc = (
    host: HTMLDivElement,
    doc: ReturnType<typeof createBlockSuiteDoc>["doc"],
  ) => {
    ensureEdgelessRoot(doc);
    const editor = createEdgelessEditorWithSpecs(doc);
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
    });
  };

  const syncCanvasBlocks = async (doc: ReturnType<typeof createBlockSuiteDoc>["doc"]) => {
    const blocks = doc
      .getBlocks()
      .map((item) => item as MarkdownPreviewBlockModel)
      .filter((item) => item.flavour === "sessio:markdown-preview")
      .map(markdownPreviewModelToCanvasBlock);
    try {
      await updateCanvasBlocks({
        sessionId,
        blocks,
      });
    } catch (error) {
      const message = `Failed to sync canvas blocks: ${String(error)}`;
      setSaveError(message);
      onError(message);
    }
  };

  const flushSaveRef = useRef<(snapshotJson: string) => Promise<void>>(async () => {});

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
        onStateLoaded({
          ...initialState,
          document: saved.document,
          draftSnapshot: snapshotJson,
        });
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
    initialSnapshot,
    initialState.document.id,
    initialState.document.title,
    onError,
    onStateLoaded,
    sessionId,
  ]);

  const handleSaveRevision = async () => {
    const doc = docRef.current;
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
      onStateLoaded({
        ...initialState,
        document: saved.document,
        draftSnapshot: snapshotJson,
        savedRevision: saved.revision,
        savedSnapshot: snapshotJson,
      });
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
    if (!savedSnapshot) return;
    if (!hostRef.current) return;
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
      onStateLoaded({
        ...initialState,
        draftSnapshot: savedSnapshot,
      });
      await syncCanvasBlocks(restored);
    } catch (error) {
      const message = `Failed to restore the last saved revision: ${String(error)}`;
      setSaveError(message);
      onError(message);
    }
  };

  const addMarkdownPreviewBlocks = async (paths: string[]) => {
    const doc = docRef.current;
    const editor = editorRef.current;
    if (!doc || !editor || !workspacePath) return;
    const rootService = (editor as { std?: { getService?: (flavour: string) => EdgelessRootService | null } }).std?.getService?.("affine:page");
    if (!rootService) {
      onError("BlockSuite edgeless service is not ready yet.");
      return;
    }
    const uniquePaths = Array.from(new Set(paths.map((path) => path.trim()).filter(Boolean)));
    const center = rootService.viewport.center;

    for (const [index, path] of uniquePaths.entries()) {
      try {
        const absolutePath = resolveCanvasFilePath(path, workspacePath);
        if (!absolutePath) {
          throw new Error("file path is unavailable");
        }
        const file = await readWorkspaceTextFile(workspacePath, absolutePath);
        const title = absolutePath.split(/[/\\]/).pop() ?? absolutePath;
        const excerpt = file.content
          .split(/\r?\n/)
          .map((line) => line.trim())
          .filter(Boolean)
          .slice(0, 4)
          .join(" ")
          .slice(0, 280);
        const bound = Bound.fromCenter(
          [center.x + index * 32, center.y + index * 24],
          420,
          260,
        );
        rootService.addBlock("sessio:markdown-preview", {
          title,
          sourcePath: absolutePath,
          sourceType: changedFiles.includes(path) || changedFiles.includes(absolutePath)
            ? "edited_file"
            : "workspace_file",
          excerpt,
          renderMode: "summary",
          collapsed: false,
          contentVersion: `${absolutePath}:${file.mtimeMs}`,
          cachedContent: "",
          xywh: bound.serialize(),
        }, doc.getBlocksByFlavour("affine:surface")[0]?.model ?? undefined);
      } catch (error) {
        onError(`Failed to add markdown preview for ${path}: ${String(error)}`);
      }
    }

    await syncCanvasBlocks(doc);
  };

  useEffect(() => {
    const requestId = selectedFileRequest?.requestId ?? null;
    if (!selectedFileRequest || requestId === null) return;
    const requestKey = `${sessionId}:${requestId}`;
    if (handledFileRequestRef.current === requestKey) return;
    handledFileRequestRef.current = requestKey;
    void addMarkdownPreviewBlocks(selectedFileRequest.paths);
  }, [selectedFileRequest, sessionId, workspacePath]);

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col">
      <div className="flex h-9 shrink-0 items-center justify-between gap-3 border-b border-ink/8 bg-surface-panel/90 px-4">
        <div className="flex items-center gap-2 text-caption text-ink/58">
          {isReady ? (
            <span>{status}</span>
          ) : (
            <>
              <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
              <span>{status}</span>
            </>
          )}
        </div>
        <div className="flex items-center gap-2 text-caption">
          <button
            type="button"
            onClick={() => void addMarkdownPreviewBlocks(changedFiles)}
            disabled={!isReady || changedFiles.length === 0}
            className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-2.5 text-ink/65 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <FilePlus2 className="h-3.5 w-3.5" />
            Add edited files
          </button>
          <span className="inline-flex h-6 items-center rounded-md border border-ink/10 px-2.5 text-ink/55">
            BlockSuite
          </span>
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
            <Save className="h-3.5 w-3.5" />
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
      <PortalHost portals={portals} />
    </div>
  );
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
