import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  ArrowUp,
  BookOpen,
  Check,
  ChevronDown,
  FileText,
  Image as ImageIcon,
  MonitorCog,
  LoaderCircle,
  Plus,
  Square,
  Sparkles,
  Trash2,
  X,
  type LucideIcon,
} from "lucide-react";
import PopupMenu, { type PopupMenuOption } from "./PopupMenu";
import { RuntimeMenuSelect } from "./RuntimeMenuSelect";
import ScreenshotComposerButton from "./ScreenshotComposerButton";
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
const CHAT_COMPOSER_COMPACT_BREAKPOINT = 400;

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
  className,
  runtimeControlsDisabled = false,
  canSend,
  active = false,
  onCancel,
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
  className?: string;
  runtimeControlsDisabled?: boolean;
  canSend?: boolean;
  active?: boolean;
  onCancel?: () => void;
  onSend: () => void;
  onTextareaKeyDown?: (event: import("react").KeyboardEvent<HTMLTextAreaElement>) => boolean;
}) {
  const { t } = useI18n();
  const [astraSweep, setAstraSweep] = useState(false);
  const previousSendButtonVariantRef = useRef(sendButtonVariant);
  const imeCompositionRef = useRef(createImeCompositionState());
  const composerBoxRef = useRef<HTMLDivElement>(null);
  const [compactControls, setCompactControls] = useState(false);
  const sendEnabled = canSend ?? composer.canSend;
  const sendBusy = sendButtonBusy ?? composer.sending;
  const canCancel = active && !sendBusy;
  const sendLabel =
    sendButtonLabel ?? (
      active
        ? "Stop"
        : sendBusy
          ? t("new_chat.sending")
          : t("new_chat.send")
    );
  const showContextMenuTrigger =
    composer.supportsAttachments || composer.availableSkills.length > 0;
  const systemSkills = composer.availableSkills.filter((skill) => skill.source === "builtin");
  const personalSkills = composer.availableSkills.filter((skill) => skill.source === "user");
  const skillSubmenuOptions: PopupMenuOption<string>[] = [
    ...(composer.selectedSkillIds.length > 0
      ? [{
          key: "skills:clear",
          label: t("new_chat.skills_clear"),
          icon: <Trash2 className="h-4 w-4" />,
        }]
      : []),
    ...(systemSkills.length > 0
      ? [{ key: "skills:system-label", label: t("new_chat.skills_system"), kind: "label" as const }]
      : []),
    ...systemSkills.map((skill) => ({
      key: `skill:${skill.id}`,
      label: skill.name,
      icon: composer.selectedSkillIds.includes(skill.id)
        ? <Check className="h-4 w-4" />
        : <BookOpen className="h-4 w-4" />,
    })),
    ...(personalSkills.length > 0
      ? [{ key: "skills:personal-label", label: t("new_chat.skills_personal"), kind: "label" as const }]
      : []),
    ...personalSkills.map((skill) => ({
      key: `skill:${skill.id}`,
      label: skill.name,
      icon: composer.selectedSkillIds.includes(skill.id)
        ? <Check className="h-4 w-4" />
        : <BookOpen className="h-4 w-4" />,
    })),
  ];
  const attachmentOptions: PopupMenuOption<string>[] = [
    ...(composer.supportsImageAttachments
      ? [{
          key: "images",
          label: t("new_chat.add_images"),
          icon: <ImageIcon className="h-4 w-4" />,
        }]
      : []),
    ...(composer.supportsEmbeddedContext
      ? [{
          key: "files",
          label: t("new_chat.add_files"),
          icon: <FileText className="h-4 w-4" />,
        }]
      : []),
    ...(composer.availableSkills.length > 0
      ? [{
          key: "skills",
          label:
            composer.selectedSkillIds.length > 0
              ? t("new_chat.add_skills_selected", {
                  count: composer.selectedSkillIds.length,
                })
              : t("new_chat.add_skills"),
          icon: <BookOpen className="h-4 w-4" />,
          children: skillSubmenuOptions,
        }]
      : []),
  ];
  const outerClassName = variant === "chat" ? "w-full" : "w-full max-w-[730px]";
  const rootClassName = className ? `${outerClassName} ${className}` : outerClassName;
  const controlsClassName =
    "flex min-h-12 items-center px-3 pb-2 " +
    (compactControls ? "gap-1 " : "gap-3 ") +
    (compactControls ? "flex-wrap " : "justify-between ") +
    (bottomRow ? "border-b border-ink/10" : "");
  const leadingControlsClassName =
    "flex min-w-0 items-center " + (compactControls ? "flex-wrap gap-1" : "gap-3");
  const trailingControlsClassName =
    "flex shrink-0 items-center " + (compactControls ? "ml-auto gap-1" : "gap-2.5");
  const computerUseTooltip = composer.computerUseActive
    ? t("computer_use.disable_tooltip")
    : t("computer_use.toggle_tooltip");

  useLayoutEffect(() => {
    const node = composerBoxRef.current;
    if (!node || typeof ResizeObserver === "undefined") return;
    const update = () => {
      setCompactControls(node.getBoundingClientRect().width < CHAT_COMPOSER_COMPACT_BREAKPOINT);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

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
    <div className={rootClassName}>
      {title && (
        <h1 className="mb-11 text-center text-[28px] font-medium leading-tight tracking-normal text-ink/92">
          {title}
        </h1>
      )}
      <div
        ref={composerBoxRef}
        className={
          "overflow-hidden rounded-2xl bg-ink/[0.055] shadow-[inset_0_0_0_1px_rgb(var(--color-ink)/0.08)] transition-shadow " +
          (composer.composerError
            ? "shadow-[inset_0_0_0_1px_rgb(var(--color-status-error)/0.35)]"
            : "focus-within:shadow-[inset_0_0_0_1px_rgb(var(--color-ink)/0.20)]")
        }
      >
        {composer.composerError && (
          <div className="flex items-start gap-2 border-b border-status-error/20 bg-status-error/10 px-3.5 py-2 text-body-sm text-status-error">
            <div className="min-w-0 flex-1 break-words">
              {composer.composerError}
            </div>
            <button
              type="button"
              aria-label="Dismiss error"
              onClick={() => composer.setComposerError(null)}
              className="-mr-1 flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-status-error/55 transition hover:bg-status-error/10 hover:text-status-error focus-visible:bg-status-error/10 focus-visible:text-status-error focus-visible:outline-none"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}
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
          <div className={leadingControlsClassName}>
            {showContextMenuTrigger && (
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
            {composer.supportsAttachments && composer.supportsImageAttachments && (
              <ScreenshotComposerButton
                composer={composer}
                disabled={runtimeControlsDisabled}
              />
            )}
            {composer.computerUseEligible && (
              <Tooltip content={computerUseTooltip} placement="top">
                <button
                  type="button"
                  disabled={runtimeControlsDisabled}
                  onClick={() => void composer.handleComputerUseToggle()}
                  className={
                    "relative flex h-7 items-center justify-center rounded-full px-2.5 text-ink/55 transition " +
                    (composer.computerUseEnabled
                      ? "bg-ink/12 text-ink"
                      : "hover:bg-ink/8 hover:text-ink") +
                    " disabled:cursor-not-allowed disabled:text-ink/28 disabled:hover:bg-transparent disabled:hover:text-ink/28"
                  }
                  aria-label={computerUseTooltip}
                  aria-pressed={composer.computerUseEnabled}
                >
                  {composer.computerUseEnabled && (
                    <span
                      className="absolute right-1 top-1 h-2 w-2 rounded-full bg-emerald ring-2 ring-surface"
                      aria-hidden="true"
                    />
                  )}
                  <MonitorCog className="h-4 w-4" />
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
                triggerDisplay={compactControls ? "icon" : "full"}
                minMenuWidth={220}
                maxWidthClassName={compactControls ? "max-w-[150px]" : "max-w-[260px]"}
              />
            )}
            {modeActions}
          </div>
          <div className={trailingControlsClassName}>
            <RuntimeMenuSelect
              ariaLabel={t("new_chat.agent")}
              value={composer.selectedAgentModelValue}
              onChange={(value) => void composer.handleAgentModelChange(value)}
              disabled={runtimeControlsDisabled || composer.agentModelOptions.length === 0}
              options={composer.agentModelOptions}
              triggerDisplay={compactControls ? "icon" : "full"}
              minMenuWidth={220}
              maxWidthClassName={compactControls ? "max-w-[210px]" : "max-w-[260px]"}
            />
            <Tooltip content={sendLabel} placement="top">
              <button
                type="button"
                disabled={active ? !canCancel : !sendEnabled}
                onClick={active ? onCancel : onSend}
                className={
                  sendButtonVariant === "astra"
                    ? "astra-send-button relative flex h-7 w-7 items-center justify-center overflow-hidden rounded-full " +
                      (astraSweep ? "astra-send-button-sweep" : "")
                    : "flex h-7 w-7 items-center justify-center rounded-full bg-ink/70 text-[rgb(var(--color-bg-panel))] transition hover:bg-ink disabled:cursor-not-allowed disabled:bg-ink/25 disabled:text-[rgb(var(--color-bg-panel)/0.7)]"
                }
                aria-label={sendLabel}
              >
                {active ? (
                  <Square className={(sendButtonVariant === "astra" ? "relative z-10 h-3.5 w-3.5 fill-current" : "h-3.5 w-3.5 fill-current")} />
                ) : sendBusy ? (
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
      {!runtimeControlsDisabled &&
        showContextMenuTrigger &&
        composer.attachmentMenuOpen &&
        composer.attachmentButtonRef.current && (
        <PopupMenu
          anchor={composer.attachmentButtonRef.current}
          options={attachmentOptions}
          onClose={() => composer.setAttachmentMenuOpen(false)}
          onSelect={(key) => {
            if (key === "skills:clear") {
              composer.clearSelectedSkills();
              return false;
            }
            if (key.startsWith("skill:")) {
              composer.toggleSkillSelection(key.slice("skill:".length));
              return false;
            }
            if (key === "images" || key === "files") {
              void composer.pickAttachments(key);
            }
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

export function AssistantModeChip({
  icon,
  name,
  onRemove,
}: {
  icon: ReactNode;
  name: string;
  onRemove: () => void;
}) {
  return (
    <span className="inline-flex h-7 max-w-[200px] items-center gap-1.5 rounded-md border border-ink/[0.12] bg-ink/[0.048] px-1.5 text-caption text-ink/70">
      {icon}
      <span className="min-w-0 truncate">{name}</span>
      <Tooltip content="Remove" placement="top">
        <button
          type="button"
          onClick={onRemove}
          className="shrink-0 rounded p-0.5 text-ink/35 transition hover:bg-ink/6 hover:text-ink/70"
          aria-label="Remove assistant"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      </Tooltip>
    </span>
  );
}

export function SlashCommandModeChip({
  name,
  onRemove,
}: {
  name: string;
  onRemove: () => void;
}) {
  return (
    <span className="inline-flex h-7 max-w-[200px] items-center gap-1.5 rounded-md border border-ink/[0.12] bg-ink/[0.048] px-1.5 text-caption text-ink/70">
      <span className="shrink-0 font-medium text-ink/45">/</span>
      <span className="min-w-0 truncate">{name}</span>
      <Tooltip content="Remove" placement="top">
        <button
          type="button"
          onClick={onRemove}
          className="shrink-0 rounded p-0.5 text-ink/35 transition hover:bg-ink/6 hover:text-ink/70"
          aria-label="Remove slash command"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      </Tooltip>
    </span>
  );
}
