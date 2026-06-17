import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type MouseEvent,
  type RefObject,
} from "react";
import { MantineProvider } from "@mantine/core";
import { BlockNoteView } from "@blocknote/mantine";
import { useCreateBlockNote } from "@blocknote/react";
import { Check, Copy, Save } from "lucide-react";
import "@blocknote/mantine/style.css";
import "./plain-editor-theme.css";
import { usePlainEditorDoc } from "../hooks/usePlainEditorDoc";
import { useEffectiveThemeType } from "./shikiHighlight";
import { useI18n } from "../i18n";
import ScrollArea from "./ScrollArea";
import Tooltip from "./Tooltip";
import {
  PlainEditorFormattingToolbarController,
  PlainEditorSideMenuController,
  PlainEditorSlashMenuController,
} from "./plainEditorFormatting";

export interface PlainEditorViewProps {
  fileKey: string;
  text: string;
  workspacePath: string | null;
  path: string | null;
  mtimeMs: number | null;
  contentVersion: string;
  editingLocked?: boolean;
  onSaved: (content: string, mtimeMs: number) => void;
  onPlainEditorLeaveCheckChange?: (handle: (() => Promise<boolean>) | null) => void;
}

export default function PlainEditorView({
  fileKey,
  text,
  workspacePath,
  path,
  mtimeMs,
  contentVersion,
  editingLocked = false,
  onSaved,
  onPlainEditorLeaveCheckChange,
}: PlainEditorViewProps) {
  const themeType = useEffectiveThemeType();
  const { t } = useI18n();
  const editor = useCreateBlockNote();
  const containerRef = useRef<HTMLDivElement>(null);
  const {
    usedSourceFallback,
    editable,
    status,
    messageKey,
    messageDetail,
    saveable,
    hasPendingChanges,
    saveNow,
    canLeaveDocument,
  } = usePlainEditorDoc(editor, {
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
  const canSave = saveable && status !== "saving" && hasPendingChanges();
  const codeCopyTarget = useCodeBlockCopyTarget(containerRef);

  useEffect(() => {
    onPlainEditorLeaveCheckChange?.(canLeaveDocument);
    return () => onPlainEditorLeaveCheckChange?.(null);
  }, [canLeaveDocument, onPlainEditorLeaveCheckChange]);

  const handleSave = useCallback(() => {
    void saveNow();
  }, [saveNow]);

  const handleKeyDownCapture = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void saveNow();
      }
    },
    [saveNow],
  );

  const handleContainerClick = useCallback(
    (event: MouseEvent<HTMLDivElement>) => {
      if (!editable) return;
      if (!(event.target instanceof HTMLElement)) return;
      if (shouldIgnoreContainerClick(event.target)) return;
      if (!containerRef.current?.contains(event.target)) return;

      const lastBlock = editor.document[editor.document.length - 1];
      if (!lastBlock) {
        editor.focus();
        return;
      }

      editor.focus();
      editor.setTextCursorPosition(lastBlock, "end");
    },
    [editable, editor],
  );

  const statusLabel = visibleMessageKey ? t(visibleMessageKey) : null;

  return (
    <MantineProvider>
      <div
        ref={containerRef}
        className="sessio-plain-editor-view flex h-full min-h-0 min-w-0 flex-col"
        data-theme-type={themeType}
        data-source-fallback={usedSourceFallback ? "true" : "false"}
        onClick={handleContainerClick}
        onFocus={codeCopyTarget.handleFocus}
        onKeyDownCapture={handleKeyDownCapture}
        onMouseMove={codeCopyTarget.handleMouseMove}
        onMouseLeave={codeCopyTarget.clearCopyTarget}
      >
        {(visibleMessageKey || hasPendingChanges()) && (
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
            {saveable && hasPendingChanges() && (
              <Tooltip content={t("chat.files.editor_save")} placement="bottom">
                <button
                  type="button"
                  aria-label={t("chat.files.editor_save")}
                  className="sessio-plain-editor-save-button"
                  disabled={!canSave}
                  onClick={handleSave}
                >
                  <Save aria-hidden="true" className="h-3.5 w-3.5" />
                </button>
              </Tooltip>
            )}
          </div>
        )}
        <ScrollArea
          className="min-h-0 flex-1"
          viewportClassName="sessio-plain-editor-scroll-viewport"
          onScroll={codeCopyTarget.clearCopyTarget}
        >
          <BlockNoteView
            editor={editor}
            editable={editable}
            theme={themeType}
            className="sessio-plain-editor-surface"
            formattingToolbar={false}
            slashMenu={false}
            sideMenu={false}
            filePanel={false}
            comments={false}
          >
            <PlainEditorFormattingToolbarController />
            <PlainEditorSlashMenuController />
            <PlainEditorSideMenuController />
          </BlockNoteView>
        </ScrollArea>
        {codeCopyTarget.copyTarget && (
          <CodeBlockCopyButton
            copyTarget={codeCopyTarget.copyTarget}
            label={t("chat.files.editor_copy_code")}
            copiedLabel={t("chat.files.editor_copied")}
          />
        )}
      </div>
    </MantineProvider>
  );
}

