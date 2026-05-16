import { createElement, type ComponentType } from "react";
import { createRoot, type Root } from "react-dom/client";
import { flushSync } from "react-dom";
import { Claude, Codex, Gemini } from "@lobehub/icons";
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
import {
  RESUME_CMD,
  buildCrossCommandForSession,
} from "./cross";

type AgentIconComponent = ComponentType<{
  size?: number | string;
  className?: string;
}>;

const AGENT_ICONS: Record<Agent, AgentIconComponent> = {
  codex: Codex.Color as AgentIconComponent,
  claude: Claude.Color as AgentIconComponent,
  gemini: Gemini.Color as AgentIconComponent,
};

// muda hard-pins macOS menu image height to 18pt regardless of source
// (muda/src/platform_impl/macos/mod.rs:1163). To get a visually smaller
// glyph we render into a padded canvas — the agent SVG occupies only the
// centre patch, so the muda downscale leaves whitespace around it.
const SVG_RENDER_PX = 64;
const ICON_CANVAS_PX = 32;
const ICON_INNER_PX = 22;

// NSMenu sizes its width to the widest item, so CJK titles end up much
// wider than ASCII for the same char count. Cap by measured pixel width
// instead so the menu column stays consistent across languages.
const TITLE_MAX_WIDTH_PX = 240;
const TITLE_MEASURE_FONT =
  '13px -apple-system, "Segoe UI", system-ui, sans-serif';

const TRAY_ID = "main";

const iconCache = new Map<Agent, Promise<Uint8Array>>();

async function renderAgentIcon(agent: Agent): Promise<Uint8Array> {
  const container = document.createElement("div");
  container.style.cssText =
    "position:absolute;left:-9999px;top:0;width:0;height:0;overflow:hidden;pointer-events:none";
  document.body.appendChild(container);
  let root: Root | null = null;
  try {
    root = createRoot(container);
    const Icon = AGENT_ICONS[agent];
    flushSync(() => {
      root!.render(createElement(Icon, { size: SVG_RENDER_PX }));
    });
    const svg = container.querySelector("svg");
    if (!svg) throw new Error(`agent svg missing for ${agent}`);
    svg.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    if (!svg.getAttribute("width")) svg.setAttribute("width", String(SVG_RENDER_PX));
    if (!svg.getAttribute("height"))
      svg.setAttribute("height", String(SVG_RENDER_PX));
    const svgString = new XMLSerializer().serializeToString(svg);

    const utf8 = new TextEncoder().encode(svgString);
    let bin = "";
    for (let i = 0; i < utf8.length; i++) bin += String.fromCharCode(utf8[i]);
    const dataUrl = "data:image/svg+xml;base64," + btoa(bin);

    const img = new globalThis.Image();
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error(`agent icon load failed: ${agent}`));
      img.src = dataUrl;
    });

    const canvas = document.createElement("canvas");
    canvas.width = ICON_CANVAS_PX;
    canvas.height = ICON_CANVAS_PX;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas 2d context unavailable");
    ctx.clearRect(0, 0, ICON_CANVAS_PX, ICON_CANVAS_PX);
    const pad = (ICON_CANVAS_PX - ICON_INNER_PX) / 2;
    ctx.drawImage(img, pad, pad, ICON_INNER_PX, ICON_INNER_PX);

    const blob: Blob = await new Promise((resolve, reject) =>
      canvas.toBlob(
        (b) => (b ? resolve(b) : reject(new Error("png encode failed"))),
        "image/png",
      ),
    );
    return new Uint8Array(await blob.arrayBuffer());
  } finally {
    if (root) root.unmount();
    container.remove();
  }
}

async function getAgentIcon(agent: Agent): Promise<Uint8Array> {
  let p = iconCache.get(agent);
  if (!p) {
    p = renderAgentIcon(agent).catch((err) => {
      iconCache.delete(agent);
      throw err;
    });
    iconCache.set(agent, p);
  }
  return p;
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
  const titleSource = s.firstUserMessage ?? texts.noMessage;
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
): Promise<Menu> {
  const iconBytes: Record<Agent, Uint8Array> = {
    codex: await getAgentIcon("codex"),
    claude: await getAgentIcon("claude"),
    gemini: await getAgentIcon("gemini"),
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
): Promise<void> {
  const token = ++currentToken;
  // Serialize against any in-flight sync so we never race two setMenu calls.
  pendingPromise = pendingPromise.then(async () => {
    if (token !== currentToken) return;
    try {
      const tray = await TrayIcon.getById(TRAY_ID);
      if (!tray) return;
      const menu = await buildMenu(recent, texts);
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
