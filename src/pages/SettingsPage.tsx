import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { DragDropProvider, type DragEndEvent } from "@dnd-kit/react";
import { isSortable, useSortable } from "@dnd-kit/react/sortable";
import { QRCodeSVG } from "@rc-component/qrcode";
import {
  Anthropic,
  Aws,
  Azure,
  Bedrock,
  Cerebras,
  Claude,
  Cloudflare,
  DeepSeek,
  Fireworks,
  GithubCopilot,
  Google,
  GoogleCloud,
  Groq,
  HuggingFace,
  Kimi,
  Minimax,
  Mistral,
  Moonshot,
  OpenAI,
  OpenCode,
  OpenRouter,
  Together,
  Vercel,
  VertexAI,
  WorkersAI,
  XAI,
  XiaomiMiMo,
  ZAI,
} from "@lobehub/icons";
import { ArrowLeft, AtSign, Bot, Check, Circle, Download, Eye, EyeOff, Globe2, GripVertical, Hash, Info, Languages, Link2, LoaderCircle, Monitor, Moon, Pencil, Plus, RefreshCw, RotateCcw, Save, Server, Settings2, Shield, Sparkles, SquareKanban, Sun, Trash2, Workflow, X } from "lucide-react";
import type { Agent, AgentAiProviderInfo, AgentCommandsInfo, AgentInfo, AstraConfig, AssistantInfo, ComputerUseSettings, DiscordBridgeConfig, FeishuBridgeConfig, ImBridgeConfig, ImBridgeWorkspaceBinding, McpKeyValue, McpServerConfig, McpSettings, NetworkConfig, ProjectInfo, ProjectStageInfo, RuntimeAgentMetadata, RuntimeAgentOptionMetadata, ProcessTemplateInfo, TelegramBridgeConfig, WechatBridgeConfig, WechatQrStatus } from "../api";
import {
  createProcessTemplate,
  detectTelegramUserIds,
  getAppshotConfig,
  getComputerUseSettings,
  getMcpSettings,
  getAstraConfig,
  getDesktopControlPermissionStatus,
  getImBridgeConfig,
  getWechatQrcode,
  getNetworkConfig,
  listAgents,
  listAssistants,
  listProjects,
  listProcessTemplateStages,
  listProcessTemplates,
  listRuntimeAgents,
  setAppshotShortcutRecording,
  testDiscordBotConnection,
  testFeishuBotConnection,
  testTelegramBotConnection,
  testWechatBotConnection,
  pollWechatQrcodeStatus,
  setComputerUseAppApproval,
  updateAgentPreferences,
  updateAppshotConfig,
  updateComputerUseSettings,
  updateMcpSettings,
  updateAstraConfig,
  updateImBridgeConfig,
  updateNetworkConfig,
  updateRuntimeAgentPreferences,
  isAgent,
} from "../api";
import CreateAssistantDialog from "../components/CreateAssistantDialog";
import CreateStageDialog from "../components/CreateStageDialog";
import AssistantCard from "../components/AssistantCard";
import { agentModelSelectOptions, agentModelSelectValue, initialRuntimeEffort, parseAgentModelSelectValue, runtimeEffortOptions } from "../components/AgentSelect";
import { AgentGlyph } from "../components/AgentIcon";
import InlineMenuSelect from "../components/InlineMenuSelect";
import StageList from "../components/StageList";
import { RuntimeEffortControl, RuntimeMenuSelect, runtimePermissionModeOptions } from "../components/RuntimeMenuSelect";
import ScrollArea from "../components/ScrollArea";
import SegmentedTabs from "../components/SegmentedTabs";
import SwitchControl from "../components/SwitchControl";
import Tooltip from "../components/Tooltip";
import { AiGenerate2Icon, ChannelShare24RegularIcon, DiscordLogoIcon, LarkLogoIcon, QrCodeIcon, Robot3LineIcon, ScreenshotRegionIcon, TelegramLogoIcon, TokenOutlineIcon, WechatLogoIcon } from "../components/IconifyIcon";
import { emitComputerUseSettingsChanged } from "../computerUseSettingsEvents";
import { desktopControlPermissionPresentation } from "../desktopControlPermissionPresentation";
import { type Lang, useI18n } from "../i18n";
import { getSessioPromptMarkers } from "../promptMarkers";
import type { ThemeMode } from "../theme";
import { formatVersionLabel, type UpdateState } from "../updater";
import acpMarkBlackUrl from "../../assets/acp_mark-black.svg?url";
import acpMarkWhiteUrl from "../../assets/acp_mark-white.svg?url";

type SettingsSection = "general" | "agents" | "assistants" | "processTemplates" | "channels";
type ChannelPlatform = "telegram" | "discord" | "feishu" | "wechat";
const SESSIO_PROMPT_MARKERS = getSessioPromptMarkers();

type AgentPreferencePatch = {
  displayName?: string | null;
  enabled?: boolean;
  order?: number;
  aiProvider?: string | null;
  aiProviders?: AgentAiProviderInfo[];
  commands?: AgentCommandsInfo;
  model?: string | null;
  effort?: string | null;
  permissionMode?: string | null;
  models?: RuntimeAgentOptionMetadata[];
};

export default function SettingsPage({
  lang,
  onLangChange,
  themeMode,
  onThemeModeChange,
  rebuilding,
  indexing,
  onBack,
  onError,
  onRebuildFinished,
  appVersion,
  update,
  onOpenUpdate,
  onOpenAppshotPermissions,
}: {
  lang: Lang;
  onLangChange: (lang: Lang) => void;
  themeMode: ThemeMode;
  onThemeModeChange: (mode: ThemeMode) => void;
  rebuilding: boolean;
  indexing: boolean;
  onBack: () => void;
  onError: (error: string | null) => void;
  onRebuildFinished: () => Promise<void> | void;
  appVersion: string;
  update: UpdateState;
  onOpenUpdate: () => void;
  onOpenAppshotPermissions: () => Promise<void> | void;
}) {
  const { t } = useI18n();
  const [section, setSection] = useState<SettingsSection>("general");

  const navItems = [
    { id: "general" as const, label: t("settings.general"), icon: Settings2 },
    { id: "agents" as const, label: t("agent.title"), icon: AiGenerate2Icon },
    { id: "assistants" as const, label: t("assistant.title"), icon: Robot3LineIcon },
    { id: "processTemplates" as const, label: t("settings.process_templates"), icon: Workflow },
    { id: "channels" as const, label: t("settings.channels"), icon: ChannelShare24RegularIcon },
  ];
  const sectionTitle = navItems.find((item) => item.id === section)?.label ?? t("settings.general");

  return (
    <div className="flex h-screen text-body text-ink">
      <aside className="flex w-[248px] shrink-0 flex-col bg-surface-sidebar">
        <header data-tauri-drag-region className="h-12 shrink-0 select-none bg-surface-sidebar" />
        <div className="min-h-0 flex-1 px-2">
          <button
            type="button"
            onClick={onBack}
            data-tauri-drag-region="false"
            className="mb-3 flex h-8 w-full items-center gap-2 rounded-md px-2.5 text-left text-body-sm text-ink/55 transition hover:bg-ink/5 hover:text-ink"
          >
            <ArrowLeft className="h-4 w-4" />
            {t("settings.back_to_app")}
          </button>
          <nav data-tauri-drag-region="false" className="grid gap-1">
            {navItems.map(({ id, label, icon: Icon }) => {
              const active = section === id;
              return (
                <button
                  key={id}
                  type="button"
                  onClick={() => setSection(id)}
                  className={
                  "flex h-8 items-center gap-2 rounded-md px-2.5 text-left text-body-sm transition " +
                  (active ? "bg-ink/[0.065] text-ink/88" : "text-ink/72 hover:bg-ink/5 hover:text-ink")
                  }
                >
                  <Icon className="h-4 w-4 shrink-0" />
                  <span className="truncate">{label}</span>
                </button>
              );
            })}
          </nav>
        </div>
      </aside>
      <main className="relative flex min-w-0 flex-1 flex-col">
        <header data-tauri-drag-region className="grid h-12 shrink-0 select-none grid-cols-3 items-center border-b border-ink/[0.12] bg-surface px-5">
          <h1 data-tauri-drag-region className="col-start-2 justify-self-center truncate text-title font-semibold text-ink/85">{sectionTitle}</h1>
        </header>
        <ScrollArea className="min-h-0 flex-1 bg-surface-panel" viewportClassName={"px-10 pt-6 " + (section === "processTemplates" ? "pb-6" : "pb-16")}>
          <div className="mx-auto max-w-[840px]">
            {section === "general" && (
              <GeneralSettings
                lang={lang}
                themeMode={themeMode}
                rebuilding={rebuilding}
                indexing={indexing}
                onLangChange={onLangChange}
                onThemeModeChange={onThemeModeChange}
                onError={onError}
                onRebuildFinished={onRebuildFinished}
                appVersion={appVersion}
                update={update}
                onOpenUpdate={onOpenUpdate}
                onOpenAppshotPermissions={onOpenAppshotPermissions}
              />
            )}
            {section === "agents" && <AgentsSettings onError={onError} />}
            {section === "assistants" && <AssistantsSettings onError={onError} />}
            {section === "channels" && <ChannelsSettings onError={onError} />}
            {section === "processTemplates" && <ProcessTemplatesSettings onError={onError} />}
          </div>
        </ScrollArea>
      </main>
    </div>
  );
}

function AboutGroup({
  appVersion,
  lang,
  update,
  onOpenUpdate,
}: {
  appVersion: string;
  lang: Lang;
  update: UpdateState;
  onOpenUpdate: () => void;
}) {
  const { t } = useI18n();
  const status = update.updateReady
    ? t("settings.update_ready")
    : update.hasUpdate && update.latestVersion
      ? t("settings.update_available_version", { version: formatVersionLabel(update.latestVersion) })
      : update.checking
        ? t("settings.update_checking")
        : t("settings.update_up_to_date");
  const actionLabel = update.updateReady
    ? t("update_dialog.restart_now")
    : update.canInstall
      ? t("update_dialog.update_now")
      : t("update_dialog.download_now");
  const ActionIcon = update.updateReady ? RotateCcw : Download;

  return (
    <SettingsGroup title={t("settings.about")} flush>
      <SettingsRow icon={<Info className="h-4 w-4" />} label={t("settings.about")} description={t("settings.about_description")}>
        <div className="text-right text-caption text-ink/58">
          <div className="font-semibold text-ink/78">{formatVersionLabel(appVersion)}</div>
          <div>{status}</div>
        </div>
      </SettingsRow>
      <SettingsRow
        icon={<RefreshCw className="h-4 w-4" />}
        label={t("settings.update_status")}
        description={t("settings.last_checked_value", {
          value: update.lastCheckedAt ? formatSettingsDate(update.lastCheckedAt, lang) : t("settings.never"),
        })}
      >
        <div className="flex flex-wrap items-center justify-end gap-2">
          {(update.hasUpdate || update.updateReady) && update.latestVersion && (
            <button
              type="button"
              disabled={update.installing}
              onClick={onOpenUpdate}
              className="inline-flex h-8 items-center gap-2 rounded-md bg-blue px-3 text-body-sm font-medium text-white outline-none transition hover:bg-blue/92 focus-visible:ring-2 focus-visible:ring-blue/45 disabled:opacity-55"
            >
              {update.installing ? (
                <LoaderCircle className="h-4 w-4 animate-spin" />
              ) : (
                <ActionIcon className="h-4 w-4" />
              )}
              {actionLabel}
            </button>
          )}
          <button
            type="button"
            disabled={update.checking || update.installing}
            onClick={update.check}
            className="inline-flex h-8 items-center gap-2 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-45"
          >
            <RefreshCw className={"h-4 w-4 " + (update.checking ? "animate-spin" : "")} />
            {t("settings.check_for_updates")}
          </button>
        </div>
      </SettingsRow>
    </SettingsGroup>
  );
}

function formatSettingsDate(ts: number, lang: Lang): string {
  return new Intl.DateTimeFormat(lang === "zh" ? "zh-CN" : "en-US", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(ts));
}

type ShortcutPlatform = "macos" | "windows" | "linux" | "other";

const shortcutModifierOrderByPlatform: Record<ShortcutPlatform, string[]> = {
  macos: ["super", "control", "alt", "shift", "commandorcontrol"],
  windows: ["control", "alt", "shift", "super", "commandorcontrol"],
  linux: ["control", "alt", "shift", "super", "commandorcontrol"],
  other: ["control", "alt", "shift", "super", "commandorcontrol"],
};

function normalizeShortcutModifierToken(token: string): string | null {
  const normalized = token.trim().toLowerCase();
  if (!normalized) return null;
  if (normalized === "shift") return "shift";
  if (normalized === "control" || normalized === "ctrl") return "control";
  if (normalized === "alt" || normalized === "option") return "alt";
  if (normalized === "super" || normalized === "command" || normalized === "cmd") {
    return "super";
  }
  if (
    normalized === "commandorcontrol" ||
    normalized === "commandorctrl" ||
    normalized === "cmdorctrl" ||
    normalized === "cmdorcontrol"
  ) {
    return "commandorcontrol";
  }
  return null;
}

const supportedShortcutCodes = new Set([
  "Backquote",
  "Backslash",
  "BracketLeft",
  "BracketRight",
  "Pause",
  "Comma",
  "Equal",
  "Minus",
  "Period",
  "Quote",
  "Semicolon",
  "Slash",
  "Backspace",
  "CapsLock",
  "Enter",
  "Space",
  "Tab",
  "Delete",
  "End",
  "Home",
  "Insert",
  "PageDown",
  "PageUp",
  "PrintScreen",
  "ScrollLock",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
  "NumLock",
  "NumpadAdd",
  "NumpadDecimal",
  "NumpadDivide",
  "NumpadEnter",
  "NumpadEqual",
  "NumpadMultiply",
  "NumpadSubtract",
  "Escape",
  "AudioVolumeDown",
  "AudioVolumeUp",
  "AudioVolumeMute",
  "MediaPlay",
  "MediaPause",
  "MediaPlayPause",
  "MediaStop",
  "MediaTrackNext",
  "MediaTrackPrevious",
]);

function resolveShortcutPlatform(platform?: string | null): ShortcutPlatform {
  if (platform === "macos" || platform === "windows" || platform === "linux") {
    return platform;
  }
  if (typeof navigator === "undefined") {
    return "other";
  }
  if (/Mac/i.test(navigator.platform)) return "macos";
  if (/Win/i.test(navigator.platform)) return "windows";
  if (/Linux/i.test(navigator.platform)) return "linux";
  return "other";
}

function shortcutKeyFromCode(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code;
  if (/^Digit[0-9]$/.test(code)) return code;
  if (/^Numpad[0-9]$/.test(code)) return code;
  if (/^F(?:[1-9]|1\d|2[0-4])$/.test(code)) return code;
  return supportedShortcutCodes.has(code) ? code : null;
}

function isModifierOnlyKey(event: { key: string; code: string }): boolean {
  return (
    event.key === "Shift" ||
    event.key === "Control" ||
    event.key === "Alt" ||
    event.key === "Meta" ||
    event.code === "ShiftLeft" ||
    event.code === "ShiftRight" ||
    event.code === "ControlLeft" ||
    event.code === "ControlRight" ||
    event.code === "AltLeft" ||
    event.code === "AltRight" ||
    event.code === "MetaLeft" ||
    event.code === "MetaRight"
  );
}

function shortcutModifiersFromEvent(event: {
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}): string[] {
  const modifiers: string[] = [];
  if (event.shiftKey) modifiers.push("shift");
  if (event.ctrlKey) modifiers.push("control");
  if (event.altKey) modifiers.push("alt");
  if (event.metaKey) modifiers.push("super");
  return modifiers;
}

function formatShortcutToken(token: string, platform: ShortcutPlatform): string {
  const normalized = token.trim().toLowerCase();
  if (!normalized) return "";
  if (normalized === "shift") return platform === "macos" ? "⇧" : "Shift";
  if (normalized === "control" || normalized === "ctrl") {
    return platform === "macos" ? "⌃" : "Ctrl";
  }
  if (normalized === "alt" || normalized === "option") {
    return platform === "macos" ? "⌥" : "Alt";
  }
  if (normalized === "super" || normalized === "command" || normalized === "cmd") {
    if (platform === "macos") return "⌘";
    if (platform === "windows") return "Win";
    return "Super";
  }
  if (
    normalized === "commandorcontrol" ||
    normalized === "commandorctrl" ||
    normalized === "cmdorctrl" ||
    normalized === "cmdorcontrol"
  ) {
    return platform === "macos" ? "⌘" : "Ctrl";
  }
  if (/^key[a-z]$/i.test(token)) return token.slice(-1).toUpperCase();
  if (/^digit[0-9]$/i.test(token)) return token.slice(-1);
  if (/^numpad[0-9]$/i.test(token)) return `Num ${token.slice(-1)}`;

  switch (token) {
    case "Backquote":
      return "`";
    case "Backslash":
      return "\\";
    case "BracketLeft":
      return "[";
    case "BracketRight":
      return "]";
    case "Comma":
      return ",";
    case "Equal":
      return "=";
    case "Minus":
      return "-";
    case "Period":
      return ".";
    case "Quote":
      return "'";
    case "Semicolon":
      return ";";
    case "Slash":
      return "/";
    case "ArrowUp":
      return "Up";
    case "ArrowDown":
      return "Down";
    case "ArrowLeft":
      return "Left";
    case "ArrowRight":
      return "Right";
    case "PageUp":
      return "Page Up";
    case "PageDown":
      return "Page Down";
    case "PrintScreen":
      return "Print Screen";
    case "ScrollLock":
      return "Scroll Lock";
    case "CapsLock":
      return "Caps Lock";
    case "NumLock":
      return "Num Lock";
    case "Backspace":
      return "Backspace";
    case "Delete":
      return "Delete";
    case "Enter":
      return "Enter";
    case "Escape":
      return "Esc";
    case "Home":
      return "Home";
    case "End":
      return "End";
    case "Insert":
      return "Insert";
    case "Space":
      return "Space";
    case "Tab":
      return "Tab";
    case "Pause":
      return "Pause";
    case "NumpadAdd":
      return "Num +";
    case "NumpadDecimal":
      return "Num .";
    case "NumpadDivide":
      return "Num /";
    case "NumpadEnter":
      return "Num Enter";
    case "NumpadEqual":
      return "Num =";
    case "NumpadMultiply":
      return "Num *";
    case "NumpadSubtract":
      return "Num -";
    case "AudioVolumeDown":
      return "Volume Down";
    case "AudioVolumeUp":
      return "Volume Up";
    case "AudioVolumeMute":
      return "Mute";
    case "MediaPlay":
      return "Media Play";
    case "MediaPause":
      return "Media Pause";
    case "MediaPlayPause":
      return "Play/Pause";
    case "MediaStop":
      return "Media Stop";
    case "MediaTrackNext":
      return "Next Track";
    case "MediaTrackPrevious":
      return "Previous Track";
    default:
      return token.replace(/([a-z0-9])([A-Z])/g, "$1 $2");
  }
}

function shortcutDisplayTokens(shortcut: string, platform: ShortcutPlatform): string[] {
  const tokens = shortcut
    .split("+")
    .map((token) => token.trim())
    .filter(Boolean);

  if (tokens.length === 0) {
    return [];
  }

  const order = shortcutModifierOrderByPlatform[platform];
  const modifiers = tokens.filter((token) => {
    const normalized = normalizeShortcutModifierToken(token);
    return normalized ? order.includes(normalized) : false;
  });
  const keys = tokens.filter((token) => {
    const normalized = normalizeShortcutModifierToken(token);
    return !normalized || !order.includes(normalized);
  });

  modifiers.sort(
    (left, right) =>
      order.indexOf(normalizeShortcutModifierToken(left) ?? left.toLowerCase())
      - order.indexOf(normalizeShortcutModifierToken(right) ?? right.toLowerCase()),
  );

  return [...modifiers, ...keys]
    .map((token) => formatShortcutToken(token, platform))
    .filter(Boolean);
}

function shortcutFromKeyboardEvent(event: {
  key: string;
  code: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}): string | null {
  const key = shortcutKeyFromCode(event.code);
  if (!key || isModifierOnlyKey(event)) {
    return null;
  }
  return [...shortcutModifiersFromEvent(event), key].join("+");
}

function shortcutPreviewFromKeyboardEvent(
  event: {
    shiftKey: boolean;
    ctrlKey: boolean;
    altKey: boolean;
    metaKey: boolean;
  },
  platform: ShortcutPlatform,
): string[] {
  const modifiers = shortcutModifiersFromEvent(event);
  if (modifiers.length === 0) {
    return [];
  }
  return modifiers.map((token) => formatShortcutToken(token, platform));
}

function splitLabelAroundToken(label: string, token: string): { before: string; after: string } | null {
  const index = label.indexOf(token);
  if (index < 0) return null;
  return {
    before: label.slice(0, index),
    after: label.slice(index + token.length),
  };
}

function ShortcutKeycap({
  children,
  recording = false,
  className = "",
}: {
  children: ReactNode;
  recording?: boolean;
  className?: string;
}) {
  return (
    <span
      className={
        "inline-flex h-7 min-w-7 items-center justify-center rounded-md border px-2 text-caption font-medium tabular-nums transition " +
        (recording
          ? "border-blue/20 bg-blue/[0.08] text-blue"
          : "border-card-border/[0.16] bg-card-chip/[0.08] text-card-fg/72") +
        (className ? ` ${className}` : "")
      }
    >
      {children}
    </span>
  );
}

function ShortcutKeycapDisplay({
  tokens,
  recording = false,
}: {
  tokens: string[];
  recording?: boolean;
}) {
  if (tokens.length === 0) return null;
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-1">
      {tokens.map((token, index) => (
        <span key={`${token}:${index}`} className="inline-flex min-w-0 items-center gap-1">
          {index > 0 ? <span className={recording ? "text-caption text-blue/55" : "text-caption text-ink/30"}>+</span> : null}
          <ShortcutKeycap recording={recording}>{token}</ShortcutKeycap>
        </span>
      ))}
    </span>
  );
}

function AppshotShortcutControl({
  value,
  platform,
  saving,
  onError,
  onSave,
}: {
  value: string;
  platform: ShortcutPlatform;
  saving: boolean;
  onError: (error: string | null) => void;
  onSave: (nextShortcut: string) => Promise<void> | void;
}) {
  const { t } = useI18n();
  const [recording, setRecording] = useState(false);
  const [previewTokens, setPreviewTokens] = useState<string[]>([]);
  const [draftValue, setDraftValue] = useState<string | null>(null);
  const [editingShortcut, setEditingShortcut] = useState<string | null>(null);
  const controlRef = useRef<HTMLDivElement | null>(null);
  const saveButtonRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    let disposed = false;
    void (async () => {
      try {
        await setAppshotShortcutRecording(recording);
        if (!disposed && !recording) {
          onError(null);
        }
      } catch (err) {
        if (disposed) return;
        setRecording(false);
        onError(String(err));
      }
    })();
    return () => {
      disposed = true;
    };
  }, [onError, recording]);

  useEffect(() => {
    if (!recording) {
      setPreviewTokens([]);
      setDraftValue(null);
      setEditingShortcut(null);
      return;
    }
    controlRef.current?.focus();
  }, [recording]);

  const displayTokens = recording
    ? previewTokens.length > 0
      ? previewTokens
      : shortcutDisplayTokens(editingShortcut ?? value, platform)
    : shortcutDisplayTokens(value, platform);

  const pendingValue = (editingShortcut ?? "").trim();
  const originalValue = (draftValue ?? value).trim();
  const canSave = recording && !saving && pendingValue.length > 0 && pendingValue !== originalValue;
  const recordingHint = splitLabelAroundToken(t("settings.appshot_shortcut_recording_hint"), "Esc");

  const startRecording = () => {
    if (recording) return;
    setDraftValue(value);
    setEditingShortcut(value);
    setRecording(true);
  };

  const cancelRecording = () => {
    setRecording(false);
  };

  const commitRecording = async () => {
    if (!canSave) return;
    try {
      await onSave(pendingValue);
      setRecording(false);
    } catch {}
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!recording && (event.key === "Enter" || event.key === " ")) {
      event.preventDefault();
      startRecording();
      return;
    }

    if (!recording) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();

    if (event.key === "Escape") {
      cancelRecording();
      return;
    }

    const captured = shortcutFromKeyboardEvent(event);
    if (!captured) {
      setPreviewTokens(shortcutPreviewFromKeyboardEvent(event, platform));
      return;
    }

    setEditingShortcut(captured);
    setPreviewTokens(shortcutDisplayTokens(captured, platform));
  };

  return (
    <div
      ref={controlRef}
      role="button"
      tabIndex={0}
      onClick={startRecording}
      onBlur={(event) => {
        if (recording) {
          if (event.relatedTarget instanceof Node && saveButtonRef.current?.contains(event.relatedTarget)) {
            return;
          }
          cancelRecording();
        }
      }}
      onKeyDown={handleKeyDown}
      className={
        "flex min-h-11 w-full items-center justify-between gap-3 rounded-md border px-3 py-2 text-left text-body-sm outline-none transition " +
        (recording
          ? "border-blue/40 bg-blue/[0.08] text-ink shadow-[0_0_0_1px_rgba(59,130,246,0.15)]"
          : "border-input-border/[0.16] bg-input text-input-fg hover:border-input-focus/22 focus-visible:border-input-focus/30")
      }
    >
      <span className="min-w-0 flex-1">
        {displayTokens.length > 0 ? (
          <ShortcutKeycapDisplay tokens={displayTokens} recording={recording} />
        ) : (
          <span className="truncate font-medium">
            {recording ? t("settings.appshot_shortcut_recording") : t("settings.appshot_shortcut_unset")}
          </span>
        )}
      </span>
      <span className="flex shrink-0 flex-wrap items-center justify-end gap-2">
        {recording ? (
          recordingHint ? (
            <span className="flex items-center gap-1 text-caption text-blue">
              {recordingHint.before ? <span>{recordingHint.before}</span> : null}
              <ShortcutKeycap recording className="min-w-8 px-2">
                Esc
              </ShortcutKeycap>
              {recordingHint.after ? <span>{recordingHint.after}</span> : null}
            </span>
          ) : (
            <span className="text-caption text-blue">{t("settings.appshot_shortcut_recording_hint")}</span>
          )
        ) : (
          <span className="text-caption text-ink/45">
            {t("settings.appshot_shortcut_capture_hint")}
          </span>
        )}
        {recording && (
          <button
            ref={saveButtonRef}
            type="button"
            aria-label={t("chat.files.editor_save")}
            title={t("chat.files.editor_save")}
            className={
              "inline-flex h-7 w-7 items-center justify-center rounded-md border transition " +
              (canSave
                ? "border-blue/20 bg-blue/[0.08] text-blue hover:bg-blue/[0.12]"
                : "border-blue/10 bg-blue/[0.04] text-blue/35")
            }
            onMouseDown={(event) => {
              event.preventDefault();
            }}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              void commitRecording();
            }}
          >
            {saving ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <Save className="h-3.5 w-3.5" />}
          </button>
        )}
      </span>
    </div>
  );
}

