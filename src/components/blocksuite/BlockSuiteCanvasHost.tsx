import { useEffect, useRef, useState } from "react";
import { LoaderCircle, RefreshCcw, Save } from "lucide-react";
import type { CanvasDocumentState } from "../../canvasTypes";
import { saveCanvasDraft, saveCanvasRevision } from "../../api";
import type { Agent } from "../../api";
import type { ChatComposerController } from "../../hooks/useChatComposer";
import {
  createBlockSuiteDoc,
  createEdgelessEditorWithSpecs,
  ensureEdgelessRoot,
  exportDocSnapshot,
  importDocSnapshot,
} from "./bootstrap";

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
  const [saveError, setSaveError] = useState<string | null>(null);
  const [status, setStatus] = useState("Initializing BlockSuite canvas…");
  const [isReady, setIsReady] = useState(false);
  const [isSaving, setIsSaving] = useState(false);

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
    } catch (error) {
      const message = `Failed to restore the last saved revision: ${String(error)}`;
      setSaveError(message);
      onError(message);
    }
  };

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
      <div className="sr-only" aria-hidden>
        BlockSuite portal host reserved for React-backed block views.
      </div>
    </div>
  );
}
