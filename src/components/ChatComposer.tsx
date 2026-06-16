import {
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  ArrowUp,
  ChevronDown,
  LoaderCircle,
  Plus,
  Sparkles,
  type LucideIcon,
} from "lucide-react";
import {
  attachmentMenuOptions,
  ComposerAttachmentMenu,
} from "./ComposerAttachments";
import { RuntimeMenuSelect } from "./RuntimeMenuSelect";
import Tooltip from "./Tooltip";
import {
  createImeCompositionState,
  getImeKeyboardDisposition,
  markImeCompositionEnd,
  markImeCompositionStart,
} from "./imeInput";
import { useI18n } from "../i18n";
import type { ChatComposerController } from "../hooks/useChatComposer";

const PROJECT_NAME_SCRAMBLE_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

export default function ChatComposer({
  composer,
  title,
  variant = "default",
  modeActions,
  sendActions,
  setupPanel,
  placeholder,
  sendButtonVariant = "chat",
  sendButtonLabel,
  sendButtonBusy,
  bottomRow,
  runtimeControlsDisabled = false,
  canSend,
  onSend,
  onTextareaKeyDown,
}: {
  composer: ChatComposerController;
  title?: ReactNode;
  variant?: "default" | "chat";
  modeActions?: ReactNode;
  sendActions?: ReactNode;
  setupPanel?: ReactNode;
  placeholder?: string;
  sendButtonVariant?: "chat" | "astra";
  sendButtonLabel?: string;
  sendButtonBusy?: boolean;
  bottomRow?: ReactNode;
  runtimeControlsDisabled?: boolean;
  canSend?: boolean;
  onSend: () => void;
  onTextareaKeyDown?: (event: import("react").KeyboardEvent<HTMLTextAreaElement>) => boolean;
}) {
  const { t } = useI18n();
  const [astraSweep, setAstraSweep] = useState(false);
  const previousSendButtonVariantRef = useRef(sendButtonVariant);
  const imeCompositionRef = useRef(createImeCompositionState());
  const sendEnabled = canSend ?? composer.canSend;
  const sendBusy = sendButtonBusy ?? composer.sending;
  const sendLabel =
    sendButtonLabel ?? (sendBusy ? t("new_chat.sending") : t("new_chat.send"));
  const attachmentOptions = attachmentMenuOptions({
    supportsImageAttachments: composer.supportsImageAttachments,
    supportsEmbeddedContext: composer.supportsEmbeddedContext,
    imageLabel: t("new_chat.add_images"),
    fileLabel: t("new_chat.add_files"),
  });
  const outerClassName = variant === "chat" ? "w-full" : "w-full max-w-[730px]";
  const controlsClassName =
    "flex h-12 items-center justify-between gap-3 px-3 pb-2 " +
    (bottomRow ? "border-b border-ink/10" : "");

  useEffect(() => {
    const previous = previousSendButtonVariantRef.current;
    previousSendButtonVariantRef.current = sendButtonVariant;
    if (previous === "astra" || sendButtonVariant !== "astra") return;
    setAstraSweep(false);
    const frame = window.requestAnimationFrame(() => setAstraSweep(true));
    const timeout = window.setTimeout(() => setAstraSweep(false), 720);
    return () => {
      window.cancelAnimationFrame(frame);
      window.clearTimeout(timeout);
    };
  }, [sendButtonVariant]);

  useEffect(() => {
    if (runtimeControlsDisabled && composer.attachmentMenuOpen) {
      composer.setAttachmentMenuOpen(false);
    }
  }, [composer, runtimeControlsDisabled]);

  return (
    <div className={outerClassName}>
      {title && (
        <h1 className="mb-11 text-center text-[28px] font-medium leading-tight tracking-normal text-ink/92">
          {title}
        </h1>
      )}
      {composer.composerError && (
        <div className="mb-2 rounded-md border border-status-error/25 bg-status-error/10 px-3 py-2 text-body-sm text-status-error">
          {composer.composerError}
        </div>
      )}
      <div
        className={
          "overflow-hidden rounded-2xl bg-ink/[0.055] shadow-[inset_0_0_0_1px_rgb(var(--color-ink)/0.08)] transition-shadow " +
          (composer.composerError
            ? "shadow-[inset_0_0_0_1px_rgb(var(--color-status-error)/0.35)]"
            : "focus-within:shadow-[inset_0_0_0_1px_rgb(var(--color-ink)/0.20)]")
        }
      >
        {composer.attachmentPreview}
        <textarea
          ref={composer.textareaRef}
          value={composer.text}
          placeholder={placeholder ?? t("new_chat.placeholder")}
          rows={2}
          onChange={(event) => {
            resizeTextareaToContent(event.currentTarget);
            composer.setText(event.target.value);
          }}
          onInput={(event) => resizeTextareaToContent(event.currentTarget)}
          onPaste={(event) => {
            if (!composer.pasteAttachments(event.clipboardData)) return;
            event.preventDefault();
          }}
          onCompositionStart={() => markImeCompositionStart(imeCompositionRef.current)}
          onCompositionEnd={() => markImeCompositionEnd(imeCompositionRef.current)}
          onKeyDown={(event) => {
            const imeDisposition = getImeKeyboardDisposition(event, imeCompositionRef.current);
            if (imeDisposition.shouldSkipShortcut) {
              if (imeDisposition.shouldPreventDefault) event.preventDefault();
              return;
            }
            if (onTextareaKeyDown?.(event)) return;
            if (event.key !== "Enter" || event.shiftKey) {
              return;
            }
            event.preventDefault();
            if (sendEnabled) onSend();
          }}
          className="chat-composer-textarea block w-full resize-none bg-transparent px-3.5 py-3.5 text-body leading-5 text-ink/88 placeholder:text-ink/38 outline-none"
        />
        {setupPanel && (
          <div className="border-t border-ink/5 px-3 py-2">
            {setupPanel}
          </div>
        )}
        <div className={controlsClassName}>
          <div className="flex min-w-0 items-center gap-3">
            {composer.supportsAttachments && (
              <Tooltip content={t("new_chat.add_context")} placement="top">
                <button
                  ref={composer.attachmentButtonRef}
                  type="button"
                  disabled={runtimeControlsDisabled}
                  onClick={() => composer.setAttachmentMenuOpen((open) => !open)}
                  className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-ink/55 transition hover:bg-ink/8 hover:text-ink disabled:cursor-not-allowed disabled:text-ink/28 disabled:hover:bg-transparent disabled:hover:text-ink/28"
                  aria-label={t("new_chat.add_context")}
                  aria-expanded={composer.attachmentMenuOpen}
                  aria-haspopup="menu"
                >
                  <Plus className="h-5 w-5" />
                </button>
              </Tooltip>
            )}
            {composer.permissionOptions.length > 0 && (
              <RuntimeMenuSelect
                ariaLabel="Default permissions"
                value={composer.permissionMode}
                onChange={(value) => void composer.handlePermissionModeChange(value)}
                disabled={runtimeControlsDisabled || !composer.selectedRuntimeAgent}
                options={composer.permissionOptions}
              />
            )}
            {modeActions}
          </div>
          <div className="flex shrink-0 items-center gap-2.5">
            <RuntimeMenuSelect
              ariaLabel={t("new_chat.agent")}
              value={composer.selectedAgentModelValue}
              onChange={(value) => void composer.handleAgentModelChange(value)}
              disabled={runtimeControlsDisabled || composer.agentModelOptions.length === 0}
              options={composer.agentModelOptions}
            />
            <Tooltip content={sendLabel} placement="top">
              <button
                type="button"
                disabled={!sendEnabled}
                onClick={onSend}
                className={
                  sendButtonVariant === "astra"
                    ? "astra-send-button relative flex h-7 w-7 items-center justify-center overflow-hidden rounded-full " +
                      (astraSweep ? "astra-send-button-sweep" : "")
                    : "flex h-7 w-7 items-center justify-center rounded-full bg-ink/70 text-[rgb(var(--color-bg-panel))] transition hover:bg-ink disabled:cursor-not-allowed disabled:bg-ink/25 disabled:text-[rgb(var(--color-bg-panel)/0.7)]"
                }
                aria-label={sendLabel}
              >
                {sendBusy ? (
                  <LoaderCircle className={(sendButtonVariant === "astra" ? "relative z-10 h-4 w-4" : "h-5 w-5") + " animate-spin"} />
                ) : sendButtonVariant === "astra" ? (
                  <Sparkles className="relative z-10 h-4 w-4" />
                ) : (
                  <ArrowUp className="h-5 w-5" />
                )}
              </button>
            </Tooltip>
            {sendActions}
          </div>
        </div>
        {bottomRow}
      </div>
      {!runtimeControlsDisabled && composer.attachmentMenuOpen && composer.attachmentButtonRef.current && (
        <ComposerAttachmentMenu
          anchor={composer.attachmentButtonRef.current}
          options={attachmentOptions}
          onClose={() => composer.setAttachmentMenuOpen(false)}
          onSelect={(key) => {
            void composer.pickAttachments(key);
          }}
        />
      )}
    </div>
  );
}