function GeneralSettings({
  lang,
  themeMode,
  rebuilding,
  indexing,
  appVersion,
  update,
  onLangChange,
  onThemeModeChange,
  onError,
  onRebuildFinished,
  onOpenUpdate,
  onOpenAppshotPermissions,
}: {
  lang: Lang;
  themeMode: ThemeMode;
  rebuilding: boolean;
  indexing: boolean;
  appVersion: string;
  update: UpdateState;
  onLangChange: (lang: Lang) => void;
  onThemeModeChange: (mode: ThemeMode) => void;
  onError: (error: string | null) => void;
  onRebuildFinished: () => Promise<void> | void;
  onOpenUpdate: () => void;
  onOpenAppshotPermissions: () => Promise<void> | void;
}) {
  const { t } = useI18n();
  const [networkConfig, setNetworkConfig] = useState<NetworkConfig | null>(null);
  const [mcpSettings, setMcpSettings] = useState<McpSettings | null>(null);
  const [savedCustomMcpServers, setSavedCustomMcpServers] = useState<McpServerConfig[]>([]);
  const [computerUseSettings, setComputerUseSettings] = useState<ComputerUseSettings | null>(null);
  const [desktopControlPermissionStatus, setDesktopControlPermissionStatus] = useState<import("../api").DesktopControlPermissionStatus | null>(null);
  const [proxyEnabled, setProxyEnabled] = useState(false);
  const [proxyUrl, setProxyUrl] = useState("");
  const [noProxy, setNoProxy] = useState("");
  const [appshotShortcut, setAppshotShortcut] = useState("");
  const [desktopControlEnabled, setDesktopControlEnabled] = useState(false);
  const [savingProxy, setSavingProxy] = useState(false);
  const [proxySaveStatus, setProxySaveStatus] = useState<"idle" | "success" | "error">("idle");
  const [savingMcp, setSavingMcp] = useState(false);
  const [mcpSaveStatus, setMcpSaveStatus] = useState<"idle" | "success" | "error">("idle");
  const [savingAppshot, setSavingAppshot] = useState(false);
  const [savingDesktopControl, setSavingDesktopControl] = useState(false);

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      getNetworkConfig(),
      getMcpSettings(),
      getAppshotConfig(),
      getComputerUseSettings(),
      getDesktopControlPermissionStatus(),
    ])
      .then(([config, nextMcpSettings, nextAppshot, nextComputerUse, desktopStatus]) => {
        if (cancelled) return;
        setNetworkConfig(config);
        setMcpSettings(nextMcpSettings);
        setSavedCustomMcpServers(extractCustomMcpServers(nextMcpSettings));
        setProxyEnabled(config.proxy.enabled);
        setProxyUrl(config.proxy.url ?? "");
        setNoProxy(config.proxy.noProxy ?? "");
        setAppshotShortcut(nextAppshot.shortcut);
        setComputerUseSettings(nextComputerUse);
        setDesktopControlEnabled(nextComputerUse.enabled);
        setDesktopControlPermissionStatus(desktopStatus);
      })
      .catch((err) => onError(String(err)));
    return () => {
      cancelled = true;
    };
  }, [onError]);

  useEffect(() => {
    let disposed = false;
    const refreshPermissions = () => {
      void getDesktopControlPermissionStatus()
        .then((desktopStatus) => {
          if (disposed) return;
          setDesktopControlPermissionStatus((current) => {
            if (
              current?.platform === desktopStatus.platform &&
              current?.requiresPermission === desktopStatus.requiresPermission &&
              current?.canObserve === desktopStatus.canObserve &&
              current?.canInspect === desktopStatus.canInspect &&
              current?.canControl === desktopStatus.canControl &&
              current?.screenshots.granted === desktopStatus.screenshots.granted &&
              current?.screenshots.supported === desktopStatus.screenshots.supported &&
              current?.accessibility.granted === desktopStatus.accessibility.granted &&
              current?.accessibility.supported === desktopStatus.accessibility.supported
            ) {
              return current;
            }
            return desktopStatus;
          });
        })
        .catch(() => {});
    };

    refreshPermissions();
    const timer = window.setInterval(refreshPermissions, 1000);
    window.addEventListener("focus", refreshPermissions);

    return () => {
      disposed = true;
      window.clearInterval(timer);
      window.removeEventListener("focus", refreshPermissions);
    };
  }, []);

  const saveProxyConfig = async () => {
    if (savingProxy) return;
    setSavingProxy(true);
    try {
      const next = await updateNetworkConfig({
        proxy: {
          enabled: proxyEnabled,
          url: proxyUrl.trim() || null,
          noProxy: noProxy.trim() || null,
        },
      });
      setNetworkConfig(next);
      setProxyEnabled(next.proxy.enabled);
      setProxyUrl(next.proxy.url ?? "");
      setNoProxy(next.proxy.noProxy ?? "");
      setProxySaveStatus("success");
      onError(null);
    } catch (err) {
      setProxySaveStatus("error");
      onError(String(err));
    } finally {
      setSavingProxy(false);
    }
  };

  const saveMcpConfig = async () => {
    if (!mcpSettings || savingMcp) return;
    setSavingMcp(true);
    try {
      const next = await updateMcpSettings(mcpSettings);
      setMcpSettings(next);
      setSavedCustomMcpServers(extractCustomMcpServers(next));
      setMcpSaveStatus("success");
      onError(null);
    } catch (err) {
      setMcpSaveStatus("error");
      onError(String(err));
    } finally {
      setSavingMcp(false);
    }
  };

  const saveAppshotShortcut = async (nextShortcut: string) => {
    if (savingAppshot) return;
    setSavingAppshot(true);
    try {
      const next = await updateAppshotConfig({
        shortcut: nextShortcut.trim(),
      });
      setAppshotShortcut(next.shortcut);
      onError(null);
    } catch (err) {
      onError(String(err));
    } finally {
      setSavingAppshot(false);
    }
  };

  const toggleDesktopControlEnabled = async () => {
    if (savingDesktopControl) return;
    const previous = desktopControlEnabled;
    const nextEnabled = !previous;
    setDesktopControlEnabled(nextEnabled);
    setSavingDesktopControl(true);
    try {
      const next = await updateComputerUseSettings({
        enabled: nextEnabled,
        approvedApps: computerUseSettings?.approvedApps ?? [],
      });
      setComputerUseSettings(next);
      setMcpSettings((current) => withBuiltinComputerUseEnabled(current, next.enabled));
      setDesktopControlEnabled(next.enabled);
      emitComputerUseSettingsChanged(next);
      onError(null);
    } catch (err) {
      setDesktopControlEnabled(previous);
      onError(String(err));
    } finally {
      setSavingDesktopControl(false);
    }
  };

  const openDesktopControlPermissions = async () => {
    try {
      await onOpenAppshotPermissions();
      setDesktopControlPermissionStatus(await getDesktopControlPermissionStatus());
      onError(null);
    } catch (err) {
      onError(String(err));
    }
  };

  const revokeComputerUseAppApproval = async (appId: string) => {
    if (savingDesktopControl) return;
    setSavingDesktopControl(true);
    try {
      const next = await setComputerUseAppApproval("settings", appId, false);
      setComputerUseSettings(next);
      setMcpSettings((current) => withBuiltinComputerUseEnabled(current, next.enabled));
      setDesktopControlEnabled(next.enabled);
      emitComputerUseSettingsChanged(next);
      onError(null);
    } catch (err) {
      onError(String(err));
    } finally {
      setSavingDesktopControl(false);
    }
  };

  const proxyChanged = networkConfig
    ? proxyEnabled !== networkConfig.proxy.enabled
      || proxyUrl.trim() !== (networkConfig.proxy.url ?? "")
      || noProxy.trim() !== (networkConfig.proxy.noProxy ?? "")
    : false;

  useEffect(() => {
    setProxySaveStatus("idle");
  }, [proxyEnabled, proxyUrl, noProxy]);

  const approvedComputerUseApps = computerUseSettings?.approvedApps ?? [];
  const builtinMcpServers = useMemo(
    () =>
      (mcpSettings?.servers ?? []).filter(
        (server) => server.source === SESSIO_PROMPT_MARKERS.mcpSourceBuiltin,
      ),
    [mcpSettings],
  );
  const customMcpServers = useMemo(
    () => extractCustomMcpServers(mcpSettings),
    [mcpSettings],
  );
  const mcpChanged = useMemo(
    () => stableMcpServerJson(customMcpServers) !== stableMcpServerJson(savedCustomMcpServers),
    [customMcpServers, savedCustomMcpServers],
  );
  useEffect(() => {
    if (stableMcpServerJson(customMcpServers) !== stableMcpServerJson(savedCustomMcpServers)) {
      setMcpSaveStatus("idle");
    }
  }, [customMcpServers, savedCustomMcpServers]);
  useEffect(() => {
    let disposed = false;
    const unlistenPromise = listen("app_config_updated", () => {
      if (disposed || proxyChanged || mcpChanged || savingProxy || savingMcp || savingDesktopControl || savingAppshot) {
        return;
      }
      void Promise.all([
        getNetworkConfig(),
        getMcpSettings(),
        getComputerUseSettings(),
        getDesktopControlPermissionStatus(),
      ])
        .then(([config, nextMcpSettings, nextComputerUse, desktopStatus]) => {
          if (disposed) return;
          setNetworkConfig(config);
          setMcpSettings(nextMcpSettings);
          setSavedCustomMcpServers(extractCustomMcpServers(nextMcpSettings));
          setProxyEnabled(config.proxy.enabled);
          setProxyUrl(config.proxy.url ?? "");
          setNoProxy(config.proxy.noProxy ?? "");
          setComputerUseSettings(nextComputerUse);
          setDesktopControlEnabled(nextComputerUse.enabled);
          setDesktopControlPermissionStatus(desktopStatus);
        })
        .catch((err) => onError(String(err)));
    });
    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [mcpChanged, onError, proxyChanged, savingAppshot, savingDesktopControl, savingMcp, savingProxy]);
  const desktopControlPermission = desktopControlPermissionPresentation(desktopControlPermissionStatus);
  const shortcutPlatform = useMemo(
    () => resolveShortcutPlatform(desktopControlPermissionStatus?.platform),
    [desktopControlPermissionStatus?.platform],
  );

  const updateCustomMcpServers = (updater: (servers: McpServerConfig[]) => McpServerConfig[]) => {
    setMcpSettings((current) => {
      if (!current) return current;
      return {
        ...current,
        servers: replaceCustomMcpServers(current.servers, updater(extractCustomMcpServers(current))),
      };
    });
  };

  const addCustomMcpServer = () => {
    updateCustomMcpServers((servers) => [...servers, createDefaultCustomMcpServer()]);
  };

  const updateCustomMcpServer = (
    serverId: string,
    updater: (server: McpServerConfig) => McpServerConfig,
  ) => {
    updateCustomMcpServers((servers) =>
      servers.map((server) => (server.id === serverId ? updater(server) : server)),
    );
  };

  const removeCustomMcpServer = (serverId: string) => {
    updateCustomMcpServers((servers) => servers.filter((server) => server.id !== serverId));
  };

  return (
    <section className="min-w-0 max-w-full">
      <SettingsGroup title={t("settings.appearance")} flush>
        <SettingsRow icon={<Languages className="h-4 w-4" />} label={t("sidebar.language")} description={t("settings.language_description")}>
          <SegmentedControl
            value={lang}
            options={[
              { value: "en", label: "EN" },
              { value: "zh", label: "中" },
            ]}
            onChange={(value) => onLangChange(value as Lang)}
          />
        </SettingsRow>
        <SettingsRow icon={<Monitor className="h-4 w-4" />} label={t("sidebar.theme")} description={t("settings.theme_description")}>
          <ThemeSelector mode={themeMode} onChange={onThemeModeChange} />
        </SettingsRow>
      </SettingsGroup>
      <SettingsGroup title={t("settings.desktop_control")} flush>
        <SettingsStackedRow
          icon={<Shield className="h-4 w-4" />}
          label={t("settings.desktop_control")}
          description={t("settings.desktop_control_enable_description")}
        >
          <div className="space-y-3">
            <div className="text-caption text-ink/60">
              {t(desktopControlPermission.descriptionKey)}
            </div>
            <div className="flex w-full flex-wrap items-center gap-3">
              <div className="flex flex-wrap items-center gap-2 text-caption">
                {desktopControlPermission.showManageButton && (
                  <button
                    type="button"
                    onClick={() => void openDesktopControlPermissions()}
                    className="inline-flex h-9 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12]"
                  >
                    <Shield className="h-4 w-4" />
                    {t("settings.appshot_manage_permissions")}
                  </button>
                )}
                <span className="text-ink/24">·</span>
                <span className={desktopControlPermissionStatus?.screenshots.granted ? "text-emerald" : "text-amber"}>
                  {t(desktopControlPermission.screenshotKey)}
                </span>
                <span className="text-ink/24">·</span>
                <span className={desktopControlPermissionStatus?.accessibility.granted ? "text-emerald" : "text-amber"}>
                  {t(desktopControlPermission.accessibilityKey)}
                </span>
              </div>
              <div className="ml-auto flex shrink-0 items-center gap-3">
                <span className="text-caption text-ink/45">
                  {desktopControlEnabled ? t("settings.proxy_enabled") : t("settings.proxy_disabled")}
                </span>
                {savingDesktopControl && <LoaderCircle className="h-4 w-4 animate-spin text-ink/45" />}
                <SwitchControl
                  checked={desktopControlEnabled}
                  tooltip={t("settings.desktop_control_enable")}
                  disabled={savingDesktopControl}
                  onToggle={() => void toggleDesktopControlEnabled()}
                />
              </div>
            </div>
            <div className="rounded-lg border border-card-border/[0.12] bg-card-chip/[0.04]">
              <div className="flex items-center justify-between gap-3 border-b border-card-border/[0.08] px-3 py-2.5">
                <div>
                  <div className="text-body-sm font-medium text-ink">
                    {t("settings.desktop_control_approved_apps")}
                  </div>
                  <div className="mt-0.5 text-caption text-ink/45">
                    {t("settings.desktop_control_approved_apps_description")}
                  </div>
                </div>
                <span className="rounded-full bg-card-chip/[0.08] px-2 py-1 text-caption text-ink/45">
                  {approvedComputerUseApps.length}
                </span>
              </div>
              {approvedComputerUseApps.length === 0 ? (
                <div className="px-3 py-3 text-caption text-ink/45">
                  {t("settings.desktop_control_approved_apps_empty")}
                </div>
              ) : (
                <div className="divide-y divide-card-border/[0.08]">
                  {approvedComputerUseApps.map((appId) => (
                    <div key={appId} className="flex items-center justify-between gap-3 px-3 py-2.5">
                      <code className="min-w-0 truncate rounded bg-card-chip/[0.08] px-2 py-1 text-caption text-ink/70">
                        {appId}
                      </code>
                      <button
                        type="button"
                        disabled={savingDesktopControl}
                        onClick={() => void revokeComputerUseAppApproval(appId)}
                        className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-status-error/20 bg-status-error/[0.08] px-2.5 text-caption font-medium text-status-error transition hover:bg-status-error/[0.12] disabled:opacity-35"
                      >
                        <X className="h-3.5 w-3.5" />
                        {t("settings.desktop_control_revoke_app")}
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        </SettingsStackedRow>
        <SettingsStackedRow
          icon={<ScreenshotRegionIcon className="h-4 w-4" />}
          label={t("settings.appshot_shortcut")}
          description={t("settings.appshot_shortcut_description")}
        >
          <div className="max-w-[32rem]">
            <AppshotShortcutControl
              value={appshotShortcut}
              platform={shortcutPlatform}
              saving={savingAppshot}
              onError={onError}
              onSave={(nextShortcut) => void saveAppshotShortcut(nextShortcut)}
            />
          </div>
        </SettingsStackedRow>
      </SettingsGroup>
      <SettingsGroup
        title={t("settings.network")}
        action={
          <button
            type="button"
            disabled={savingProxy || !proxyChanged}
            onClick={() => void saveProxyConfig()}
            className={channelSaveButtonClass(proxySaveStatus)}
          >
            {savingProxy
              ? <LoaderCircle className="h-4 w-4 animate-spin" />
              : proxySaveStatus === "error"
                ? <X className="h-4 w-4" />
                : <Check className="h-4 w-4" />}
            {savingProxy
              ? t("project.save")
              : proxySaveStatus === "success"
                ? t("settings.saved")
                : proxySaveStatus === "error"
                  ? t("settings.save_failed")
                  : t("project.save")}
          </button>
        }
        flush
      >
        <SettingsRow icon={<Globe2 className="h-4 w-4" />} label={t("settings.proxy")} description={t("settings.proxy_description")}>
          <div className="flex items-center justify-end gap-3">
            <span className="text-caption text-ink/45">{proxyEnabled ? t("settings.proxy_enabled") : t("settings.proxy_disabled")}</span>
            <SwitchControl checked={proxyEnabled} tooltip={t("settings.proxy")} onToggle={() => setProxyEnabled((value) => !value)} />
          </div>
        </SettingsRow>
        <SettingsRow icon={<Globe2 className="h-4 w-4" />} label={t("settings.proxy_url")} description={t("settings.proxy_url_description")}>
          <input value={proxyUrl} onChange={(event) => setProxyUrl(event.target.value)} placeholder="http://127.0.0.1:7890" className={inputClassName + " w-[280px]"} />
        </SettingsRow>
        <SettingsRow icon={<Globe2 className="h-4 w-4" />} label={t("settings.no_proxy")} description={t("settings.no_proxy_description")}>
          <input value={noProxy} onChange={(event) => setNoProxy(event.target.value)} placeholder="localhost,127.0.0.1" className={inputClassName + " w-[280px]"} />
        </SettingsRow>
      </SettingsGroup>
      <SettingsGroup
        title={t("settings.mcp")}
        action={
          <div className="flex items-center gap-2">
            <button
              type="button"
              disabled={!mcpSettings}
              onClick={addCustomMcpServer}
              className="inline-flex h-9 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-40"
            >
              <Plus className="h-4 w-4" />
              {t("settings.mcp_add_server")}
            </button>
            <button
              type="button"
              disabled={savingMcp || !mcpChanged}
              onClick={() => void saveMcpConfig()}
              className={channelSaveButtonClass(mcpSaveStatus)}
            >
              {savingMcp
                ? <LoaderCircle className="h-4 w-4 animate-spin" />
                : mcpSaveStatus === "error"
                  ? <X className="h-4 w-4" />
                  : <Check className="h-4 w-4" />}
              {savingMcp
                ? t("project.save")
                : mcpSaveStatus === "success"
                  ? t("settings.saved")
                  : mcpSaveStatus === "error"
                    ? t("settings.save_failed")
                    : t("project.save")}
            </button>
          </div>
        }
        flush
      >
        <SettingsStackedRow
          icon={<Server className="h-4 w-4" />}
          label={t("settings.mcp_builtin")}
          description={t("settings.mcp_builtin_description")}
        >
          <div className="space-y-3">
            {builtinMcpServers.map((server) => (
              <McpBuiltinCard key={server.id} server={server} />
            ))}
          </div>
        </SettingsStackedRow>
        <SettingsStackedRow
          icon={<Pencil className="h-4 w-4" />}
          label={t("settings.mcp_custom")}
          description={t("settings.mcp_custom_description")}
        >
          <div className="space-y-3">
            {customMcpServers.length === 0 ? (
              <EmptyState label={t("settings.mcp_custom_empty")} />
            ) : (
              customMcpServers.map((server) => (
                <McpCustomServerCard
                  key={server.id}
                  server={server}
                  onUpdate={(nextServer) => updateCustomMcpServer(server.id, () => nextServer)}
                  onRemove={() => removeCustomMcpServer(server.id)}
                />
              ))
            )}
          </div>
        </SettingsStackedRow>
      </SettingsGroup>
      <SettingsGroup title={t("settings.index")} flush>
        <SettingsRow icon={<RefreshCw className="h-4 w-4" />} label={t("sidebar.rebuild_index")} description={t("settings.rebuild_description")}>
          <button
            type="button"
            disabled={indexing}
            onClick={() => {
              import("../api")
                .then(({ rebuildSessionIndex }) => rebuildSessionIndex())
                .catch((err) => onError(String(err)))
                .finally(() => {
                  void onRebuildFinished();
                });
            }}
            className="inline-flex h-8 items-center gap-2 rounded-md border border-ink/10 bg-ink/[0.035] px-3 text-body-sm text-ink/70 transition hover:bg-ink/[0.06] hover:text-ink disabled:opacity-45"
          >
            <RefreshCw className={"h-3.5 w-3.5 " + (rebuilding ? "animate-spin" : "")} />
            {indexing ? t("sidebar.status_indexing") : t("sidebar.rebuild_index")}
          </button>
        </SettingsRow>
      </SettingsGroup>
      <AboutGroup
        appVersion={appVersion}
        lang={lang}
        update={update}
        onOpenUpdate={onOpenUpdate}
      />
    </section>
  );
}

function ChannelsSettings({ onError }: { onError: (error: string | null) => void }) {
  const { t } = useI18n();
  const [activePlatform, setActivePlatform] = useState<ChannelPlatform>("telegram");
  const [config, setConfig] = useState<ImBridgeConfig | null>(null);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [runtimeAgents, setRuntimeAgents] = useState<RuntimeAgentMetadata[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState<"idle" | "success" | "error">("idle");
  const [detecting, setDetecting] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testStatus, setTestStatus] = useState<"idle" | "success" | "error">("idle");
  const [telegramEnabled, setTelegramEnabled] = useState(false);
  const [botToken, setBotToken] = useState("");
  const [showToken, setShowToken] = useState(false);
  const [allowedUserIds, setAllowedUserIds] = useState<number[]>([]);
  const [userIdInput, setUserIdInput] = useState("");
  const [telegramAgent, setTelegramAgent] = useState<Agent | null>(null);
  const [telegramModel, setTelegramModel] = useState<string | null>(null);
  const [telegramEffort, setTelegramEffort] = useState<string | null>(null);
  const [telegramWorkspace, setTelegramWorkspace] = useState("");
  const [bindings, setBindings] = useState<ImBridgeWorkspaceBinding[]>([]);
  const [bindingChatId, setBindingChatId] = useState("");
  const [bindingWorkspace, setBindingWorkspace] = useState("");
  const [discordEnabled, setDiscordEnabled] = useState(false);
  const [discordBotToken, setDiscordBotToken] = useState("");
  const [showDiscordToken, setShowDiscordToken] = useState(false);
  const [discordTesting, setDiscordTesting] = useState(false);
  const [discordTestStatus, setDiscordTestStatus] = useState<"idle" | "success" | "error">("idle");
  const [discordAgent, setDiscordAgent] = useState<Agent | null>(null);
  const [discordModel, setDiscordModel] = useState<string | null>(null);
  const [discordEffort, setDiscordEffort] = useState<string | null>(null);
  const [discordWorkspace, setDiscordWorkspace] = useState("");
  const [discordBindings, setDiscordBindings] = useState<ImBridgeWorkspaceBinding[]>([]);
  const [discordBindingChannelId, setDiscordBindingChannelId] = useState("");
  const [discordBindingWorkspace, setDiscordBindingWorkspace] = useState("");
  const [discordAllowedServerIds, setDiscordAllowedServerIds] = useState<string[]>([]);
  const [discordAllowedChannelIds, setDiscordAllowedChannelIds] = useState<string[]>([]);
  const [discordServerIdInput, setDiscordServerIdInput] = useState("");
  const [discordChannelIdInput, setDiscordChannelIdInput] = useState("");
  const [discordMentionOnly, setDiscordMentionOnly] = useState(true);
  const [feishuEnabled, setFeishuEnabled] = useState(false);
  const [feishuAppId, setFeishuAppId] = useState("");
  const [feishuAppSecret, setFeishuAppSecret] = useState("");
  const [showFeishuSecret, setShowFeishuSecret] = useState(false);
  const [feishuTesting, setFeishuTesting] = useState(false);
  const [feishuTestStatus, setFeishuTestStatus] = useState<"idle" | "success" | "error">("idle");
  const [feishuAgent, setFeishuAgent] = useState<Agent | null>(null);
  const [feishuModel, setFeishuModel] = useState<string | null>(null);
  const [feishuEffort, setFeishuEffort] = useState<string | null>(null);
  const [feishuWorkspace, setFeishuWorkspace] = useState("");
  const [feishuBindings, setFeishuBindings] = useState<ImBridgeWorkspaceBinding[]>([]);
  const [feishuBindingChatId, setFeishuBindingChatId] = useState("");
  const [feishuBindingWorkspace, setFeishuBindingWorkspace] = useState("");
  const [feishuDomain, setFeishuDomain] = useState<string | null>(null);
  const [wechatEnabled, setWechatEnabled] = useState(false);
  const [wechatBotToken, setWechatBotToken] = useState("");
  const [showWechatToken, setShowWechatToken] = useState(false);
  const [wechatTesting, setWechatTesting] = useState(false);
  const [wechatTestStatus, setWechatTestStatus] = useState<"idle" | "success" | "error">("idle");
  const [wechatAgent, setWechatAgent] = useState<Agent | null>(null);
  const [wechatModel, setWechatModel] = useState<string | null>(null);
  const [wechatEffort, setWechatEffort] = useState<string | null>(null);
  const [wechatWorkspace, setWechatWorkspace] = useState("");
  const [wechatBindings, setWechatBindings] = useState<ImBridgeWorkspaceBinding[]>([]);
  const [wechatBindingUserId, setWechatBindingUserId] = useState("");
  const [wechatBindingWorkspace, setWechatBindingWorkspace] = useState("");
  const [wechatBaseUrl, setWechatBaseUrl] = useState<string | null>(null);
  const [wechatBotId, setWechatBotId] = useState<string | null>(null);
  const [wechatUserId, setWechatUserId] = useState<string | null>(null);
  const [wechatQrContent, setWechatQrContent] = useState<string | null>(null);
  const [wechatQrImageContent, setWechatQrImageContent] = useState<string | null>(null);
  const [wechatQrId, setWechatQrId] = useState<string | null>(null);
  const [wechatQrPolling, setWechatQrPolling] = useState(false);
  const [wechatQrPollBaseUrl, setWechatQrPollBaseUrl] = useState<string | null>(null);
  const [wechatQrStatus, setWechatQrStatus] = useState<"idle" | "loading" | "waiting" | "scanned" | "confirmed" | "expired" | "error">("idle");

  const load = async () => {
    setLoading(true);
    setSaveStatus("idle");
    try {
      const [bridgeConfig, projectRows, runtimeAgentRows] = await Promise.all([
        getImBridgeConfig(),
        listProjects(),
        listRuntimeAgents(),
      ]);
      setConfig(bridgeConfig);
      setProjects(projectRows);
      setRuntimeAgents(runtimeAgentRows);
      const telegram = bridgeConfig.telegram ?? defaultTelegramBridgeConfig();
      const discord = bridgeConfig.discord ?? defaultDiscordBridgeConfig();
      const feishu = bridgeConfig.feishu ?? defaultFeishuBridgeConfig();
      const wechat = bridgeConfig.wechat ?? defaultWechatBridgeConfig();
      setTelegramEnabled(Boolean(bridgeConfig.enabled && telegram.enabled));
      setBotToken(telegram.botToken ?? "");
      setAllowedUserIds(telegram.allowedUserIds ?? []);
      const resolvedSelection = resolveTelegramAgentSelection(runtimeAgentRows, telegram);
      setTelegramAgent(resolvedSelection.agent);
      setTelegramModel(resolvedSelection.model);
      setTelegramEffort(resolvedSelection.effort);
      setTelegramWorkspace(telegram.defaultWorkspace ?? telegram.allowedWorkspaces[0] ?? "");
      setBindings(telegram.workspaceBindings ?? []);
      setDiscordEnabled(Boolean(bridgeConfig.enabled && discord.enabled));
      setDiscordBotToken(discord.botToken ?? "");
      const resolvedDiscordSelection = resolveRuntimeSelection(runtimeAgentRows, discord);
      setDiscordAgent(resolvedDiscordSelection.agent);
      setDiscordModel(resolvedDiscordSelection.model);
      setDiscordEffort(resolvedDiscordSelection.effort);
      setDiscordWorkspace(discord.defaultWorkspace ?? discord.allowedWorkspaces[0] ?? "");
      setDiscordBindings(discord.workspaceBindings ?? []);
      setDiscordAllowedServerIds(discord.allowedServerIds ?? []);
      setDiscordAllowedChannelIds(discord.allowedChannelIds ?? []);
      setDiscordMentionOnly(discord.mentionOnly ?? true);
      setFeishuEnabled(Boolean(bridgeConfig.enabled && feishu.enabled));
      setFeishuAppId(feishu.appId ?? "");
      setFeishuAppSecret(feishu.appSecret ?? "");
      const resolvedFeishuSelection = resolveRuntimeSelection(runtimeAgentRows, feishu);
      setFeishuAgent(resolvedFeishuSelection.agent);
      setFeishuModel(resolvedFeishuSelection.model);
      setFeishuEffort(resolvedFeishuSelection.effort);
      setFeishuWorkspace(feishu.defaultWorkspace ?? feishu.allowedWorkspaces[0] ?? "");
      setFeishuBindings(feishu.workspaceBindings ?? []);
      setFeishuDomain(feishu.domain ?? null);
      setWechatEnabled(Boolean(bridgeConfig.enabled && wechat.enabled));
      setWechatBotToken(wechat.botToken ?? "");
      const resolvedWechatSelection = resolveRuntimeSelection(runtimeAgentRows, wechat);
      setWechatAgent(resolvedWechatSelection.agent);
      setWechatModel(resolvedWechatSelection.model);
      setWechatEffort(resolvedWechatSelection.effort);
      setWechatWorkspace(wechat.defaultWorkspace ?? wechat.allowedWorkspaces[0] ?? "");
      setWechatBindings(wechat.workspaceBindings ?? []);
      setWechatBaseUrl(wechat.baseUrl ?? null);
      setWechatBotId(wechat.botId ?? null);
      setWechatUserId(wechat.userId ?? null);
      setWechatQrContent(null);
      setWechatQrImageContent(null);
      setWechatQrId(null);
      setWechatQrPolling(false);
      setWechatQrPollBaseUrl(null);
      setWechatQrStatus("idle");
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const selectedTelegramRuntimeAgent = useMemo(
    () => runtimeAgents.find((agent) => agent.agent === telegramAgent && agent.enabled) ?? firstEnabledRuntimeAgent(runtimeAgents),
    [runtimeAgents, telegramAgent],
  );
  const selectedTelegramAgent = selectedTelegramRuntimeAgent?.agent ?? telegramAgent;
  const enabledRuntimeAgents = useMemo(
    () => runtimeAgents.filter((agent) => agent.enabled),
    [runtimeAgents],
  );
  const telegramModelValue = selectedTelegramAgent && telegramModel
    ? agentModelSelectValue(selectedTelegramAgent, telegramModel)
    : "";
  const modelOptions = useMemo(() => {
    const effortControls = Object.fromEntries(
      enabledRuntimeAgents.map((runtimeAgent) => [
        runtimeAgent.agent,
        <RuntimeEffortControl
          value={runtimeAgent.agent === selectedTelegramAgent ? telegramEffort ?? initialRuntimeEffort(runtimeAgent) : initialRuntimeEffort(runtimeAgent)}
          options={runtimeEffortOptions(runtimeAgent)}
          onChange={(value) => {
            if (runtimeAgent.agent !== selectedTelegramAgent) {
              setTelegramAgent(runtimeAgent.agent);
              setTelegramModel(defaultRuntimeModel(runtimeAgent));
            }
            setTelegramEffort(value);
          }}
        />,
      ]),
    ) as Partial<Record<Agent, ReactNode>>;
    const selectedEfforts = selectedTelegramAgent && telegramEffort ? { [selectedTelegramAgent]: telegramEffort } : {};
    const options = agentModelSelectOptions(enabledRuntimeAgents, effortControls, selectedEfforts);
    const selectedExists = options.some((option) => option.value === telegramModelValue);
    return [
      ...(!selectedExists && selectedTelegramAgent && telegramModel
        ? [{
          value: telegramModelValue,
          label: telegramModel,
          icon: <AgentGlyph agent={selectedTelegramAgent} className="h-3.5 w-3.5" />,
        }]
        : []),
      ...options,
    ];
  }, [enabledRuntimeAgents, selectedTelegramAgent, telegramEffort, telegramModel, telegramModelValue]);
  useEffect(() => {
    if (runtimeAgents.length === 0) return;
    const selected = selectedTelegramRuntimeAgent ?? firstEnabledRuntimeAgent(runtimeAgents);
    if (!selected) return;
    if (telegramAgent !== selected.agent) {
      setTelegramAgent(selected.agent);
    }
    if (!telegramModel) {
      setTelegramModel(defaultRuntimeModel(selected));
    }
    if (!telegramEffort) {
      setTelegramEffort(initialRuntimeEffort(selected) || null);
    }
  }, [runtimeAgents, selectedTelegramRuntimeAgent, telegramAgent, telegramModel, telegramEffort]);
  const workspaceChoices = useMemo(
    () => uniqueStrings([
      ...projects.map((project) => project.path),
      telegramWorkspace,
      ...bindings.map((binding) => binding.workspacePath),
    ]),
    [bindings, telegramWorkspace, projects],
  );
  const selectedDiscordRuntimeAgent = useMemo(
    () => runtimeAgents.find((agent) => agent.agent === discordAgent && agent.enabled) ?? firstEnabledRuntimeAgent(runtimeAgents),
    [runtimeAgents, discordAgent],
  );
  const selectedDiscordAgent = selectedDiscordRuntimeAgent?.agent ?? discordAgent;
  const discordModelValue = selectedDiscordAgent && discordModel
    ? agentModelSelectValue(selectedDiscordAgent, discordModel)
    : "";
  const discordModelOptions = useMemo(() => {
    const effortControls = Object.fromEntries(
      enabledRuntimeAgents.map((runtimeAgent) => [
        runtimeAgent.agent,
        <RuntimeEffortControl
          value={runtimeAgent.agent === selectedDiscordAgent ? discordEffort ?? initialRuntimeEffort(runtimeAgent) : initialRuntimeEffort(runtimeAgent)}
          options={runtimeEffortOptions(runtimeAgent)}
          onChange={(value) => {
            if (runtimeAgent.agent !== selectedDiscordAgent) {
              setDiscordAgent(runtimeAgent.agent);
              setDiscordModel(defaultRuntimeModel(runtimeAgent));
            }
            setDiscordEffort(value);
          }}
        />,
      ]),
    ) as Partial<Record<Agent, ReactNode>>;
    const selectedEfforts = selectedDiscordAgent && discordEffort ? { [selectedDiscordAgent]: discordEffort } : {};
    const options = agentModelSelectOptions(enabledRuntimeAgents, effortControls, selectedEfforts);
    const selectedExists = options.some((option) => option.value === discordModelValue);
    return [
      ...(!selectedExists && selectedDiscordAgent && discordModel
        ? [{
          value: discordModelValue,
          label: discordModel,
          icon: <AgentGlyph agent={selectedDiscordAgent} className="h-3.5 w-3.5" />,
        }]
        : []),
      ...options,
    ];
  }, [enabledRuntimeAgents, selectedDiscordAgent, discordEffort, discordModel, discordModelValue]);
  useEffect(() => {
    if (runtimeAgents.length === 0) return;
    const selected = selectedDiscordRuntimeAgent ?? firstEnabledRuntimeAgent(runtimeAgents);
    if (!selected) return;
    if (discordAgent !== selected.agent) {
      setDiscordAgent(selected.agent);
    }
    if (!discordModel) {
      setDiscordModel(defaultRuntimeModel(selected));
    }
    if (!discordEffort) {
      setDiscordEffort(initialRuntimeEffort(selected) || null);
    }
  }, [runtimeAgents, selectedDiscordRuntimeAgent, discordAgent, discordModel, discordEffort]);
  const discordWorkspaceChoices = useMemo(
    () => uniqueStrings([
      ...projects.map((project) => project.path),
      discordWorkspace,
      ...discordBindings.map((binding) => binding.workspacePath),
    ]),
    [discordBindings, discordWorkspace, projects],
  );
  const selectedFeishuRuntimeAgent = useMemo(
    () => runtimeAgents.find((agent) => agent.agent === feishuAgent && agent.enabled) ?? firstEnabledRuntimeAgent(runtimeAgents),
    [runtimeAgents, feishuAgent],
  );
  const selectedFeishuAgent = selectedFeishuRuntimeAgent?.agent ?? feishuAgent;
  const feishuModelValue = selectedFeishuAgent && feishuModel
    ? agentModelSelectValue(selectedFeishuAgent, feishuModel)
    : "";
  const feishuModelOptions = useMemo(() => {
    const effortControls = Object.fromEntries(
      enabledRuntimeAgents.map((runtimeAgent) => [
        runtimeAgent.agent,
        <RuntimeEffortControl
          value={runtimeAgent.agent === selectedFeishuAgent ? feishuEffort ?? initialRuntimeEffort(runtimeAgent) : initialRuntimeEffort(runtimeAgent)}
          options={runtimeEffortOptions(runtimeAgent)}
          onChange={(value) => {
            if (runtimeAgent.agent !== selectedFeishuAgent) {
              setFeishuAgent(runtimeAgent.agent);
              setFeishuModel(defaultRuntimeModel(runtimeAgent));
            }
            setFeishuEffort(value);
          }}
        />,
      ]),
    ) as Partial<Record<Agent, ReactNode>>;
    const selectedEfforts = selectedFeishuAgent && feishuEffort ? { [selectedFeishuAgent]: feishuEffort } : {};
    const options = agentModelSelectOptions(enabledRuntimeAgents, effortControls, selectedEfforts);
    const selectedExists = options.some((option) => option.value === feishuModelValue);
    return [
      ...(!selectedExists && selectedFeishuAgent && feishuModel
        ? [{
          value: feishuModelValue,
          label: feishuModel,
          icon: <AgentGlyph agent={selectedFeishuAgent} className="h-3.5 w-3.5" />,
        }]
        : []),
      ...options,
    ];
  }, [enabledRuntimeAgents, selectedFeishuAgent, feishuEffort, feishuModel, feishuModelValue]);
  useEffect(() => {
    if (runtimeAgents.length === 0) return;
    const selected = selectedFeishuRuntimeAgent ?? firstEnabledRuntimeAgent(runtimeAgents);
    if (!selected) return;
    if (feishuAgent !== selected.agent) {
      setFeishuAgent(selected.agent);
    }
    if (!feishuModel) {
      setFeishuModel(defaultRuntimeModel(selected));
    }
    if (!feishuEffort) {
      setFeishuEffort(initialRuntimeEffort(selected) || null);
    }
  }, [runtimeAgents, selectedFeishuRuntimeAgent, feishuAgent, feishuModel, feishuEffort]);
  const feishuWorkspaceChoices = useMemo(
    () => uniqueStrings([
      ...projects.map((project) => project.path),
      feishuWorkspace,
      ...feishuBindings.map((binding) => binding.workspacePath),
    ]),
    [feishuBindings, feishuWorkspace, projects],
  );
  const selectedWechatRuntimeAgent = useMemo(
    () => runtimeAgents.find((agent) => agent.agent === wechatAgent && agent.enabled) ?? firstEnabledRuntimeAgent(runtimeAgents),
    [runtimeAgents, wechatAgent],
  );
  const selectedWechatAgent = selectedWechatRuntimeAgent?.agent ?? wechatAgent;
  const wechatModelValue = selectedWechatAgent && wechatModel
    ? agentModelSelectValue(selectedWechatAgent, wechatModel)
    : "";
  const wechatModelOptions = useMemo(() => {
    const effortControls = Object.fromEntries(
      enabledRuntimeAgents.map((runtimeAgent) => [
        runtimeAgent.agent,
        <RuntimeEffortControl
          value={runtimeAgent.agent === selectedWechatAgent ? wechatEffort ?? initialRuntimeEffort(runtimeAgent) : initialRuntimeEffort(runtimeAgent)}
          options={runtimeEffortOptions(runtimeAgent)}
          onChange={(value) => {
            if (runtimeAgent.agent !== selectedWechatAgent) {
              setWechatAgent(runtimeAgent.agent);
              setWechatModel(defaultRuntimeModel(runtimeAgent));
            }
            setWechatEffort(value);
          }}
        />,
      ]),
    ) as Partial<Record<Agent, ReactNode>>;
    const selectedEfforts = selectedWechatAgent && wechatEffort ? { [selectedWechatAgent]: wechatEffort } : {};
    const options = agentModelSelectOptions(enabledRuntimeAgents, effortControls, selectedEfforts);
    const selectedExists = options.some((option) => option.value === wechatModelValue);
    return [
      ...(!selectedExists && selectedWechatAgent && wechatModel
        ? [{
          value: wechatModelValue,
          label: wechatModel,
          icon: <AgentGlyph agent={selectedWechatAgent} className="h-3.5 w-3.5" />,
        }]
        : []),
      ...options,
    ];
  }, [enabledRuntimeAgents, selectedWechatAgent, wechatEffort, wechatModel, wechatModelValue]);
  useEffect(() => {
    if (runtimeAgents.length === 0) return;
    const selected = selectedWechatRuntimeAgent ?? firstEnabledRuntimeAgent(runtimeAgents);
    if (!selected) return;
    if (wechatAgent !== selected.agent) {
      setWechatAgent(selected.agent);
    }
    if (!wechatModel) {
      setWechatModel(defaultRuntimeModel(selected));
    }
    if (!wechatEffort) {
      setWechatEffort(initialRuntimeEffort(selected) || null);
    }
  }, [runtimeAgents, selectedWechatRuntimeAgent, wechatAgent, wechatModel, wechatEffort]);
  const wechatWorkspaceChoices = useMemo(
    () => uniqueStrings([
      ...projects.map((project) => project.path),
      wechatWorkspace,
      ...wechatBindings.map((binding) => binding.workspacePath),
    ]),
    [wechatBindings, wechatWorkspace, projects],
  );
  const telegramApiBase = config?.telegram?.apiBase ?? null;
  const discordApiBase = config?.discord?.apiBase ?? null;
  const effectiveFeishuDomain = feishuDomain?.trim() || null;
  const effectiveWechatBaseUrl = wechatBaseUrl?.trim() || null;
  const wechatQrImage = wechatQrImageSrc(wechatQrImageContent);
  const channelTabs = useMemo(
    () => [
      { value: "telegram" as const, label: "Telegram", icon: TelegramLogoIcon },
      { value: "discord" as const, label: "Discord", icon: DiscordLogoIcon },
      { value: "feishu" as const, label: t("settings.feishu_platform"), icon: LarkLogoIcon },
      { value: "wechat" as const, label: "WeChat", icon: WechatLogoIcon },
    ],
    [t],
  );

  const addAllowedUserId = () => {
    const value = Number(userIdInput.trim());
    if (!Number.isSafeInteger(value)) {
      onError(t("settings.channels_invalid_user_id"));
      return;
    }
    setAllowedUserIds((current) => current.includes(value) ? current : [...current, value]);
    setUserIdInput("");
    onError(null);
  };

  const detectUserIds = async () => {
    if (!botToken.trim() || detecting) return;
    setDetecting(true);
    try {
      const ids = await detectTelegramUserIds(botToken.trim(), telegramApiBase);
      setAllowedUserIds((current) => uniqueNumbers([...current, ...ids]));
      onError(null);
    } catch (err) {
      onError(String(err));
    } finally {
      setDetecting(false);
    }
  };

  const sendTest = async () => {
    if (!botToken.trim() || testing) return;
    setTesting(true);
    setTestStatus("idle");
    try {
      await testTelegramBotConnection(botToken.trim(), telegramApiBase);
      setTestStatus("success");
      onError(null);
    } catch (err) {
      setTestStatus("error");
      onError(String(err));
    } finally {
      setTesting(false);
    }
  };

  const sendDiscordTest = async () => {
    if (!discordBotToken.trim() || discordTesting) return;
    setDiscordTesting(true);
    setDiscordTestStatus("idle");
    try {
      await testDiscordBotConnection(discordBotToken.trim(), discordApiBase);
      setDiscordTestStatus("success");
      onError(null);
    } catch (err) {
      setDiscordTestStatus("error");
      onError(String(err));
    } finally {
      setDiscordTesting(false);
    }
  };

  const sendFeishuTest = async () => {
    if (!feishuAppId.trim() || !feishuAppSecret.trim() || feishuTesting) return;
    setFeishuTesting(true);
    setFeishuTestStatus("idle");
    try {
      await testFeishuBotConnection(feishuAppId.trim(), feishuAppSecret.trim(), effectiveFeishuDomain);
      setFeishuTestStatus("success");
      onError(null);
    } catch (err) {
      setFeishuTestStatus("error");
      onError(String(err));
    } finally {
      setFeishuTesting(false);
    }
  };

  const buildNextImBridgeConfig = (
    wechatOverrides: Partial<WechatBridgeConfig> = {},
    forceWechatEnabled?: boolean,
  ): ImBridgeConfig => {
    const nextTelegram: TelegramBridgeConfig = {
      ...(config?.telegram ?? defaultTelegramBridgeConfig()),
      enabled: telegramEnabled,
      agent: selectedTelegramAgent ?? null,
      model: telegramModel?.trim() || null,
      effort: telegramEffort?.trim() || null,
      defaultWorkspace: telegramWorkspace.trim() || null,
      allowedWorkspaces: uniqueStrings([
        telegramWorkspace.trim(),
        ...bindings.map((binding) => binding.workspacePath),
      ]),
      workspaceBindings: bindings
        .map((binding) => ({
          platform: "telegram",
          chatId: binding.chatId.trim(),
          workspacePath: binding.workspacePath.trim(),
        }))
        .filter((binding) => binding.chatId && binding.workspacePath),
      botToken: botToken.trim(),
      allowedUserIds,
    };
    const nextDiscord: DiscordBridgeConfig = {
      ...(config?.discord ?? defaultDiscordBridgeConfig()),
      enabled: discordEnabled,
      agent: selectedDiscordAgent ?? null,
      model: discordModel?.trim() || null,
      effort: discordEffort?.trim() || null,
      defaultWorkspace: discordWorkspace.trim() || null,
      allowedWorkspaces: uniqueStrings([
        discordWorkspace.trim(),
        ...discordBindings.map((binding) => binding.workspacePath),
      ]),
      workspaceBindings: discordBindings
        .map((binding) => ({
          platform: "discord",
          chatId: binding.chatId.trim(),
          workspacePath: binding.workspacePath.trim(),
        }))
        .filter((binding) => binding.chatId && binding.workspacePath),
      botToken: discordBotToken.trim(),
      allowedServerIds: uniqueStrings(discordAllowedServerIds),
      allowedChannelIds: uniqueStrings(discordAllowedChannelIds),
      mentionOnly: discordMentionOnly,
    };
    const nextFeishu: FeishuBridgeConfig = {
      ...(config?.feishu ?? defaultFeishuBridgeConfig()),
      enabled: feishuEnabled,
      agent: selectedFeishuAgent ?? null,
      model: feishuModel?.trim() || null,
      effort: feishuEffort?.trim() || null,
      defaultWorkspace: feishuWorkspace.trim() || null,
      allowedWorkspaces: uniqueStrings([
        feishuWorkspace.trim(),
        ...feishuBindings.map((binding) => binding.workspacePath),
      ]),
      workspaceBindings: feishuBindings
        .map((binding) => ({
          platform: "feishu",
          chatId: binding.chatId.trim(),
          workspacePath: binding.workspacePath.trim(),
        }))
        .filter((binding) => binding.chatId && binding.workspacePath),
      appId: feishuAppId.trim(),
      appSecret: feishuAppSecret.trim(),
      domain: effectiveFeishuDomain,
    };
    const nextWechatEnabled = forceWechatEnabled ?? wechatEnabled;
    const nextWechat: WechatBridgeConfig = {
      ...(config?.wechat ?? defaultWechatBridgeConfig()),
      enabled: nextWechatEnabled,
      agent: wechatOverrides.agent !== undefined ? wechatOverrides.agent : selectedWechatAgent ?? null,
      model: wechatOverrides.model !== undefined ? wechatOverrides.model : wechatModel?.trim() || null,
      effort: wechatOverrides.effort !== undefined ? wechatOverrides.effort : wechatEffort?.trim() || null,
      defaultWorkspace: wechatOverrides.defaultWorkspace !== undefined ? wechatOverrides.defaultWorkspace : wechatWorkspace.trim() || null,
      allowedWorkspaces: wechatOverrides.allowedWorkspaces ?? uniqueStrings([
        wechatWorkspace.trim(),
        ...wechatBindings.map((binding) => binding.workspacePath),
      ]),
      workspaceBindings: wechatOverrides.workspaceBindings ?? wechatBindings
        .map((binding) => ({
          platform: "wechat",
          chatId: binding.chatId.trim(),
          workspacePath: binding.workspacePath.trim(),
        }))
        .filter((binding) => binding.chatId && binding.workspacePath),
      botToken: wechatOverrides.botToken !== undefined ? wechatOverrides.botToken : wechatBotToken.trim(),
      botId: wechatOverrides.botId !== undefined ? wechatOverrides.botId : wechatBotId,
      userId: wechatOverrides.userId !== undefined ? wechatOverrides.userId : wechatUserId,
      baseUrl: wechatOverrides.baseUrl !== undefined ? wechatOverrides.baseUrl : effectiveWechatBaseUrl,
      pollTimeoutSecs: wechatOverrides.pollTimeoutSecs ?? config?.wechat?.pollTimeoutSecs ?? 40,
    };
    return {
      ...(config ?? defaultImBridgeConfig()),
      enabled: telegramEnabled || discordEnabled || feishuEnabled || nextWechatEnabled,
      telegram: nextTelegram,
      discord: nextDiscord,
      feishu: nextFeishu,
      wechat: nextWechat,
    };
  };

  const sendWechatTest = async () => {
    if (!wechatBotToken.trim() || wechatTesting) return;
    setWechatTesting(true);
    setWechatTestStatus("idle");
    try {
      await testWechatBotConnection(wechatBotToken.trim(), effectiveWechatBaseUrl);
      setWechatTestStatus("success");
      onError(null);
    } catch (err) {
      setWechatTestStatus("error");
      onError(String(err));
    } finally {
      setWechatTesting(false);
    }
  };

  const startWechatQrLogin = async () => {
    if (wechatQrStatus === "loading") return;
    setWechatQrPolling(false);
    setWechatQrPollBaseUrl(null);
    setWechatQrStatus("loading");
    try {
      const qr = await getWechatQrcode(effectiveWechatBaseUrl);
      setWechatQrContent(qr.qrcodeContent);
      setWechatQrImageContent(qr.qrcodeImageContent ?? null);
      setWechatQrId(qr.qrcodeId);
      setWechatQrPolling(true);
      setWechatQrStatus("waiting");
      onError(null);
    } catch (err) {
      setWechatQrStatus("error");
      onError(String(err));
    }
  };

  const pollWechatQrLogin = async () => {
    if (!wechatQrId || wechatQrStatus === "loading" || wechatQrPolling) return;
    setWechatQrStatus("loading");
    try {
      const status = await pollWechatQrcodeStatus(wechatQrId, wechatQrPollBaseUrl);
      await applyWechatQrStatus(status);
    } catch (err) {
      setWechatQrStatus("error");
      onError(String(err));
    }
  };

  const applyWechatQrStatus = async (status: WechatQrStatus) => {
    const normalized = status.status === "scaned" ? "scanned" : status.status;
    if (normalized === "scannedRedirect") {
      if (status.redirectHost) {
        setWechatQrPollBaseUrl(`https://${status.redirectHost}`);
      }
      setWechatQrStatus("scanned");
      onError(null);
      return;
    }
    if (normalized === "alreadyConnected") {
      setWechatQrPolling(false);
      setWechatQrStatus("confirmed");
      onError(null);
      return;
    }
    if (normalized === "needVerifyCode" || normalized === "verifyCodeBlocked") {
      setWechatQrPolling(false);
      setWechatQrStatus("error");
      onError(status.error ?? normalized);
      return;
    }
    if (normalized === "confirmed" && status.botToken) {
      const nextBaseUrl = status.baseUrl ?? effectiveWechatBaseUrl;
      setWechatBotToken(status.botToken);
      setWechatBotId(status.botId ?? null);
      setWechatUserId(status.userId ?? null);
      setWechatBaseUrl(nextBaseUrl);
      setWechatEnabled(true);
      setWechatTestStatus("success");
      setSaveStatus("idle");
      setSaving(true);
      try {
        const saved = await updateImBridgeConfig(buildNextImBridgeConfig({
          enabled: true,
          botToken: status.botToken,
          botId: status.botId ?? null,
          userId: status.userId ?? null,
          baseUrl: nextBaseUrl,
        }, true));
        setConfig(saved);
        setSaveStatus("success");
        setWechatQrPolling(false);
        setWechatQrStatus("confirmed");
        onError(null);
      } catch (err) {
        setSaveStatus("error");
        setWechatQrPolling(false);
        setWechatQrStatus("error");
        onError(String(err));
      } finally {
        setSaving(false);
      }
      return;
    }
    if (normalized === "scanned" || normalized === "waiting" || normalized === "expired" || normalized === "error") {
      if (normalized === "expired" || normalized === "error") {
        setWechatQrPolling(false);
      }
      setWechatQrStatus(normalized);
      onError(status.error ? status.error : null);
      return;
    }
    setWechatQrStatus("waiting");
    onError(status.error ? status.error : null);
  };

  useEffect(() => {
    if (!wechatQrId || !wechatQrPolling) return;
    let cancelled = false;
    const pollBaseUrl = wechatQrPollBaseUrl;
    const run = async () => {
      while (!cancelled) {
        try {
          const status = await pollWechatQrcodeStatus(wechatQrId, pollBaseUrl);
          if (cancelled) return;
          await applyWechatQrStatus(status);
          if (
            status.status === "confirmed"
            || status.status === "expired"
            || status.status === "error"
            || status.status === "needVerifyCode"
            || status.status === "verifyCodeBlocked"
            || status.status === "alreadyConnected"
          ) {
            return;
          }
        } catch (err) {
          if (!cancelled) {
            onError(String(err));
          }
        }
        await new Promise((resolve) => window.setTimeout(resolve, 1000));
      }
    };
    void run();
    return () => {
      cancelled = true;
    };
  }, [wechatQrId, wechatQrPolling, wechatQrPollBaseUrl]);

  useEffect(() => {
    setSaveStatus("idle");
  }, [
    telegramEnabled,
    botToken,
    allowedUserIds,
    selectedTelegramAgent,
    telegramModel,
    telegramEffort,
    telegramWorkspace,
    bindings,
    discordEnabled,
    discordBotToken,
    selectedDiscordAgent,
    discordModel,
    discordEffort,
    discordWorkspace,
    discordBindings,
    discordAllowedServerIds,
    discordAllowedChannelIds,
    discordMentionOnly,
    feishuEnabled,
    feishuAppId,
    feishuAppSecret,
    selectedFeishuAgent,
    feishuModel,
    feishuEffort,
    feishuWorkspace,
    feishuBindings,
    feishuDomain,
    wechatEnabled,
    wechatBotToken,
    selectedWechatAgent,
    wechatModel,
    wechatEffort,
    wechatWorkspace,
    wechatBindings,
    wechatBaseUrl,
  ]);

  const addBinding = () => {
    const chatId = bindingChatId.trim();
    if (!chatId || !bindingWorkspace) return;
    setBindings((current) => [
      ...current.filter((binding) => binding.chatId !== chatId),
      { platform: "telegram", chatId, workspacePath: bindingWorkspace },
    ]);
    setBindingChatId("");
  };

  const addDiscordServerId = () => {
    const value = discordServerIdInput.trim();
    if (!value) return;
    setDiscordAllowedServerIds((current) => current.includes(value) ? current : [...current, value]);
    setDiscordServerIdInput("");
  };

  const addDiscordChannelId = () => {
    const value = discordChannelIdInput.trim();
    if (!value) return;
    setDiscordAllowedChannelIds((current) => current.includes(value) ? current : [...current, value]);
    setDiscordChannelIdInput("");
  };

  const addDiscordBinding = () => {
    const chatId = discordBindingChannelId.trim();
    if (!chatId || !discordBindingWorkspace) return;
    setDiscordBindings((current) => [
      ...current.filter((binding) => binding.chatId !== chatId),
      { platform: "discord", chatId, workspacePath: discordBindingWorkspace },
    ]);
    setDiscordBindingChannelId("");
  };

  const addFeishuBinding = () => {
    const chatId = feishuBindingChatId.trim();
    if (!chatId || !feishuBindingWorkspace) return;
    setFeishuBindings((current) => [
      ...current.filter((binding) => binding.chatId !== chatId),
      { platform: "feishu", chatId, workspacePath: feishuBindingWorkspace },
    ]);
    setFeishuBindingChatId("");
  };

  const addWechatBinding = () => {
    const chatId = wechatBindingUserId.trim();
    if (!chatId || !wechatBindingWorkspace) return;
    setWechatBindings((current) => [
      ...current.filter((binding) => binding.chatId !== chatId),
      { platform: "wechat", chatId, workspacePath: wechatBindingWorkspace },
    ]);
    setWechatBindingUserId("");
  };

  const save = async () => {
    if (saving) return;
    setSaving(true);
    setSaveStatus("idle");
    try {
      const saved = await updateImBridgeConfig(buildNextImBridgeConfig());
      setConfig(saved);
      setSaveStatus("success");
      onError(null);
    } catch (err) {
      setSaveStatus("error");
      onError(String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="min-w-0 max-w-full">
      <div className="mb-4">
        <SegmentedTabs<ChannelPlatform>
          items={channelTabs}
          value={activePlatform}
          onChange={setActivePlatform}
          itemHeight={30}
          fullWidth
        />
      </div>

      {activePlatform === "telegram" ? (
        <>
          <SettingsGroup
            title={t("settings.telegram_bot")}
            action={
              <button type="button" disabled={saving || loading} onClick={() => void save()} className={channelSaveButtonClass(saveStatus)}>
                {saving
                  ? <LoaderCircle className="h-4 w-4 animate-spin" />
                  : saveStatus === "error"
                    ? <X className="h-4 w-4" />
                    : <Check className="h-4 w-4" />}
                {saving
                  ? t("project.save")
                  : saveStatus === "success"
                    ? t("settings.saved")
                    : saveStatus === "error"
                      ? t("settings.save_failed")
                      : t("project.save")}
              </button>
            }
            flush
          >
            <SettingsRow icon={<TelegramLogoIcon className="h-4 w-4" />} label={t("settings.telegram_enable")} description={t("settings.telegram_enable_description")}>
              <div className="flex items-center gap-3">
                <span className="text-caption text-ink/45">{telegramEnabled ? t("settings.proxy_enabled") : t("settings.proxy_disabled")}</span>
                <SwitchControl checked={telegramEnabled} tooltip={t("settings.telegram_enable")} onToggle={() => setTelegramEnabled((value) => !value)} />
              </div>
            </SettingsRow>
            <SettingsStackedRow icon={<TokenOutlineIcon className="h-4 w-4" />} label={t("settings.telegram_bot_token")} description={t("settings.telegram_bot_token_description")}>
              <div className="flex w-full min-w-0 flex-wrap items-center gap-2">
                <input value={botToken} type={showToken ? "text" : "password"} onChange={(event) => {
                  setBotToken(event.target.value);
                  setTestStatus("idle");
                }} placeholder="123456:ABC-DEF..." className={inputClassName + " min-w-0 flex-1"} />
                <button type="button" onClick={() => setShowToken((value) => !value)} className="inline-flex h-9 w-9 items-center justify-center rounded-md text-ink/55 transition hover:bg-ink/[0.06] hover:text-ink" aria-label={showToken ? t("settings.telegram_hide_token") : t("settings.telegram_show_token")}>
                  {showToken ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                </button>
                <button type="button" disabled={testing || !botToken.trim()} onClick={() => void sendTest()} className={telegramTestButtonClass(testStatus)}>
                  {testing ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Link2 className="h-4 w-4" />}
                  {testing
                    ? t("settings.testing_connection")
                    : testStatus === "success"
                      ? t("settings.connected")
                      : testStatus === "error"
                        ? t("settings.connection_failed")
                        : t("settings.test_connection")}
                </button>
              </div>
            </SettingsStackedRow>
            <SettingsStackedRow icon={<Bot className="h-4 w-4" />} label={t("settings.telegram_allowed_users")} description={t("settings.telegram_allowed_users_description")}>
              <div className="flex w-full min-w-0 flex-col gap-2">
                <div className="flex w-full min-w-0 flex-wrap items-center gap-2">
                  <input value={userIdInput} onChange={(event) => setUserIdInput(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") addAllowedUserId(); }} placeholder="Telegram user ID" className={inputClassName + " min-w-[180px] flex-1"} />
                  <button type="button" onClick={addAllowedUserId} className="inline-flex h-9 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12]">
                    <Plus className="h-4 w-4" />
                    {t("settings.add")}
                  </button>
                  <button type="button" disabled={detecting || !botToken.trim()} onClick={() => void detectUserIds()} className="inline-flex h-9 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-35">
                    {detecting ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                    {t("settings.telegram_detect")}
                  </button>
                </div>
                <div className="flex max-w-full flex-wrap gap-1.5">
                  {allowedUserIds.map((id) => (
                    <button key={id} type="button" onClick={() => setAllowedUserIds((current) => current.filter((item) => item !== id))} className="inline-flex h-7 items-center gap-1 rounded-md bg-ink/[0.07] px-2 text-caption text-ink/70 transition hover:bg-ink/[0.1]">
                      {id}
                      <X className="h-3 w-3" />
                    </button>
                  ))}
                </div>
              </div>
            </SettingsStackedRow>
            <SettingsRow icon={<Bot className="h-4 w-4" />} label={t("settings.telegram_default_model")} description={t("settings.telegram_default_model_description")}>
              <RuntimeMenuSelect
                ariaLabel={t("settings.telegram_default_model")}
                value={telegramModelValue}
                options={modelOptions}
                onChange={(value) => {
                  const parsed = parseAgentModelSelectValue(value);
                  if (!parsed) return;
                  const runtimeAgent = runtimeAgents.find((agent) => agent.agent === parsed.agent);
                  if (parsed.agent !== selectedTelegramAgent) {
                    setTelegramEffort(runtimeAgent ? initialRuntimeEffort(runtimeAgent) : null);
                  }
                  setTelegramAgent(parsed.agent);
                  setTelegramModel(parsed.model || null);
                }}
                minMenuWidth={240}
                maxWidthClassName="max-w-[300px]"
              />
            </SettingsRow>
          </SettingsGroup>

          <SettingsGroup title={t("settings.channel_workspace_bindings")} flush>
            <SettingsStackedRow icon={<SquareKanban className="h-4 w-4" />} label={t("settings.channels_default_workspace")} description={t("settings.channels_default_workspace_description")}>
              <select value={telegramWorkspace} onChange={(event) => setTelegramWorkspace(event.target.value)} className={inputClassName + " w-full"}>
                <option value="">{t("settings.channels_select_workspace")}</option>
                {workspaceChoices.map((path) => (
                  <option key={path} value={path}>{projectLabel(projects, path)}</option>
                ))}
              </select>
            </SettingsStackedRow>
            <SettingsStackedRow icon={<Server className="h-4 w-4" />} label={t("settings.channels_add_binding")} description={t("settings.channels_add_binding_description")}>
              <div className="flex w-full min-w-0 flex-wrap items-center gap-2">
                <input value={bindingChatId} onChange={(event) => setBindingChatId(event.target.value)} placeholder="e.g. 123456789" className={inputClassName + " min-w-[170px] flex-1"} />
                <select value={bindingWorkspace} onChange={(event) => setBindingWorkspace(event.target.value)} className={inputClassName + " min-w-[220px] flex-[1.4]"}>
                  <option value="">{t("settings.channels_select_workspace")}</option>
                  {workspaceChoices.map((path) => (
                    <option key={path} value={path}>{projectLabel(projects, path)}</option>
                  ))}
                </select>
                <button type="button" disabled={!bindingChatId.trim() || !bindingWorkspace} onClick={addBinding} className="inline-flex h-9 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-35">
                  <Plus className="h-4 w-4" />
                  {t("settings.add")}
                </button>
              </div>
            </SettingsStackedRow>
            {bindings.map((binding) => (
              <SettingsStackedRow key={binding.chatId} icon={<Server className="h-4 w-4" />} label={binding.chatId} description={projectLabel(projects, binding.workspacePath)}>
                <button type="button" onClick={() => setBindings((current) => current.filter((item) => item.chatId !== binding.chatId))} className="inline-flex h-8 w-8 items-center justify-center rounded-md text-ink/55 transition hover:bg-red-500/10 hover:text-red-600" aria-label={t("sidebar.remove")}>
                  <Trash2 className="h-4 w-4" />
                </button>
              </SettingsStackedRow>
            ))}
          </SettingsGroup>
        </>
      ) : activePlatform === "discord" ? (
        <>
          <SettingsGroup
            title={t("settings.discord_bot")}
            action={
              <button type="button" disabled={saving || loading} onClick={() => void save()} className={channelSaveButtonClass(saveStatus)}>
                {saving
                  ? <LoaderCircle className="h-4 w-4 animate-spin" />
                  : saveStatus === "error"
                    ? <X className="h-4 w-4" />
                    : <Check className="h-4 w-4" />}
                {saving
                  ? t("project.save")
                  : saveStatus === "success"
                    ? t("settings.saved")
                    : saveStatus === "error"
                      ? t("settings.save_failed")
                      : t("project.save")}
              </button>
            }
            flush
          >
            <SettingsRow icon={<DiscordLogoIcon className="h-4 w-4" />} label={t("settings.discord_enable")} description={t("settings.discord_enable_description")}>
              <div className="flex items-center gap-3">
                <span className="text-caption text-ink/45">{discordEnabled ? t("settings.proxy_enabled") : t("settings.proxy_disabled")}</span>
                <SwitchControl checked={discordEnabled} tooltip={t("settings.discord_enable")} onToggle={() => setDiscordEnabled((value) => !value)} />
              </div>
            </SettingsRow>
            <SettingsStackedRow icon={<TokenOutlineIcon className="h-4 w-4" />} label={t("settings.discord_bot_token")} description={t("settings.discord_bot_token_description")}>
              <div className="flex w-full min-w-0 flex-wrap items-center gap-2">
                <input value={discordBotToken} type={showDiscordToken ? "text" : "password"} onChange={(event) => {
                  setDiscordBotToken(event.target.value);
                  setDiscordTestStatus("idle");
                }} placeholder="MTIz..." className={inputClassName + " min-w-0 flex-1"} />
                <button type="button" onClick={() => setShowDiscordToken((value) => !value)} className="inline-flex h-9 w-9 items-center justify-center rounded-md text-ink/55 transition hover:bg-ink/[0.06] hover:text-ink" aria-label={showDiscordToken ? t("settings.telegram_hide_token") : t("settings.telegram_show_token")}>
                  {showDiscordToken ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                </button>
                <button type="button" disabled={discordTesting || !discordBotToken.trim()} onClick={() => void sendDiscordTest()} className={telegramTestButtonClass(discordTestStatus)}>
                  {discordTesting ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Link2 className="h-4 w-4" />}
                  {discordTesting
                    ? t("settings.testing_connection")
                    : discordTestStatus === "success"
                      ? t("settings.connected")
                      : discordTestStatus === "error"
                        ? t("settings.connection_failed")
                        : t("settings.test_connection")}
                </button>
              </div>
            </SettingsStackedRow>
            <SettingsRow icon={<Bot className="h-4 w-4" />} label={t("settings.discord_default_model")} description={t("settings.discord_default_model_description")}>
              <RuntimeMenuSelect
                ariaLabel={t("settings.discord_default_model")}
                value={discordModelValue}
                options={discordModelOptions}
                onChange={(value) => {
                  const parsed = parseAgentModelSelectValue(value);
                  if (!parsed) return;
                  const runtimeAgent = runtimeAgents.find((agent) => agent.agent === parsed.agent);
                  if (parsed.agent !== selectedDiscordAgent) {
                    setDiscordEffort(runtimeAgent ? initialRuntimeEffort(runtimeAgent) : null);
                  }
                  setDiscordAgent(parsed.agent);
                  setDiscordModel(parsed.model || null);
                }}
                minMenuWidth={240}
                maxWidthClassName="max-w-[300px]"
              />
            </SettingsRow>
          </SettingsGroup>

          <SettingsGroup title={t("settings.discord_access_control")} flush>
            <SettingsStackedRow icon={<Shield className="h-4 w-4" />} label={t("settings.discord_allowed_servers")} description={t("settings.discord_allowed_servers_description")}>
              <IdListEditor
                value={discordServerIdInput}
                placeholder={t("settings.discord_server_id_placeholder")}
                ids={discordAllowedServerIds}
                addLabel={t("settings.add")}
                onInputChange={setDiscordServerIdInput}
                onAdd={addDiscordServerId}
                onRemove={(id) => setDiscordAllowedServerIds((current) => current.filter((item) => item !== id))}
              />
            </SettingsStackedRow>
            <SettingsStackedRow icon={<Hash className="h-4 w-4" />} label={t("settings.discord_allowed_channels")} description={t("settings.discord_allowed_channels_description")}>
              <IdListEditor
                value={discordChannelIdInput}
                placeholder={t("settings.discord_channel_id_placeholder")}
                ids={discordAllowedChannelIds}
                addLabel={t("settings.add")}
                onInputChange={setDiscordChannelIdInput}
                onAdd={addDiscordChannelId}
                onRemove={(id) => setDiscordAllowedChannelIds((current) => current.filter((item) => item !== id))}
              />
            </SettingsStackedRow>
            <SettingsRow icon={<AtSign className="h-4 w-4" />} label={t("settings.discord_mention_only")} description={t("settings.discord_mention_only_description")}>
              <SwitchControl checked={discordMentionOnly} tooltip={t("settings.discord_mention_only")} onToggle={() => setDiscordMentionOnly((value) => !value)} />
            </SettingsRow>
          </SettingsGroup>

          <SettingsGroup title={t("settings.discord_setup_guide")} flush>
            <SettingsStackedRow icon={<Bot className="h-4 w-4" />} label={t("settings.discord_setup_guide")} description={t("settings.discord_setup_description")}>
              <ol className="space-y-2 pl-4 text-caption leading-relaxed text-ink/65">
                <li>{t("settings.discord_setup_step_1")}</li>
                <li>{t("settings.discord_setup_step_2")}</li>
                <li>{t("settings.discord_setup_step_3")}</li>
                <li>{t("settings.discord_setup_step_4")}</li>
                <li>{t("settings.discord_setup_step_5")}</li>
                <li>{t("settings.discord_setup_step_6")}</li>
              </ol>
            </SettingsStackedRow>
          </SettingsGroup>

          <SettingsGroup title={t("settings.discord_workspace_bindings")} flush>
            <SettingsStackedRow icon={<SquareKanban className="h-4 w-4" />} label={t("settings.channels_default_workspace")} description={t("settings.discord_default_workspace_description")}>
              <select value={discordWorkspace} onChange={(event) => setDiscordWorkspace(event.target.value)} className={inputClassName + " w-full"}>
                <option value="">{t("settings.channels_select_workspace")}</option>
                {discordWorkspaceChoices.map((path) => (
                  <option key={path} value={path}>{projectLabel(projects, path)}</option>
                ))}
              </select>
            </SettingsStackedRow>
            <SettingsStackedRow icon={<Server className="h-4 w-4" />} label={t("settings.discord_add_binding")} description={t("settings.discord_add_binding_description")}>
              <div className="flex w-full min-w-0 flex-wrap items-center gap-2">
                <input value={discordBindingChannelId} onChange={(event) => setDiscordBindingChannelId(event.target.value)} placeholder="Channel ID" className={inputClassName + " min-w-[170px] flex-1"} />
                <select value={discordBindingWorkspace} onChange={(event) => setDiscordBindingWorkspace(event.target.value)} className={inputClassName + " min-w-[220px] flex-[1.4]"}>
                  <option value="">{t("settings.channels_select_workspace")}</option>
                  {discordWorkspaceChoices.map((path) => (
                    <option key={path} value={path}>{projectLabel(projects, path)}</option>
                  ))}
                </select>
                <button type="button" disabled={!discordBindingChannelId.trim() || !discordBindingWorkspace} onClick={addDiscordBinding} className="inline-flex h-9 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-35">
                  <Plus className="h-4 w-4" />
                  {t("settings.add")}
                </button>
              </div>
            </SettingsStackedRow>
            {discordBindings.map((binding) => (
              <SettingsStackedRow key={binding.chatId} icon={<Server className="h-4 w-4" />} label={binding.chatId} description={projectLabel(projects, binding.workspacePath)}>
                <button type="button" onClick={() => setDiscordBindings((current) => current.filter((item) => item.chatId !== binding.chatId))} className="inline-flex h-8 w-8 items-center justify-center rounded-md text-ink/55 transition hover:bg-red-500/10 hover:text-red-600" aria-label={t("sidebar.remove")}>
                  <Trash2 className="h-4 w-4" />
                </button>
              </SettingsStackedRow>
            ))}
          </SettingsGroup>
        </>
      ) : activePlatform === "feishu" ? (
        <>
          <SettingsGroup
            title={t("settings.feishu_bot")}
            action={
              <button type="button" disabled={saving || loading} onClick={() => void save()} className={channelSaveButtonClass(saveStatus)}>
                {saving
                  ? <LoaderCircle className="h-4 w-4 animate-spin" />
                  : saveStatus === "error"
                    ? <X className="h-4 w-4" />
                    : <Check className="h-4 w-4" />}
                {saving
                  ? t("project.save")
                  : saveStatus === "success"
                    ? t("settings.saved")
                    : saveStatus === "error"
                      ? t("settings.save_failed")
                      : t("project.save")}
              </button>
            }
            flush
          >
            <SettingsRow icon={<LarkLogoIcon className="h-4 w-4" />} label={t("settings.feishu_enable")} description={t("settings.feishu_enable_description")}>
              <div className="flex items-center gap-3">
                <span className="text-caption text-ink/45">{feishuEnabled ? t("settings.proxy_enabled") : t("settings.proxy_disabled")}</span>
                <SwitchControl checked={feishuEnabled} tooltip={t("settings.feishu_enable")} onToggle={() => setFeishuEnabled((value) => !value)} />
              </div>
            </SettingsRow>
            <SettingsStackedRow icon={<TokenOutlineIcon className="h-4 w-4" />} label={t("settings.feishu_app_credentials")} description={t("settings.feishu_app_credentials_description")}>
              <div className="flex w-full min-w-0 flex-col gap-2">
                <div className="flex w-full min-w-0 flex-wrap items-center gap-2">
                  <input value={feishuAppId} onChange={(event) => {
                    setFeishuAppId(event.target.value);
                    setFeishuTestStatus("idle");
                  }} placeholder="cli_xxxxxxxxxxxxxxxx" className={inputClassName + " min-w-[190px] flex-1"} />
                  <input value={feishuAppSecret} type={showFeishuSecret ? "text" : "password"} onChange={(event) => {
                    setFeishuAppSecret(event.target.value);
                    setFeishuTestStatus("idle");
                  }} placeholder={t("settings.feishu_app_secret_placeholder")} className={inputClassName + " min-w-[190px] flex-1"} />
                  <button type="button" onClick={() => setShowFeishuSecret((value) => !value)} className="inline-flex h-9 w-9 items-center justify-center rounded-md text-ink/55 transition hover:bg-ink/[0.06] hover:text-ink" aria-label={showFeishuSecret ? t("settings.telegram_hide_token") : t("settings.telegram_show_token")}>
                    {showFeishuSecret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                  </button>
                  <button type="button" disabled={feishuTesting || !feishuAppId.trim() || !feishuAppSecret.trim()} onClick={() => void sendFeishuTest()} className={telegramTestButtonClass(feishuTestStatus)}>
                    {feishuTesting ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Link2 className="h-4 w-4" />}
                    {feishuTesting
                      ? t("settings.testing_connection")
                      : feishuTestStatus === "success"
                        ? t("settings.connected")
                        : feishuTestStatus === "error"
                          ? t("settings.connection_failed")
                          : t("settings.test_connection")}
                  </button>
                </div>
                <select value={feishuDomain ?? ""} onChange={(event) => {
                  setFeishuDomain(event.target.value || null);
                  setFeishuTestStatus("idle");
                }} className={inputClassName + " w-full max-w-[300px]"}>
                  <option value="">{t("settings.feishu_domain_feishu")}</option>
                  <option value="https://open.larksuite.com">{t("settings.feishu_domain_lark")}</option>
                </select>
              </div>
            </SettingsStackedRow>
            <SettingsRow icon={<Bot className="h-4 w-4" />} label={t("settings.feishu_default_model")} description={t("settings.feishu_default_model_description")}>
              <RuntimeMenuSelect
                ariaLabel={t("settings.feishu_default_model")}
                value={feishuModelValue}
                options={feishuModelOptions}
                onChange={(value) => {
                  const parsed = parseAgentModelSelectValue(value);
                  if (!parsed) return;
                  const runtimeAgent = runtimeAgents.find((agent) => agent.agent === parsed.agent);
                  if (parsed.agent !== selectedFeishuAgent) {
                    setFeishuEffort(runtimeAgent ? initialRuntimeEffort(runtimeAgent) : null);
                  }
                  setFeishuAgent(parsed.agent);
                  setFeishuModel(parsed.model || null);
                }}
                minMenuWidth={240}
                maxWidthClassName="max-w-[300px]"
              />
            </SettingsRow>
            <SettingsStackedRow icon={<Globe2 className="h-4 w-4" />} label={t("settings.feishu_connection_mode")} description={t("settings.feishu_connection_mode_description")}>
              <p className="text-caption leading-relaxed text-ink/62">{t("settings.feishu_connection_mode_detail")}</p>
            </SettingsStackedRow>
          </SettingsGroup>

          <SettingsGroup title={t("settings.feishu_workspace_bindings")} flush>
            <SettingsStackedRow icon={<SquareKanban className="h-4 w-4" />} label={t("settings.channels_default_workspace")} description={t("settings.feishu_default_workspace_description")}>
              <select value={feishuWorkspace} onChange={(event) => setFeishuWorkspace(event.target.value)} className={inputClassName + " w-full"}>
                <option value="">{t("settings.channels_select_workspace")}</option>
                {feishuWorkspaceChoices.map((path) => (
                  <option key={path} value={path}>{projectLabel(projects, path)}</option>
                ))}
              </select>
            </SettingsStackedRow>
            <SettingsStackedRow icon={<Server className="h-4 w-4" />} label={t("settings.feishu_add_binding")} description={t("settings.feishu_add_binding_description")}>
              <div className="flex w-full min-w-0 flex-wrap items-center gap-2">
                <input value={feishuBindingChatId} onChange={(event) => setFeishuBindingChatId(event.target.value)} placeholder="oc_xxxxxx" className={inputClassName + " min-w-[170px] flex-1"} />
                <select value={feishuBindingWorkspace} onChange={(event) => setFeishuBindingWorkspace(event.target.value)} className={inputClassName + " min-w-[220px] flex-[1.4]"}>
                  <option value="">{t("settings.channels_select_workspace")}</option>
                  {feishuWorkspaceChoices.map((path) => (
                    <option key={path} value={path}>{projectLabel(projects, path)}</option>
                  ))}
                </select>
                <button type="button" disabled={!feishuBindingChatId.trim() || !feishuBindingWorkspace} onClick={addFeishuBinding} className="inline-flex h-9 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-35">
                  <Plus className="h-4 w-4" />
                  {t("settings.add")}
                </button>
              </div>
            </SettingsStackedRow>
            {feishuBindings.map((binding) => (
              <SettingsStackedRow key={binding.chatId} icon={<Server className="h-4 w-4" />} label={binding.chatId} description={projectLabel(projects, binding.workspacePath)}>
                <button type="button" onClick={() => setFeishuBindings((current) => current.filter((item) => item.chatId !== binding.chatId))} className="inline-flex h-8 w-8 items-center justify-center rounded-md text-ink/55 transition hover:bg-red-500/10 hover:text-red-600" aria-label={t("sidebar.remove")}>
                  <Trash2 className="h-4 w-4" />
                </button>
              </SettingsStackedRow>
            ))}
          </SettingsGroup>
        </>
      ) : (
        <>
          <SettingsGroup
            title={t("settings.wechat_bot")}
            action={
              <button type="button" disabled={saving || loading} onClick={() => void save()} className={channelSaveButtonClass(saveStatus)}>
                {saving
                  ? <LoaderCircle className="h-4 w-4 animate-spin" />
                  : saveStatus === "error"
                    ? <X className="h-4 w-4" />
                    : <Check className="h-4 w-4" />}
                {saving
                  ? t("project.save")
                  : saveStatus === "success"
                    ? t("settings.saved")
                    : saveStatus === "error"
                      ? t("settings.save_failed")
                      : t("project.save")}
              </button>
            }
            flush
          >
            <SettingsRow icon={<WechatLogoIcon className="h-4 w-4" />} label={t("settings.wechat_enable")} description={t("settings.wechat_enable_description")}>
              <div className="flex items-center gap-3">
                <span className="text-caption text-ink/45">{wechatEnabled ? t("settings.proxy_enabled") : t("settings.proxy_disabled")}</span>
                <SwitchControl checked={wechatEnabled} tooltip={t("settings.wechat_enable")} onToggle={() => setWechatEnabled((value) => !value)} />
              </div>
            </SettingsRow>
            <SettingsStackedRow icon={<TokenOutlineIcon className="h-4 w-4" />} label={t("settings.wechat_bot_token")} description={t("settings.wechat_bot_token_description")}>
              <div className="flex w-full min-w-0 flex-col gap-2">
                <div className="flex w-full min-w-0 flex-wrap items-center gap-2">
                  <input value={wechatBotToken} type={showWechatToken ? "text" : "password"} onChange={(event) => {
                    setWechatBotToken(event.target.value);
                    setWechatTestStatus("idle");
                  }} placeholder="iLink bot token" className={inputClassName + " min-w-0 flex-1"} />
                  <button type="button" onClick={() => setShowWechatToken((value) => !value)} className="inline-flex h-9 w-9 items-center justify-center rounded-md text-ink/55 transition hover:bg-ink/[0.06] hover:text-ink" aria-label={showWechatToken ? t("settings.telegram_hide_token") : t("settings.telegram_show_token")}>
                    {showWechatToken ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                  </button>
                  <button type="button" disabled={wechatTesting || !wechatBotToken.trim()} onClick={() => void sendWechatTest()} className={telegramTestButtonClass(wechatTestStatus)}>
                    {wechatTesting ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Link2 className="h-4 w-4" />}
                    {wechatTesting
                      ? t("settings.testing_connection")
                      : wechatTestStatus === "success"
                        ? t("settings.connected")
                        : wechatTestStatus === "error"
                          ? t("settings.connection_failed")
                          : t("settings.test_connection")}
                  </button>
                </div>
                <input value={wechatBaseUrl ?? ""} onChange={(event) => {
                  setWechatBaseUrl(event.target.value || null);
                  setWechatTestStatus("idle");
                }} placeholder="https://ilinkai.weixin.qq.com" className={inputClassName + " w-full max-w-[360px]"} />
                {(wechatBotId || wechatUserId) && (
                  <p className="text-caption text-ink/50">
                    {wechatBotId ? `Bot: ${wechatBotId}` : ""}
                    {wechatBotId && wechatUserId ? " · " : ""}
                    {wechatUserId ? `User: ${wechatUserId}` : ""}
                  </p>
                )}
              </div>
            </SettingsStackedRow>
            <SettingsStackedRow icon={<QrCodeIcon className="h-4 w-4" />} label={t("settings.wechat_qr_login")} description={t("settings.wechat_qr_login_description")}>
              <div className="flex w-full min-w-0 flex-col gap-2">
                <div className="flex flex-wrap items-center gap-2">
                  <button type="button" disabled={wechatQrStatus === "loading"} onClick={() => void startWechatQrLogin()} className="inline-flex h-9 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-35">
                    {wechatQrStatus === "loading" ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                    {t("settings.wechat_get_qr")}
                  </button>
                  <button type="button" disabled={!wechatQrId || wechatQrStatus === "loading" || wechatQrPolling} onClick={() => void pollWechatQrLogin()} className={telegramTestButtonClass(wechatQrStatus === "confirmed" ? "success" : wechatQrStatus === "error" || wechatQrStatus === "expired" ? "error" : "idle")}>
                    {wechatQrStatus === "loading" || wechatQrPolling ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Link2 className="h-4 w-4" />}
                    {wechatQrStatus === "confirmed"
                      ? t("settings.wechat_qr_confirmed")
                      : wechatQrStatus === "scanned"
                        ? t("settings.wechat_qr_scanned")
                        : wechatQrStatus === "expired"
                          ? t("settings.wechat_qr_expired")
                          : wechatQrPolling
                            ? t("settings.wechat_polling_qr")
                            : t("settings.wechat_poll_qr")}
                  </button>
                </div>
                {wechatQrContent && (
                  <div className="flex w-full min-w-0 flex-col gap-2">
                    {wechatQrImage ? (
                      <div className="inline-flex w-fit rounded-md border border-card-border/[0.12] bg-white p-3">
                        <img
                          src={wechatQrImage}
                          alt={t("settings.wechat_qr_login")}
                          className="h-48 w-48 object-contain"
                          onError={() => setWechatQrImageContent(null)}
                        />
                      </div>
                    ) : (
                      <div className="inline-flex w-fit rounded-md border border-card-border/[0.12] bg-white p-3">
                        <QRCodeSVG
                          value={wechatQrContent}
                          size={192}
                          level="M"
                          marginSize={2}
                          title={t("settings.wechat_qr_login")}
                        />
                      </div>
                    )}
                  </div>
                )}
                <p className="text-caption leading-relaxed text-ink/55">{t("settings.wechat_qr_content_hint")}</p>
              </div>
            </SettingsStackedRow>
            <SettingsRow icon={<Bot className="h-4 w-4" />} label={t("settings.wechat_default_model")} description={t("settings.wechat_default_model_description")}>
              <RuntimeMenuSelect
                ariaLabel={t("settings.wechat_default_model")}
                value={wechatModelValue}
                options={wechatModelOptions}
                onChange={(value) => {
                  const parsed = parseAgentModelSelectValue(value);
                  if (!parsed) return;
                  const runtimeAgent = runtimeAgents.find((agent) => agent.agent === parsed.agent);
                  if (parsed.agent !== selectedWechatAgent) {
                    setWechatEffort(runtimeAgent ? initialRuntimeEffort(runtimeAgent) : null);
                  }
                  setWechatAgent(parsed.agent);
                  setWechatModel(parsed.model || null);
                }}
                minMenuWidth={240}
                maxWidthClassName="max-w-[300px]"
              />
            </SettingsRow>
          </SettingsGroup>

          <SettingsGroup title={t("settings.wechat_workspace_bindings")} flush>
            <SettingsStackedRow icon={<SquareKanban className="h-4 w-4" />} label={t("settings.channels_default_workspace")} description={t("settings.wechat_default_workspace_description")}>
              <select value={wechatWorkspace} onChange={(event) => setWechatWorkspace(event.target.value)} className={inputClassName + " w-full"}>
                <option value="">{t("settings.channels_select_workspace")}</option>
                {wechatWorkspaceChoices.map((path) => (
                  <option key={path} value={path}>{projectLabel(projects, path)}</option>
                ))}
              </select>
            </SettingsStackedRow>
            <SettingsStackedRow icon={<Server className="h-4 w-4" />} label={t("settings.wechat_add_binding")} description={t("settings.wechat_add_binding_description")}>
              <div className="flex w-full min-w-0 flex-wrap items-center gap-2">
                <input value={wechatBindingUserId} onChange={(event) => setWechatBindingUserId(event.target.value)} placeholder="WeChat user ID" className={inputClassName + " min-w-[170px] flex-1"} />
                <select value={wechatBindingWorkspace} onChange={(event) => setWechatBindingWorkspace(event.target.value)} className={inputClassName + " min-w-[220px] flex-[1.4]"}>
                  <option value="">{t("settings.channels_select_workspace")}</option>
                  {wechatWorkspaceChoices.map((path) => (
                    <option key={path} value={path}>{projectLabel(projects, path)}</option>
                  ))}
                </select>
                <button type="button" disabled={!wechatBindingUserId.trim() || !wechatBindingWorkspace} onClick={addWechatBinding} className="inline-flex h-9 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-35">
                  <Plus className="h-4 w-4" />
                  {t("settings.add")}
                </button>
              </div>
            </SettingsStackedRow>
            {wechatBindings.map((binding) => (
              <SettingsStackedRow key={binding.chatId} icon={<Server className="h-4 w-4" />} label={binding.chatId} description={projectLabel(projects, binding.workspacePath)}>
                <button type="button" onClick={() => setWechatBindings((current) => current.filter((item) => item.chatId !== binding.chatId))} className="inline-flex h-8 w-8 items-center justify-center rounded-md text-ink/55 transition hover:bg-red-500/10 hover:text-red-600" aria-label={t("sidebar.remove")}>
                  <Trash2 className="h-4 w-4" />
                </button>
              </SettingsStackedRow>
            ))}
          </SettingsGroup>
        </>
      )}
    </section>
  );
}

function defaultImBridgeConfig(): ImBridgeConfig {
  return {
    enabled: false,
    idleTimeoutSecs: 900,
    telegram: defaultTelegramBridgeConfig(),
    discord: defaultDiscordBridgeConfig(),
    feishu: defaultFeishuBridgeConfig(),
    wechat: defaultWechatBridgeConfig(),
  };
}

function defaultTelegramBridgeConfig(): TelegramBridgeConfig {
  return {
    enabled: false,
    agent: null,
    model: null,
    effort: null,
    defaultWorkspace: null,
    allowedWorkspaces: [],
    workspaceBindings: [],
    botToken: "",
    allowedUserIds: [],
    pollTimeoutSecs: 30,
    apiBase: null,
  };
}

function defaultDiscordBridgeConfig(): DiscordBridgeConfig {
  return {
    enabled: false,
    agent: null,
    model: null,
    effort: null,
    defaultWorkspace: null,
    allowedWorkspaces: [],
    workspaceBindings: [],
    botToken: "",
    allowedServerIds: [],
    allowedChannelIds: [],
    mentionOnly: true,
    apiBase: null,
    gatewayUrl: null,
  };
}

function defaultFeishuBridgeConfig(): FeishuBridgeConfig {
  return {
    enabled: false,
    agent: null,
    model: null,
    effort: null,
    defaultWorkspace: null,
    allowedWorkspaces: [],
    workspaceBindings: [],
    appId: "",
    appSecret: "",
    domain: null,
  };
}

function defaultWechatBridgeConfig(): WechatBridgeConfig {
  return {
    enabled: false,
    agent: null,
    model: null,
    effort: null,
    defaultWorkspace: null,
    allowedWorkspaces: [],
    workspaceBindings: [],
    botToken: "",
    botId: null,
    userId: null,
    baseUrl: null,
    pollTimeoutSecs: 40,
  };
}

function firstEnabledRuntimeAgent(runtimeAgents: RuntimeAgentMetadata[]): RuntimeAgentMetadata | null {
  return runtimeAgents.find((agent) => agent.enabled) ?? runtimeAgents[0] ?? null;
}

function resolveTelegramAgentSelection(
  runtimeAgents: RuntimeAgentMetadata[],
  telegram: TelegramBridgeConfig,
): { agent: Agent | null; model: string | null; effort: string | null } {
  return resolveRuntimeSelection(runtimeAgents, telegram);
}

function resolveRuntimeSelection(
  runtimeAgents: RuntimeAgentMetadata[],
  config: { agent: Agent | null; model: string | null; effort: string | null },
): { agent: Agent | null; model: string | null; effort: string | null } {
  const selected =
    runtimeAgents.find((agent) => agent.agent === config.agent && agent.enabled)
    ?? firstEnabledRuntimeAgent(runtimeAgents);
  if (!selected) {
    return {
      agent: config.agent ?? null,
      model: config.model ?? null,
      effort: config.effort ?? null,
    };
  }
  return {
    agent: selected.agent,
    model: config.model ?? defaultRuntimeModel(selected),
    effort: config.effort ?? (initialRuntimeEffort(selected) || null),
  };
}

function defaultRuntimeModel(agent: RuntimeAgentMetadata): string | null {
  return agent.model ?? agent.models.find((model) => model.enabled)?.value ?? agent.models[0]?.value ?? null;
}

function telegramTestButtonClass(status: "idle" | "success" | "error"): string {
  const base = "inline-flex h-9 items-center gap-1.5 rounded-md border px-3 text-body-sm font-medium transition disabled:opacity-35 ";
  if (status === "success") {
    return base + "border-emerald/45 bg-emerald/14 text-emerald hover:bg-emerald/20";
  }
  if (status === "error") {
    return base + "border-status-error/45 bg-status-error/12 text-status-error hover:bg-status-error/18";
  }
  return base + "border-card-border/[0.12] bg-card-chip/[0.08] text-card-fg/75 hover:border-card-border/[0.18] hover:bg-card-chip/[0.12]";
}

function wechatQrImageSrc(content: string | null): string | null {
  const value = content?.trim();
  if (!value) return null;
  if (/^(data:image\/|https?:\/\/|blob:)/i.test(value)) {
    return value;
  }
  if (value.startsWith("<svg") || value.startsWith("<?xml")) {
    return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(value)}`;
  }
  const compact = value.replace(/\s/g, "");
  if (/^[A-Za-z0-9+/=]+$/.test(compact) && compact.length > 80 && looksLikeImageBase64(compact)) {
    return `data:${imageMimeFromBase64(compact)};base64,${compact}`;
  }
  return null;
}

function looksLikeImageBase64(value: string): boolean {
  return imageMimeFromBase64(value) !== null;
}

function imageMimeFromBase64(value: string): string | null {
  if (value.startsWith("iVBORw0KGgo")) return "image/png";
  if (value.startsWith("/9j/")) return "image/jpeg";
  if (value.startsWith("R0lGOD")) return "image/gif";
  if (value.startsWith("UklGR")) return "image/webp";
  if (value.startsWith("PHN2Zy") || value.startsWith("PD94bWwg")) return "image/svg+xml";
  return null;
}

function channelSaveButtonClass(status: "idle" | "success" | "error"): string {
  const base = "inline-flex h-8 items-center gap-1.5 rounded-md border px-3 text-body-sm font-medium transition disabled:opacity-35 ";
  if (status === "success") {
    return base + "border-emerald/45 bg-emerald/14 text-emerald hover:bg-emerald/20";
  }
  if (status === "error") {
    return base + "border-status-error/45 bg-status-error/12 text-status-error hover:bg-status-error/18";
  }
  return base + "border-card-border/[0.12] bg-card-chip/[0.08] text-card-fg/75 hover:border-card-border/[0.18] hover:bg-card-chip/[0.12]";
}

function IdListEditor({
  value,
  placeholder,
  ids,
  addLabel,
  onInputChange,
  onAdd,
  onRemove,
}: {
  value: string;
  placeholder: string;
  ids: string[];
  addLabel: string;
  onInputChange: (value: string) => void;
  onAdd: () => void;
  onRemove: (id: string) => void;
}) {
  return (
    <div className="flex w-full min-w-0 flex-col gap-2">
      <div className="flex w-full min-w-0 flex-wrap items-center gap-2">
        <input
          value={value}
          onChange={(event) => onInputChange(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter") onAdd(); }}
          placeholder={placeholder}
          className={inputClassName + " min-w-[180px] flex-1"}
        />
        <button type="button" disabled={!value.trim()} onClick={onAdd} className="inline-flex h-9 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-35">
          <Plus className="h-4 w-4" />
          {addLabel}
        </button>
      </div>
      <div className="flex max-w-full flex-wrap gap-1.5">
        {ids.map((id) => (
          <button key={id} type="button" onClick={() => onRemove(id)} className="inline-flex h-7 items-center gap-1 rounded-md bg-ink/[0.07] px-2 text-caption text-ink/70 transition hover:bg-ink/[0.1]">
            {id}
            <X className="h-3 w-3" />
          </button>
        ))}
      </div>
    </div>
  );
}

function projectLabel(projects: ProjectInfo[], path: string): string {
  const project = projects.find((item) => item.path === path);
  return project ? `${project.name} · ${project.path}` : path;
}

function uniqueNumbers(values: number[]): number[] {
  return Array.from(new Set(values));
}

function uniqueStrings(values: string[]): string[] {
  return Array.from(new Set(values.map((value) => value.trim()).filter(Boolean)));
}

function AgentsSettings({ onError }: { onError: (error: string | null) => void }) {
  const { t } = useI18n();
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [selectedView, setSelectedView] = useState<"astra" | "agent">("astra");
  const [selectedAgentId, setSelectedAgentId] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [astraConfig, setAstraConfig] = useState<AstraConfig | null>(null);
  const builtinAgents = useMemo(
    () => agents.filter((agent) => agent.type === "builtin" && isSettingsAgent(agent.id)),
    [agents],
  );
  const selectedAgent =
    selectedView === "agent"
      ? builtinAgents.find((agent) => agent.id === selectedAgentId) ?? builtinAgents[0] ?? null
      : null;

  const reload = async () => {
    setLoading(true);
    try {
      const [agentsData, configData] = await Promise.all([
        listAgents(),
        getAstraConfig(),
      ]);
      setAgents(agentsData);
      setAstraConfig(configData);
      setSelectedAgentId((current) => {
        const currentExists = agentsData.some((agent) => agent.id === current && isSettingsAgent(agent.id));
        return currentExists ? current : agentsData.find((agent) => isSettingsAgent(agent.id))?.id ?? current;
      });
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const updateAgent = (next: AgentInfo) => {
    setAgents((prev) => prev.map((agent) => agent.id === next.id ? next : agent));
  };

  const moveAgent = async (from: number, to: number) => {
    const nextAgents = moveAgentOption(builtinAgents, from, to).map((agent, index) => ({ ...agent, order: index }));
    setAgents((prev) => prev.map((agent) => nextAgents.find((item) => item.id === agent.id) ?? agent));
    try {
      await Promise.all(nextAgents.map((agent) => updateAgentPreferences({
        agentId: agent.id,
        order: agent.order,
      })));
      const rows = await listAgents();
      setAgents(rows);
    } catch (err) {
      onError(String(err));
    }
  };

  const handleAgentDragEnd = (event: DragEndEvent) => {
    if (event.canceled) return;
    const { source } = event.operation;
    if (!isSortable(source)) return;
    if (source.initialIndex === source.index) return;
    void moveAgent(source.initialIndex, source.index);
  };

  return (
    <section>
      <div className="grid grid-cols-[240px_minmax(0,1fr)] gap-5">
        <div className="min-w-0">
          <div className="mb-8">
            <h2 className="mb-3 text-body-sm font-semibold text-ink/[0.88]">{t("agent.title")}</h2>
            <button
              type="button"
              onClick={() => setSelectedView("astra")}
              className={
                "process-template-list-item mb-3 flex h-10 w-full min-w-0 items-center gap-2 overflow-hidden rounded-lg border border-card-border/[0.12] bg-card px-3 text-left text-body-sm transition " +
                (selectedView === "astra" ? "process-template-list-item-active" : "")
              }
            >
              <Sparkles className="h-4 w-4 shrink-0 text-card-icon/60" />
              <span className="min-w-0 flex-1 truncate font-medium text-card-fg/78">{t("astra.orchestrator")}</span>
            </button>
            <div className="overflow-hidden rounded-lg border border-card-border/[0.12] bg-card">
              <DragDropProvider onDragEnd={handleAgentDragEnd}>
                <div className="divide-y divide-card-border/10">
                  {builtinAgents.map((agent, index) => (
                    <AgentListRow
                      key={agent.id}
                      agent={agent}
                      index={index}
                      active={selectedView === "agent" && agent.id === selectedAgent?.id}
                      draggable
                      onSelect={(agentId) => {
                        setSelectedAgentId(agentId);
                        setSelectedView("agent");
                      }}
                    />
                  ))}
                  {!loading && builtinAgents.length === 0 && <div className="p-3"><EmptyState label={t("agent.empty")} /></div>}
                </div>
              </DragDropProvider>
            </div>
          </div>
        </div>
        <div className="min-w-0">
          {selectedView === "astra" && astraConfig ? (
            <SettingsGroup title={t("astra.config_title")}>
              <AstraAgentSettings
                title={t("astra.agent")}
                agentValue={astraConfig.agent ?? ""}
                modelValue={astraConfig.model ?? ""}
                effortValue={astraConfig.effort ?? ""}
                permissionValue={astraConfig.permissionMode ?? ""}
                agents={agents}
                onAgentChange={(value) => {
                  updateAstraConfig({ agent: value || null })
                    .then(setAstraConfig)
                    .catch((err) => onError(String(err)));
                }}
                onModelChange={(value) => {
                  updateAstraConfig({ model: value || null })
                    .then(setAstraConfig)
                    .catch((err) => onError(String(err)));
                }}
                onEffortChange={(value) => {
                  updateAstraConfig({ effort: value || null })
                    .then(setAstraConfig)
                    .catch((err) => onError(String(err)));
                }}
                onPermissionChange={(value) => {
                  updateAstraConfig({ permissionMode: value || null })
                    .then(setAstraConfig)
                    .catch((err) => onError(String(err)));
                }}
              />
            </SettingsGroup>
          ) : selectedAgent ? (
            <AgentEditor
              key={selectedAgent.id}
              agent={selectedAgent}
              onUpdated={updateAgent}
              onError={onError}
            />
          ) : (
            !loading && <EmptyState label={t("agent.empty")} />
          )}
        </div>
      </div>
    </section>
  );
}

function AstraAgentSettings({
  title,
  agentValue,
  modelValue,
  effortValue,
  permissionValue,
  agents,
  onAgentChange,
  onModelChange,
  onEffortChange,
  onPermissionChange,
}: {
  title: string;
  agentValue: string;
  modelValue: string;
  effortValue: string;
  permissionValue: string;
  agents: AgentInfo[];
  onAgentChange: (value: string) => void;
  onModelChange: (value: string) => void;
  onEffortChange: (value: string) => void;
  onPermissionChange: (value: string) => void;
}) {
  const { t } = useI18n();
  const selectableAgents = useMemo(
    () => astraSelectableAgents(agents, agentValue),
    [agentValue, agents],
  );
  const agentOptions = useMemo(
    () => selectableAgents.map((agent) => ({
      value: agent.id,
      label: agent.displayName,
      icon: <SettingsAgentGlyph agentId={agent.id} className="h-4 w-4" />,
    })),
    [selectableAgents],
  );
  const effectiveAgentValue = agentOptions.some((option) => option.value === agentValue)
    ? agentValue
    : agentOptions[0]?.value ?? "";
  const selectedAgent = useMemo(
    () => selectableAgents.find((agent) => agent.id === effectiveAgentValue) ?? null,
    [effectiveAgentValue, selectableAgents],
  );
  const modelOptions = optionRows(astraAgentModelOptions(selectedAgent), modelValue);
  const effectiveModelValue = modelValue || (modelOptions[0]?.value ?? "");
  const effortOptions = optionRows(
    astraPreferenceSource(selectedAgent?.efforts ?? [], selectedAgent?.effort),
    effortValue,
  );
  const effectiveEffortValue = effortValue || (effortOptions[0]?.value ?? "");
  const permissionRows = optionRows(
    astraPreferenceSource(selectedAgent?.permissionModes ?? [], selectedAgent?.permissionMode),
    permissionValue,
  );
  const permissionOptions = selectedAgent && isRuntimeAgent(selectedAgent.id)
    ? runtimePermissionModeOptions(permissionRows, permissionValue, selectedAgent.id)
    : permissionRows;
  const effectivePermissionValue = permissionValue || (permissionOptions[0]?.value ?? "");
  const showPermissionMode = permissionOptions.length > 0;

  return (
    <section className="grid gap-3">
      <div className="min-w-0">
        <AgentInlineSelect
          value={effectiveAgentValue}
          options={agentOptions}
          placeholder={title}
          onChange={onAgentChange}
        />
      </div>
      <div className="min-w-0">
        <div className="grid gap-2 rounded-lg border border-card-border/[0.12] p-3">
          <AgentPreferenceRow label={t("assistant.model")}>
            <AstraPreferenceSelect
              value={effectiveModelValue}
              options={modelOptions}
              placeholder={t("agent.no_model")}
              onChange={onModelChange}
            />
          </AgentPreferenceRow>
          <AgentPreferenceRow label={t("assistant.effort")}>
            <AstraPreferenceSelect
              value={effectiveEffortValue}
              options={effortOptions}
              placeholder={t("assistant.effort")}
              onChange={onEffortChange}
            />
          </AgentPreferenceRow>
          {showPermissionMode && (
            <AgentPreferenceRow label={t("assistant.permission_mode")}>
              <AstraPreferenceSelect
                value={effectivePermissionValue}
                options={permissionOptions}
                placeholder={t("assistant.permission_mode")}
                onChange={onPermissionChange}
              />
            </AgentPreferenceRow>
          )}
        </div>
      </div>
    </section>
  );
}

function AstraPreferenceSelect({
  value,
  options,
  placeholder,
  onChange,
}: {
  value: string;
  options: Array<{ value: string; label: string; icon?: ReactNode }>;
  placeholder: string;
  onChange: (value: string) => void;
}) {
  return (
    <AgentInlineSelect
      value={value}
      options={options}
      placeholder={placeholder}
      onChange={onChange}
    />
  );
}

function astraPreferenceSource(
  options: RuntimeAgentOptionMetadata[],
  fallbackValue: string | null | undefined,
): RuntimeAgentOptionMetadata[] {
  if (options.length > 0) return options;
  if (!fallbackValue) return [];
  return [{ value: fallbackValue, label: fallbackValue, displayName: fallbackValue, enabled: true, order: 0 }];
}

function astraAgentModelOptions(agent: AgentInfo | null): RuntimeAgentOptionMetadata[] {
  if (!agent) return [];
  return astraPreferenceSource(agent.models, agent.model);
}

function astraSelectableAgents(agents: AgentInfo[], selectedAgentId: string): AgentInfo[] {
  const supported = agents.filter((agent) => isRuntimeAgent(agent.id));
  const enabled = supported.filter((agent) => agent.enabled);
  if (!selectedAgentId || enabled.some((agent) => agent.id === selectedAgentId)) {
    return enabled;
  }
  const selected = supported.find((agent) => agent.id === selectedAgentId);
  return selected ? [selected, ...enabled] : enabled;
}

function AgentListRow({
  agent,
  index,
  active,
  draggable,
  onSelect,
}: {
  agent: AgentInfo;
  index: number;
  active: boolean;
  draggable: boolean;
  onSelect: (agentId: string) => void;
}) {
  const { t } = useI18n();
  const { handleRef, isDragSource, isDropTarget, ref } = useSortable({
    id: agent.id,
    index,
    group: "settings-agents",
    transition: {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
      idle: true,
    },
  });
  return (
    <div
      ref={ref}
      className={
        "process-template-list-item flex h-12 w-full min-w-0 items-center gap-2 px-2 text-left text-body-sm transition " +
        (active ? "process-template-list-item-active " : "") +
        (isDragSource ? "z-20 cursor-grabbing bg-card shadow-[0_12px_28px_rgba(0,0,0,0.22)] " : "") +
        (isDropTarget ? "bg-card-active shadow-[inset_3px_0_0_rgb(var(--color-card-fg)/0.32)] " : "")
      }
    >
      <button
        ref={handleRef}
        type="button"
        disabled={!draggable}
        className="cursor-grab touch-none rounded p-0.5 text-card-subtle/35 hover:bg-card-action-hover/5 hover:text-card-fg/60 active:cursor-grabbing disabled:cursor-default disabled:opacity-25"
      >
        <GripVertical className="h-4 w-4" />
      </button>
      <button type="button" onClick={() => onSelect(agent.id)} className="flex min-w-0 flex-1 items-center gap-2 text-left">
        <SettingsAgentGlyph agentId={agent.id} className="h-4 w-4 shrink-0" />
        <span className="min-w-0 flex-1">
          <span className="flex min-w-0 items-center gap-1.5">
            <span className="truncate font-medium text-card-fg/78">{agent.displayName}</span>
            <SettingsTransportGlyph transport={agent.transport} className="h-2 w-auto shrink-0 opacity-70" />
          </span>
          <span className="mt-0.5 block truncate text-meta text-card-muted/45">{agent.model || t("agent.no_model")}</span>
        </span>
        <span className={"h-1.5 w-1.5 rounded-full " + (agent.enabled ? "bg-brand" : "bg-ink/20")} />
      </button>
    </div>
  );
}

function AgentEditor({
  agent,
  onUpdated,
  onError,
}: {
  agent: AgentInfo;
  onUpdated: (agent: AgentInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [model, setModel] = useState(agent.model ?? agent.models.find((item) => item.enabled)?.value ?? "");
  const [effort, setEffort] = useState(agent.effort ?? agent.efforts[0]?.value ?? "");
  const [permissionMode, setPermissionMode] = useState(agent.permissionMode ?? agent.permissionModes[0]?.value ?? "");
  const [models, setModels] = useState<RuntimeAgentOptionMetadata[]>(agent.models);
  const [newModelValue, setNewModelValue] = useState("");
  const [newModelDisplayName, setNewModelDisplayName] = useState("");
  const runtimeAgent = isRuntimeAgent(agent.id) ? agent.id : null;
  const isAstra = false;
  const [aiProvider, setAiProvider] = useState(agent.aiProvider ?? "");
  const [editingAiProvider, setEditingAiProvider] = useState(agent.aiProvider ?? "");
  const [aiProviders, setAiProviders] = useState<AgentAiProviderInfo[]>(agent.aiProviders);
  const [providerDialog, setProviderDialog] = useState<{ mode: "add" | "edit"; provider: AgentAiProviderInfo } | null>(null);
  const [sessionCommandInput, setSessionCommandInput] = useState(agent.commands.session[0] ?? "");
  const [versionCommandInput, setVersionCommandInput] = useState(agent.commands.version[0] ?? "");
  const activeAiProvider = aiProviders.find((provider) => provider.id === aiProvider) ?? aiProviders.find((provider) => provider.enabled) ?? aiProviders[0] ?? null;
  const selectedAiProvider = aiProviders.find((provider) => provider.id === editingAiProvider) ?? activeAiProvider;

  useEffect(() => {
    setModel(agent.model ?? defaultModelValue(agent.models) ?? "");
    setEffort(agent.effort ?? agent.efforts[0]?.value ?? "");
    setPermissionMode(agent.permissionMode ?? agent.permissionModes[0]?.value ?? "");
    setModels(agent.models);
    setNewModelValue("");
    setNewModelDisplayName("");
    setAiProvider(agent.aiProvider ?? "");
    setEditingAiProvider((current) => {
      const defaultProviderId = agent.aiProvider ?? agent.aiProviders.find((provider) => provider.enabled)?.id ?? agent.aiProviders[0]?.id ?? "";
      return isAstra && agent.aiProviders.some((provider) => provider.id === current) ? current : defaultProviderId;
    });
    setAiProviders(agent.aiProviders);
    setSessionCommandInput(agent.commands.session[0] ?? "");
    setVersionCommandInput(agent.commands.version[0] ?? "");
    setProviderDialog(null);
  }, [agent]);

  const persist = async (patch: AgentPreferencePatch): Promise<AgentInfo | null> => {
    try {
      let next: AgentInfo | undefined;
      if (runtimeAgent) {
        const runtimePatch = {
          displayName: patch.displayName,
          enabled: patch.enabled,
          order: patch.order,
          model: patch.model,
          effort: patch.effort,
          permissionMode: patch.permissionMode,
          models: patch.models,
          aiProvider: patch.aiProvider,
          aiProviders: patch.aiProviders,
        };
        await updateRuntimeAgentPreferences({
          agent: runtimeAgent,
          ...runtimePatch,
        });
        const rows = await listAgents();
        next = rows.find((item) => item.id === agent.id);
      } else {
        next = await updateAgentPreferences({
          agentId: agent.id,
          ...patch,
        });
      }
      if (next) {
        onUpdated(next);
        return next;
      }
    } catch (err) {
      onError(String(err));
    }
    return null;
  };

  const selectModel = async (nextModel: string) => {
    setModel(nextModel);
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(aiProviders, aiProvider, nextModel, selectedAiProvider.id);
      return;
    }
    await persist({ model: nextModel });
  };

  const selectEffort = async (nextEffort: string) => {
    setEffort(nextEffort);
    await persist({ effort: nextEffort });
  };

  const selectPermissionMode = async (nextMode: string) => {
    setPermissionMode(nextMode);
    await persist({ permissionMode: nextMode });
  };

  const addModel = async () => {
    const value = newModelValue.trim();
    const sourceModels = activeModels;
    if (!value || sourceModels.some((item) => item.value === value)) return;
    const displayName = newModelDisplayName.trim() || value;
    const nextModels = normalizeModelOrders([...sourceModels, { value, label: displayName, displayName, enabled: true, order: sourceModels.length }]);
    setModels(nextModels);
    setNewModelValue("");
    setNewModelDisplayName("");
    const nextModel = effectiveModel || value;
    setModel(nextModel ?? "");
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, nextModels), aiProvider, nextModel, selectedAiProvider.id);
      return;
    }
    await persist({ models: nextModels, model: nextModel });
  };

  const deleteModel = async (value: string) => {
    const sourceModels = activeModels;
    const nextModels = sourceModels.filter((item) => item.value !== value);
    const orderedModels = normalizeModelOrders(nextModels);
    const nextModel = effectiveModel === value ? defaultModelValue(orderedModels) : effectiveModel || defaultModelValue(orderedModels);
    setModels(orderedModels);
    setModel(nextModel ?? "");
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, orderedModels), aiProvider, nextModel, selectedAiProvider.id);
      return;
    }
    await persist({ models: orderedModels, model: nextModel });
  };

  const saveModel = async (previousValue: string, nextValue: string, nextDisplayName: string) => {
    const value = nextValue.trim();
    const sourceModels = activeModels;
    if (!value || sourceModels.some((item) => item.value === value && item.value !== previousValue)) return;
    const displayName = nextDisplayName.trim() || value;
    const nextModels = normalizeModelOrders(sourceModels.map((item) => item.value === previousValue ? { ...item, value, label: displayName, displayName } : item));
    const nextModel = effectiveModel === previousValue ? value : effectiveModel || defaultModelValue(nextModels);
    setModels(nextModels);
    setModel(nextModel ?? "");
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, nextModels), aiProvider, nextModel, selectedAiProvider.id);
      return;
    }
    await persist({ models: nextModels, model: nextModel });
  };

  const moveModel = async (from: number, to: number) => {
    const nextModels = normalizeModelOrders(moveOption(activeModels, from, to));
    setModels(nextModels);
    if (isAstra && selectedAiProvider) {
      const nextModel = effectiveModel || defaultModelValue(nextModels);
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, nextModels), aiProvider, nextModel, selectedAiProvider.id);
      return;
    }
    await persist({ models: nextModels });
  };

  const setDefaultModel = async (value: string) => {
    if (effectiveModel === value) return;
    setModel(value);
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(aiProviders, aiProvider, value, selectedAiProvider.id);
      return;
    }
    await persist({ model: value });
  };

  const toggleModelEnabled = async (value: string) => {
    const sourceModels = activeModels;
    const nextModels = normalizeModelOrders(sourceModels.map((item) => item.value === value ? { ...item, enabled: !item.enabled } : item));
    const nextModel = nextModels.some((item) => item.value === effectiveModel && item.enabled)
      ? effectiveModel
      : defaultModelValue(nextModels);
    setModels(nextModels);
    setModel(nextModel ?? "");
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, nextModels), aiProvider, nextModel, selectedAiProvider.id);
      return;
    }
    await persist({ models: nextModels, model: nextModel });
  };

  const handleModelDragEnd = (event: DragEndEvent) => {
    if (event.canceled) return;
    const { source } = event.operation;
    if (!isSortable(source)) return;
    if (source.initialIndex === source.index) return;
    void moveModel(source.initialIndex, source.index);
  };

  const activeModels = isAstra && selectedAiProvider ? selectedAiProvider.models : models;
  const preferenceModels = activeModels;
  const effectiveModel = !preferenceModels.some((item) => item.value === model) ? "" : model;
  const modelOptions = optionRows(preferenceModels, effectiveModel);
  const effortOptions = optionRows(agent.efforts, effort);
  const permissionOptions = runtimeAgent
    ? runtimePermissionModeOptions(optionRows(agent.permissionModes, permissionMode), permissionMode, runtimeAgent)
    : optionRows(agent.permissionModes, permissionMode);
  const showCommandSettings = runtimeAgent !== null && !isAstra;
  const saveCommands = async (nextSessionCommand: string, nextVersionCommand: string) => {
    const normalizedSession = nextSessionCommand.trim();
    const normalizedVersion = nextVersionCommand.trim();
    const currentSession = agent.commands.session[0]?.trim() ?? "";
    const currentVersion = agent.commands.version[0]?.trim() ?? "";
    if (normalizedSession === currentSession && normalizedVersion === currentVersion) return;
    await persist({
      commands: {
        session: normalizedSession ? [normalizedSession] : [],
        version: normalizedVersion ? [normalizedVersion] : [],
      },
    });
  };
  const saveAstraProviders = async (
    nextProviders: AgentAiProviderInfo[],
    nextProviderId = aiProvider,
    nextModel: string | null | undefined = undefined,
    nextEditingProviderId = editingAiProvider,
  ): Promise<AgentInfo | null> => {
    const orderedProviders = normalizeProviderOrders(nextProviders);
    const providersWithModel = nextEditingProviderId && nextModel !== undefined
      ? updateProviderInfo(orderedProviders, nextEditingProviderId, { model: nextModel ?? null })
      : orderedProviders;
    const selectedProvider = orderedProviders.find((provider) => provider.id === nextEditingProviderId)
      ?? orderedProviders.find((provider) => provider.id === nextProviderId)
      ?? orderedProviders[0]
      ?? null;
    const activeProvider = providersWithModel.find((provider) => provider.id === nextProviderId)
      ?? providersWithModel.find((provider) => provider.enabled)
      ?? providersWithModel[0]
      ?? null;
    const persistedProviderId = activeProvider?.id ?? nextProviderId;
    const selectedProviderWithModel = providersWithModel.find((provider) => provider.id === selectedProvider?.id) ?? selectedProvider;
    const activeModelsForPersist = activeProvider?.models ?? [];
    const persistedModel = activeModelsForPersist.some((item) => item.value === (activeProvider?.model ?? ""))
      ? activeProvider?.model
      : defaultModelValue(activeModelsForPersist);
    const persistedAgent = await persist({
      aiProvider: persistedProviderId,
      aiProviders: providersWithModel,
      models: activeModelsForPersist,
      model: persistedModel ?? "",
    });
    if (!persistedAgent) {
      return null;
    }
    const persistedActiveProvider =
      persistedAgent.aiProviders.find((provider) => provider.id === persistedAgent.aiProvider)
      ?? persistedAgent.aiProviders.find((provider) => provider.enabled)
      ?? persistedAgent.aiProviders[0]
      ?? null;
    const persistedSelectedProvider =
      findPersistedProvider(persistedAgent.aiProviders, selectedProviderWithModel)
      ?? persistedActiveProvider;
    const persistedSelectedModels = persistedSelectedProvider?.models ?? [];
    setAiProviders(persistedAgent.aiProviders);
    setAiProvider(persistedActiveProvider?.id ?? persistedAgent.aiProvider ?? "");
    setEditingAiProvider(persistedSelectedProvider?.id ?? "");
    setModels(persistedSelectedModels);
    setModel(persistedSelectedProvider?.model ?? defaultModelValue(persistedSelectedModels) ?? "");
    return persistedAgent;
  };
  const selectAiProviderForEditing = (nextProviderId: string) => {
    const provider = aiProviders.find((item) => item.id === nextProviderId) ?? null;
    setEditingAiProvider(nextProviderId);
    setModels(provider?.models ?? []);
    setModel(provider?.model ?? defaultModelValue(provider?.models ?? []) ?? "");
    setNewModelValue("");
    setNewModelDisplayName("");
  };
  const activateAiProvider = async (nextProviderId: string) => {
    const nextProviders = aiProviders.map((provider) => ({ ...provider, enabled: provider.id === nextProviderId }));
    const provider = nextProviders.find((item) => item.id === nextProviderId) ?? null;
    const nextModel = provider?.models.some((item) => item.value === provider.model)
      ? provider.model
      : defaultModelValue(provider?.models ?? []);
    setEditingAiProvider(nextProviderId);
    await saveAstraProviders(nextProviders, nextProviderId, nextModel, nextProviderId);
  };
  const createProviderDraft = (): AgentAiProviderInfo => {
    return {
      id: "",
      displayName: t("agent.custom_provider"),
      provider: "openai",
      api: "openai-responses",
      baseUrl: defaultBaseUrlForPiProvider("openai"),
      apiKey: null,
      model: null,
      models: [],
      enabled: aiProviders.length === 0,
      order: aiProviders.length,
    };
  };
  const openAddProviderDialog = () => {
    setProviderDialog({ mode: "add", provider: createProviderDraft() });
  };
  const openEditProviderDialog = (provider: AgentAiProviderInfo) => {
    setProviderDialog({ mode: "edit", provider: { ...provider, models: provider.models.slice() } });
  };
  const saveProviderDialog = async (provider: AgentAiProviderInfo): Promise<boolean> => {
    const nextProvider = normalizeProviderOrders([provider])[0];
    if (!nextProvider) return false;
    if (providerDialog?.mode === "edit") {
      const nextProviders = updateProviderInfo(aiProviders, provider.id, nextProvider);
      const nextProviderId = aiProvider || activeAiProvider?.id || nextProvider.id;
      const selectedProvider = nextProviders.find((item) => item.id === editingAiProvider) ?? nextProvider;
      const saved = await saveAstraProviders(nextProviders, nextProviderId, undefined, selectedProvider.id);
      if (!saved) return false;
    } else {
      const providerToAdd = { ...nextProvider, enabled: aiProviders.length === 0 };
      const nextProviders = [...aiProviders, providerToAdd];
      const nextProviderId = aiProvider || activeAiProvider?.id || providerToAdd.id;
      const saved = await saveAstraProviders(nextProviders, nextProviderId, undefined, nextProvider.id);
      if (!saved) return false;
    }
    setProviderDialog(null);
    return true;
  };
  const deleteProvider = async (providerId: string) => {
    if (aiProviders.length <= 1) return;
    const nextProviders = aiProviders.filter((provider) => provider.id !== providerId);
    const nextActiveProvider = nextProviders.find((provider) => provider.id === aiProvider)
      ?? nextProviders.find((provider) => provider.enabled)
      ?? nextProviders[0]
      ?? null;
    const nextEditingProvider = nextProviders.find((provider) => provider.id === editingAiProvider)
      ?? nextActiveProvider;
    setEditingAiProvider(nextEditingProvider?.id ?? "");
    await saveAstraProviders(nextProviders, nextActiveProvider?.id ?? "", undefined, nextEditingProvider?.id ?? "");
  };

  return (
    <>
      <SettingsGroup title={agent.displayName}>
        <div className="grid gap-5">
          <div className="grid grid-cols-[auto_minmax(0,1fr)] items-start gap-3 rounded-md border border-card-border/[0.10] bg-card-chip/[0.025] p-3">
            <SettingsAgentGlyph agentId={agent.id} className="mt-1 h-5 w-5 shrink-0" />
            <div className="min-w-0">
              <div className="flex min-w-0 items-start justify-between gap-3">
                <div className="flex min-w-0 flex-wrap items-center gap-2">
                  <span className={"rounded px-1.5 py-0.5 text-meta " + (agent.enabled ? "bg-ink/[0.09] text-ink/70" : "bg-card-chip/8 text-card-muted/50")}>
                    {agent.enabled ? t("agent.active") : t("agent.disabled")}
                  </span>
                  <SettingsTransportGlyph transport={agent.transport} className="h-2.5 w-auto shrink-0 opacity-75" />
                </div>
                <SwitchControl
                  checked={agent.enabled}
                  tooltip={agent.enabled ? t("agent.disable") : t("agent.enable")}
                  onToggle={() => void persist({ enabled: !agent.enabled })}
                />
              </div>
              {showCommandSettings && (
                <div className="mt-2 grid gap-2 rounded-md border border-card-border/[0.10] bg-card-chip/[0.03] p-2.5">
                <AgentPreferenceRow label={t("agent.session_command")}>
                  <input
                    value={sessionCommandInput}
                    onChange={(event) => setSessionCommandInput(event.target.value)}
                    onBlur={() => void saveCommands(sessionCommandInput, versionCommandInput)}
                    placeholder={t("agent.session_command_placeholder")}
                    className={inputClassName + " w-full"}
                  />
                </AgentPreferenceRow>
                <AgentPreferenceRow label={t("agent.version_command")}>
                  <input
                    value={versionCommandInput}
                    onChange={(event) => setVersionCommandInput(event.target.value)}
                    onBlur={() => void saveCommands(sessionCommandInput, versionCommandInput)}
                    placeholder={t("agent.version_command_placeholder")}
                    className={inputClassName + " w-full"}
                  />
                </AgentPreferenceRow>
              </div>
              )}
            </div>
          </div>
          {isAstra && (
            <div className="grid gap-2">
              <div className="flex items-center justify-between gap-3">
                <h3 className="text-caption font-semibold text-card-fg/72">{t("agent.providers")}</h3>
                <button type="button" onClick={openAddProviderDialog} className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md px-2 text-body-sm font-medium leading-none text-card-fg/75 transition hover:bg-card-action-hover/5 hover:text-card-fg/90">
                  <Plus className="h-4 w-4" />
                  {t("agent.add_provider")}
                </button>
              </div>
              <div className="overflow-hidden rounded-md border border-card-border/[0.10] bg-card-chip/[0.025]">
                {aiProviders.map((provider) => (
                  <AgentProviderRow
                    key={provider.id}
                    provider={provider}
                    selected={provider.id === selectedAiProvider?.id}
                    canDelete={aiProviders.length > 1}
                    onSelect={selectAiProviderForEditing}
                    onActivate={activateAiProvider}
                    onEdit={openEditProviderDialog}
                    onDelete={deleteProvider}
                  />
                ))}
                {aiProviders.length === 0 && <div className="p-3"><EmptyState label={t("agent.no_providers")} /></div>}
              </div>
            </div>
          )}
          <div className="grid gap-2">
            <h3 className="text-caption font-semibold text-card-fg/72">{t("agent.preferences")}</h3>
            <div className="grid gap-2 rounded-md border border-card-border/[0.10] bg-card-chip/[0.025] p-3">
              <AgentPreferenceRow label={t("assistant.model")}>
                <AgentInlineSelect
                  value={effectiveModel}
                  options={modelOptions}
                  placeholder={t("agent.no_model")}
                  onChange={(value) => void selectModel(value)}
                />
              </AgentPreferenceRow>
              {agent.efforts.length > 0 && (
                <AgentPreferenceRow label={isAstra ? t("agent.thinking_level") : t("assistant.effort")}>
                  <AgentInlineSelect
                    value={effort}
                    options={effortOptions}
                    placeholder={isAstra ? t("agent.thinking_level") : t("assistant.effort")}
                    onChange={(value) => void selectEffort(value)}
                  />
                </AgentPreferenceRow>
              )}
              {agent.permissionModes.length > 0 && (
                <AgentPreferenceRow label={t("assistant.permission_mode")}>
                  <AgentInlineSelect
                    value={permissionMode}
                    options={permissionOptions}
                    placeholder={t("assistant.permission_mode")}
                    onChange={(value) => void selectPermissionMode(value)}
                  />
                </AgentPreferenceRow>
              )}
            </div>
          </div>
          <div className="grid gap-3">
            <h3 className="text-caption font-semibold text-card-fg/72">{isAstra ? t("agent.provider_models") : t("agent.models")}</h3>
            <div className="grid gap-3 rounded-md border border-card-border/[0.10] bg-card-chip/[0.025] p-3">
              <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,0.75fr)_auto] gap-2">
                <input value={newModelValue} onChange={(event) => setNewModelValue(event.target.value)} placeholder={t("agent.model_id")} className={inputClassName} />
                <input value={newModelDisplayName} onChange={(event) => setNewModelDisplayName(event.target.value)} placeholder={t("agent.model_name")} className={inputClassName} />
                <button type="button" onClick={() => void addModel()} disabled={!newModelValue.trim() || (isAstra && !selectedAiProvider)} className="inline-flex h-9 items-center justify-center gap-1.5 rounded-md bg-ink px-3 text-body-sm font-medium text-[rgb(var(--color-bg-panel))] disabled:opacity-35">
                  <Plus className="h-4 w-4" />
                  {t("agent.add_model")}
                </button>
              </div>
              <DragDropProvider onDragEnd={handleModelDragEnd}>
                <div className="overflow-hidden rounded-md border border-card-border/[0.10]">
                  {activeModels.map((item, index) => (
                    <AgentModelRow
                      key={item.value}
                      item={item}
                      index={index}
                      defaultModel={item.value === effectiveModel}
                      canSetDefault
                      onSetDefault={setDefaultModel}
                      onSave={saveModel}
                      onToggleEnabled={toggleModelEnabled}
                      onDelete={deleteModel}
                    />
                  ))}
                  {activeModels.length === 0 && <div className="p-3"><EmptyState label={t("agent.no_models")} /></div>}
                </div>
              </DragDropProvider>
            </div>
          </div>
        </div>
      </SettingsGroup>
      {providerDialog && (
        <AgentProviderDialog
          mode={providerDialog.mode}
          provider={providerDialog.provider}
          onClose={() => setProviderDialog(null)}
          onSave={saveProviderDialog}
        />
      )}
    </>
  );
}

function AgentProviderRow({
  provider,
  selected,
  canDelete,
  onSelect,
  onActivate,
  onEdit,
  onDelete,
}: {
  provider: AgentAiProviderInfo;
  selected: boolean;
  canDelete: boolean;
  onSelect: (providerId: string) => void;
  onActivate: (providerId: string) => Promise<void>;
  onEdit: (provider: AgentAiProviderInfo) => void;
  onDelete: (providerId: string) => Promise<void>;
}) {
  const { t } = useI18n();
  const detailItems = [
    provider.provider,
    provider.api,
    t("agent.model_count", { count: provider.models.length }),
  ].filter((item) => item && item.trim().length > 0);

  return (
    <div className={"grid min-h-14 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 border-b px-3 py-2.5 transition last:border-b-0 " + (selected ? "border-card-border/[0.16] bg-[rgb(var(--color-card-fg)/0.105)]" : "border-card-border/[0.08] hover:bg-card-action-hover/5")}>
      <button type="button" onClick={() => void onSelect(provider.id)} className="flex min-w-0 items-center gap-3 text-left">
        <ProviderGlyph provider={provider} />
        <span className="min-w-0">
          <span className={"block truncate text-body-sm font-medium " + (selected ? "text-card-fg/92" : "text-card-fg/82")}>{provider.displayName || provider.provider || provider.id}</span>
          <span className="mt-0.5 block truncate text-caption text-card-muted/56">{detailItems.join(" / ")}</span>
        </span>
      </button>
      <div className="flex items-center gap-1.5">
        <Tooltip content={t("agent.edit_provider")} placement="top">
          <button type="button" onClick={() => onEdit(provider)} className="rounded p-1.5 text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75">
            <Pencil className="h-4 w-4" />
          </button>
        </Tooltip>
        <Tooltip content={t("agent.delete_provider")} placement="top">
          <button type="button" onClick={() => void onDelete(provider.id)} disabled={!canDelete} className="rounded p-1.5 text-card-subtle/45 hover:bg-status-error/10 hover:text-status-error disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-card-subtle/45">
            <Trash2 className="h-4 w-4" />
          </button>
        </Tooltip>
        <SwitchControl
          checked={provider.enabled}
          tooltip={provider.enabled ? t("agent.active_provider") : t("agent.activate_provider")}
          onToggle={() => void onActivate(provider.id)}
        />
      </div>
    </div>
  );
}

function AgentProviderDialog({
  mode,
  provider,
  onClose,
  onSave,
}: {
  mode: "add" | "edit";
  provider: AgentAiProviderInfo;
  onClose: () => void;
  onSave: (provider: AgentAiProviderInfo) => Promise<boolean>;
}) {
  const { t } = useI18n();
  const [displayName, setDisplayName] = useState(provider.displayName);
  const [piProvider, setPiProvider] = useState(provider.provider);
  const [api, setApi] = useState(provider.api ?? "");
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl ?? "");
  const [apiKey, setApiKey] = useState(provider.apiKey ?? "");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setDisplayName(provider.displayName);
    setPiProvider(provider.provider);
    setApi(provider.api ?? "");
    setBaseUrl(provider.baseUrl ?? "");
    setApiKey(provider.apiKey ?? "");
    setSaving(false);
  }, [provider]);

  const save = async () => {
    if (saving) return;
    const nextProvider: AgentAiProviderInfo = {
      ...provider,
      displayName: displayName.trim() || piProvider.trim() || provider.id,
      provider: piProvider.trim() || provider.id,
      api: api.trim() || null,
      baseUrl: baseUrl.trim() || null,
      apiKey: apiKey.trim() || null,
    };
    setSaving(true);
    const saved = await onSave(nextProvider);
    if (!saved) setSaving(false);
  };
  const selectPiProvider = (nextPiProvider: string) => {
    const currentDefaultBaseUrl = defaultBaseUrlForPiProvider(piProvider);
    const nextDefaultBaseUrl = defaultBaseUrlForPiProvider(nextPiProvider);
    const shouldReplaceBaseUrl =
      !baseUrl.trim() || (currentDefaultBaseUrl !== null && baseUrl.trim() === currentDefaultBaseUrl);
    setPiProvider(nextPiProvider);
    if (nextDefaultBaseUrl && shouldReplaceBaseUrl) setBaseUrl(nextDefaultBaseUrl);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 px-4" onClick={() => {
      if (!saving) onClose();
    }}>
      <div className="w-full max-w-[520px] rounded-lg border border-card-border/[0.12] bg-surface-panel p-4 shadow-[0_24px_80px_rgba(0,0,0,0.22)]" onClick={(event) => event.stopPropagation()}>
        <div className="mb-3 text-body-sm font-semibold text-ink/[0.88]">{mode === "add" ? t("agent.add_provider") : t("agent.edit_provider")}</div>
        <div className="grid gap-2">
          <AgentProviderDialogField label={t("agent.provider_name")}>
            <input value={displayName} onChange={(event) => setDisplayName(event.target.value)} placeholder="OpenAI" className={inputClassName} />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.pi_provider")}>
            <AgentDialogSelect
              value={piProvider}
              options={piProviderOptions(piProvider)}
              placeholder={t("agent.pi_provider")}
              onChange={selectPiProvider}
            />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.pi_api")}>
            <AgentDialogSelect
              value={api}
              options={piApiOptions(api)}
              placeholder={t("agent.pi_api")}
              onChange={setApi}
            />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.api_base_url")}>
            <input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} placeholder="https://api.openai.com/v1" className={inputClassName} />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.api_key")}>
            <input value={apiKey} type="password" onChange={(event) => setApiKey(event.target.value)} placeholder="sk-..." className={inputClassName} />
          </AgentProviderDialogField>
          <div className="mt-1 flex justify-end gap-2">
            <button type="button" onClick={onClose} disabled={saving} className="rounded-md px-3 py-1.5 text-body-sm text-ink/45 hover:bg-ink/5 disabled:opacity-35">{t("delete.cancel")}</button>
            <button type="button" onClick={() => void save()} disabled={saving || !piProvider.trim()} className="inline-flex h-8 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] hover:text-card-fg/90 disabled:opacity-35">
              <Check className="h-4 w-4" />
              {t("project.save")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function AgentProviderDialogField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="grid gap-1.5">
      <span className="text-caption font-medium text-card-muted/60">{label}</span>
      {children}
    </label>
  );
}

function AgentDialogSelect({
  value,
  options,
  placeholder,
  onChange,
}: {
  value: string;
  options: Array<{ value: string; label: string; description?: string }>;
  placeholder: string;
  onChange: (value: string) => void;
}) {
  return (
    <InlineMenuSelect
      value={value}
      options={options}
      onChange={onChange}
      menuAlign="trigger"
      placeholder={placeholder}
      ariaLabel={placeholder}
      className="h-9 w-full !max-w-full rounded-md border border-input-border/[0.16] bg-input px-3 text-input-fg hover:text-input-fg"
      menuClassName="bg-surface-panel"
      minMenuWidth={260}
      emptyContent={placeholder}
    />
  );
}

function ProviderGlyph({ provider }: { provider: AgentAiProviderInfo }) {
  const Icon = providerIconFor(provider.provider);
  const className = "h-5 w-5 shrink-0";
  if (Icon) return <Icon className={className} />;
  const label = provider.displayName.trim() || provider.provider.trim() || provider.id;
  const initial = label.trim().charAt(0).toUpperCase() || "?";
  return (
    <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-card-border/[0.16] bg-card-chip/[0.08] text-caption font-semibold text-card-fg/70">
      {initial}
    </span>
  );
}

type ProviderIconComponent = (props: { className?: string }) => ReactNode;

function providerIconFor(provider: string): ProviderIconComponent | null {
  const key = normalizeProviderKey(provider);
  const icon = PROVIDER_ICON_BY_KEY[key];
  if (icon) return icon;
  if (key.includes("openai")) return OpenAI;
  if (key.includes("anthropic") || key.includes("claude")) return iconColor(Anthropic, Claude.Color);
  if (key.includes("google")) return iconColor(Google, Google.Color);
  if (key.includes("azure")) return iconColor(Azure, Azure);
  if (key.includes("githubcopilot")) return iconColor(GithubCopilot, GithubCopilot);
  if (key.includes("cloudflare")) return iconColor(Cloudflare, Cloudflare);
  if (key.includes("moonshot") || key.includes("kimi")) return iconColor(Moonshot, iconColor(Kimi, Moonshot));
  if (key.includes("minimax")) return iconColor(Minimax, Minimax);
  if (key.includes("xiaomi")) return iconColor(XiaomiMiMo, XiaomiMiMo);
  return null;
}

function iconColor(icon: unknown, fallback: ProviderIconComponent): ProviderIconComponent {
  return ((icon as { Color?: ProviderIconComponent }).Color ?? fallback);
}

function normalizeProviderKey(provider: string): string {
  return provider.trim().toLowerCase().replace(/[^a-z0-9]/g, "");
}

function AgentModelRow({
  item,
  index,
  defaultModel,
  canSetDefault,
  onSetDefault,
  onSave,
  onToggleEnabled,
  onDelete,
}: {
  item: RuntimeAgentOptionMetadata;
  index: number;
  defaultModel: boolean;
  canSetDefault: boolean;
  onSetDefault: (value: string) => Promise<void>;
  onSave: (previousValue: string, nextValue: string, nextLabel: string) => Promise<void>;
  onToggleEnabled: (value: string) => Promise<void>;
  onDelete: (value: string) => Promise<void>;
}) {
  const { t } = useI18n();
  const { handleRef, isDragSource, isDropTarget, ref } = useSortable({
    id: item.value,
    index,
    group: "agent-models",
    transition: {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
      idle: true,
    },
  });
  const [editing, setEditing] = useState(false);
  const [valueDraft, setValueDraft] = useState(item.value);
  const [displayNameDraft, setDisplayNameDraft] = useState(item.displayName);

  useEffect(() => {
    setValueDraft(item.value);
    setDisplayNameDraft(item.displayName);
    setEditing(false);
  }, [item]);

  const save = async () => {
    await onSave(item.value, valueDraft, displayNameDraft);
    setEditing(false);
  };

  return (
    <div
      ref={ref}
      className={
        "grid min-h-12 grid-cols-[auto_minmax(0,1fr)_minmax(0,0.72fr)_auto_auto_auto_auto] items-center gap-2 border-b border-card-border/[0.08] px-3 py-2 transition last:border-b-0 " +
        (isDragSource
          ? "z-20 cursor-grabbing bg-card shadow-[0_12px_28px_rgba(0,0,0,0.22)]"
          : isDropTarget
            ? "bg-card-active shadow-[inset_3px_0_0_rgb(var(--color-card-fg)/0.32)]"
            : "")
      }
    >
      <button ref={handleRef} type="button" className="cursor-grab touch-none rounded p-0.5 text-card-subtle/35 hover:bg-card-action-hover/5 hover:text-card-fg/60 active:cursor-grabbing">
        <GripVertical className="h-4 w-4" />
      </button>
      <div className="min-w-0">
        {editing ? (
          <input
            value={valueDraft}
            onChange={(event) => setValueDraft(event.target.value)}
            className="h-8 w-full min-w-0 rounded-md border border-input-border/[0.12] bg-input px-2 text-caption text-input-fg outline-none focus:border-input-focus/30"
          />
        ) : (
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-body-sm font-medium text-card-fg/80">{item.value}</span>
            {defaultModel && <span className="rounded bg-card-chip/10 px-1.5 py-0.5 text-meta text-card-chip-fg/60">{t("agent.default_model")}</span>}
          </div>
        )}
      </div>
      <div className="min-w-0">
        {editing ? (
          <input
            value={displayNameDraft}
            onChange={(event) => setDisplayNameDraft(event.target.value)}
            className="h-8 w-full min-w-0 rounded-md border border-input-border/[0.12] bg-input px-2 text-caption text-input-fg outline-none focus:border-input-focus/30"
          />
        ) : (
          <span className="block truncate text-caption text-card-muted/62">{item.displayName || item.value}</span>
        )}
      </div>
      <Tooltip content={canSetDefault ? t("agent.make_default") : t("agent.activate_provider")} placement="top">
        <button
          type="button"
          role="radio"
          aria-checked={defaultModel}
          disabled={!canSetDefault}
          onClick={() => void onSetDefault(item.value)}
          className={`rounded p-1 disabled:opacity-35 ${defaultModel ? "bg-card-chip/[0.12] text-card-fg/75" : "text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75 disabled:hover:bg-transparent disabled:hover:text-card-subtle/45"}`}
        >
          {defaultModel ? <Check className="h-4 w-4" /> : <Circle className="h-4 w-4" />}
        </button>
      </Tooltip>
      <Tooltip content={editing ? t("project.save") : t("agent.edit_model")} placement="top">
        <button type="button" onClick={() => editing ? void save() : setEditing(true)} className="rounded p-1 text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75">
          {editing ? <Check className="h-4 w-4" /> : <Pencil className="h-4 w-4" />}
        </button>
      </Tooltip>
      <Tooltip content={t("agent.delete_model")} placement="top">
        <button type="button" onClick={() => void onDelete(item.value)} className="rounded p-1 text-card-subtle/45 hover:bg-status-error/10 hover:text-status-error">
          <Trash2 className="h-4 w-4" />
        </button>
      </Tooltip>
      <SwitchControl
        checked={item.enabled}
        tooltip={item.enabled ? t("agent.disable_model") : t("agent.enable_model")}
        onToggle={() => void onToggleEnabled(item.value)}
      />
    </div>
  );
}

function AgentPreferenceRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid min-h-10 grid-cols-[120px_minmax(0,1fr)] items-center gap-3 rounded-md bg-card-chip/[0.04] px-3">
      <span className="text-caption font-medium text-card-muted/60">{label}</span>
      <div className="min-w-0 justify-self-stretch">{children}</div>
    </div>
  );
}

function AgentInlineSelect({
  value,
  options,
  placeholder,
  onChange,
}: {
  value: string;
  options: Array<{ value: string; label: string; icon?: ReactNode }>;
  placeholder: string;
  onChange: (value: string) => void;
}) {
  return (
    <div className="flex min-w-0 max-w-[320px] items-center rounded-md text-ink/55 transition hover:bg-ink/8 hover:text-ink">
      <InlineMenuSelect
        value={value}
        options={options}
        onChange={onChange}
        menuAlign="trigger"
        placeholder={placeholder}
        ariaLabel={placeholder}
        className="h-7 max-w-[320px] border-r-0 px-1.5 py-1 text-ink/60 hover:text-ink"
        menuClassName="bg-surface-panel"
        minMenuWidth={220}
        emptyContent={placeholder}
      />
    </div>
  );
}

function optionRows(options: RuntimeAgentOptionMetadata[], selected: string): RuntimeAgentOptionMetadata[] {
  const rows = options
    .filter((option) => option.value.trim().length > 0 && option.enabled)
    .map((option) => ({ ...option, label: option.displayName || option.label || option.value }));
  if (selected && !rows.some((option) => option.value === selected)) {
    return [{ value: selected, label: selected, displayName: selected, enabled: true, order: -1 }, ...rows];
  }
  return rows;
}

function defaultModelValue(options: RuntimeAgentOptionMetadata[]): string | null {
  return options.find((item) => item.enabled)?.value ?? options[0]?.value ?? null;
}

function moveOption(options: RuntimeAgentOptionMetadata[], from: number, to: number): RuntimeAgentOptionMetadata[] {
  if (from < 0 || to < 0 || from >= options.length || to >= options.length || from === to) return options;
  const next = [...options];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

function normalizeModelOrders(options: RuntimeAgentOptionMetadata[]): RuntimeAgentOptionMetadata[] {
  return options.map((option, index) => ({
    ...option,
    label: option.displayName || option.label || option.value,
    displayName: option.displayName || option.label || option.value,
    enabled: option.enabled ?? true,
    order: index,
  }));
}

function normalizeProviderOrders(options: AgentAiProviderInfo[]): AgentAiProviderInfo[] {
  return options.map((provider, index) => ({
    ...provider,
    id: provider.id.trim(),
    displayName: provider.displayName.trim() || provider.provider.trim() || provider.id,
    provider: provider.provider.trim() || provider.id.trim() || `provider-${index + 1}`,
    api: provider.api?.trim() || null,
    baseUrl: provider.baseUrl?.trim() || null,
    apiKey: provider.apiKey?.trim() || null,
    model: provider.model?.trim() || null,
    models: normalizeModelOrders(provider.models),
    enabled: provider.enabled ?? true,
    order: index,
  }));
}

const PI_PROVIDER_PRESETS = [
  "openai",
  "anthropic",
  "google",
  "google-vertex",
  "amazon-bedrock",
  "azure-openai-responses",
  "openai-codex",
  "deepseek",
  "github-copilot",
  "xai",
  "groq",
  "cerebras",
  "openrouter",
  "vercel-ai-gateway",
  "zai",
  "mistral",
  "minimax",
  "minimax-cn",
  "moonshotai",
  "moonshotai-cn",
  "huggingface",
  "fireworks",
  "together",
  "opencode",
  "opencode-go",
  "kimi-coding",
  "cloudflare-workers-ai",
  "cloudflare-ai-gateway",
  "xiaomi",
  "xiaomi-token-plan-cn",
  "xiaomi-token-plan-ams",
  "xiaomi-token-plan-sgp",
];

const PROVIDER_LABEL_BY_ID: Record<string, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  google: "Google AI",
  "google-vertex": "Google Vertex AI",
  "amazon-bedrock": "Amazon Bedrock",
  "azure-openai-responses": "Azure OpenAI",
  "openai-codex": "OpenAI Codex",
  deepseek: "DeepSeek",
  "github-copilot": "GitHub Copilot",
  xai: "xAI",
  groq: "Groq",
  cerebras: "Cerebras",
  openrouter: "OpenRouter",
  "vercel-ai-gateway": "Vercel AI Gateway",
  zai: "Z.ai",
  mistral: "Mistral",
  minimax: "MiniMax",
  "minimax-cn": "MiniMax CN",
  moonshotai: "Moonshot AI",
  "moonshotai-cn": "Moonshot AI CN",
  huggingface: "Hugging Face",
  fireworks: "Fireworks",
  together: "Together AI",
  opencode: "OpenCode",
  "opencode-go": "OpenCode Go",
  "kimi-coding": "Kimi Coding",
  "cloudflare-workers-ai": "Cloudflare Workers AI",
  "cloudflare-ai-gateway": "Cloudflare AI Gateway",
  xiaomi: "Xiaomi",
  "xiaomi-token-plan-cn": "Xiaomi Token Plan CN",
  "xiaomi-token-plan-ams": "Xiaomi Token Plan AMS",
  "xiaomi-token-plan-sgp": "Xiaomi Token Plan SGP",
};

const PROVIDER_ICON_BY_KEY: Record<string, ProviderIconComponent> = {
  openai: OpenAI,
  anthropic: iconColor(Anthropic, Claude.Color),
  google: iconColor(Google, Google.Color),
  googlevertex: iconColor(VertexAI, iconColor(GoogleCloud, iconColor(Google, Google.Color))),
  amazonbedrock: iconColor(Bedrock, iconColor(Aws, Aws)),
  azureopenairesponses: iconColor(Azure, Azure),
  openaicodex: OpenAI,
  deepseek: iconColor(DeepSeek, DeepSeek),
  githubcopilot: iconColor(GithubCopilot, GithubCopilot),
  xai: iconColor(XAI, XAI),
  groq: iconColor(Groq, Groq),
  cerebras: iconColor(Cerebras, Cerebras),
  openrouter: iconColor(OpenRouter, OpenRouter),
  vercelaigateway: iconColor(Vercel, Vercel),
  zai: iconColor(ZAI, ZAI),
  mistral: iconColor(Mistral, Mistral),
  minimax: iconColor(Minimax, Minimax),
  minimaxcn: iconColor(Minimax, Minimax),
  moonshotai: iconColor(Moonshot, Moonshot),
  moonshotaicn: iconColor(Moonshot, Moonshot),
  huggingface: iconColor(HuggingFace, HuggingFace),
  fireworks: iconColor(Fireworks, Fireworks),
  together: iconColor(Together, Together),
  opencode: iconColor(OpenCode, OpenCode),
  opencodego: iconColor(OpenCode, OpenCode),
  kimicoding: iconColor(Kimi, iconColor(Moonshot, Kimi)),
  cloudflareworkersai: iconColor(WorkersAI, iconColor(Cloudflare, Cloudflare)),
  cloudflareaigateway: iconColor(Cloudflare, Cloudflare),
  xiaomi: iconColor(XiaomiMiMo, XiaomiMiMo),
  xiaomitokenplancn: iconColor(XiaomiMiMo, XiaomiMiMo),
  xiaomitokenplanams: iconColor(XiaomiMiMo, XiaomiMiMo),
  xiaomitokenplansgp: iconColor(XiaomiMiMo, XiaomiMiMo),
};

const PI_API_PRESETS = [
  "openai-responses",
  "openai-completions",
  "openai-codex-responses",
  "anthropic-messages",
  "google-generative-ai",
  "google-vertex",
  "azure-openai-responses",
  "bedrock-converse-stream",
  "mistral-conversations",
];

const DEFAULT_BASE_URL_BY_PI_PROVIDER: Record<string, string> = {
  openai: "https://api.openai.com/v1",
  anthropic: "https://api.anthropic.com/v1",
  google: "https://generativelanguage.googleapis.com/v1beta",
  "google-vertex": "https://generativelanguage.googleapis.com/v1beta",
  "azure-openai-responses": "https://{resource}.openai.azure.com/openai/v1",
  deepseek: "https://api.deepseek.com/v1",
  xai: "https://api.x.ai/v1",
  groq: "https://api.groq.com/openai/v1",
  cerebras: "https://api.cerebras.ai/v1",
  openrouter: "https://openrouter.ai/api/v1",
  "vercel-ai-gateway": "https://ai-gateway.vercel.sh/v1",
  zai: "https://api.z.ai/api/paas/v4",
  mistral: "https://api.mistral.ai/v1",
  minimax: "https://api.minimax.io/v1",
  "minimax-cn": "https://api.minimax.chat/v1",
  moonshotai: "https://api.moonshot.ai/v1",
  "moonshotai-cn": "https://api.moonshot.cn/v1",
  huggingface: "https://router.huggingface.co/v1",
  fireworks: "https://api.fireworks.ai/inference/v1",
  together: "https://api.together.xyz/v1",
  "kimi-coding": "https://api.kimi.com/v1",
  "cloudflare-ai-gateway": "https://gateway.ai.cloudflare.com/v1",
};

function presetOptions(presets: string[], selected: string, labels: Record<string, string> = {}): Array<{ value: string; label: string }> {
  const options = presets.map((value) => ({ value, label: labels[value] ?? value }));
  if (selected && !presets.includes(selected)) return [{ value: selected, label: labels[selected] ?? selected }, ...options];
  return options;
}

function piProviderOptions(selected: string): Array<{ value: string; label: string; icon?: ReactNode; menuIcon?: ReactNode }> {
  return presetOptions(PI_PROVIDER_PRESETS, selected, PROVIDER_LABEL_BY_ID).map((option) => {
    const Icon = providerIconFor(option.value);
    return {
      ...option,
      icon: Icon ? <Icon className="h-4 w-4" /> : undefined,
      menuIcon: Icon ? <Icon className="h-4 w-4" /> : undefined,
    };
  });
}

function piApiOptions(selected: string): Array<{ value: string; label: string }> {
  return presetOptions(PI_API_PRESETS, selected);
}

function defaultBaseUrlForPiProvider(provider: string): string | null {
  return DEFAULT_BASE_URL_BY_PI_PROVIDER[provider.trim()] ?? null;
}

function updateProviderInfo(
  providers: AgentAiProviderInfo[],
  providerId: string,
  patch: Partial<AgentAiProviderInfo>,
): AgentAiProviderInfo[] {
  return providers.map((provider) => provider.id === providerId ? { ...provider, ...patch } : provider);
}

function updateProviderModels(
  providers: AgentAiProviderInfo[],
  providerId: string,
  models: RuntimeAgentOptionMetadata[],
): AgentAiProviderInfo[] {
  return updateProviderInfo(providers, providerId, { models });
}

function findPersistedProvider(
  providers: AgentAiProviderInfo[],
  draft: AgentAiProviderInfo | null,
): AgentAiProviderInfo | null {
  if (!draft) return null;
  if (draft.id) {
    const byId = providers.find((provider) => provider.id === draft.id);
    if (byId) return byId;
  }
  return providers.find((provider) => provider.order === draft.order) ?? null;
}

function moveAgentOption(options: AgentInfo[], from: number, to: number): AgentInfo[] {
  if (from < 0 || to < 0 || from >= options.length || to >= options.length || from === to) return options;
  const next = [...options];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

function SettingsAgentGlyph({
  agentId,
  className,
}: {
  agentId: string;
  className?: string;
}) {
  if (isRuntimeAgent(agentId)) {
    return <AgentGlyph agent={agentId} className={className} />;
  }
  return <AiGenerate2Icon className={className} />;
}

function SettingsTransportGlyph({
  transport,
  className,
}: {
  transport: AgentInfo["transport"];
  className?: string;
}) {
  if (transport !== "acp") return null;
  return <AcpLogo className={className} />;
}

function AcpLogo({ className }: { className?: string }) {
  const theme = useEffectiveThemeType();
  return (
    <Tooltip content="ACP" placement="top">
      <span className="inline-flex shrink-0 items-center">
        <img src={theme === "light" ? acpMarkBlackUrl : acpMarkWhiteUrl} alt="ACP" className={className} draggable={false} />
      </span>
    </Tooltip>
  );
}

function useEffectiveThemeType(): "light" | "dark" {
  const [themeType, setThemeType] = useState<"light" | "dark">(() =>
    document.documentElement.getAttribute("data-theme") === "light" ? "light" : "dark",
  );

  useEffect(() => {
    const root = document.documentElement;
    const update = () => {
      setThemeType(root.getAttribute("data-theme") === "light" ? "light" : "dark");
    };
    update();
    const observer = new MutationObserver(update);
    observer.observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    return () => observer.disconnect();
  }, []);

  return themeType;
}

function isRuntimeAgent(id: string): id is Agent {
  return isAgent(id);
}

function isSettingsAgent(id: string): boolean {
  return isRuntimeAgent(id);
}

function AssistantsSettings({ onError }: { onError: (error: string | null) => void }) {
  const { t } = useI18n();
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [tab, setTab] = useState<"builtin" | "custom">("builtin");
  const [showCreate, setShowCreate] = useState(false);

  const reload = async () => {
    setLoading(true);
    try {
      const [assistantRows, agentRows] = await Promise.all([
        listAssistants(null),
        listAgents(),
      ]);
      setAssistants(assistantRows);
      setAgents(agentRows);
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const sharedAssistants = assistants.filter((assistant) => assistant.projectId === null && assistant.processTemplateId === null);
  const builtin = sharedAssistants.filter((assistant) => assistant.type === "builtin");
  const custom = sharedAssistants.filter((assistant) => assistant.type === "custom");
  const visible = tab === "builtin" ? builtin : custom;

  return (
    <section>
      <div className="mb-3 flex items-center justify-between gap-4">
        <SegmentedTabs
          items={[
            { value: "builtin", label: t("assistant.builtin") },
            { value: "custom", label: t("assistant.custom") },
          ]}
          value={tab}
          onChange={setTab}
          itemWidth={96}
          itemHeight={28}
          padding={3}
        />
        <button type="button" onClick={() => setShowCreate(true)} className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] hover:text-card-fg/90">
          <Plus className="h-4 w-4" />
          {t("assistant.add")}
        </button>
      </div>
      <div className="rounded-lg border border-card-border/[0.12] bg-card p-5">
        <div className="grid gap-2">
          {visible.map((assistant) => (
            <AssistantCard
              key={assistant.id}
              assistant={assistant}
              agents={agents}
              onUpdated={(next) => setAssistants((prev) => prev.map((item) => item.id === next.id ? next : item))}
              onDeleted={(id) => setAssistants((prev) => prev.filter((item) => item.id !== id))}
              onError={onError}
            />
          ))}
          {!loading && visible.length === 0 && <EmptyState label={t("assistant.empty")} />}
        </div>
      </div>
      {showCreate && (
        <CreateAssistantDialog
          agents={agents}
          onCreated={(assistant) => setAssistants((prev) => [assistant, ...prev])}
          onClose={() => setShowCreate(false)}
          onError={onError}
        />
      )}
    </section>
  );
}

function ProcessTemplatesSettings({ onError }: { onError: (error: string | null) => void }) {
  const { t } = useI18n();
  const [processTemplates, setProcessTemplates] = useState<ProcessTemplateInfo[]>([]);
  const [selectedProcessTemplateId, setSelectedProcessTemplateId] = useState("code");
  const [stages, setStages] = useState<ProjectStageInfo[]>([]);
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);
  const [newProcessTemplateName, setNewProcessTemplateName] = useState("");
  const [newProcessTemplateDescription, setNewProcessTemplateDescription] = useState("");
  const [showCreateProcessTemplate, setShowCreateProcessTemplate] = useState(false);
  const [loading, setLoading] = useState(true);

  const selectedProcessTemplate = processTemplates.find((process) => process.id === selectedProcessTemplateId) ?? processTemplates[0] ?? null;
  const availableAssistants = assistants.filter((assistant) => assistant.enabled && assistant.projectId === null && (
    assistant.processTemplateId === selectedProcessTemplateId ||
    (assistant.processTemplateId === null && assistant.type === "custom")
  ));

  const reloadAll = async () => {
    setLoading(true);
    try {
      const [processRows, assistantRows] = await Promise.all([listProcessTemplates(), listAssistants(null)]);
      setProcessTemplates(processRows);
      setAssistants(assistantRows);
      setSelectedProcessTemplateId((current) => processRows.some((process) => process.id === current) ? current : processRows[0]?.id ?? "code");
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  };

  const reloadStages = async (processTemplateId: string) => {
    try {
      setStages(await listProcessTemplateStages(processTemplateId));
    } catch (err) {
      onError(String(err));
    }
  };

  useEffect(() => {
    void reloadAll();
  }, []);

  useEffect(() => {
    if (selectedProcessTemplateId) void reloadStages(selectedProcessTemplateId);
  }, [selectedProcessTemplateId]);

  const createNewProcessTemplate = async () => {
    const name = newProcessTemplateName.trim();
    if (!name) return;
    try {
      const process = await createProcessTemplate(name, newProcessTemplateDescription);
      setProcessTemplates((prev) => [...prev, process]);
      setSelectedProcessTemplateId(process.id);
      setNewProcessTemplateName("");
      setNewProcessTemplateDescription("");
      setShowCreateProcessTemplate(false);
    } catch (err) {
      onError(String(err));
    }
  };

  const refreshStages = async () => {
    if (selectedProcessTemplateId) await reloadStages(selectedProcessTemplateId);
  };

  const processDescription = (process: ProcessTemplateInfo) => {
    if (!process.description) return t("settings.process_template_no_description");
    return process.type === "builtin" ? t(process.description) : process.description;
  };

  return (
    <section>
      <div className="grid grid-cols-[240px_minmax(0,1fr)] gap-5">
        <div className="min-w-0">
          <SettingsGroup
            title={t("settings.process_templates")}
            flush
            action={
              <button type="button" onClick={() => setShowCreateProcessTemplate(true)} className="inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 text-body-sm font-medium leading-none text-card-fg/75 transition hover:text-card-fg/90">
                <Plus className="h-3.5 w-3.5" />
                {t("settings.add_process_template")}
              </button>
            }
          >
            <div className="divide-y divide-card-border/10">
              {processTemplates.map((process) => (
                <Tooltip key={process.id} content={processDescription(process)} placement="right">
                  <button
                    type="button"
                    onClick={() => setSelectedProcessTemplateId(process.id)}
                    className={"process-template-list-item flex h-10 w-full min-w-0 items-center justify-between px-3 text-left text-body-sm transition " + (process.id === selectedProcessTemplateId ? "process-template-list-item-active" : "")}
                  >
                    <span className="truncate">{process.name}</span>
                    <span className="rounded bg-card-chip/8 px-1.5 py-0.5 text-meta text-card-chip-fg/55">{process.type}</span>
                  </button>
                </Tooltip>
              ))}
            </div>
          </SettingsGroup>
        </div>
        <div className="min-w-0">
          {selectedProcessTemplate && (
            <ProcessTemplateEditor
              stages={stages}
              assistants={availableAssistants}
              loading={loading}
              processTemplateId={selectedProcessTemplateId}
              onStageCreated={(stage) => setStages((prev) => [...prev, stage].sort((a, b) => a.order - b.order))}
              onStageUpdated={(stage) => setStages((prev) => prev.map((item) => item.id === stage.id ? stage : item).sort((a, b) => a.order - b.order))}
              onStagesReload={refreshStages}
              onStageDeleted={(id) => setStages((prev) => prev.filter((stage) => stage.id !== id))}
              onError={onError}
            />
          )}
        </div>
      </div>
      {showCreateProcessTemplate && (
        <CreateProcessTemplateDialog
          name={newProcessTemplateName}
          description={newProcessTemplateDescription}
          onNameChange={setNewProcessTemplateName}
          onDescriptionChange={setNewProcessTemplateDescription}
          onCreate={() => void createNewProcessTemplate()}
          onClose={() => setShowCreateProcessTemplate(false)}
        />
      )}
    </section>
  );
}

function CreateProcessTemplateDialog({
  name,
  description,
  onNameChange,
  onDescriptionChange,
  onCreate,
  onClose,
}: {
  name: string;
  description: string;
  onNameChange: (value: string) => void;
  onDescriptionChange: (value: string) => void;
  onCreate: () => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 px-4" onClick={onClose}>
      <div className="w-full max-w-[520px] rounded-lg border border-card-border/[0.12] bg-surface-panel p-4 shadow-[0_24px_80px_rgba(0,0,0,0.22)]" onClick={(event) => event.stopPropagation()}>
        <div className="mb-3 text-body-sm font-semibold text-ink/[0.88]">{t("settings.add_process_template")}</div>
        <div className="grid gap-2">
          <input value={name} onChange={(event) => onNameChange(event.target.value)} placeholder={t("settings.process_template_name")} className={inputClassName} />
          <textarea value={description} onChange={(event) => onDescriptionChange(event.target.value)} placeholder={t("settings.process_template_description")} rows={3} className={textareaClassName} />
          <div className="flex justify-end gap-2">
            <button type="button" onClick={onClose} className="rounded-md px-3 py-1.5 text-body-sm text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
            <button type="button" onClick={onCreate} disabled={!name.trim()} className="inline-flex h-8 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] hover:text-card-fg/90 disabled:opacity-35">
              <Plus className="h-4 w-4" />
              {t("settings.add_process_template")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function ProcessTemplateEditor({
  stages,
  assistants,
  loading,
  processTemplateId,
  onStageCreated,
  onStageUpdated,
  onStagesReload,
  onStageDeleted,
  onError,
}: {
  stages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  loading: boolean;
  processTemplateId: string;
  onStageCreated: (stage: ProjectStageInfo) => void;
  onStageUpdated: (stage: ProjectStageInfo) => void;
  onStagesReload: () => Promise<void>;
  onStageDeleted: (stageId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [showCreateStage, setShowCreateStage] = useState(false);

  return (
    <>
      <SettingsGroup
        title={t("stage.project_stages")}
        action={
          <button type="button" onClick={() => setShowCreateStage(true)} className="inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 text-body-sm font-medium leading-none text-card-fg/75 transition hover:text-card-fg/90">
            <Plus className="h-3.5 w-3.5" />
            {t("stage.add")}
          </button>
        }
      >
        <StageList
          stages={stages}
          assistants={assistants}
          loading={loading}
          dragGroup="process-stages"
          onUpdated={onStageUpdated}
          onDeleted={onStageDeleted}
          onReload={onStagesReload}
          onError={onError}
        />
      </SettingsGroup>
      {showCreateStage && (
        <CreateStageDialog
          processTemplateId={processTemplateId}
          onCreated={onStageCreated}
          onClose={() => setShowCreateStage(false)}
          onError={onError}
        />
      )}
    </>
  );
}

function SettingsGroup({ title, children, flush = false, action = null }: { title: string; children: ReactNode; flush?: boolean; action?: ReactNode }) {
  return (
    <div className="mb-8 last:mb-0">
      <div className="mb-3 flex items-center justify-between gap-3">
        <h2 className="text-body-sm font-semibold text-ink/[0.88]">{title}</h2>
        {action}
      </div>
      <div className={"overflow-hidden rounded-lg border border-card-border/[0.12] bg-card " + (flush ? "" : "p-3")}>{children}</div>
    </div>
  );
}

function SettingsRow({
  icon,
  label,
  description,
  disabled = false,
  children,
}: {
  icon: ReactNode;
  label: string;
  description: string;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={"grid min-h-[72px] grid-cols-[minmax(0,1fr)_auto] items-center gap-4 border-b border-ink/[0.12] px-3 py-3 last:border-b-0" + (disabled ? " opacity-55" : "")}>
      <div className="flex min-w-0 gap-3">
        <span className="mt-0.5 text-ink/55">{icon}</span>
        <span className="min-w-0">
          <span className="block text-body-sm font-medium text-ink/75">{label}</span>
          <span className="mt-1 block text-caption leading-relaxed text-ink/60">{description}</span>
        </span>
      </div>
      {children}
    </div>
  );
}

function SettingsStackedRow({
  icon,
  label,
  description,
  children,
}: {
  icon: ReactNode;
  label: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <div className="border-b border-ink/[0.12] px-3 py-4 last:border-b-0">
      <div className="mb-3 flex min-w-0 gap-3">
        <span className="mt-0.5 shrink-0 text-ink/55">{icon}</span>
        <span className="min-w-0">
          <span className="block text-body-sm font-medium text-ink/75">{label}</span>
          <span className="mt-1 block max-w-[56rem] text-caption leading-relaxed text-ink/60">{description}</span>
        </span>
      </div>
      <div className="min-w-0 pl-7">{children}</div>
    </div>
  );
}

function SegmentedControl({
  value,
  options,
  onChange,
}: {
  value: string;
  options: Array<{ value: string; label: string }>;
  onChange: (value: string) => void;
}) {
  return (
    <div className="inline-flex rounded-md bg-ink/[0.045] p-0.5">
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          onClick={() => onChange(option.value)}
          className={"h-7 min-w-9 rounded px-2 text-body-sm transition " + (value === option.value ? "bg-ink/[0.06] text-ink/85" : "text-ink/45 hover:text-ink/75")}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function ThemeSelector({ mode, onChange }: { mode: ThemeMode; onChange: (mode: ThemeMode) => void }) {
  const { t } = useI18n();
  const items = [
    { value: "light" as const, icon: Sun, label: t("theme.light") },
    { value: "dark" as const, icon: Moon, label: t("theme.dark") },
    { value: "system" as const, icon: Monitor, label: t("theme.system") },
  ];
  return (
    <div className="inline-flex rounded-md bg-ink/[0.045] p-0.5">
      {items.map(({ value, icon: Icon, label }) => (
        <Tooltip key={value} content={label} placement="top">
          <button
            type="button"
            onClick={() => onChange(value)}
            className={"flex h-7 w-8 items-center justify-center rounded transition " + (mode === value ? "bg-ink/[0.06] text-ink/85" : "text-ink/45 hover:text-ink/75")}
          >
            <Icon className="h-4 w-4" />
          </button>
        </Tooltip>
      ))}
    </div>
  );
}

function McpBuiltinCard({ server }: { server: McpServerConfig }) {
  const { t } = useI18n();
  return (
    <div className="rounded-lg border border-card-border/[0.12] bg-card-chip/[0.04] px-3 py-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-body-sm font-medium text-ink">{server.name}</div>
          <div className="mt-1 text-caption leading-relaxed text-ink/50">
            {server.builtinKind === SESSIO_PROMPT_MARKERS.builtinMcpKindComputerUse
              ? t("settings.mcp_builtin_computer_use_note")
              : t("settings.mcp_builtin_description")}
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <McpBadge>{t("settings.mcp_builtin_badge")}</McpBadge>
          <McpBadge>{mcpTransportLabel(t, server.transport)}</McpBadge>
          <McpBadge>{t("settings.mcp_scope_session")}</McpBadge>
          <McpStatusBadge enabled={server.enabled} />
        </div>
      </div>
    </div>
  );
}

function McpCustomServerCard({
  server,
  onUpdate,
  onRemove,
}: {
  server: McpServerConfig;
  onUpdate: (server: McpServerConfig) => void;
  onRemove: () => void;
}) {
  const { t } = useI18n();
  const isStdio = server.transport === "stdio";

  return (
    <div className="rounded-lg border border-card-border/[0.12] bg-card-chip/[0.04] p-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-body-sm font-medium text-ink">{server.name || t("settings.mcp_new_server")}</div>
          <div className="mt-1 flex flex-wrap items-center gap-2 text-caption text-ink/45">
            <McpBadge>{t("settings.mcp_custom_badge")}</McpBadge>
            <McpBadge>{mcpTransportLabel(t, server.transport)}</McpBadge>
            <McpBadge>{t("settings.mcp_scope_global")}</McpBadge>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <span className="text-caption text-ink/45">
            {server.enabled ? t("settings.mcp_enabled") : t("settings.mcp_disabled")}
          </span>
          <SwitchControl
            checked={server.enabled}
            tooltip={t("settings.mcp_enabled")}
            onToggle={() => onUpdate({ ...server, enabled: !server.enabled })}
          />
          <button
            type="button"
            onClick={onRemove}
            className="inline-flex h-9 w-9 items-center justify-center rounded-md border border-status-error/20 bg-status-error/[0.08] text-status-error transition hover:bg-status-error/[0.12]"
            aria-label={t("sidebar.remove")}
          >
            <Trash2 className="h-4 w-4" />
          </button>
        </div>
      </div>
      <div className="mt-3 grid gap-3 md:grid-cols-2">
        <label className="grid gap-1.5">
          <span className="text-caption text-ink/45">{t("settings.mcp_name")}</span>
          <input
            value={server.name}
            onChange={(event) => onUpdate({ ...server, name: event.target.value })}
            placeholder={t("settings.mcp_name_placeholder")}
            className={inputClassName}
          />
        </label>
        <div className="grid gap-1.5">
          <span className="text-caption text-ink/45">{t("settings.mcp_transport")}</span>
          <SegmentedControl
            value={server.transport}
            options={[
              { value: "http", label: "HTTP" },
              { value: "sse", label: "SSE" },
              { value: "stdio", label: "stdio" },
            ]}
            onChange={(value) => onUpdate(applyCustomMcpTransport(server, value as McpServerConfig["transport"]))}
          />
        </div>
        {isStdio ? (
          <label className="grid gap-1.5 md:col-span-2">
            <span className="text-caption text-ink/45">{t("settings.mcp_command")}</span>
            <input
              value={server.command ?? ""}
              onChange={(event) => onUpdate({ ...server, command: event.target.value })}
              placeholder="npx @modelcontextprotocol/server-filesystem"
              className={inputClassName}
            />
          </label>
        ) : (
          <label className="grid gap-1.5 md:col-span-2">
            <span className="text-caption text-ink/45">{t("settings.mcp_url")}</span>
            <input
              value={server.url ?? ""}
              onChange={(event) => onUpdate({ ...server, url: event.target.value })}
              placeholder="http://127.0.0.1:3000/mcp"
              className={inputClassName}
            />
          </label>
        )}
        {isStdio ? (
          <>
            <label className="grid gap-1.5">
              <span className="text-caption text-ink/45">{t("settings.mcp_args")}</span>
              <textarea
                value={server.args.join("\n")}
                onChange={(event) => onUpdate({ ...server, args: parseLineList(event.target.value) })}
                rows={4}
                placeholder={t("settings.mcp_args_placeholder")}
                className={textareaClassName}
              />
            </label>
            <label className="grid gap-1.5">
              <span className="text-caption text-ink/45">{t("settings.mcp_env")}</span>
              <textarea
                value={keyValuesToText(server.env)}
                onChange={(event) => onUpdate({ ...server, env: parseKeyValueText(event.target.value) })}
                rows={4}
                placeholder={t("settings.mcp_env_placeholder")}
                className={textareaClassName}
              />
            </label>
          </>
        ) : (
          <label className="grid gap-1.5 md:col-span-2">
            <span className="text-caption text-ink/45">{t("settings.mcp_headers")}</span>
            <textarea
              value={keyValuesToText(server.headers)}
              onChange={(event) => onUpdate({ ...server, headers: parseKeyValueText(event.target.value) })}
              rows={4}
              placeholder={t("settings.mcp_headers_placeholder")}
              className={textareaClassName}
            />
          </label>
        )}
      </div>
    </div>
  );
}

function McpBadge({ children }: { children: ReactNode }) {
  return (
    <span className="rounded-full bg-card-chip/[0.08] px-2 py-1 text-caption text-ink/55">
      {children}
    </span>
  );
}

function McpStatusBadge({ enabled }: { enabled: boolean }) {
  const { t } = useI18n();
  return (
    <span
      className={
        "rounded-full px-2 py-1 text-caption " +
        (enabled
          ? "bg-emerald/10 text-emerald"
          : "bg-amber/10 text-amber")
      }
    >
      {enabled ? t("settings.mcp_enabled") : t("settings.mcp_disabled")}
    </span>
  );
}

function EmptyState({ label }: { label: string }) {
  return <div className="rounded-md border border-dashed border-ink/10 py-8 text-center text-body-sm text-ink/35">{label}</div>;
}

function createDefaultCustomMcpServer(): McpServerConfig {
  return {
    id: `custom-${globalThis.crypto?.randomUUID?.() ?? Date.now()}`,
    name: "",
    enabled: true,
    source: SESSIO_PROMPT_MARKERS.mcpSourceCustom,
    transport: "http",
    injectionMode: "always",
    builtinKind: null,
    url: "",
    headers: [],
    command: null,
    args: [],
    env: [],
  };
}

function extractCustomMcpServers(settings: McpSettings | null): McpServerConfig[] {
  return (settings?.servers ?? []).filter(
    (server) => server.source === SESSIO_PROMPT_MARKERS.mcpSourceCustom,
  );
}

function replaceCustomMcpServers(
  servers: McpServerConfig[],
  nextCustomServers: McpServerConfig[],
): McpServerConfig[] {
  return [
    ...servers.filter(
      (server) => server.source !== SESSIO_PROMPT_MARKERS.mcpSourceCustom,
    ),
    ...nextCustomServers,
  ];
}

function stableMcpServerJson(servers: McpServerConfig[]): string {
  return JSON.stringify(servers);
}

function withBuiltinComputerUseEnabled(
  settings: McpSettings | null,
  enabled: boolean,
): McpSettings | null {
  if (!settings) return settings;
  return {
    ...settings,
    servers: settings.servers.map((server) =>
      server.source === SESSIO_PROMPT_MARKERS.mcpSourceBuiltin
      && server.builtinKind === SESSIO_PROMPT_MARKERS.builtinMcpKindComputerUse
        ? { ...server, enabled }
        : server),
  };
}

function applyCustomMcpTransport(
  server: McpServerConfig,
  transport: McpServerConfig["transport"],
): McpServerConfig {
  if (transport === "stdio") {
    return {
      ...server,
      transport,
      url: null,
      headers: [],
      command: server.command ?? "",
    };
  }
  return {
    ...server,
    transport,
    url: server.url ?? "",
    command: null,
    args: [],
    env: [],
  };
}

function keyValuesToText(entries: McpKeyValue[]): string {
  return entries.map((entry) => `${entry.name}=${entry.value}`).join("\n");
}

function parseKeyValueText(value: string): McpKeyValue[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const [name, ...rest] = line.split("=");
      return {
        name: name?.trim() ?? "",
        value: rest.join("=").trim(),
      };
    })
    .filter((entry) => entry.name.length > 0);
}

function parseLineList(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

function mcpTransportLabel(
  t: (key: string, values?: Record<string, string | number>) => string,
  transport: McpServerConfig["transport"],
): string {
  return transport === "http"
    ? t("settings.mcp_transport_http")
    : transport === "sse"
      ? t("settings.mcp_transport_sse")
      : t("settings.mcp_transport_stdio");
}

const inputClassName = "h-9 min-w-0 rounded-md border border-input-border/[0.16] bg-input px-3 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";
const textareaClassName = "min-w-0 resize-none rounded-md border border-input-border/[0.16] bg-input px-3 py-2 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";
