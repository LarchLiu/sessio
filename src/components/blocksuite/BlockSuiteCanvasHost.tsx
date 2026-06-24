import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { addImages } from "@blocksuite/affine/blocks/image";
import { EdgelessRootService } from "@blocksuite/affine/blocks/root";
import { NoteDisplayMode, type GroupElementModel, DEFAULT_NOTE_HEIGHT, DEFAULT_NOTE_WIDTH } from "@blocksuite/affine/model";
import { createGroupFromSelectedCommand, ungroupCommand } from "@blocksuite/affine/gfx/group";
import { Bound, serializeXYWH } from "@blocksuite/global/gfx";
import { BLOCKSUITE_STYLE_SCOPE_CLASS } from "@blocksuite/std";
import { GfxControllerIdentifier } from "@blocksuite/std/gfx";
import { createPortal } from "react-dom";
import { Check, FileImage, FilePlus2, FolderOpen, Layers3, LoaderCircle, Save, StickyNote, Workflow, X } from "lucide-react";
import type { ComposerAttachment } from "../ComposerAttachments";
import PopupMenu, { type PopupMenuOption } from "../PopupMenu";
import ScrollArea from "../ScrollArea";
import type {
  CanvasDocumentState,
} from "../../canvasTypes";
import {
  captureWindowAreaPng,
  createAstraRun,
  getThreadWorkSnapshot,
  readLocalImageDataUrl,
  readWorkspaceTextFile,
  saveCanvasDraft,
  saveCanvasRevision,
  savePastedAttachment,
  updateCanvasBlocks,
  type Agent,
} from "../../api";
import type { ChatComposerController } from "../../hooks/useChatComposer";
import {
  createBlockSuiteDoc,
  createEdgelessEditorWithSpecs,
  ensureEdgelessRoot,
  exportDocSnapshot,
  importDocSnapshot,
} from "./bootstrap";
import { PortalHost } from "./portalHost";
import ToastStack, { type ToastStackMessage } from "../ToastStack";
import { useReactToLitBridge } from "../../lib/blocksuite/reactToLit";
import { setBlockSuitePortalBridge } from "../../lib/blocksuite/portalBridge";
import {
  canvasInteropModelToCanvasBlock,
  surfaceElementToCanvasBlock,
  workflowSnapshotToMarkdown,
} from "../../lib/blocksuite/persistence";
import type { MarkdownPreviewBlockModel } from "../../lib/blocksuite/blocks/markdown-preview";
import {
  DEFAULT_FILE_CARD_COLLAPSED_HEIGHT,
  DEFAULT_FILE_CARD_HEIGHT,
  DEFAULT_FILE_CARD_WIDTH,
  type FileCardBlockModel,
} from "../../lib/blocksuite/blocks/file-card";
import {
  DEFAULT_WORKFLOW_CARD_HEIGHT,
  DEFAULT_WORKFLOW_CARD_WIDTH,
  type WorkflowCardBlockModel,
} from "../../lib/blocksuite/blocks/workflow-card";
import {
  CANVAS_SNAPSHOT_SELECTION_EVENT,
  type CanvasSnapshotSelectionEventDetail,
} from "../../lib/blocksuite/toolbar";

const CANVAS_ADD_FILES_EVENT = "sessio:canvas-add-files";
const CANVAS_GROUP_SELECTION_EVENT = "sessio:canvas-group-selection";
const CANVAS_UNGROUP_SELECTION_EVENT = "sessio:canvas-ungroup-selection";
const AUTOSAVE_DEBOUNCE_MS = 900;
const ROOT_SERVICE_RETRY_MS = 80;
const ROOT_SERVICE_RETRY_LIMIT = 125;
const BOX_SELECTING_CLASS_NAME = "sessio-box-selecting";
const SNAPSHOT_CAPTURING_CLASS_NAME = "sessio-snapshot-capturing";
const CANVAS_NODE_PADDING = 32;
const CANVAS_PLACEMENT_SEARCH_RADIUS = 64;

type BlockSuiteEditor = HTMLElement & {
  std?: {
    get?: <T>(token: unknown) => T;
    getOptional?: <T>(token: unknown) => T | null;
    command?: {
      exec: <TArgs extends object, TResult extends object>(
        command: unknown,
        args?: TArgs,
      ) => [boolean, TResult];
    };
    getService?: <T>(flavour: string) => T | null;
    host?: HTMLElement & {
      view?: {
        getBlock?: (blockId: string) => HTMLElement | null;
      };
    };
  };
};

type SelectionElementLike = {
  id: string;
  flavour?: string;
  type?: string;
  xywh?: string;
  title?: string;
  childIds?: string[];
  group?: unknown;
};

type EdgelessSelectable = {
  id: string;
  xywh?: string;
  elementBound?: Bound;
  responseBound?: Bound;
  flavour?: string;
  props?: {
    xywh?: string;
  };
};

type GfxControllerLike = {
  viewport: {
    toViewCoord: (x: number, y: number) => [number, number];
    toViewBound: (bound: Bound) => Bound;
    toModelCoord: (x: number, y: number) => [number, number];
    viewportUpdated?: {
      subscribe: (listener: () => void) => { unsubscribe: () => void };
    };
  };
  surfaceComponent?: {
    renderer?: unknown;
  } | null;
  selection?: {
    selectedIds?: string[];
    selectedElements?: EdgelessSelectable[];
    slots?: {
      updated?: {
        subscribe: (listener: () => void) => { unsubscribe: () => void };
      };
    };
  };
  layer?: {
    slots?: {
      layerUpdated?: {
        subscribe: (listener: () => void) => { unsubscribe: () => void };
      };
    };
  } | null;
  getElementById?: (id: string) => EdgelessSelectable | null;
  getElementsByBound?: (
    bound: Bound,
    options: { type: "canvas" | "block" | "all" },
  ) => EdgelessSelectable[];
};

type SnapshotExport = {
  path: string;
  attachment: ComposerAttachment;
};

type SnapshotExportFailureReason =
  | "empty"
  | "unavailable"
  | "render-failed"
  | "blob-failed"
  | "save-failed";

type SnapshotProgress = (message: string) => void;

type CanvasPlacementSpec = {
  width: number;
  height: number;
  anchor?: [number, number];
};

