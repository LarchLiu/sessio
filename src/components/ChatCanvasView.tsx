import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { AlertCircle, LoaderCircle, RefreshCcw } from "lucide-react";
import type { CanvasDocumentState, CanvasKey } from "../canvasTypes";
import {
  getCanvas,
  type ThreadWorkSnapshot,
} from "../api";
import type { ChatComposerController } from "../hooks/useChatComposer";
import type { LiveRuntimeState } from "../runtimeChat";

const BlockSuiteCanvasHost = lazy(() => import("./blocksuite/BlockSuiteCanvasHost"));

export interface ChatCanvasViewProps {
  canvasKey: CanvasKey;
  sessionAgent: "pi" | "omp" | "codex" | "claude" | "opencode";
  sessionTitle: string;
  workspacePath: string | null;
  sessionThreadId?: string | null;
  editedFiles?: string[];
  autoAddedEditedFiles?: string[];
  latestEditedFiles?: string[];
  liveState: LiveRuntimeState;
  runtimeSessionAliases: Record<string, string>;
  fallbackWorkflowSnapshot?: ThreadWorkSnapshot | null;
  autoThreadIds?: string[];
  autoThreadRefreshKey?: string;
  selectedCanvasFileRequest?: {
    paths: string[];
    requestId: number;
  } | null;
  composer: ChatComposerController;
  onError: (message: string) => void;
  onOpenProjectFile?: (path: string) => void;
  onOpenThreadMultiSessionChat?: (threadId: string) => void;
}

export default function ChatCanvasView({
  canvasKey,
  sessionAgent,
  sessionTitle,
  workspacePath,
  sessionThreadId = null,
  editedFiles = [],
  autoAddedEditedFiles = [],
  latestEditedFiles = [],
  liveState,
  runtimeSessionAliases,
  fallbackWorkflowSnapshot = null,
  autoThreadIds = [],
  autoThreadRefreshKey = "",
  selectedCanvasFileRequest = null,
  composer,
  onError,
  onOpenProjectFile,
  onOpenThreadMultiSessionChat,
}: ChatCanvasViewProps) {
  const [state, setState] = useState<CanvasDocumentState | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  const canvasKeySignature = `${canvasKey.kind}:${canvasKey.id}`;

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    getCanvas(canvasKey)
      .then((next) => {
        if (cancelled) return;
        setState(next);
        setLoading(false);
      })
      .catch((error) => {
        if (cancelled) return;
        const message = String(error);
        setLoadError(message);
        setLoading(false);
        onError(message);
      });
    return () => {
      cancelled = true;
    };
  }, [canvasKey.kind, canvasKey.id, canvasKeySignature, onError, reloadKey]);

  const initialSnapshot = useMemo(() => {
    if (!state) return null;
    return state.draftSnapshot ?? state.savedSnapshot ?? null;
  }, [state]);

  if (loading) {
    return (
      <div className="flex flex-1 min-h-0 items-center justify-center text-ink/45">
        <div className="flex items-center gap-2 text-body-sm">
          <LoaderCircle className="h-4 w-4 animate-spin" />
          <span>Loading canvas…</span>
        </div>
      </div>
    );
  }

  if (loadError || !state) {
    return (
      <div className="m-4 rounded-2xl border border-status-error/20 bg-status-error/8 p-4 text-status-error">
        <div className="flex items-start gap-3">
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
          <div className="min-w-0">
            <div className="text-body-sm font-medium">Canvas failed to load</div>
            <div className="mt-1 text-caption opacity-90">{loadError ?? "Unknown error"}</div>
            <button
              type="button"
              onClick={() => setReloadKey((value) => value + 1)}
              className="mt-3 inline-flex items-center gap-2 rounded-full border border-status-error/25 px-3 py-1.5 text-caption transition hover:bg-status-error/10"
            >
              <RefreshCcw className="h-3.5 w-3.5" />
              Retry
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-1 min-h-0 flex-col">
      <div className="relative flex-1 min-h-0">
        <Suspense
          fallback={
            <div className="flex h-full items-center justify-center text-ink/45">
              <div className="flex items-center gap-2 text-body-sm">
                <LoaderCircle className="h-4 w-4 animate-spin" />
                <span>Loading canvas tools…</span>
              </div>
            </div>
          }
        >
          <BlockSuiteCanvasHost
            canvasKey={canvasKey}
            sessionAgent={sessionAgent}
            workspacePath={workspacePath}
            sessionThreadId={sessionThreadId}
            editedFiles={editedFiles}
            autoAddedEditedFiles={autoAddedEditedFiles}
            latestEditedFiles={latestEditedFiles}
            liveState={liveState}
            runtimeSessionAliases={runtimeSessionAliases}
            fallbackWorkflowSnapshot={fallbackWorkflowSnapshot}
            autoThreadIds={autoThreadIds}
            autoThreadRefreshKey={autoThreadRefreshKey}
            selectedFileRequest={selectedCanvasFileRequest}
            initialState={state}
            initialSnapshot={initialSnapshot}
            composer={composer}
            onOpenProjectFile={onOpenProjectFile}
            onOpenThreadMultiSessionChat={onOpenThreadMultiSessionChat}
            onStateLoaded={setState}
            onError={onError}
          />
        </Suspense>
      </div>
      <div className="sr-only" aria-hidden>
        {sessionTitle}
      </div>
    </div>
  );
}
