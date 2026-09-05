import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch } from "react";
import { ArrowLeft, FileWarning, LoaderCircle } from "lucide-react";
import {
  cancelAgentTurn,
  ensureAgentRuntimeSession,
  getSessionHistory,
  linkSessioAppSession,
  listSessioAppSessions,
  readLocalTextFile,
  respondAgentPermission,
  sendAgentInput,
  setComputerUseSessionApproval,
  type RuntimeAgentMetadata,
  type RuntimeAgentSelection,
  type SessionInfo,
  type SetRuntimeAgentSelectionRequest,
  type SessioAppInfo,
} from "../api";
import { useAppshotComposerRegistration } from "../appshot";
import { mergeAppHistoryTurns } from "../appChatDrawer";
import ChatTranscriptDrawer from "../components/ChatTranscriptDrawer";
import {
  FilePreviewNotice,
  FilePreviewOverlay,
  ImagePreviewOverlay,
} from "../components/AcpTranscriptPanel";
import type { FilePreview, MarkdownImage } from "../components/AcpTranscriptPanel";
import {
  FileDisplayModeToggle,
  type ChatFilesDisplayMode,
} from "../components/ChatFilesView";
import {
  ComposerTopAttachments,
  EditedFilesBar,
  MinimalMessageStrip,
} from "../components/ChatBottomStrips";
import ChatComposer from "../components/ChatComposer";
import FileViewer from "../components/FileViewer";
import PlainHtmlPreview from "../components/PlainHtmlPreview";
import PlainMarkdownPreview from "../components/PlainMarkdownPreview";
import {
  isPlainEditorEditableDocumentPath,
  isPlainEditorMarkdownDocumentPath,
  isPlainEditorPreviewableDocumentPath,
} from "../hooks/plainEditorFileTypes";
import { languageFromPath, useFileContent } from "../hooks/useFileContent";
import { runtimeSessionOptions, useChatComposer } from "../hooks/useChatComposer";
import { useI18n } from "../i18n";
import {
  dispatchSessionStartedFallback,
  historyTurnsToAcpViewModel,
  liveSessionToAcpViewModel,
  type AcpPermissionRequest,
  type LiveRuntimeAction,
  type LiveRuntimeState,
  type LiveTurn,
} from "../runtimeChat";
import { mergeHistoryAndLiveViewModels } from "../components/AcpTranscriptPanel";
import { liveOrLatestTurnFileEdits } from "../acpRenderItems";

