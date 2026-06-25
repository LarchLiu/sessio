import { Claude, OpenAI, Gemini } from "@lobehub/icons";
import { Hash } from "lucide-react";
import { Menu } from "@tauri-apps/api/menu/menu";
import { MenuItem } from "@tauri-apps/api/menu/menuItem";
import { PredefinedMenuItem } from "@tauri-apps/api/menu/predefinedMenuItem";
import { IconMenuItem } from "@tauri-apps/api/menu/iconMenuItem";
import { TrayIcon } from "@tauri-apps/api/tray";
import { Image } from "@tauri-apps/api/image";
import {
  Agent,
  SessionInfo,
  ThreadIndexItemInfo,
} from "./api";
import { getMenuIconBytes, type MenuIconComponent } from "./menuIcon";
import { sessionDisplayTitle } from "./appUtils";

const AGENT_ICONS: Record<Agent, MenuIconComponent> = {
  "astra-pi": OpenAI as MenuIconComponent,
  pi: OpenAI as MenuIconComponent,
  codex: OpenAI as MenuIconComponent,
  claude: Claude.Color as MenuIconComponent,
  gemini: Gemini.Color as MenuIconComponent,
  opencode: OpenAI as MenuIconComponent,
};

// NSMenu sizes its width to the widest item, so CJK titles end up much
// wider than ASCII for the same char count. Cap by measured pixel width
// instead so the menu column stays consistent across languages.
const TITLE_MAX_WIDTH_PX = 240;
const TITLE_MEASURE_FONT =
  '13px -apple-system, "Segoe UI", system-ui, sans-serif';

const TRAY_ID = "main";

type TrayTheme = "light" | "dark";

export type TrayRecentEntry =
  | { kind: "session"; session: SessionInfo; time: number }
  | { kind: "thread"; thread: ThreadIndexItemInfo; time: number };

function themedAgentIconColor(agent: Agent, theme: TrayTheme): string | undefined {
  if (agent !== "codex" && agent !== "astra-pi" && agent !== "pi") return undefined;
  return theme === "dark" ? "#ffffff" : "#1c1c20";
}

async function getAgentIcon(agent: Agent, theme: TrayTheme): Promise<Uint8Array> {
  return getMenuIconBytes(AGENT_ICONS[agent], {
    color: themedAgentIconColor(agent, theme),
  });
}

async function getThreadIcon(theme: TrayTheme): Promise<Uint8Array> {
  return getMenuIconBytes(Hash as MenuIconComponent, {
    color: theme === "dark" ? "#ffffff" : "#1c1c20",
  });
}

let measureCtx: CanvasRenderingContext2D | null = null;
function getMeasureCtx(): CanvasRenderingContext2D {
  if (measureCtx) return measureCtx;
  const ctx = document.createElement("canvas").getContext("2d");
  if (!ctx) throw new Error("canvas 2d context unavailable");
  ctx.font = TITLE_MEASURE_FONT;
  measureCtx = ctx;
  return ctx;
}

function fitTitle(raw: string): string {
  const s = raw.replace(/\s+/g, " ").trim();
  if (!s) return s;
  const ctx = getMeasureCtx();
  if (ctx.measureText(s).width <= TITLE_MAX_WIDTH_PX) return s;
  const ellipsis = "…";
  const ellipsisW = ctx.measureText(ellipsis).width;
  let lo = 0;
  let hi = s.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    const w = ctx.measureText(s.slice(0, mid)).width + ellipsisW;
    if (w <= TITLE_MAX_WIDTH_PX) lo = mid;
    else hi = mid - 1;
  }
  return s.slice(0, lo).trimEnd() + ellipsis;
}

export interface TrayTexts {
  show: string;
  quit: string;
  noSessions: string;
  noMessage: string;
  updateAvailable: string;
  updateInstalling: string;
}

export interface TrayUpdateState {
  hasUpdate: boolean;
  latestVersion: string | null;
  installing: boolean;
  install: () => void | Promise<void>;
}

type ItemHandle =
  | MenuItem
  | PredefinedMenuItem
  | IconMenuItem
  ;

export interface TrayRecentActions {
  onSelectSession: (session: SessionInfo) => void;
  onSelectThread: (thread: ThreadIndexItemInfo) => void;
}