export function ScrambledProjectName({ name }: { name: string }) {
  const [display, setDisplay] = useState(name);
  const previousNameRef = useRef(name);

  useEffect(() => {
    const previousName = previousNameRef.current;
    previousNameRef.current = name;
    if (previousName === name) {
      setDisplay(name);
      return;
    }
    if (
      typeof window === "undefined" ||
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ) {
      setDisplay(name);
      return;
    }

    let frame = 0;
    let raf = 0;
    const maxLength = Math.max(previousName.length, name.length);
    const frames = Math.min(24, Math.max(12, maxLength + 8));

    const tick = () => {
      frame += 1;
      const settled = Math.floor((frame / frames) * maxLength);
      let next = "";
      for (let index = 0; index < maxLength; index += 1) {
        const target = name[index] ?? "";
        if (index < settled || frame >= frames) {
          next += target;
        } else if (target) {
          next += PROJECT_NAME_SCRAMBLE_CHARS[
            Math.floor(Math.random() * PROJECT_NAME_SCRAMBLE_CHARS.length)
          ];
        }
      }
      setDisplay(next || name);
      if (frame < frames) {
        raf = window.requestAnimationFrame(tick);
      }
    };

    raf = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(raf);
  }, [name]);

  return (
    <span className="inline-block min-w-[3ch] font-mono tabular-nums text-ink">
      {display}
    </span>
  );
}

export function NewChatMenuButton({
  icon: Icon,
  label,
  text,
}: {
  icon: LucideIcon;
  label: string;
  text?: boolean;
}) {
  return (
    <button
      type="button"
      className={
        "flex min-w-0 items-center gap-1.5 rounded-md py-1 text-body-sm text-ink/55 transition hover:bg-ink/8 hover:text-ink " +
        (text ? "max-w-[220px] px-1.5" : "h-7 w-7 justify-center px-0")
      }
      aria-label={label}
    >
      <Icon className="h-4 w-4 shrink-0" />
      {text && <span className="truncate">{label}</span>}
      {text && <ChevronDown className="h-3.5 w-3.5 shrink-0" />}
    </button>
  );
}

export function resizeTextareaToContent(el: HTMLTextAreaElement) {
  el.style.height = "auto";
  const lineHeight = parseFloat(getComputedStyle(el).lineHeight) || 20;
  const minHeight = lineHeight * 2;
  const maxHeight = lineHeight * 6;
  const nextHeight = Math.min(Math.max(el.scrollHeight, minHeight), maxHeight);
  el.style.height = `${nextHeight}px`;
  el.style.overflowY = el.scrollHeight > maxHeight ? "auto" : "hidden";
}