function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  label: string,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const timeout = window.setTimeout(() => {
      reject(new Error(`${label} timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    promise
      .then((value) => resolve(value))
      .catch((error) => reject(error))
      .finally(() => window.clearTimeout(timeout));
  });
}

function waitForNextFrame(): Promise<void> {
  return new Promise((resolve) => {
    window.requestAnimationFrame(() => resolve());
  });
}

async function reportSnapshotProgress(
  onProgress: SnapshotProgress | undefined,
  message: string,
) {
  onProgress?.(message);
  if (onProgress) await waitForNextFrame();
}

function getGfxController(editor: BlockSuiteEditor | null): GfxControllerLike | null {
  return editor?.std?.get?.(GfxControllerIdentifier) as GfxControllerLike | null ?? null;
}

function getSelectableXYWH(item: EdgelessSelectable): string | null {
  return typeof item.xywh === "string" && item.xywh
    ? item.xywh
    : typeof item.props?.xywh === "string" && item.props.xywh
      ? item.props.xywh
      : null;
}

function boundFromUnknown(value: unknown): Bound | null {
  if (!value || typeof value !== "object") return null;
  const candidate = value as {
    x?: unknown;
    y?: unknown;
    w?: unknown;
    h?: unknown;
    width?: unknown;
    height?: unknown;
    toXYWH?: () => unknown;
  };
  if (typeof candidate.toXYWH === "function") {
    const xywh = candidate.toXYWH();
    if (
      Array.isArray(xywh) &&
      xywh.length >= 4 &&
      xywh.every((item) => typeof item === "number" && Number.isFinite(item))
    ) {
      return new Bound(xywh[0], xywh[1], xywh[2], xywh[3]);
    }
  }
  if (
    typeof candidate.x === "number" &&
    typeof candidate.y === "number" &&
    typeof candidate.w === "number" &&
    typeof candidate.h === "number"
  ) {
    return new Bound(candidate.x, candidate.y, candidate.w, candidate.h);
  }
  if (
    typeof candidate.x === "number" &&
    typeof candidate.y === "number" &&
    typeof candidate.width === "number" &&
    typeof candidate.height === "number"
  ) {
    return new Bound(candidate.x, candidate.y, candidate.width, candidate.height);
  }
  return null;
}

function getSelectableBound(item: EdgelessSelectable): Bound | null {
  const elementBound = boundFromUnknown(item.elementBound);
  if (elementBound) return elementBound;
  const responseBound = boundFromUnknown(item.responseBound);
  if (responseBound) return responseBound;
  const xywh = getSelectableXYWH(item);
  if (!xywh) return null;
  try {
    return Bound.deserialize(xywh);
  } catch {
    return null;
  }
}

function isEdgelessSelectable(item: EdgelessSelectable | null | undefined): item is EdgelessSelectable {
  return Boolean(item?.id && getSelectableBound(item));
}

function getCanvasElementById(
  gfx: GfxControllerLike | null,
  rootService: EdgelessRootService | null,
  id: string,
): EdgelessSelectable | null {
  const fromGfx = gfx?.getElementById?.(id);
  if (fromGfx) return fromGfx;
  const crud = (rootService as unknown as {
    crud?: {
      getElementById?: (elementId: string) => EdgelessSelectable | null;
    };
  } | null)?.crud;
  return crud?.getElementById?.(id) ?? null;
}

function snapshotExportFailureMessage(reason: SnapshotExportFailureReason): string {
  switch (reason) {
    case "empty":
      return "Select one or more blocks before attaching a snapshot.";
    case "unavailable":
      return "Canvas editor is not ready for snapshot export yet.";
    case "render-failed":
      return "Could not render a visible PNG for the selected canvas nodes.";
    case "blob-failed":
      return "BlockSuite rendered the snapshot, but PNG encoding failed.";
    case "save-failed":
      return "BlockSuite rendered the snapshot, but saving the PNG attachment failed.";
  }
}

async function snapshotExportFromPath(path: string): Promise<SnapshotExport> {
  const previewDataUrl = await readLocalImageDataUrl(path).catch(() => null);
  return {
    path,
    attachment: {
      path,
      kind: "image",
      mimeType: "image/png",
      previewDataUrl,
      displayName: "Canvas selection",
      name: "Canvas selection",
    },
  };
}

function isCanvasVisuallyEmpty(canvas: HTMLCanvasElement): boolean {
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context || canvas.width <= 0 || canvas.height <= 0) return true;
  const sampleWidth = Math.min(canvas.width, 260);
  const sampleHeight = Math.min(canvas.height, 260);
  const stepX = Math.max(1, Math.floor(canvas.width / sampleWidth));
  const stepY = Math.max(1, Math.floor(canvas.height / sampleHeight));
  const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
  for (let y = 0; y < canvas.height; y += stepY) {
    for (let x = 0; x < canvas.width; x += stepX) {
      const offset = (y * canvas.width + x) * 4;
      const alpha = data[offset + 3] ?? 0;
      if (alpha > 4) return false;
    }
  }
  return true;
}

function unionDOMRects(rects: DOMRect[]): DOMRect | null {
  if (rects.length === 0) return null;
  let left = rects[0].left;
  let top = rects[0].top;
  let right = rects[0].right;
  let bottom = rects[0].bottom;
  for (const rect of rects.slice(1)) {
    left = Math.min(left, rect.left);
    top = Math.min(top, rect.top);
    right = Math.max(right, rect.right);
    bottom = Math.max(bottom, rect.bottom);
  }
  return new DOMRect(left, top, right - left, bottom - top);
}

function shouldIgnoreSnapshotElement(element: Element): boolean {
  const tagName = element.tagName.toLowerCase();
  if (tagName === "editor-toolbar") return true;
  return Boolean(
    element.closest("editor-toolbar") ||
      element.closest("[data-sessio-snapshot-ignore='true']") ||
      element.closest(".widgets-container") ||
      element.closest("[class*='toolbar' i]") ||
      element.closest("[class*='popover' i]") ||
      element.closest("[class*='menu' i]") ||
      element.closest("[class*='selection' i]") ||
      element.closest("[class*='resize' i]"),
  );
}

function getSnapshotCropBounds(
  shell: HTMLElement,
  selectionRect: DOMRect,
  padding: number,
) {
  const shellRect = shell.getBoundingClientRect();
  const left = Math.max(0, selectionRect.left - shellRect.left - padding);
  const top = Math.max(0, selectionRect.top - shellRect.top - padding);
  const right = Math.min(shellRect.width, selectionRect.right - shellRect.left + padding);
  const bottom = Math.min(shellRect.height, selectionRect.bottom - shellRect.top + padding);
  if (right <= left || bottom <= top) return null;
  return {
    left,
    top,
    width: right - left,
    height: bottom - top,
    shellWidth: shellRect.width,
    shellHeight: shellRect.height,
  };
}

function cropCanvas(
  source: HTMLCanvasElement,
  crop: { left: number; top: number; width: number; height: number },
  scale: number,
): HTMLCanvasElement | null {
  const width = Math.max(1, Math.ceil(crop.width * scale));
  const height = Math.max(1, Math.ceil(crop.height * scale));
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  canvas.style.width = `${Math.ceil(crop.width)}px`;
  canvas.style.height = `${Math.ceil(crop.height)}px`;
  const context = canvas.getContext("2d");
  if (!context) return null;
  context.drawImage(
    source,
    Math.max(0, Math.floor(crop.left * scale)),
    Math.max(0, Math.floor(crop.top * scale)),
    width,
    height,
    0,
    0,
    width,
    height,
  );
  return canvas;
}

async function renderCanvasDomSnapshot(
  shell: HTMLElement,
  selectionRect: DOMRect,
  padding = 16,
): Promise<HTMLCanvasElement | null> {
  const html2canvas = (await import("html2canvas")).default;
  const crop = getSnapshotCropBounds(shell, selectionRect, padding);
  if (!crop) return null;
  const scale = window.devicePixelRatio || 1;
  const fullCanvas = await html2canvas(shell, {
    backgroundColor: null,
    height: Math.ceil(crop.shellHeight),
    ignoreElements: (element) => {
      if (element === shell) return false;
      return shouldIgnoreSnapshotElement(element);
    },
    logging: false,
    scale,
    width: Math.ceil(crop.shellWidth),
    useCORS: true,
  });
  return cropCanvas(fullCanvas, crop, scale);
}

async function withSnapshotCaptureMode<T>(
  shell: HTMLElement,
  task: () => Promise<T>,
): Promise<T> {
  shell.classList.add(SNAPSHOT_CAPTURING_CLASS_NAME);
  document.documentElement.classList.add(SNAPSHOT_CAPTURING_CLASS_NAME);
  document.body.classList.add(SNAPSHOT_CAPTURING_CLASS_NAME);
  try {
    await waitForNextFrame();
    await waitForNextFrame();
    return await task();
  } finally {
    shell.classList.remove(SNAPSHOT_CAPTURING_CLASS_NAME);
    document.documentElement.classList.remove(SNAPSHOT_CAPTURING_CLASS_NAME);
    document.body.classList.remove(SNAPSHOT_CAPTURING_CLASS_NAME);
  }
}

async function captureNativeWindowSnapshot(
  shell: HTMLElement,
  selectionRect: DOMRect,
  padding = 16,
): Promise<SnapshotExport | null> {
  const crop = getSnapshotCropBounds(shell, selectionRect, padding);
  if (!crop) return null;
  const shellRect = shell.getBoundingClientRect();
  const path = await captureWindowAreaPng({
    fileName: `blocksuite-selection-${Date.now()}.png`,
    x: shellRect.left + crop.left,
    y: shellRect.top + crop.top,
    width: crop.width,
    height: crop.height,
  })
    .then((result) => result.path)
    .catch((error) => {
      console.warn("Native BlockSuite snapshot capture failed.", error);
      return null;
    });
  return path ? snapshotExportFromPath(path) : null;
}

function getSelectedOverlayElements(
  roots: HTMLElement[],
  selected: EdgelessSelectable[],
): HTMLElement[] {
  const selectedIds = new Set(selected.map((item) => item.id));
  const seen = new Set<HTMLElement>();
  return roots
    .flatMap((root) => Array.from(root.querySelectorAll<HTMLElement>("[data-sessio-overlay-block-id]")))
    .filter((element) => {
      if (seen.has(element)) return false;
      seen.add(element);
      return true;
    })
    .filter((element) => selectedIds.has(element.dataset.sessioOverlayBlockId ?? ""))
    .sort((a, b) => {
      const az = Number.parseFloat(window.getComputedStyle(a).zIndex || "0") || 0;
      const bz = Number.parseFloat(window.getComputedStyle(b).zIndex || "0") || 0;
      return az - bz;
    });
}

function getVisibleSelectionRect(
  roots: HTMLElement[],
  baseRect: DOMRect,
  selected: EdgelessSelectable[],
): DOMRect {
  const overlayRects = getSelectedOverlayElements(roots, selected)
    .map((element) => element.getBoundingClientRect())
    .filter((rect) => rect.width > 0 && rect.height > 0);
  return unionDOMRects([baseRect, ...overlayRects]) ?? baseRect;
}

function getSelectionClientRect(
  editor: BlockSuiteEditor,
  gfx: GfxControllerLike | null,
  selected: EdgelessSelectable[],
): DOMRect | null {
  const hostRect = editor.getBoundingClientRect();
  const rects = selected
    .map((item) => {
      const block = editor.std?.host?.view?.getBlock?.(item.id);
      if (block?.isConnected) {
        const rect = block.getBoundingClientRect();
        if (rect.width > 0 && rect.height > 0) return rect;
      }
      const bound = getSelectableBound(item);
      if (!bound || !gfx?.viewport) return null;
      const viewBound = gfx.viewport.toViewBound(bound);
      const [x, y, width, height] = viewBound.toXYWH();
      return new DOMRect(hostRect.left + x, hostRect.top + y, width, height);
    })
    .filter((rect): rect is DOMRect => Boolean(rect && rect.width > 0 && rect.height > 0));
  return unionDOMRects(rects);
}

function getSelectedCanvasElements(
  editor: BlockSuiteEditor | null,
  rootService: EdgelessRootService | null,
  elementIds?: string[] | null,
) {
  const gfx = getGfxController(editor);
  const gfxSelected = (gfx?.selection?.selectedElements ?? []).filter(isEdgelessSelectable);
  const rootSelected = ((rootService?.selection.selectedElements ?? []) as EdgelessSelectable[])
    .filter(isEdgelessSelectable);
  if (elementIds?.length) {
    const selectedById = new Map(
      [...gfxSelected, ...rootSelected].map((item) => [item.id, item]),
    );
    const elements = elementIds
      .map((id) => getCanvasElementById(gfx, rootService, id) ?? selectedById.get(id) ?? null)
      .filter(isEdgelessSelectable);
    if (elements.length > 0) {
      return {
        ids: elements.map((item) => item.id),
        elements,
        gfx,
      };
    }
  }
  if (gfxSelected.length > 0) {
    return {
      ids: gfx?.selection?.selectedIds ?? gfxSelected.map((item) => item.id),
      elements: gfxSelected,
      gfx,
    };
  }
  return {
    ids: (rootService?.selection.selectedIds ?? rootSelected.map((item) => item.id)) as string[],
    elements: rootSelected,
    gfx,
  };
}

export interface BlockSuiteCanvasHostProps {
  sessionId: string;
  sessionAgent: Agent;
  workspacePath: string | null;
  sessionThreadId?: string | null;
  editedFiles?: string[];
  selectedFileRequest?: {
    paths: string[];
    requestId: number;
  } | null;
  initialState: CanvasDocumentState;
  initialSnapshot: string | null;
  composer: ChatComposerController;
  onStateLoaded: (state: CanvasDocumentState) => void;
  onError: (message: string) => void;
  onOpenProjectFile?: (path: string) => void;
  onOpenThreadMultiSessionChat?: (threadId: string) => void;
}

function snapshotToJson(doc: ReturnType<typeof createBlockSuiteDoc>["doc"]) {
  const snapshot = exportDocSnapshot(doc);
  return snapshot ? JSON.stringify(snapshot) : null;
}

function addEdgelessNote(
  rootService: EdgelessRootService,
  editor: BlockSuiteEditor,
  bound?: Bound,
) {
  const std = editor.std;
  if (!std) {
    throw new Error("Canvas editor context is not ready.");
  }

  const gfx = std.get?.(GfxControllerIdentifier) as GfxControllerLike | null;
  const crud = rootService.crud;
  if (!gfx || !crud) {
    throw new Error("Canvas drawing services are not ready.");
  }

  const center = rootService.viewport.center;
  const [viewX, viewY] = gfx.viewport.toViewCoord(center.x, center.y);
  const [modelX, modelY] = gfx.viewport.toModelCoord(viewX, viewY);
  const noteBound = bound ?? new Bound(
    modelX - DEFAULT_NOTE_WIDTH / 2,
    modelY - DEFAULT_NOTE_HEIGHT / 2,
    DEFAULT_NOTE_WIDTH,
    DEFAULT_NOTE_HEIGHT,
  );

  return crud.addBlock(
    "affine:note",
    {
      xywh: serializeXYWH(
        noteBound.x,
        noteBound.y,
        noteBound.w,
        noteBound.h,
      ),
      displayMode: NoteDisplayMode.EdgelessOnly,
    },
    rootService.surface.id,
  );
}

function boundFromModelXYWH(model: { xywh?: string } | null | undefined): Bound | null {
  if (!model?.xywh) return null;
  try {
    return Bound.deserialize(model.xywh);
  } catch {
    return null;
  }
}

function normalizeCanvasFileKey(path: string | null | undefined, workspacePath: string | null): string | null {
  const trimmed = path?.trim();
  if (!trimmed) return null;
  const resolved = resolveCanvasFilePath(trimmed, workspacePath) ?? trimmed;
  const normalized = normalizePathSegment(resolved);
  return /^[a-zA-Z]:\//.test(normalized) ? normalized.toLowerCase() : normalized;
}

function collectCanvasOccupiedBounds(doc: ReturnType<typeof createBlockSuiteDoc>["doc"]): Bound[] {
  const occupied: Bound[] = [];
  const seen = new Set<string>();
  const pushBound = (bound: Bound | null) => {
    if (!bound) return;
    const key = `${bound.x}:${bound.y}:${bound.w}:${bound.h}`;
    if (seen.has(key)) return;
    seen.add(key);
    occupied.push(bound);
  };

  const blockModels = doc.getBlocksByFlavour([
    "sessio:markdown-preview",
    "sessio:file-card",
    "sessio:workflow-card",
    "affine:note",
    "affine:image",
  ]);
  for (const block of blockModels) {
    pushBound(boundFromModelXYWH(block.model as { xywh?: string } | null));
  }

  const surface = doc.getBlocksByFlavour("affine:surface")[0]?.model as {
    elementModels?: unknown[];
  } | null;
  if (Array.isArray(surface?.elementModels)) {
    for (const element of surface.elementModels) {
      pushBound(getSelectableBound(element as EdgelessSelectable));
    }
  }

  return occupied;
}

function getExistingCanvasFileBlockIds(
  doc: ReturnType<typeof createBlockSuiteDoc>["doc"],
  workspacePath: string | null,
): Map<string, string[]> {
  const existing = new Map<string, string[]>();
  const fileBlocks = doc.getBlocksByFlavour([
    "sessio:file-card",
    "sessio:markdown-preview",
  ]);
  for (const block of fileBlocks) {
    const model = block.model as { id: string; sourcePath?: string | null };
    const key = normalizeCanvasFileKey(model.sourcePath ?? null, workspacePath);
    if (!key) continue;
    const ids = existing.get(key);
    if (ids) {
      ids.push(model.id);
    } else {
      existing.set(key, [model.id]);
    }
  }
  return existing;
}

function boundsOverlapWithPadding(a: Bound, b: Bound, padding: number): boolean {
  return (
    a.x < b.x + b.w + padding &&
    a.x + a.w + padding > b.x &&
    a.y < b.y + b.h + padding &&
    a.y + a.h + padding > b.y
  );
}

function canPlaceCanvasBound(candidate: Bound, occupied: Bound[], padding: number): boolean {
  return occupied.every((bound) => !boundsOverlapWithPadding(candidate, bound, padding));
}

function* iteratePlacementGrid(radiusLimit: number): Generator<[number, number]> {
  yield [0, 0];
  for (let radius = 1; radius <= radiusLimit; radius += 1) {
    for (let x = -radius; x <= radius; x += 1) {
      yield [x, -radius];
    }
    for (let y = -radius + 1; y <= radius; y += 1) {
      yield [radius, y];
    }
    for (let x = radius - 1; x >= -radius; x -= 1) {
      yield [x, radius];
    }
    for (let y = radius - 1; y > -radius; y -= 1) {
      yield [-radius, y];
    }
  }
}

function findAvailableCanvasBound(
  occupied: Bound[],
  spec: CanvasPlacementSpec,
  fallbackAnchor: [number, number],
  padding: number,
): Bound {
  const anchor = spec.anchor ?? fallbackAnchor;
  const stepX = spec.width + padding;
  const stepY = spec.height + padding;
  for (const [gridX, gridY] of iteratePlacementGrid(CANVAS_PLACEMENT_SEARCH_RADIUS)) {
    const candidate = Bound.fromCenter(
      [anchor[0] + gridX * stepX, anchor[1] + gridY * stepY],
      spec.width,
      spec.height,
    );
    if (canPlaceCanvasBound(candidate, occupied, padding)) {
      return candidate;
    }
  }
  return Bound.fromCenter(
    [anchor[0], anchor[1] + (occupied.length + 1) * stepY],
    spec.width,
    spec.height,
  );
}

function placeCanvasNodes(
  doc: ReturnType<typeof createBlockSuiteDoc>["doc"],
  rootService: EdgelessRootService,
  specs: CanvasPlacementSpec[],
  padding = CANVAS_NODE_PADDING,
): Bound[] {
  const viewportCenter = rootService.viewport.center;
  const fallbackAnchor: [number, number] = [viewportCenter.x, viewportCenter.y];
  const occupied = collectCanvasOccupiedBounds(doc);
  return specs.map((spec) => {
    const bound = findAvailableCanvasBound(occupied, spec, fallbackAnchor, padding);
    occupied.push(bound);
    return bound;
  });
}

function readBlobImageSize(blob: Blob): Promise<{ width: number; height: number }> {
  return new Promise((resolve) => {
    if (!blob.type.startsWith("image/")) {
      resolve({ width: 0, height: 0 });
      return;
    }
    const objectUrl = URL.createObjectURL(blob);
    const image = new Image();
    image.onload = () => {
      resolve({
        width: image.width,
        height: image.height,
      });
      URL.revokeObjectURL(objectUrl);
    };
    image.onerror = () => {
      resolve({ width: 0, height: 0 });
      URL.revokeObjectURL(objectUrl);
    };
    image.src = objectUrl;
  });
}

export default function BlockSuiteCanvasHost({
  sessionId,
  sessionAgent,
  workspacePath,
  sessionThreadId = null,
  editedFiles = [],
  selectedFileRequest = null,
  initialState,
  initialSnapshot,
  composer,
  onStateLoaded,
  onError,
  onOpenProjectFile,
  onOpenThreadMultiSessionChat,
}: BlockSuiteCanvasHostProps) {
  const canvasShellRef = useRef<HTMLDivElement | null>(null);
  const hostRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<BlockSuiteEditor | null>(null);
  const docRef = useRef<ReturnType<typeof createBlockSuiteDoc>["doc"] | null>(null);
  const latestStateRef = useRef(initialState);
  const blockUpdatedDisposeRef = useRef<{ dispose: () => void } | null>(null);
  const selectionUpdatedDisposeRef = useRef<{ dispose: () => void } | null>(null);
  const selectionUiFrameRef = useRef<number | null>(null);
  const selectionUiStateRef = useRef({
    count: 0,
    canUngroup: false,
  });
  const autosaveTimerRef = useRef<number | null>(null);
  const selectionUiDeferredRef = useRef(false);
  const boxSelectingObserverRef = useRef<MutationObserver | null>(null);
  const boxSelectingHostRef = useRef<HTMLElement | null>(null);
  const isBoxSelectingRef = useRef(false);
  const inflightSaveRef = useRef(false);
  const queuedSnapshotRef = useRef<string | null>(null);
  const currentSnapshotRef = useRef(initialSnapshot);
  const handledFileRequestRef = useRef<string | null>(null);
  const expandedFileCardHeightsRef = useRef(new Map<string, number>());
  const addMenuButtonRef = useRef<HTMLButtonElement>(null);
  const editedFilesButtonRef = useRef<HTMLButtonElement>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [status, setStatus] = useState("Initializing BlockSuite canvas…");
  const [snapshotToast, setSnapshotToast] = useState<ToastStackMessage | null>(null);
  const [isReady, setIsReady] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [selectionCount, setSelectionCount] = useState(0);
  const [canUngroupSelection, setCanUngroupSelection] = useState(false);
  const [bridgeBusy, setBridgeBusy] = useState<null | "ask" | "snapshot" | "workflow">(null);
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  const [editedFilesPickerOpen, setEditedFilesPickerOpen] = useState(false);
  const [pendingEditedFiles, setPendingEditedFiles] = useState<string[]>([]);
  const [canvasBlockRecords, setCanvasBlockRecords] = useState(initialState.blockRecords);
  const [localCanvasFileKeys, setLocalCanvasFileKeys] = useState<Set<string>>(() => {
    const keys = new Set<string>();
    for (const record of initialState.blockRecords) {
      const key = normalizeCanvasFileKey(record.sourcePath, workspacePath);
      if (key) keys.add(key);
    }
    return keys;
  });
  const [reactToLit, portals] = useReactToLitBridge();

  const changedFiles = useMemo(
    () => Array.from(new Set(editedFiles.map((path) => path.trim()).filter(Boolean))),
    [editedFiles],
  );
  const persistedCanvasFileKeys = useMemo(() => {
    const keys = new Set<string>();
    for (const record of canvasBlockRecords) {
      const key = normalizeCanvasFileKey(record.sourcePath, workspacePath);
      if (key) keys.add(key);
    }
    return keys;
  }, [canvasBlockRecords, workspacePath]);
  const availableEditedFiles = useMemo(
    () => changedFiles.filter((path) => {
      const key = normalizeCanvasFileKey(path, workspacePath);
      return !key || !localCanvasFileKeys.has(key);
    }),
    [changedFiles, localCanvasFileKeys, workspacePath],
  );
  const lastSavedRevision = initialState.savedRevision?.revision ?? null;

  const addMenuOptions = useMemo<PopupMenuOption<string>[]>(() => [
    { key: "file", label: "Choose files", icon: <FolderOpen className="h-4 w-4" /> },
    { key: "image", label: "Add image", icon: <FileImage className="h-4 w-4" /> },
    { key: "workflow", label: "Add workflow", icon: <Workflow className="h-4 w-4" /> },
    { key: "note", label: "Add note", icon: <StickyNote className="h-4 w-4" /> },
  ], []);

  const showSnapshotToast = useCallback((message: string, tone: ToastStackMessage["tone"] = "info") => {
    setSnapshotToast({ message, tone });
  }, []);

  useEffect(() => {
    latestStateRef.current = initialState;
  }, [initialState]);

  useEffect(() => {
    setCanvasBlockRecords(initialState.blockRecords);
  }, [initialState.blockRecords]);

  useEffect(() => {
    setLocalCanvasFileKeys(persistedCanvasFileKeys);
  }, [persistedCanvasFileKeys]);

  useEffect(() => {
    setPendingEditedFiles((current) => {
      if (current.length === 0) return current;
      const next = current.filter((path) => {
        const key = normalizeCanvasFileKey(path, workspacePath);
        return !key || !localCanvasFileKeys.has(key);
      });
      return next.length === current.length ? current : next;
    });
  }, [localCanvasFileKeys, workspacePath]);

  const getEditor = useCallback(() => editorRef.current, []);
  const getDoc = useCallback(() => docRef.current, []);

  const getRootService = useCallback((): EdgelessRootService | null => {
    try {
      return getEditor()?.std?.get?.(EdgelessRootService) as EdgelessRootService | null ?? null;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setStatus(`Canvas startup failed: ${message}`);
      return null;
    }
  }, [getEditor]);

  const waitForRootService = useCallback(async (): Promise<EdgelessRootService | null> => {
    let rootService = getRootService();
    for (let attempt = 0; attempt < ROOT_SERVICE_RETRY_LIMIT && !rootService; attempt += 1) {
      await new Promise((resolve) => window.setTimeout(resolve, ROOT_SERVICE_RETRY_MS));
      rootService = getRootService();
    }
    return rootService;
  }, [getRootService]);

  const insertEdgelessBlock = useCallback((
    flavour: string,
    props: Record<string, unknown>,
  ) => {
    const rootService = getRootService();
    const doc = getDoc();
    if (!rootService || !doc) {
      throw new Error("Canvas root service is not ready");
    }
    if (!rootService.surface) {
      throw new Error("Canvas surface is not ready");
    }
    return doc.addBlock(flavour as never, props, rootService.surface.id);
  }, [getDoc, getRootService]);

  const commitSelectionState = useCallback((count: number, canUngroup: boolean) => {
    const current = selectionUiStateRef.current;
    if (current.count === count && current.canUngroup === canUngroup) {
      return;
    }
    selectionUiStateRef.current = {
      count,
      canUngroup,
    };
    setSelectionCount(count);
    setCanUngroupSelection(canUngroup);
  }, []);

  const updateSelectionState = useCallback(() => {
    const rootService = getRootService();
    const editor = getEditor();
    if (!rootService && !editor) {
      commitSelectionState(0, false);
      return;
    }
    const { ids: selectedIds, elements: selectedElements } = getSelectedCanvasElements(editor, rootService);
    const selectedElement =
      selectedIds.length === 1
        ? (selectedElements[0] as SelectionElementLike | undefined)
        : undefined;
    commitSelectionState(
      selectedIds.length,
      selectedIds.length === 1 && selectedElement?.type === "group",
    );
  }, [commitSelectionState, getEditor, getRootService]);

  const scheduleSelectionStateUpdate = useCallback(() => {
    if (selectionUiFrameRef.current !== null) {
      return;
    }
    selectionUiFrameRef.current = window.requestAnimationFrame(() => {
      selectionUiFrameRef.current = null;
      updateSelectionState();
    });
  }, [updateSelectionState]);

  const clearBoxSelectingObserver = useCallback(() => {
    boxSelectingObserverRef.current?.disconnect();
    boxSelectingObserverRef.current = null;
    boxSelectingHostRef.current = null;
    isBoxSelectingRef.current = false;
    selectionUiDeferredRef.current = false;
  }, []);

  const flushDeferredCanvasUiSync = useCallback(() => {
    if (selectionUiDeferredRef.current) {
      selectionUiDeferredRef.current = false;
      scheduleSelectionStateUpdate();
    }
  }, [scheduleSelectionStateUpdate]);

  const attachBoxSelectingObserver = useCallback(() => {
    const editor = getEditor();
    const nextHost = ((editor?.std?.host as HTMLElement | undefined) ?? editor) ?? null;
    if (!nextHost) {
      clearBoxSelectingObserver();
      return;
    }
    if (boxSelectingHostRef.current === nextHost && boxSelectingObserverRef.current) {
      return;
    }

    clearBoxSelectingObserver();
    boxSelectingHostRef.current = nextHost;
    isBoxSelectingRef.current = nextHost.classList.contains(BOX_SELECTING_CLASS_NAME);

    const syncBoxSelectingState = () => {
      const nextIsSelecting = nextHost.classList.contains(BOX_SELECTING_CLASS_NAME);
      if (nextIsSelecting === isBoxSelectingRef.current) {
        return;
      }
      isBoxSelectingRef.current = nextIsSelecting;
      if (nextIsSelecting) {
        if (selectionUiFrameRef.current !== null) {
          window.cancelAnimationFrame(selectionUiFrameRef.current);
          selectionUiFrameRef.current = null;
        }
        return;
      }
      flushDeferredCanvasUiSync();
    };

    const observer = new MutationObserver(syncBoxSelectingState);
    observer.observe(nextHost, {
      attributes: true,
      attributeFilter: ["class"],
    });
    boxSelectingObserverRef.current = observer;
  }, [clearBoxSelectingObserver, flushDeferredCanvasUiSync, getEditor]);

  const finishCanvasInitialization = useCallback(async (nextStatus: string) => {
    if (!getRootService()) {
      setStatus("Finishing canvas startup…");
    }
    const rootService = await waitForRootService();
    if (!rootService) {
      setStatus("Canvas initialization is taking longer than expected.");
      setIsReady(false);
      return false;
    }
    setStatus(nextStatus);
    setIsReady(true);
    updateSelectionState();
    return true;
  }, [getRootService, updateSelectionState, waitForRootService]);

  const syncCanvasBlocks = useCallback(async (doc: NonNullable<ReturnType<typeof getDoc>>) => {
    const records = doc
      .getBlocksByFlavour([
        "sessio:markdown-preview",
        "sessio:file-card",
        "sessio:workflow-card",
        "affine:note",
        "affine:image",
      ])
      .map((item) => canvasInteropModelToCanvasBlock(
        item.model as MarkdownPreviewBlockModel | FileCardBlockModel | WorkflowCardBlockModel,
      ))
      .filter((item): item is NonNullable<typeof item> => Boolean(item));

    const surface = doc.getBlocksByFlavour("affine:surface")[0]?.model as {
      elementModels?: unknown[];
    } | null;
    const surfaceRecords = Array.isArray(surface?.elementModels)
      ? surface.elementModels
          .map((item) => surfaceElementToCanvasBlock(item as { id: string; type?: string; title?: string; childIds?: string[] }))
          .filter((item): item is NonNullable<typeof item> => Boolean(item))
      : [];
    try {
      const saved = await updateCanvasBlocks({
        sessionId,
        blocks: [...records, ...surfaceRecords],
      });
      setCanvasBlockRecords(saved);
      latestStateRef.current = {
        ...latestStateRef.current,
        blockRecords: saved,
      };
      onStateLoaded({
        ...latestStateRef.current,
        blockRecords: saved,
      });
    } catch (error) {
      const message = `Failed to sync canvas blocks: ${String(error)}`;
      setSaveError(message);
      onError(message);
    } finally {
      updateSelectionState();
    }
  }, [onError, onStateLoaded, sessionId, updateSelectionState]);

  const scheduleSyncBlocks = useCallback(() => {
    const doc = getDoc();
    if (!doc) return;
    void syncCanvasBlocks(doc);
  }, [getDoc, syncCanvasBlocks]);

  const flushSaveRef = useRef<(snapshotJson: string) => Promise<void>>(async () => {});

  const scheduleAutosave = useCallback((doc: ReturnType<typeof createBlockSuiteDoc>["doc"]) => {
    if (autosaveTimerRef.current !== null) {
      window.clearTimeout(autosaveTimerRef.current);
    }
    autosaveTimerRef.current = window.setTimeout(() => {
      const snapshotJson = snapshotToJson(doc);
      if (!snapshotJson || snapshotJson === currentSnapshotRef.current) {
        return;
      }
      if (inflightSaveRef.current) {
        queuedSnapshotRef.current = snapshotJson;
        return;
      }
      void flushSaveRef.current(snapshotJson);
    }, AUTOSAVE_DEBOUNCE_MS);
  }, []);

  const recomputeLocalCanvasFileKeys = useCallback((
    doc: ReturnType<typeof createBlockSuiteDoc>["doc"],
  ) => {
    const keys = new Set<string>();
    for (const block of doc.getBlocksByFlavour([
      "sessio:file-card",
      "sessio:markdown-preview",
    ])) {
      const model = block.model as { sourcePath?: string | null };
      const key = normalizeCanvasFileKey(model.sourcePath ?? null, workspacePath);
      if (key) keys.add(key);
    }
    setLocalCanvasFileKeys(keys);
  }, [workspacePath]);

  const scheduleRecomputeLocalCanvasFileKeys = useCallback((
    doc: ReturnType<typeof createBlockSuiteDoc>["doc"],
  ) => {
    window.requestAnimationFrame(() => {
      if (docRef.current !== doc) return;
      recomputeLocalCanvasFileKeys(doc);
    });
  }, [recomputeLocalCanvasFileKeys]);

  const attachDoc = useCallback((host: HTMLDivElement, doc: ReturnType<typeof createBlockSuiteDoc>["doc"]) => {
    ensureEdgelessRoot(doc);
    removePlaceholderNotes(doc);
    const editor = createEdgelessEditorWithSpecs(doc) as BlockSuiteEditor;
    editorRef.current?.remove();
    editorRef.current = editor;
    docRef.current = doc;
    host.replaceChildren(editor);
    blockUpdatedDisposeRef.current?.dispose();
    selectionUpdatedDisposeRef.current?.dispose();
    clearBoxSelectingObserver();
    {
      const subscription = doc.slots.blockUpdated.subscribe((payload) => {
        if (payload?.type === "delete") {
          scheduleRecomputeLocalCanvasFileKeys(doc);
        } else {
          recomputeLocalCanvasFileKeys(doc);
        }
        scheduleAutosave(doc);
      });
      blockUpdatedDisposeRef.current = {
        dispose: () => subscription.unsubscribe(),
      };
    }
    window.requestAnimationFrame(() => {
      attachBoxSelectingObserver();
      void waitForRootService().then((rootService) => {
        if (!rootService || docRef.current !== doc) return;
        attachBoxSelectingObserver();
        selectionUpdatedDisposeRef.current?.dispose();
        const subscription = rootService.selection.slots.updated.subscribe(() => {
          if (isBoxSelectingRef.current) {
            selectionUiDeferredRef.current = true;
            return;
          }
          scheduleSelectionStateUpdate();
        });
        selectionUpdatedDisposeRef.current = {
          dispose: () => subscription.unsubscribe(),
        };
        updateSelectionState();
      });
      recomputeLocalCanvasFileKeys(doc);
      scheduleSyncBlocks();
    });
  }, [attachBoxSelectingObserver, clearBoxSelectingObserver, recomputeLocalCanvasFileKeys, scheduleAutosave, scheduleRecomputeLocalCanvasFileKeys, scheduleSelectionStateUpdate, scheduleSyncBlocks, updateSelectionState, waitForRootService]);

  const openEditedFilesPicker = () => {
    const filteredAvailableFiles = changedFiles.filter((path) => {
      const key = normalizeCanvasFileKey(path, workspacePath);
      return !key || !localCanvasFileKeys.has(key);
    });
    if (filteredAvailableFiles.length === 0) return;
    if (availableEditedFiles.length === 0) return;
    setPendingEditedFiles((current) => (
      current.length > 0
        ? current.filter((path) => filteredAvailableFiles.includes(path))
        : filteredAvailableFiles
    ));
    setEditedFilesPickerOpen(true);
  };

  const closeEditedFilesPicker = () => {
    setEditedFilesPickerOpen(false);
  };

  const togglePendingEditedFile = (path: string) => {
    setPendingEditedFiles((current) => (
      current.includes(path)
        ? current.filter((item) => item !== path)
        : [...current, path]
    ));
  };

  const resolveFileSourceType = useCallback((path: string) => {
    if (!workspacePath) return "inline_markdown";
    const normalizedWorkspace = normalizePathSegment(workspacePath);
    const normalizedPath = normalizePathSegment(resolveCanvasFilePath(path, workspacePath) ?? path);
    if (
      normalizedPath === normalizedWorkspace ||
      normalizedPath.startsWith(`${normalizedWorkspace}/`)
    ) {
      return changedFiles.includes(path) || changedFiles.includes(normalizedPath)
        ? "edited_file"
        : "workspace_file";
    }
    return "inline_markdown";
  }, [changedFiles, workspacePath]);

  const addFileCards = useCallback(async (paths: string[]) => {
    const doc = getDoc();
    if (!doc) {
      onError("Canvas document is not ready yet.");
      return false;
    }
    const rootService = await waitForRootService();
    if (!rootService) {
      onError("Canvas is still initializing. Please try adding files again in a moment.");
      return false;
    }
    if (!workspacePath) {
      onError("This session is not linked to a workspace, so files can not be added to the canvas.");
      return false;
    }
    const requestedFiles = new Map<string, { inputPath: string; absolutePath: string }>();
    for (const path of paths.map((item) => item.trim()).filter(Boolean)) {
      const absolutePath = resolveCanvasFilePath(path, workspacePath);
      if (!absolutePath) {
        onError(`Failed to add file card for ${path}: file path is unavailable`);
        continue;
      }
      const key = normalizeCanvasFileKey(absolutePath, workspacePath) ?? normalizePathSegment(absolutePath);
      if (!requestedFiles.has(key)) {
        requestedFiles.set(key, {
          inputPath: path,
          absolutePath,
        });
      }
    }

    const existingByPath = getExistingCanvasFileBlockIds(doc, workspacePath);
    const focusBlockIds = new Set<string>();
    const filesToInsert: Array<{ inputPath: string; absolutePath: string; key: string }> = [];
    let duplicateCount = 0;
    for (const [key, file] of requestedFiles.entries()) {
      const existingIds = existingByPath.get(key) ?? [];
      if (existingIds.length > 0) {
        duplicateCount += 1;
        for (const id of existingIds) {
          focusBlockIds.add(id);
        }
        continue;
      }
      filesToInsert.push({ ...file, key });
    }

    const plannedBounds = placeCanvasNodes(
      doc,
      rootService,
      filesToInsert.map(() => ({
        width: DEFAULT_FILE_CARD_WIDTH,
        height: DEFAULT_FILE_CARD_HEIGHT,
      })),
    );
    let addedCount = 0;
    const insertedBlockIds: string[] = [];
    for (const [index, fileToInsert] of filesToInsert.entries()) {
      try {
        const file = await readWorkspaceTextFile(workspacePath, fileToInsert.absolutePath).catch(() => null);
        const title = fileToInsert.absolutePath.split(/[/\\]/).pop() ?? fileToInsert.absolutePath;
        const bound = plannedBounds[index];
        const blockId = insertEdgelessBlock(
          "sessio:file-card",
          {
            title,
            sourcePath: fileToInsert.absolutePath,
            sourceType: resolveFileSourceType(fileToInsert.inputPath),
            subtitle: fileToInsert.absolutePath,
            summary: summarizeText(file?.content ?? "", 260),
            status: file ? "ready" : "unavailable",
            contentVersion: file ? `${fileToInsert.absolutePath}:${file.mtimeMs}` : fileToInsert.absolutePath,
            previewCollapsed: false,
            xywh: bound.serialize(),
          },
        );
        insertedBlockIds.push(blockId);
        focusBlockIds.add(blockId);
        addedCount += 1;
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        onError(`Failed to add file card for ${fileToInsert.inputPath}: ${message}`);
      }
    }

    const finalFocusBlockIds = Array.from(focusBlockIds);
    if (finalFocusBlockIds.length > 0) {
      focusBlocksInViewport(rootService, doc, finalFocusBlockIds);
    }
    if (addedCount === 0) {
      if (duplicateCount > 0) {
        setStatus(
          duplicateCount === 1
            ? "File is already on the canvas."
            : `${duplicateCount} files are already on the canvas.`,
        );
        updateSelectionState();
        return true;
      }
      return false;
    }
    const snapshotJson = snapshotToJson(doc);
    if (!snapshotJson) {
      onError("Canvas snapshot export failed after adding file cards.");
      return false;
    }
    await flushSaveRef.current(snapshotJson);
    await syncCanvasBlocks(doc);
    setStatus(
      duplicateCount > 0
        ? `Added ${addedCount} file card${addedCount === 1 ? "" : "s"} to canvas. Skipped ${duplicateCount} duplicate file${duplicateCount === 1 ? "" : "s"}.`
        : `Added ${addedCount} file card${addedCount === 1 ? "" : "s"} to canvas.`,
    );
    updateSelectionState();
    return true;
  }, [getDoc, insertEdgelessBlock, onError, resolveFileSourceType, syncCanvasBlocks, updateSelectionState, waitForRootService, workspacePath]);

  const addPendingEditedFiles = () => {
    if (pendingEditedFiles.length === 0) return;
    void addFileCards(pendingEditedFiles);
    setEditedFilesPickerOpen(false);
  };

  const addNoteNode = useCallback(async () => {
    const rootService = getRootService();
    const editor = getEditor();
    const doc = getDoc();
    if (!rootService || !editor || !doc) return;
    const [noteBound] = placeCanvasNodes(doc, rootService, [{
      width: DEFAULT_NOTE_WIDTH,
      height: DEFAULT_NOTE_HEIGHT,
    }]);
    const noteId = addEdgelessNote(rootService, editor, noteBound);
    const note = doc.getModelById(noteId);
    if (note) {
      doc.updateBlock(note, {
        displayMode: NoteDisplayMode.EdgelessOnly,
      });
    }
    scheduleSyncBlocks();
  }, [getDoc, getEditor, getRootService, scheduleSyncBlocks]);

  const addImageNode = useCallback(async () => {
    const editor = getEditor();
    const doc = getDoc();
    const rootService = getRootService();
    if (!editor || !doc || !rootService) return;
    try {
      const selection = await open({
        multiple: true,
        directory: false,
        filters: [
          {
            name: "Images",
            extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "heif"],
          },
        ],
      });
      if (!selection) return;
      const paths = Array.isArray(selection) ? selection : [selection];
      const imageFiles = await Promise.all(paths.map(async (path) => {
        const response = await fetch(convertFileSrc(path));
        const blob = await response.blob();
        const size = await readBlobImageSize(blob);
        return {
          file: new File([blob], path.split(/[/\\]/).pop() ?? "image", {
            type: blob.type || "image/png",
          }),
          width: Math.max(size.width || 320, 120),
          height: Math.max(size.height || 180, 120),
        };
      }));
      const plannedBounds = placeCanvasNodes(
        doc,
        rootService,
        imageFiles.map(({ width, height }) => ({ width, height })),
      );
      const insertedBlockIds: string[] = [];
      for (const [index, { file }] of imageFiles.entries()) {
        const bound = plannedBounds[index];
        const blockIds = await addImages(editor.std as never, [file], {
          point: bound.center,
          shouldTransformPoint: false,
        });
        insertedBlockIds.push(...blockIds);
      }
      if (insertedBlockIds.length > 0) {
        focusBlocksInViewport(rootService, doc, insertedBlockIds);
      }
      scheduleSyncBlocks();
    } catch (error) {
      onError(`Failed to add image node: ${String(error)}`);
    }
  }, [getDoc, getEditor, getRootService, onError, scheduleSyncBlocks]);

  const addWorkflowCard = useCallback(async () => {
    const doc = getDoc();
    const rootService = getRootService();
    if (!doc || !rootService) return;
    const snapshotResult = await getThreadWorkSnapshot(sessionAgent, sessionId).catch(() => null);
    const snapshot = snapshotResult?.snapshot ?? null;
    const title = snapshot?.goal?.trim() || "Workflow";
    const summaryMarkdown = workflowSnapshotToMarkdown(snapshot);
    const [bound] = placeCanvasNodes(doc, rootService, [{
      width: DEFAULT_WORKFLOW_CARD_WIDTH,
      height: DEFAULT_WORKFLOW_CARD_HEIGHT,
    }]);
    insertEdgelessBlock(
      "sessio:workflow-card",
      {
        title,
        threadId: snapshotResult?.threadId ?? "",
        threadStageId: snapshotResult?.stageId ?? "",
        sourceType: "workflow_definition",
        workflowSummaryMarkdown: summaryMarkdown,
        executionState: "idle",
        lastRunId: "",
        workflowSnapshotJson: snapshot ? JSON.stringify(snapshot) : "",
        threadGoal: snapshot?.goal ?? "",
        status: "ready",
        xywh: bound.serialize(),
      },
    );
    await syncCanvasBlocks(doc);
    updateSelectionState();
  }, [getDoc, getRootService, insertEdgelessBlock, sessionAgent, sessionId, syncCanvasBlocks, updateSelectionState]);

  const groupSelection = useCallback(() => {
    const rootService = getRootService();
    const editor = getEditor();
    if (!rootService || !editor?.std?.command || getSelectedCanvasElements(editor, rootService).elements.length < 2) return;
    editor.std.command.exec(createGroupFromSelectedCommand);
    scheduleSyncBlocks();
  }, [getEditor, getRootService, scheduleSyncBlocks]);

  const ungroupSelection = useCallback(() => {
    const rootService = getRootService();
    const editor = getEditor();
    if (!rootService || !editor?.std?.command) return;
    const selectedElements = getSelectedCanvasElements(editor, rootService).elements;
    if (selectedElements.length !== 1) return;
    const selected = selectedElements[0] as SelectionElementLike | undefined;
    if (!selected || selected.type !== "group") return;
    editor.std.command.exec(ungroupCommand, { group: selected as unknown as GroupElementModel });
    scheduleSyncBlocks();
  }, [getEditor, getRootService, scheduleSyncBlocks]);

  const runWorkflowBlock = useCallback(async (blockId: string) => {
    const doc = getDoc();
    if (!doc) return;
    const model = doc.getModelById(blockId) as WorkflowCardBlockModel | null;
    const threadId = model?.threadId?.trim() || "";
    if (!model || !threadId) {
      onError("This workflow card is not linked to a thread workflow.");
      return;
    }
    setBridgeBusy("workflow");
    try {
      doc.updateBlock(model, {
        executionState: "running",
        status: "running",
      });
      const run = await createAstraRun(threadId, composer.text.trim() || null);
      doc.updateBlock(model, {
        executionState: run.status,
        lastRunId: run.runId,
        status: run.status,
      });
      await syncCanvasBlocks(doc);
      updateSelectionState();
    } catch (error) {
      doc.updateBlock(model, {
        executionState: "failed",
        status: "failed",
      });
      onError(`Failed to start workflow run: ${String(error)}`);
    } finally {
      setBridgeBusy(null);
    }
  }, [composer.text, getDoc, onError, syncCanvasBlocks, updateSelectionState]);

  const openWorkflowThread = useCallback((blockId: string) => {
    const doc = getDoc();
    const model = doc?.getModelById(blockId) as WorkflowCardBlockModel | null;
    const targetThreadId = model?.threadId?.trim() || sessionThreadId || "";
    if (!targetThreadId) {
      onError("This workflow card is not linked to a thread workflow yet.");
      return;
    }
    if (!onOpenThreadMultiSessionChat) {
      onError("Thread multi-session chat is not available from this view.");
      return;
    }
    onOpenThreadMultiSessionChat(targetThreadId);
  }, [getDoc, onError, onOpenThreadMultiSessionChat, sessionThreadId]);

  useEffect(() => {
    setBlockSuitePortalBridge({
      reactToLit,
      workspacePath,
      updateBlock: (blockId, props) => {
        const doc = getDoc();
        const model = doc?.getModelById(blockId) ?? null;
        if (!doc || !model) return;
        if ("previewCollapsed" in props && typeof props.previewCollapsed === "boolean") {
          const bound = boundFromModelXYWH(model as { xywh?: string } | null);
          if (bound) {
            const expandedHeights = expandedFileCardHeightsRef.current;
            const rememberedHeight = expandedHeights.get(blockId);
            const nextHeight = props.previewCollapsed
              ? DEFAULT_FILE_CARD_COLLAPSED_HEIGHT
              : rememberedHeight ?? Math.max(bound.h, DEFAULT_FILE_CARD_HEIGHT);
            if (props.previewCollapsed) {
              expandedHeights.set(blockId, Math.max(bound.h, DEFAULT_FILE_CARD_HEIGHT));
            }
            doc.updateBlock(model, {
              ...props,
              xywh: serializeXYWH(bound.x, bound.y, bound.w, nextHeight),
            });
            scheduleSyncBlocks();
            return;
          }
        }
        doc.updateBlock(model, props);
        scheduleSyncBlocks();
      },
      runWorkflowBlock: (blockId) => {
        void runWorkflowBlock(blockId);
      },
      openWorkflowThread,
      openProjectFile: onOpenProjectFile,
    });
    return () => {
      setBlockSuitePortalBridge(null);
    };
  }, [
    getDoc,
    onOpenProjectFile,
    openWorkflowThread,
    reactToLit,
    runWorkflowBlock,
    scheduleSyncBlocks,
    workspacePath,
  ]);

  useEffect(() => {
    currentSnapshotRef.current = initialSnapshot;
  }, [initialSnapshot, sessionId]);

  useEffect(() => {
    return () => {
      if (autosaveTimerRef.current !== null) {
        window.clearTimeout(autosaveTimerRef.current);
      }
      if (selectionUiFrameRef.current !== null) {
        window.cancelAnimationFrame(selectionUiFrameRef.current);
        selectionUiFrameRef.current = null;
      }
      clearBoxSelectingObserver();
    };
  }, [clearBoxSelectingObserver]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    setIsReady(false);
    setStatus("Initializing BlockSuite canvas…");

    let activeDoc = createBlockSuiteDoc(initialState.document.id).doc;
    const documentTitle = initialState.document.title;

    const clearSaveError = () => {
      setSaveError(null);
    };

    const mountDoc = (doc: typeof activeDoc) => {
      activeDoc = doc;
      attachDoc(host, doc);
    };

    const flushSave = async (snapshotJson: string) => {
      inflightSaveRef.current = true;
      setIsSaving(true);
      try {
        const saved = await saveCanvasDraft({
          sessionId,
          title: documentTitle,
          snapshotJson,
        });
        currentSnapshotRef.current = snapshotJson;
        clearSaveError();
        setStatus("Draft saved.");
        latestStateRef.current = {
          ...latestStateRef.current,
          document: saved.document,
          draftSnapshot: snapshotJson,
        };
        setCanvasBlockRecords(latestStateRef.current.blockRecords);
        onStateLoaded(latestStateRef.current);
      } catch (error) {
        const message = `Canvas draft save failed: ${String(error)}`;
        setSaveError(message);
        onError(message);
      } finally {
        inflightSaveRef.current = false;
        setIsSaving(false);
        const queued = queuedSnapshotRef.current;
        queuedSnapshotRef.current = null;
        if (queued && queued !== currentSnapshotRef.current) {
          void flushSave(queued);
        }
      }
    };
    flushSaveRef.current = flushSave;

    const restore = async () => {
      try {
        if (!initialSnapshot) {
          mountDoc(activeDoc);
          clearSaveError();
          await finishCanvasInitialization("Opened a fresh BlockSuite canvas.");
          return;
        }
        const snapshot = JSON.parse(initialSnapshot);
        const restored = await importDocSnapshot(snapshot);
        mountDoc(restored ?? activeDoc);
        clearSaveError();
        await finishCanvasInitialization("Restored canvas snapshot.");
      } catch (error) {
        mountDoc(activeDoc);
        const message = `Failed to restore saved canvas state: ${String(error)}`;
        setSaveError(message);
        onError(message);
        await finishCanvasInitialization("Opened a fresh BlockSuite canvas after restore failure.");
      }
    };

    void restore();

    return () => {
      if (autosaveTimerRef.current !== null) {
        window.clearTimeout(autosaveTimerRef.current);
      }
      if (selectionUiFrameRef.current !== null) {
        window.cancelAnimationFrame(selectionUiFrameRef.current);
        selectionUiFrameRef.current = null;
      }
      blockUpdatedDisposeRef.current?.dispose();
      blockUpdatedDisposeRef.current = null;
      selectionUpdatedDisposeRef.current?.dispose();
      selectionUpdatedDisposeRef.current = null;
      clearBoxSelectingObserver();
      editorRef.current?.remove();
      editorRef.current = null;
    };
  }, [
    attachDoc,
    clearBoxSelectingObserver,
    finishCanvasInitialization,
    initialState.document.id,
    initialState.document.title,
    onError,
    onStateLoaded,
    sessionId,
    syncCanvasBlocks,
  ]);

  const handleSaveRevision = async () => {
    const doc = getDoc();
    if (!doc) return;
    const snapshotJson = snapshotToJson(doc);
    if (!snapshotJson) {
      onError("Canvas snapshot export failed.");
      return;
    }
    setIsSaving(true);
    try {
      const saved = await saveCanvasRevision({
        sessionId,
        title: initialState.document.title,
        snapshotJson,
        source: "manual",
      });
      currentSnapshotRef.current = snapshotJson;
      setSaveError(null);
      setStatus(`Saved revision ${saved.revision.revision}.`);
      latestStateRef.current = {
        ...latestStateRef.current,
        document: saved.document,
        draftSnapshot: snapshotJson,
        savedRevision: saved.revision,
        savedSnapshot: snapshotJson,
      };
      setCanvasBlockRecords(latestStateRef.current.blockRecords);
      onStateLoaded(latestStateRef.current);
      await syncCanvasBlocks(doc);
    } catch (error) {
      const message = `Canvas save failed: ${String(error)}`;
      setSaveError(message);
      onError(message);
    } finally {
      setIsSaving(false);
    }
  };

  const handleAddMenuSelect = async (key: string) => {
    setAddMenuOpen(false);
    if (key === "note") {
      await addNoteNode();
      return;
    }
    if (key === "workflow") {
      await addWorkflowCard();
      return;
    }
    if (key === "image") {
      await addImageNode();
      return;
    }
    if (key === "file") {
      try {
        const selection = await open({
          multiple: true,
          directory: false,
        });
        if (!selection) return;
        await addFileCards(Array.isArray(selection) ? selection : [selection]);
      } catch (error) {
        onError(`Failed to add file card: ${String(error)}`);
      }
    }
  };

  const exportSelectionSnapshot = useCallback(async (
    elementIds?: string[] | null,
    onProgress?: SnapshotProgress,
  ): Promise<{ ok: true; snapshot: SnapshotExport } | { ok: false; reason: SnapshotExportFailureReason }> => {
    const editor = getEditor();
    const rootService = getRootService();
    if (!editor || !rootService) return { ok: false, reason: "unavailable" };
    await reportSnapshotProgress(onProgress, "Resolving selected BlockSuite nodes...");
    const { elements: selected, gfx } = getSelectedCanvasElements(editor, rootService, elementIds);
    if (selected.length === 0) return { ok: false, reason: "empty" };
    const shell = canvasShellRef.current;
    const overlayRoots: HTMLElement[] = [];
    if (shell?.isConnected) overlayRoots.push(shell);
    const selectionRect = getSelectionClientRect(editor, gfx, selected);
    console.info("BlockSuite snapshot selection resolved.", {
      requestedElementIds: elementIds ?? null,
      selectedIds: selected.map((item) => item.id),
      hasShell: Boolean(shell),
      hasSelectionRect: Boolean(selectionRect),
      overlayCount: overlayRoots.reduce(
        (count, root) => count + root.querySelectorAll("[data-sessio-overlay-block-id]").length,
        0,
      ),
      hasOverlayMount: false,
    });
    if (!shell || !selectionRect) {
      console.warn("BlockSuite DOM snapshot skipped because selection bounds were unavailable.", {
        hasShell: Boolean(shell),
        hasSelectionRect: Boolean(selectionRect),
        selectedIds: selected.map((item) => item.id),
      });
      await reportSnapshotProgress(onProgress, "Could not locate the selected nodes on screen.");
      return { ok: false, reason: "render-failed" };
    }
    const visibleSelectionRect = getVisibleSelectionRect(overlayRoots, selectionRect, selected);
    const captured = await withSnapshotCaptureMode(shell, async () => {
      await reportSnapshotProgress(onProgress, "Capturing visible selection pixels...");
      const nativeSnapshot = await captureNativeWindowSnapshot(shell, visibleSelectionRect);
      if (nativeSnapshot) return { kind: "native" as const, snapshot: nativeSnapshot };
      await reportSnapshotProgress(onProgress, "Trying DOM snapshot fallback...");
      const domCanvas = await withTimeout(
        renderCanvasDomSnapshot(shell, visibleSelectionRect),
        5000,
        "Visible canvas DOM snapshot",
      ).catch((error) => {
        console.warn("BlockSuite DOM snapshot failed.", error);
        return null;
      });
      const nextCanvas = domCanvas && !isCanvasVisuallyEmpty(domCanvas) ? domCanvas : null;
      return { kind: "canvas" as const, canvas: nextCanvas };
    });
    if (captured.kind === "native") {
      await reportSnapshotProgress(onProgress, "Saved snapshot PNG.");
      return { ok: true, snapshot: captured.snapshot };
    }
    const canvas = captured.canvas;
    if (!canvas) return { ok: false, reason: "render-failed" };
    if (isCanvasVisuallyEmpty(canvas)) {
      console.warn("BlockSuite DOM snapshot was visually empty.", {
        selectedIds: selected.map((item) => item.id),
        selectionRect: {
          left: visibleSelectionRect.left,
          top: visibleSelectionRect.top,
          width: visibleSelectionRect.width,
          height: visibleSelectionRect.height,
        },
      });
      await reportSnapshotProgress(onProgress, "Visible selection snapshot was empty.");
      return { ok: false, reason: "render-failed" };
    }
    await reportSnapshotProgress(onProgress, "Encoding snapshot PNG...");
    const blob = await withTimeout(
      new Promise<Blob | null>((resolve) => {
        canvas.toBlob((next: Blob | null) => resolve(next), "image/png");
      }),
      2000,
      "Snapshot PNG encoding",
    ).catch((error) => {
      console.warn("BlockSuite snapshot PNG encoding failed.", error);
      return null;
    });
    if (!blob) return { ok: false, reason: "blob-failed" };
    await reportSnapshotProgress(onProgress, "Saving snapshot PNG...");
    const path = await withTimeout(
      saveBlobAsAttachment(blob, `blocksuite-selection-${Date.now()}.png`),
      3000,
      "Snapshot PNG save",
    )
      .catch((error) => {
        console.warn("BlockSuite snapshot PNG save failed.", error);
        return null;
      });
    if (!path) return { ok: false, reason: "save-failed" };
    const snapshot = await snapshotExportFromPath(path);
    return { ok: true, snapshot };
  }, [getEditor, getRootService]);

  const attachSelectionSnapshot = useCallback(async (elementIds?: string[] | null) => {
    setBridgeBusy("snapshot");
    try {
      const snapshotResult = await exportSelectionSnapshot(elementIds);
      if (!snapshotResult.ok) {
        showSnapshotToast(snapshotExportFailureMessage(snapshotResult.reason), "error");
        return;
      }
      if (composer.supportsImageAttachments) {
        await composer.appendAttachments([snapshotResult.snapshot.attachment]);
        showSnapshotToast(`Snapshot captured: ${snapshotResult.snapshot.path}`);
      } else {
        showSnapshotToast(`Snapshot saved locally: ${snapshotResult.snapshot.path}`);
      }
    } catch (error) {
      console.warn("Failed to attach selection snapshot.", error);
      showSnapshotToast("Failed to capture snapshot.", "error");
    } finally {
      setBridgeBusy(null);
    }
  }, [composer, exportSelectionSnapshot, showSnapshotToast]);

  useEffect(() => {
    const requestId = selectedFileRequest?.requestId ?? null;
    if (!selectedFileRequest || requestId === null || !isReady) return;
    const requestKey = `${sessionId}:${requestId}`;
    if (handledFileRequestRef.current === requestKey) return;
    void addFileCards(selectedFileRequest.paths).then((added) => {
      if (added) {
        handledFileRequestRef.current = requestKey;
      }
    });
  }, [addFileCards, isReady, selectedFileRequest, sessionId]);

  useEffect(() => {
    const handleCanvasAddFiles = (event: Event) => {
      const detail = (event as CustomEvent<{ paths?: string[]; sessionId?: string | null }>).detail;
      if (!detail || detail.sessionId !== sessionId || !Array.isArray(detail.paths)) return;
      void addFileCards(detail.paths);
    };
    window.addEventListener(CANVAS_ADD_FILES_EVENT, handleCanvasAddFiles);
    return () => window.removeEventListener(CANVAS_ADD_FILES_EVENT, handleCanvasAddFiles);
  }, [addFileCards, sessionId]);

  useEffect(() => {
    if (!isReady) return;
    const handleGroupSelection = () => {
      groupSelection();
    };
    const handleUngroupSelection = () => {
      ungroupSelection();
    };
    window.addEventListener(CANVAS_GROUP_SELECTION_EVENT, handleGroupSelection);
    window.addEventListener(CANVAS_UNGROUP_SELECTION_EVENT, handleUngroupSelection);
    return () => {
      window.removeEventListener(CANVAS_GROUP_SELECTION_EVENT, handleGroupSelection);
      window.removeEventListener(CANVAS_UNGROUP_SELECTION_EVENT, handleUngroupSelection);
    };
  }, [groupSelection, isReady, ungroupSelection]);

  useEffect(() => {
    if (!isReady) return;
    let lastHandledAt = 0;
    const handleCanvasSnapshotSelection = (event: Event) => {
      const now = Date.now();
      if (now - lastHandledAt < 250) return;
      lastHandledAt = now;
      event.stopPropagation();
      const detail = (event as CustomEvent<CanvasSnapshotSelectionEventDetail>).detail;
      void attachSelectionSnapshot(detail?.elementIds ?? null);
    };
    const shell = canvasShellRef.current;
    shell?.addEventListener(CANVAS_SNAPSHOT_SELECTION_EVENT, handleCanvasSnapshotSelection);
    window.addEventListener(CANVAS_SNAPSHOT_SELECTION_EVENT, handleCanvasSnapshotSelection);
    return () => {
      shell?.removeEventListener(CANVAS_SNAPSHOT_SELECTION_EVENT, handleCanvasSnapshotSelection);
      window.removeEventListener(CANVAS_SNAPSHOT_SELECTION_EVENT, handleCanvasSnapshotSelection);
    };
  }, [attachSelectionSnapshot, isReady]);

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col">
      <div className="relative flex h-9 shrink-0 items-center gap-3 bg-surface-panel/90 px-4 text-caption text-ink/50 after:pointer-events-none after:absolute after:inset-x-4 after:bottom-0 after:h-px after:bg-gradient-to-r after:from-transparent after:via-ink/[0.1] after:to-transparent">
        <button
          ref={addMenuButtonRef}
          type="button"
          onClick={() => setAddMenuOpen((value) => !value)}
          className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-3 text-ink/70 transition hover:bg-ink/5"
        >
          <Layers3 className="h-3.5 w-3.5" />
          Add to canvas
        </button>
        <button
          ref={editedFilesButtonRef}
          type="button"
          onClick={openEditedFilesPicker}
          disabled={availableEditedFiles.length === 0}
          className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-3 text-ink/70 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
        >
          <FilePlus2 className="h-3.5 w-3.5" />
          Add edited files
        </button>
        <button
          type="button"
          onClick={() => void handleSaveRevision()}
          disabled={!isReady || isSaving}
          className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-2.5 text-ink/70 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {isReady ? <Save className="h-3.5 w-3.5" /> : <LoaderCircle className="h-3.5 w-3.5 animate-spin" />}
          Save
        </button>
      </div>
      <div
        className="sr-only"
        aria-live="polite"
        data-bridge-busy={bridgeBusy ?? "idle"}
        data-can-ungroup={canUngroupSelection ? "true" : "false"}
        data-selection-count={selectionCount}
      >
        {saveError ?? status}
      </div>
      {saveError && (
        <div className="mx-4 mt-3 rounded-xl border border-status-error/20 bg-status-error/8 px-3 py-2 text-caption text-status-error">
          <div>
            {saveError}
            {lastSavedRevision !== null ? ` Last saved revision: ${lastSavedRevision}.` : ""}
          </div>
        </div>
      )}
      <div ref={canvasShellRef} className="relative min-h-0 flex-1">
        <div ref={hostRef} className={`${BLOCKSUITE_STYLE_SCOPE_CLASS} absolute inset-0`} />
      </div>
      <ToastStack
        message={snapshotToast}
        durationMs={3600}
        onMessageConsumed={() => setSnapshotToast(null)}
      />
      {addMenuOpen && addMenuButtonRef.current && (
        <PopupMenu
          anchor={addMenuButtonRef.current}
          options={addMenuOptions}
          placement="bottom-start"
          className="z-[1010]"
          overlayClassName="z-[1009]"
          onSelect={(key) => void handleAddMenuSelect(key)}
          onClose={() => setAddMenuOpen(false)}
        />
      )}
      {editedFilesPickerOpen && editedFilesButtonRef.current && (
        <EditedFilesPopover
          anchor={editedFilesButtonRef.current}
          files={availableEditedFiles}
          selectedFiles={pendingEditedFiles}
          onToggleFile={togglePendingEditedFile}
          onAdd={addPendingEditedFiles}
          onClose={closeEditedFilesPicker}
        />
      )}
      <PortalHost portals={portals} />
    </div>
  );
}

function normalizePathSegment(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+$/, "");
}

function removePlaceholderNotes(doc: ReturnType<typeof createBlockSuiteDoc>["doc"]) {
  const noteBlocks = doc.getBlocksByFlavour("affine:note");
  for (const block of noteBlocks) {
    if (!isPlaceholderNoteBlock(block.model as { children?: Array<{ text?: { toString(): string } | null; children?: unknown[] }> })) {
      continue;
    }
    doc.deleteBlock(block.model);
  }
}

function isPlaceholderNoteBlock(
  model: { children?: Array<{ text?: { toString(): string } | null; caption?: string | null; children?: unknown[] }> } | null,
) {
  if (!model) return false;
  const chunks: string[] = [];
  collectNoteTextChunks(model.children ?? [], chunks);
  return chunks.join("\n").trim().length === 0;
}

function collectNoteTextChunks(
  children: Array<{ text?: { toString(): string } | null; caption?: string | null; children?: unknown[] }>,
  chunks: string[],
) {
  for (const child of children) {
    const direct = child.text?.toString().trim();
    if (direct) {
      chunks.push(direct);
    } else if (typeof child.caption === "string" && child.caption.trim()) {
      chunks.push(child.caption.trim());
    }
    if (Array.isArray(child.children)) {
      collectNoteTextChunks(
        child.children as Array<{ text?: { toString(): string } | null; caption?: string | null; children?: unknown[] }>,
        chunks,
      );
    }
  }
}

function focusBlocksInViewport(
  rootService: EdgelessRootService,
  doc: ReturnType<typeof createBlockSuiteDoc>["doc"],
  blockIds: string[],
) {
  const bounds = blockIds
    .map((blockId) => doc.getModelById(blockId) as { xywh?: string } | null)
    .map((model) => (model?.xywh ? Bound.deserialize(model.xywh) : null))
    .filter((bound): bound is Bound => Boolean(bound));
  if (bounds.length === 0) return;
  const commonBound = bounds.reduce((current, next) => current.unite(next));
  rootService.selection.set({
    editing: false,
    elements: blockIds,
  });
  rootService.viewport.setViewportByBound(commonBound, [96, 96, 96, 96], true);
}

function summarizeText(content: string, maxLength: number): string {
  return content
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .slice(0, 6)
    .join(" ")
    .slice(0, maxLength);
}

function resolveCanvasFilePath(path: string, workspacePath: string | null): string | null {
  if (!path) return null;
  if (/^([a-zA-Z]:[\\/]|\/)/.test(path)) return path;
  if (!workspacePath) return null;
  const separator = workspacePath.includes("\\") ? "\\" : "/";
  const trimmedRoot = workspacePath.replace(/[\\/]+$/, "");
  const trimmedPath = path.replace(/^[\\/]+/, "");
  return `${trimmedRoot}${separator}${trimmedPath}`;
}

async function saveBlobAsAttachment(blob: Blob, fileName: string): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = "";
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunkSize));
  }
  const { path } = await savePastedAttachment({
    fileName,
    mimeType: blob.type || "image/png",
    dataBase64: btoa(binary),
  });
  return path;
}

function EditedFilesPopover({
  anchor,
  files,
  selectedFiles,
  onToggleFile,
  onAdd,
  onClose,
}: {
  anchor: HTMLElement;
  files: string[];
  selectedFiles: string[];
  onToggleFile: (path: string) => void;
  onAdd: () => void;
  onClose: () => void;
}) {
  const popoverRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{
    top: number;
    left: number;
    width: number;
    maxHeight: number;
  } | null>(null);

  useLayoutEffect(() => {
    const update = () => {
      const rect = anchor.getBoundingClientRect();
      const gap = 6;
      const margin = 8;
      const width = Math.min(420, Math.max(320, rect.width + 120));
      const top = rect.bottom + gap;
      const maxHeight = Math.max(200, window.innerHeight - top - margin);
      const left = Math.min(
        Math.max(margin, rect.left),
        window.innerWidth - margin - width,
      );
      setPos({
        top,
        left,
        width,
        maxHeight,
      });
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [anchor]);

  useEffect(() => {
    const onPointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (!target) return;
      if (popoverRef.current?.contains(target)) return;
      if (anchor.contains(target)) return;
      onClose();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [anchor, onClose]);

  if (!pos) return null;

  return createPortal(
    <div
      ref={popoverRef}
      className="fixed z-[1012] overflow-hidden rounded-xl border border-ink/10 bg-surface-panel shadow-[0_20px_60px_rgba(0,0,0,0.22)]"
      style={{
        top: pos.top,
        left: pos.left,
        width: pos.width,
      }}
    >
      <div className="flex items-center justify-between gap-3 border-b border-ink/8 px-3 py-2.5">
        <div className="min-w-0">
          <div className="text-body-sm font-medium text-ink/82">Edited files</div>
          <div className="text-caption text-ink/48">
            Select multiple files to add to canvas
          </div>
        </div>
        <button
          type="button"
          onClick={onClose}
          className="inline-flex h-7 w-7 items-center justify-center rounded-md text-ink/45 transition hover:bg-ink/5 hover:text-ink/72"
          aria-label="Close edited files picker"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
      <ScrollArea
        className="overscroll-contain"
        style={{ maxHeight: Math.min(pos.maxHeight, 320) }}
        viewportClassName="py-1"
        persistScrollbars
      >
        <ul className="flex flex-col px-1.5 py-1">
          {files.map((path) => {
            const checked = selectedFiles.includes(path);
            const fileName = path.split(/[/\\]/).pop() ?? path;
            return (
              <li key={path}>
                <button
                  type="button"
                  onClick={() => onToggleFile(path)}
                  className="flex w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-left transition hover:bg-ink/[0.05]"
                >
                  <span className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded border border-ink/15 bg-ink/[0.04]">
                    {checked && <Check className="h-3 w-3 text-ink/72" />}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-body-sm text-ink/82">{fileName}</span>
                    <span className="mt-0.5 block break-all font-mono text-caption text-ink/45">{path}</span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </ScrollArea>
      <div className="flex items-center justify-between gap-3 border-t border-ink/8 px-3 py-2.5">
        <div className="text-caption text-ink/48">
          {selectedFiles.length} selected
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={onClose}
            className="inline-flex items-center gap-1.5 rounded-md border border-ink/10 px-3 py-1.5 text-caption text-ink/58 transition hover:bg-ink/[0.05]"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onAdd}
            disabled={selectedFiles.length === 0}
            className="inline-flex items-center gap-1.5 rounded-md border border-ink/10 px-3 py-1.5 text-caption text-ink/72 transition hover:bg-ink/[0.05] disabled:cursor-not-allowed disabled:opacity-40"
          >
            <FilePlus2 className="h-3.5 w-3.5" />
            Add to canvas
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