const CONTAINER_CLICK_IGNORE_SELECTOR = [
  '[contenteditable="true"]',
  "button",
  "input",
  "select",
  "textarea",
  ".bn-formatting-toolbar",
  ".bn-link-toolbar",
  ".bn-panel",
  ".bn-side-menu",
  ".bn-suggestion-menu",
  ".bn-grid-suggestion-menu",
  ".bn-form-popover",
  ".sessio-plain-editor-code-copy",
  '[role="menu"]',
  '[role="dialog"]',
].join(", ");

type CodeBlockCopyTarget = {
  codeBlock: HTMLElement;
  left: number;
  top: number;
};

function shouldIgnoreContainerClick(target: HTMLElement) {
  return Boolean(target.closest(CONTAINER_CLICK_IGNORE_SELECTOR));
}

function codeBlockText(codeBlock: HTMLElement) {
  const code = codeBlock.querySelector("code");
  return (code ?? codeBlock).textContent ?? "";
}

function sameCopyTarget(left: CodeBlockCopyTarget | null, right: CodeBlockCopyTarget) {
  return Boolean(
    left &&
      left.codeBlock === right.codeBlock &&
      left.left === right.left &&
      left.top === right.top,
  );
}

function codeBlockCopyTarget(codeBlock: HTMLElement, container: HTMLElement): CodeBlockCopyTarget {
  const codeBlockRect = codeBlock.getBoundingClientRect();
  const containerRect = container.getBoundingClientRect();
  return {
    codeBlock,
    left: codeBlockRect.right - containerRect.left - 34,
    top: codeBlockRect.top - containerRect.top + 8,
  };
}

function useCodeBlockCopyTarget(containerRef: RefObject<HTMLDivElement | null>) {
  const [copyTarget, setCopyTarget] = useState<CodeBlockCopyTarget | null>(null);

  const showCopyTarget = useCallback(
    (codeBlock: HTMLElement) => {
      const container = containerRef.current;
      if (!container || !container.contains(codeBlock)) return;
      const nextTarget = codeBlockCopyTarget(codeBlock, container);
      setCopyTarget((previous) => (sameCopyTarget(previous, nextTarget) ? previous : nextTarget));
    },
    [containerRef],
  );

  const updateFromEventTarget = useCallback(
    (target: EventTarget | null) => {
      const container = containerRef.current;
      if (!(target instanceof HTMLElement) || !container) return;
      if (target.closest(".sessio-plain-editor-code-copy")) return;
      const codeBlock = target.closest<HTMLElement>('[data-content-type="codeBlock"]');
      if (codeBlock && container.contains(codeBlock)) {
        showCopyTarget(codeBlock);
        return;
      }
      setCopyTarget(null);
    },
    [containerRef, showCopyTarget],
  );

  return useMemo(
    () => ({
      clearCopyTarget: () => setCopyTarget(null),
      copyTarget,
      handleFocus: (event: FocusEvent<HTMLDivElement>) => updateFromEventTarget(event.target),
      handleMouseMove: (event: MouseEvent<HTMLDivElement>) => updateFromEventTarget(event.target),
    }),
    [copyTarget, updateFromEventTarget],
  );
}

function CodeBlockCopyButton({
  copiedLabel,
  copyTarget,
  label,
}: {
  copiedLabel: string;
  copyTarget: CodeBlockCopyTarget;
  label: string;
}) {
  const [copied, setCopied] = useState(false);
  const resetTimerRef = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (resetTimerRef.current !== null) {
        window.clearTimeout(resetTimerRef.current);
      }
    },
    [],
  );

  const handleCopy = useCallback(
    (event: MouseEvent<HTMLButtonElement>) => {
      event.preventDefault();
      event.stopPropagation();
      void navigator.clipboard
        .writeText(codeBlockText(copyTarget.codeBlock))
        .then(() => {
          setCopied(true);
          if (resetTimerRef.current !== null) {
            window.clearTimeout(resetTimerRef.current);
          }
          resetTimerRef.current = window.setTimeout(() => {
            setCopied(false);
            resetTimerRef.current = null;
          }, 1200);
        })
        .catch((error) => {
          console.warn("[plain-editor] Failed to copy code block:", error);
        });
    },
    [copyTarget],
  );

  return (
    <div
      className="sessio-plain-editor-code-copy"
      contentEditable={false}
      style={{ left: copyTarget.left, top: copyTarget.top }}
    >
      <Tooltip content={copied ? copiedLabel : label} placement="left">
        <button
          type="button"
          aria-label={label}
          className="sessio-plain-editor-code-copy-button"
          onClick={handleCopy}
          onMouseDown={(event) => {
            event.preventDefault();
            event.stopPropagation();
          }}
        >
          {copied ? (
            <Check aria-hidden="true" className="h-3.5 w-3.5" />
          ) : (
            <Copy aria-hidden="true" className="h-3.5 w-3.5" />
          )}
        </button>
      </Tooltip>
    </div>
  );
}
