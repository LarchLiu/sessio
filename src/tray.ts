import { Claude, OpenAI, Gemini } from "@lobehub/icons";
import { Menu } from "@tauri-apps/api/menu/menu";
import { MenuItem } from "@tauri-apps/api/menu/menuItem";
import { PredefinedMenuItem } from "@tauri-apps/api/menu/predefinedMenuItem";
import { IconMenuItem } from "@tauri-apps/api/menu/iconMenuItem";
import { Submenu } from "@tauri-apps/api/menu/submenu";
import { TrayIcon } from "@tauri-apps/api/tray";
import { Image } from "@tauri-apps/api/image";
import {
  Agent,
  SessionInfo,
} from "./api";
import { getMenuIconBytes, type MenuIconComponent } from "./menuIcon";
import {
  RESUME_CMD,
  buildCrossCommandForSession,
} from "./cross";

const AGENT_ICONS: Record<Agent, MenuIconComponent> = {
  codex: OpenAI as MenuIconComponent,
  claude: Claude.Color as MenuIconComponent,
  gemini: Gemini.Color as MenuIconComponent,
};

// NSMenu sizes its width to the widest item, so CJK titles end up much
// wider than ASCII for the same char count. Cap by measured pixel width
// instead so the menu column stays consistent across languages.
const TITLE_MAX_WIDTH_PX = 240;
const TITLE_MEASURE_FONT =
  '13px -apple-system, "Segoe UI", system-ui, sans-serif';

const TRAY_ID = "main";

type TrayTheme = "light" | "dark";

function themedAgentIconColor(agent: Agent, theme: TrayTheme): string | undefined {
  if (agent !== "codex") return undefined;
  return theme === "dark" ? "#ffffff" : "#1c1c20";
}

async function getAgentIcon(agent: Agent, theme: TrayTheme): Promise<Uint8Array> {
  return getMenuIconBytes(AGENT_ICONS[agent], {
    color: themedAgentIconColor(agent, theme),
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
  resumeCommand: string;
  crossCommand: string;
  crossPromptPlaceholder: string;
}

type ItemHandle =
  | MenuItem
  | PredefinedMenuItem
  | IconMenuItem
  | Submenu;

let currentMenu: Menu | null = null;
let currentToken = 0;
let pendingPromise: Promise<void> = Promise.resolve();

async function buildSessionSubmenu(
  s: SessionInfo,
  texts: TrayTexts,
  iconBytes: Record<Agent, Uint8Array>,
): Promise<Submenu> {
  const orderedAgents: Agent[] = ["codex", "claude", "gemini"];
  const subItems: IconMenuItem[] = [];
  for (const a of orderedAgents) {
    const label = a === s.agent ? texts.resumeCommand : texts.crossCommand;
    const icon = await Image.fromBytes(iconBytes[a]);
    const item = await IconMenuItem.new({
      id: `tray-session-${s.agent}-${s.id}-${a}`,
      text: label,
      icon,
      action: async () => {
        try {
          if (a === s.agent) {
            await navigator.clipboard.writeText(RESUME_CMD[a](s.id));
            return;
          }
          const cmd = await buildCrossCommandForSession(
            s.agent,
            a,
            s.id,
            s.filePath,
            texts.crossPromptPlaceholder,
          );
          if (cmd) await navigator.clipboard.writeText(cmd);
        } catch (err) {
          console.error("tray submenu action failed", err);
        }
      },
    });
    subItems.push(item);
  }
  const titleSource = s.title ?? s.firstUserMessage ?? texts.noMessage;
  const text = fitTitle(titleSource) || texts.noMessage;
  const icon = await Image.fromBytes(iconBytes[s.agent]);
  return Submenu.new({
    id: `tray-session-${s.agent}-${s.id}`,
    text,
    icon,
    items: subItems,
  });
}

async function buildMenu(
  recent: SessionInfo[],
  texts: TrayTexts,
  theme: TrayTheme,
): Promise<Menu> {
  const iconBytes: Record<Agent, Uint8Array> = {
    codex: await getAgentIcon("codex", theme),
    claude: await getAgentIcon("claude", theme),
    gemini: await getAgentIcon("gemini", theme),
  };

  const items: ItemHandle[] = [];

  if (recent.length === 0) {
    const empty = await MenuItem.new({
      id: "tray-recent-empty",
      text: texts.noSessions,
      enabled: false,
    });
    items.push(empty);
  } else {
    for (const s of recent) {
      items.push(await buildSessionSubmenu(s, texts, iconBytes));
    }
  }

  items.push(await PredefinedMenuItem.new({ item: "Separator" }));
  items.push(await MenuItem.new({ id: "show", text: texts.show }));
  items.push(await PredefinedMenuItem.new({ item: "Separator" }));
  items.push(await MenuItem.new({ id: "quit", text: texts.quit }));

  return Menu.new({ items });
}

export async function syncTrayMenu(
  recent: SessionInfo[],
  texts: TrayTexts,
  theme: TrayTheme,
): Promise<void> {
  const token = ++currentToken;
  // Serialize against any in-flight sync so we never race two setMenu calls.
  pendingPromise = pendingPromise.then(async () => {
    if (token !== currentToken) return;
    try {
      const tray = await TrayIcon.getById(TRAY_ID);
      if (!tray) return;
      const menu = await buildMenu(recent, texts, theme);
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
