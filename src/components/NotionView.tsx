import { useEffect } from "react";
import { MantineProvider } from "@mantine/core";
import { BlockNoteView } from "@blocknote/mantine";
import { useCreateBlockNote } from "@blocknote/react";
import "@blocknote/mantine/style.css";
import "./notion-theme.css";
import { useNotionDoc } from "../hooks/useNotionDoc";
import { useEffectiveThemeType } from "./shikiHighlight";
import { useI18n } from "../i18n";

export interface NotionViewProps {
  fileKey: string;
  text: string;
  workspacePath: string | null;
  path: string | null;
  mtimeMs: number | null;
  contentVersion: string;
  editingLocked?: boolean;
  onSaved: (content: string, mtimeMs: number) => void;
  onFlushHandleChange?: (handle: (() => Promise<boolean>) | null) => void;
}

export default function NotionView({
  fileKey,
  text,
  workspacePath,
  path,
  mtimeMs,
  contentVersion,
  editingLocked = false,
  onSaved,
  onFlushHandleChange,
}: NotionViewProps) {
  const themeType = useEffectiveThemeType();
  const { t } = useI18n();
  const editor = useCreateBlockNote();
  const {
    usedSourceFallback,
    editable,
    status,
    messageKey,
    messageDetail,
    hasPendingChanges,
    flushPendingSave,
  } = useNotionDoc(editor, {
    fileKey,
    text,
    workspacePath,
    path,
    mtimeMs,
    contentVersion,
    editingLocked,
    onSaved,
  });

  const lockMessage = editingLocked && !hasPendingChanges()
    ? "chat.files.editor_locked_agent"
    : null;
  const visibleMessageKey = lockMessage ?? messageKey;

  useEffect(() => {
    onFlushHandleChange?.(flushPendingSave);
    return () => onFlushHandleChange?.(null);
  }, [flushPendingSave, onFlushHandleChange]);

  return (
    <MantineProvider>
      <div
        className="sessio-notion-view flex h-full min-h-0 min-w-0 flex-col"
        data-theme-type={themeType}
        data-source-fallback={usedSourceFallback ? "true" : "false"}
      >
        {visibleMessageKey && (
          <div
            className={
              "mx-4 mt-3 rounded-md border px-3 py-2 text-body-sm " +
              (status === "error" || status === "conflict"
                ? "border-status-warn/30 bg-status-warn/[0.08] text-status-warn"
                : "border-ink/10 bg-ink/[0.04] text-ink/60")
            }
          >
            {t(visibleMessageKey)}
            {messageDetail && (
              <div className="mt-1 font-mono text-caption opacity-70">{messageDetail}</div>
            )}
          </div>
        )}
        <div className="min-h-0 flex-1 overflow-auto">
          <BlockNoteView
            editor={editor}
            editable={editable}
            theme={themeType}
            className="sessio-notion-editor"
          />
        </div>
      </div>
    </MantineProvider>
  );
}