export default function AppsPage({
  app,
  appDisplayName,
  runtimeSessionId,
  onRuntimeSessionIdChange,
  runtimeAgents,
  lastRuntimeAgentSelection,
  rememberRuntimeAgentSelection,
  liveState,
  dispatchLiveEvent,
  onError,
  selectedFilePath,
  onClearFileSelection,
  onOpenFile,
}: {
  app: SessioAppInfo;
  appDisplayName: string;
  runtimeSessionId: string | null;
  onRuntimeSessionIdChange: (runtimeSessionId: string) => void;
  runtimeAgents: RuntimeAgentMetadata[];
  lastRuntimeAgentSelection: RuntimeAgentSelection | null;
  rememberRuntimeAgentSelection: (selection: SetRuntimeAgentSelectionRequest) => Promise<void>;
  liveState: LiveRuntimeState;
  dispatchLiveEvent: Dispatch<LiveRuntimeAction>;
  onError: (error: string | null) => void;
  selectedFilePath: string | null;
  onClearFileSelection: () => void;
  onOpenFile: (path: string) => void;
}) {
  const { t } = useI18n();
  const [html, setHtml] = useState<string | null>(null);
  const [loadingHtml, setLoadingHtml] = useState(false);
  const [continuationSending, setContinuationSending] = useState(false);
  const [resolvingPermissionId, setResolvingPermissionId] = useState<string | null>(null);
  const [chatExpanded, setChatExpanded] = useState(false);
  const [linkedSessions, setLinkedSessions] = useState<SessionInfo[]>([]);
  const [historyTurns, setHistoryTurns] = useState<LiveTurn[]>([]);
  const [loadingHistory, setLoadingHistory] = useState(true);
  const [previewRevision, setPreviewRevision] = useState(0);
  const [previewImage, setPreviewImage] = useState<MarkdownImage | null>(null);
  const [previewFile, setPreviewFile] = useState<FilePreview | null>(null);
  const [filePreviewNotice, setFilePreviewNotice] = useState<string | null>(null);
  const linkedSessionKeysRef = useRef(new Set<string>());
  const reloadedTurnKeysRef = useRef(new Set<string>());
  const fallbackRuntimeSequenceRef = useRef(0);
  const appliedLinkedSessionRef = useRef<string | null>(null);
  const composer = useChatComposer({
    runtimeAgents,
    lastRuntimeAgentSelection,
    rememberRuntimeAgentSelection,
    liveState,
    dispatchLiveEvent,
    onError,
    onPendingSession: (pending) => {
      onRuntimeSessionIdChange(pending.sessioRuntimeSessionId);
    },
    onPreviewImageAttachment: setPreviewImage,
  });
  useAppshotComposerRegistration(composer, true);

  const loadHtml = useCallback(async () => {
    if (!app.htmlPath) {
      setHtml(null);
      return;
    }
    setLoadingHtml(true);
    try {
      const source = await readLocalTextFile(app.htmlPath);
      setHtml(source);
      setPreviewRevision((current) => current + 1);
      onError(null);
    } catch (error) {
      setHtml(null);
      onError(String(error));
    } finally {
      setLoadingHtml(false);
    }
  }, [app.htmlPath, onError]);

  useEffect(() => {
    void loadHtml();
  }, [loadHtml]);

  useEffect(() => {
    let cancelled = false;
    setLinkedSessions([]);
    setHistoryTurns([]);
    setLoadingHistory(true);
    listSessioAppSessions(app.id)
      .then(async (sessions) => {
        const readable = sessions.filter((session) => session.available && session.filePath);
        const histories = await Promise.all(
          readable.map((session) =>
            getSessionHistory(session.agent, session.filePath, session.id),
          ),
        );
        if (cancelled) return;
        setLinkedSessions(sessions);
        setHistoryTurns(mergeAppHistoryTurns(histories.map((history) => history.turns as LiveTurn[])));
      })
      .catch((error) => {
        if (cancelled) return;
        onError(String(error));
      })
      .finally(() => {
        if (!cancelled) setLoadingHistory(false);
      });
    return () => {
      cancelled = true;
    };
  }, [app.id, onError]);

  const liveSession = runtimeSessionId
    ? liveState.sessions[runtimeSessionId] ?? null
    : null;
  const activeTurn = useMemo(
    () =>
      liveSession?.turns.find(
        (turn) =>
          turn.status === "pending" ||
          turn.status === "streaming" ||
          turn.status === "cancelling",
      ) ?? null,
    [liveSession],
  );
  const liveViewModel = useMemo(
    () => (liveSession ? liveSessionToAcpViewModel(liveSession) : null),
    [liveSession],
  );
  const historyViewModel = useMemo(
    () => historyTurnsToAcpViewModel(historyTurns),
    [historyTurns],
  );
  const viewModel = useMemo(() => {
    if (liveViewModel) {
      return mergeHistoryAndLiveViewModels(historyViewModel, liveViewModel);
    }
    return historyTurns.length > 0 ? historyViewModel : null;
  }, [historyTurns.length, historyViewModel, liveViewModel]);
  const currentTurnFileEdits = useMemo(
    () => liveOrLatestTurnFileEdits(
      viewModel ?? historyViewModel,
      new Set(liveSession?.turns.map((turn) => turn.turnId) ?? []),
    ),
    [historyViewModel, liveSession, viewModel],
  );
  const latestLinkedSession = useMemo(
    () => linkedSessions.find((session) => session.available && session.filePath) ?? null,
    [linkedSessions],
  );
  const drawerRuntimeSessionId = runtimeSessionId ?? latestLinkedSession?.id ?? null;

  useEffect(() => {
    if (!latestLinkedSession) return;
    const key = `${latestLinkedSession.agent}:${latestLinkedSession.id}`;
    if (appliedLinkedSessionRef.current === key) return;
    appliedLinkedSessionRef.current = key;
    composer.applyAgentSelection({ agent: latestLinkedSession.agent });
  }, [latestLinkedSession?.agent, latestLinkedSession?.id]);
  const pendingPermissions = useMemo(
    () =>
      liveSession?.turns.flatMap((turn) =>
        turn.permissions.filter(
          (permission) =>
            permission.options.length > 0 &&
            !permission.selectedOptionId &&
            !permission.cancelled,
        ),
      ) ?? [],
    [liveSession],
  );

  useEffect(() => {
    const agentSessionId = liveSession?.agentRuntimeSessionId?.trim() ?? "";
    if (
      !liveSession ||
      !agentSessionId ||
      agentSessionId === "pending" ||
      agentSessionId.startsWith("fake-agent-session")
    ) {
      return;
    }
    const key = `${app.id}:${liveSession.agent}:${agentSessionId}`;
    if (linkedSessionKeysRef.current.has(key)) return;
    linkedSessionKeysRef.current.add(key);
    linkSessioAppSession(app.id, liveSession.agent, agentSessionId).catch((error) => {
      linkedSessionKeysRef.current.delete(key);
      onError(String(error));
    });
  }, [app.id, liveSession, onError]);

  useEffect(() => {
    const latestTurn = liveSession?.turns.at(-1);
    if (
      !latestTurn ||
      (latestTurn.status !== "completed" &&
        latestTurn.status !== "failed" &&
        latestTurn.status !== "cancelled")
    ) {
      return;
    }
    const key = `${runtimeSessionId}:${latestTurn.turnId}:${latestTurn.status}`;
    if (reloadedTurnKeysRef.current.has(key)) return;
    reloadedTurnKeysRef.current.add(key);
    void loadHtml();
  }, [liveSession, loadHtml, runtimeSessionId]);

  const sendMessage = async () => {
    if (
      !composer.canSendWithWorkspace(app.directoryPath) ||
      activeTurn ||
      continuationSending ||
      loadingHistory
    ) {
      return;
    }
    if (runtimeSessionId && liveSession && !liveSession.ended) {
      setContinuationSending(true);
      try {
        await sendAgentInput(runtimeSessionId, {
          text: composer.text.trim(),
          attachments: composer.attachments.map(({ path, mimeType, kind, displayName }) => ({
            path,
            mimeType,
            kind,
            displayName,
          })),
        });
        composer.setText("");
        for (const attachment of composer.attachments) {
          composer.removeAttachment(attachment.path);
        }
        onError(null);
      } catch (error) {
        onError(String(error));
      } finally {
        setContinuationSending(false);
      }
      return;
    }
    if (latestLinkedSession && composer.selectedAgent === latestLinkedSession.agent) {
      setContinuationSending(true);
      try {
        const sessionOptions = runtimeSessionOptions(
          composer.selectedModel,
          composer.permissionMode,
          composer.selectedEffort,
          composer.selectedSkills.map((skill) => skill.id),
          composer.selectedMcpIds,
        );
        const handle = await ensureAgentRuntimeSession({
          agent: latestLinkedSession.agent,
          sessioRuntimeSessionId: latestLinkedSession.id,
          workspacePath: app.directoryPath,
          agentRuntimeSessionId: latestLinkedSession.id,
          sourceAgent: latestLinkedSession.agent,
          options: sessionOptions,
        });
        if (composer.computerUseEnabled) {
          await setComputerUseSessionApproval(handle.sessioRuntimeSessionId, true);
        }
        dispatchSessionStartedFallback({
          dispatch: dispatchLiveEvent,
          handle,
          liveState,
          sequenceRef: fallbackRuntimeSequenceRef,
          timestamp: Date.now(),
          metadata: sessionOptions,
        });
        onRuntimeSessionIdChange(handle.sessioRuntimeSessionId);
        await sendAgentInput(handle.sessioRuntimeSessionId, {
          text: composer.text.trim(),
          attachments: composer.attachments.map(({ path, mimeType, kind, displayName }) => ({
            path,
            mimeType,
            kind,
            displayName,
          })),
        });
        composer.setText("");
        for (const attachment of composer.attachments) {
          composer.removeAttachment(attachment.path);
        }
        onError(null);
      } catch (error) {
        onError(String(error));
      } finally {
        setContinuationSending(false);
      }
      return;
    }
    await composer.runStartSession(composer.text, {
      workspacePath: app.directoryPath,
      projectName: appDisplayName,
    });
  };

  const cancelTurn = async () => {
    if (!runtimeSessionId || !activeTurn) return;
    try {
      await cancelAgentTurn(runtimeSessionId, activeTurn.turnId);
    } catch (error) {
      onError(String(error));
    }
  };

  const resolvePermission = async (permission: AcpPermissionRequest, optionId: string) => {
    if (!runtimeSessionId || resolvingPermissionId) return;
    setResolvingPermissionId(permission.requestId);
    try {
      await respondAgentPermission(runtimeSessionId, permission.requestId, optionId);
      onError(null);
    } catch (error) {
      onError(String(error));
    } finally {
      setResolvingPermissionId(null);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface-panel">
      <div className="flex min-h-0 flex-1 flex-col">
        {selectedFilePath ? (
          <AppFilePreview
            path={selectedFilePath}
            appDirectoryPath={app.directoryPath}
            onBack={onClearFileSelection}
          />
        ) : loadingHtml ? (
          <div className="flex min-h-0 flex-1 items-center justify-center text-ink/40">
            <LoaderCircle className="h-5 w-5 animate-spin" />
          </div>
        ) : app.htmlPath && html !== null ? (
          <PlainHtmlPreview
            key={`${app.htmlPath}:${previewRevision}`}
            html={html}
            filePath={app.htmlPath}
            scriptsInitiallyEnabled
            showScriptsControl={false}
          />
        ) : (
          <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-ink/45">
            <FileWarning className="h-6 w-6" />
            <p className="text-body-sm">{t("apps.no_html")}</p>
          </div>
        )}
        <ComposerTopAttachments>
          {!chatExpanded &&
            pendingPermissions.map((permission) => (
              <div
                key={permission.requestId}
                className="flex min-h-8 items-center gap-2 px-3 py-1 text-caption text-ink/65"
              >
                <span className="min-w-0 flex-1 truncate">{permission.toolName}</span>
                <div className="flex shrink-0 items-center gap-1">
                  {permission.options.map((option) => (
                    <button
                      key={option.optionId}
                      type="button"
                      disabled={Boolean(resolvingPermissionId)}
                      onClick={() => void resolvePermission(permission, option.optionId)}
                      className="rounded px-2 py-0.5 font-medium text-ink/70 transition hover:bg-ink/8 hover:text-ink disabled:cursor-not-allowed disabled:opacity-45"
                    >
                      {option.name}
                    </button>
                  ))}
                </div>
              </div>
            ))}
          {chatExpanded && viewModel && drawerRuntimeSessionId ? (
            <ChatTranscriptDrawer
              appId={app.id}
              viewModel={viewModel}
              runtimeSessionId={drawerRuntimeSessionId}
              liveTurnIds={liveSession?.turns.map((turn) => turn.turnId) ?? []}
              workingTurnId={activeTurn?.turnId ?? null}
              onCollapse={() => setChatExpanded(false)}
              onError={onError}
              onPreviewImage={setPreviewImage}
              onPreviewFile={setPreviewFile}
              onFilePreviewError={setFilePreviewNotice}
              onOpenProjectFile={onOpenFile}
            />
          ) : (
            <MinimalMessageStrip
              viewModel={viewModel}
              workingTurnId={activeTurn?.turnId ?? null}
              emptyText={loadingHistory ? t("apps.chat_loading") : t("apps.chat_empty")}
              ariaLabel={t("apps.chat_expand")}
              onClick={
                viewModel && drawerRuntimeSessionId
                  ? () => setChatExpanded(true)
                  : undefined
              }
            />
          )}
          <EditedFilesBar
            fileCount={currentTurnFileEdits.edits.length}
            additions={currentTurnFileEdits.additions}
            deletions={currentTurnFileEdits.deletions}
            edits={currentTurnFileEdits.edits}
            onOpenFile={onOpenFile}
          />
        </ComposerTopAttachments>
        <ChatComposer
          composer={composer}
          variant="chat"
          className="shrink-0 bg-gradient-to-t from-surface-panel via-surface-panel to-surface-panel/80 px-10 pb-4"
          placeholder={t("apps.chat_placeholder")}
          runtimeControlsDisabled={Boolean(liveSession && !liveSession.ended)}
          canSend={
            composer.canSendWithWorkspace(app.directoryPath) &&
            !activeTurn &&
            !continuationSending &&
            !loadingHistory
          }
          active={Boolean(activeTurn)}
          sendButtonBusy={composer.sending || continuationSending}
          onCancel={() => void cancelTurn()}
          onSend={() => void sendMessage()}
        />
      </div>
      {previewImage && (
        <ImagePreviewOverlay image={previewImage} onClose={() => setPreviewImage(null)} />
      )}
      {previewFile && (
        <FilePreviewOverlay file={previewFile} onClose={() => setPreviewFile(null)} />
      )}
      {filePreviewNotice && (
        <FilePreviewNotice
          message={filePreviewNotice}
          onClose={() => setFilePreviewNotice(null)}
        />
      )}
    </div>
  );
}

function AppFilePreview({
  path,
  appDirectoryPath,
  onBack,
}: {
  path: string;
  appDirectoryPath: string;
  onBack: () => void;
}) {
  const { t } = useI18n();
  const fileContent = useFileContent({ path, displayPath: path }, appDirectoryPath);
  const [displayMode, setDisplayMode] = useState<ChatFilesDisplayMode>("code");
  const selectedPath = fileContent.path ?? path;
  const documentFile = isPlainEditorEditableDocumentPath(selectedPath);
  const previewDocument = isPlainEditorPreviewableDocumentPath(selectedPath);
  const isMarkdown = isPlainEditorMarkdownDocumentPath(selectedPath);
  const fileName = path.split(/[\\/]/).pop() || path;

  useEffect(() => {
    setDisplayMode("code");
  }, [path]);

  return (
    <div className="sessio-plain-editor-view flex h-full min-h-0 min-w-0 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-ink/10 px-4">
        <button
          type="button"
          aria-label={t("apps.file_preview_back")}
          title={t("apps.file_preview_back")}
          onClick={onBack}
          className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-ink/55 transition hover:bg-ink/5 hover:text-ink"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
        </button>
        <span className="min-w-0 flex-1 truncate font-mono text-caption text-ink/72" title={path}>
          {fileName}
        </span>
        {documentFile && (
          <FileDisplayModeToggle
            value={displayMode}
            previewAvailable={previewDocument}
            onChange={setDisplayMode}
          />
        )}
      </div>
      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
        {fileContent.loading && (
          <div className="absolute inset-0 z-10 flex items-center justify-center text-ink/40">
            <LoaderCircle className="h-5 w-5 animate-spin" />
          </div>
        )}
        {!fileContent.loading && fileContent.error && (
          <div className="flex h-full items-center justify-center px-6 text-center text-body-sm text-status-warn">
            {t("chat.files.unavailable")}
            <span className="ml-1 font-mono text-caption opacity-70">{fileContent.error}</span>
          </div>
        )}
        {!fileContent.loading && !fileContent.error && fileContent.imageDataUrl && (
          <div className="flex h-full items-center justify-center overflow-auto p-8">
            <img src={fileContent.imageDataUrl} alt={fileName} className="max-h-full max-w-full object-contain" />
          </div>
        )}
        {!fileContent.loading && !fileContent.error && fileContent.text !== null && (
          displayMode === "preview" && previewDocument ? (
            isMarkdown ? (
              <PlainMarkdownPreview text={fileContent.text} filePath={selectedPath} />
            ) : (
              <PlainHtmlPreview
                html={fileContent.text}
                filePath={selectedPath}
                scriptsInitiallyEnabled
              />
            )
          ) : (
            <FileViewer
              fileKey={`${selectedPath}:${fileContent.contentVersion}`}
              text={fileContent.text}
              language={languageFromPath(selectedPath)}
              mode={displayMode === "edit" ? "plain" : "code"}
              workspacePath={appDirectoryPath}
              path={selectedPath}
              mtimeMs={fileContent.mtimeMs}
              contentVersion={fileContent.contentVersion}
              plainEditorMode="edit"
              onSaved={fileContent.applyLocalSave}
            />
          )
        )}
      </div>
    </div>
  );
}