let currentMenu: Menu | null = null;
let currentToken = 0;
let pendingPromise: Promise<void> = Promise.resolve();

async function buildSessionItem(
  s: SessionInfo,
  texts: TrayTexts,
  iconBytes: Record<Agent, Uint8Array>,
  actions: TrayRecentActions,
): Promise<IconMenuItem> {
  const titleSource = sessionDisplayTitle(s) ?? texts.noMessage;
  const text = fitTitle(titleSource) || texts.noMessage;
  const icon = await Image.fromBytes(iconBytes[s.agent]);
  return IconMenuItem.new({
    id: `tray-session-${s.agent}-${s.id}`,
    text,
    icon,
    action: () => {
      actions.onSelectSession(s);
    },
  });
}

async function buildThreadItem(
  entry: Extract<TrayRecentEntry, { kind: "thread" }>,
  texts: TrayTexts,
  threadIconBytes: Uint8Array,
  actions: TrayRecentActions,
): Promise<IconMenuItem> {
  const icon = await Image.fromBytes(threadIconBytes);
  return IconMenuItem.new({
    id: `tray-thread-${entry.thread.threadId}`,
    text: fitTitle(entry.thread.goal) || texts.noMessage,
    icon,
    action: () => {
      actions.onSelectThread(entry.thread);
    },
  });
}

async function buildMenu(
  recent: TrayRecentEntry[],
  texts: TrayTexts,
  theme: TrayTheme,
  update: TrayUpdateState,
  actions: TrayRecentActions,
): Promise<Menu> {
  const iconBytes: Record<Agent, Uint8Array> = {
    "astra-pi": await getAgentIcon("astra-pi", theme),
    pi: await getAgentIcon("pi", theme),
    codex: await getAgentIcon("codex", theme),
    claude: await getAgentIcon("claude", theme),
    gemini: await getAgentIcon("gemini", theme),
    opencode: await getAgentIcon("opencode", theme),
  };
  const threadIconBytes = await getThreadIcon(theme);

  const items: ItemHandle[] = [];

  if (recent.length === 0) {
    const empty = await MenuItem.new({
      id: "tray-recent-empty",
      text: texts.noSessions,
      enabled: false,
    });
    items.push(empty);
  } else {
    for (const entry of recent) {
      items.push(
        entry.kind === "thread"
          ? await buildThreadItem(entry, texts, threadIconBytes, actions)
          : await buildSessionItem(entry.session, texts, iconBytes, actions),
      );
    }
  }

  if (update.hasUpdate && update.latestVersion) {
    items.push(await PredefinedMenuItem.new({ item: "Separator" }));
    items.push(await MenuItem.new({
      id: "tray-update",
      text: update.installing
        ? texts.updateInstalling
        : texts.updateAvailable.replace("{version}", update.latestVersion),
      enabled: !update.installing,
      action: () => {
        void update.install();
      },
    }));
  }

  items.push(await PredefinedMenuItem.new({ item: "Separator" }));
  items.push(await MenuItem.new({ id: "show", text: texts.show }));
  items.push(await PredefinedMenuItem.new({ item: "Separator" }));
  items.push(await MenuItem.new({ id: "quit", text: texts.quit }));

  return Menu.new({ items });
}

export async function syncTrayMenu(
  recent: TrayRecentEntry[],
  texts: TrayTexts,
  theme: TrayTheme,
  update: TrayUpdateState,
  actions: TrayRecentActions,
): Promise<void> {
  const token = ++currentToken;
  // Serialize against any in-flight sync so we never race two setMenu calls.
  pendingPromise = pendingPromise.then(async () => {
    if (token !== currentToken) return;
    try {
      const tray = await TrayIcon.getById(TRAY_ID);
      if (!tray) return;
      const menu = await buildMenu(recent, texts, theme, update, actions);
      if (token !== currentToken) {
        await menu.close();
        return;
      }
      await tray.setMenu(menu);
      const previous = currentMenu;
      currentMenu = menu;
      if (previous) {
        await previous.close().catch(() => {});
      }
    } catch (err) {
      console.error("tray menu sync failed", err);
    }
  });
  return pendingPromise;
}
