import { createElement, type ComponentType } from "react";
import { createRoot, type Root } from "react-dom/client";
import { flushSync } from "react-dom";

export type MenuIconComponent = ComponentType<{
  size?: number | string;
  className?: string;
  color?: string;
  strokeWidth?: number | string;
  absoluteStrokeWidth?: boolean;
}>;

export interface MenuIconRenderOptions {
  svgPx?: number;
  canvasPx?: number;
  innerPx?: number;
}

const DEFAULT_OPTIONS: Required<MenuIconRenderOptions> = {
  svgPx: 64,
  canvasPx: 32,
  innerPx: 22,
};

const iconCache = new WeakMap<
  MenuIconComponent,
  Map<string, Promise<Uint8Array>>
>();

function optionsKey(options: Required<MenuIconRenderOptions>): string {
  return `${options.svgPx}:${options.canvasPx}:${options.innerPx}`;
}

async function renderIconBytes(
  Icon: MenuIconComponent,
  options: Required<MenuIconRenderOptions>,
): Promise<Uint8Array> {
  const container = document.createElement("div");
  container.style.cssText =
    "position:absolute;left:-9999px;top:0;width:0;height:0;overflow:hidden;pointer-events:none";
  document.body.appendChild(container);
  let root: Root | null = null;
  try {
    root = createRoot(container);
    flushSync(() => {
      root!.render(createElement(Icon, { size: options.svgPx }));
    });
    const svg = container.querySelector("svg");
    if (!svg) throw new Error("menu icon svg missing");
    svg.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    if (!svg.getAttribute("width")) svg.setAttribute("width", String(options.svgPx));
    if (!svg.getAttribute("height")) svg.setAttribute("height", String(options.svgPx));
    const svgString = new XMLSerializer().serializeToString(svg);

    const utf8 = new TextEncoder().encode(svgString);
    let bin = "";
    for (let i = 0; i < utf8.length; i++) bin += String.fromCharCode(utf8[i]);
    const dataUrl = "data:image/svg+xml;base64," + btoa(bin);

    const img = new globalThis.Image();
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error("menu icon load failed"));
      img.src = dataUrl;
    });

    const canvas = document.createElement("canvas");
    canvas.width = options.canvasPx;
    canvas.height = options.canvasPx;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas 2d context unavailable");
    ctx.clearRect(0, 0, options.canvasPx, options.canvasPx);
    const pad = (options.canvasPx - options.innerPx) / 2;
    ctx.drawImage(img, pad, pad, options.innerPx, options.innerPx);

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

export async function getMenuIconBytes(
  Icon: MenuIconComponent,
  options: MenuIconRenderOptions = {},
): Promise<Uint8Array> {
  const normalized: Required<MenuIconRenderOptions> = {
    svgPx: options.svgPx ?? DEFAULT_OPTIONS.svgPx,
    canvasPx: options.canvasPx ?? DEFAULT_OPTIONS.canvasPx,
    innerPx: options.innerPx ?? DEFAULT_OPTIONS.innerPx,
  };
  let byOptions = iconCache.get(Icon);
  if (!byOptions) {
    byOptions = new Map();
    iconCache.set(Icon, byOptions);
  }
  const key = optionsKey(normalized);
  let promise = byOptions.get(key);
  if (!promise) {
    promise = renderIconBytes(Icon, normalized).catch((err) => {
      byOptions!.delete(key);
      throw err;
    });
    byOptions.set(key, promise);
  }
  return promise;
}
