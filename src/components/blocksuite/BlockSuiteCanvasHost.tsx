import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";
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
import { Camera, Check, FileImage, FilePlus2, FolderOpen, Layers3, LoaderCircle, MessageCircleQuestionMark, RefreshCcw, Save, StickyNote, Workflow, X } from "lucide-react";
import type { ComposerAttachment } from "../ComposerAttachments";
import PopupMenu, { type PopupMenuOption } from "../PopupMenu";
import ScrollArea from "../ScrollArea";
import type {
  CanvasBlockKind,
  CanvasBlockRecord,
  CanvasContextOption,
  CanvasDocumentState,
} from "../../canvasTypes";
import {
  captureWindowAreaPng,
  createAstraRun,
  createCanvasAnchor,
  createCanvasContextFile,
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
import {
  CanvasCustomBlockOverlay,
  type CanvasCustomBlockOverlayItem,
} from "./CanvasCustomBlockOverlay";
import ToastStack, { type ToastStackMessage } from "../ToastStack";
import { setBlockSuitePortalBridge } from "../../lib/blocksuite/portalBridge";
import {
  canvasBlockRecordToContextRef,
  canvasInteropModelToCanvasBlock,
  renderSelectionSummaryMarkdown,
  renderWorkflowSummaryMarkdown,
  surfaceElementToCanvasBlock,
  tryParseJson,
  workflowSnapshotToMarkdown,
} from "../../lib/blocksuite/persistence";
import type { MarkdownPreviewBlockModel } from "../../lib/blocksuite/blocks/markdown-preview";
import type { FileCardBlockModel } from "../../lib/blocksuite/blocks/file-card";
import type { WorkflowCardBlockModel } from "../../lib/blocksuite/blocks/workflow-card";
import {
  CANVAS_SNAPSHOT_SELECTION_EVENT,
  type CanvasSnapshotSelectionEventDetail,
} from "../../lib/blocksuite/toolbar";

const CANVAS_ADD_FILES_EVENT = "sessio:canvas-add-files";
const AUTOSAVE_DEBOUNCE_MS = 900;
const ROOT_SERVICE_RETRY_MS = 80;
const ROOT_SERVICE_RETRY_LIMIT = 125;
const BOX_SELECTING_CLASS_NAME = "sessio-box-selecting";
const SNAPSHOT_CAPTURING_CLASS_NAME = "sessio-snapshot-capturing";

type CanvasSelectionRef = {
  blockId: string;
  title: string;
  sourcePath: string | null;
  blockKind: CanvasBlockKind;
  meta: Record<string, unknown> | null;
};

type CanvasSelectionContext = {
  refs: CanvasSelectionRef[];
  elementIds: string[];
};

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
  return editor?.std?.get?.<GfxControllerLike>(GfxControllerIdentifier) ?? null;
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
  try {
    await waitForNextFrame();
    await waitForNextFrame();
    return await task();
  } finally {
    shell.classList.remove(SNAPSHOT_CAPTURING_CLASS_NAME);
    document.documentElement.classList.remove(SNAPSHOT_CAPTURING_CLASS_NAME);
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
) {
  const std = editor.std;
  if (!std) {
    throw new Error("Canvas editor context is not ready.");
  }

  const gfx = std.get?.<GfxControllerLike>(GfxControllerIdentifier);
  const crud = rootService.crud;
  if (!gfx || !crud) {
    throw new Error("Canvas drawing services are not ready.");
  }

  const center = rootService.viewport.center;
  const [viewX, viewY] = gfx.viewport.toViewCoord(center.x, center.y);
  const [modelX, modelY] = gfx.viewport.toModelCoord(viewX, viewY);

  return crud.addBlock(
    "affine:note",
    {
      xywh: serializeXYWH(
        modelX - DEFAULT_NOTE_WIDTH / 2,
        modelY - DEFAULT_NOTE_HEIGHT / 2,
        DEFAULT_NOTE_WIDTH,
        DEFAULT_NOTE_HEIGHT,
      ),
      displayMode: NoteDisplayMode.EdgelessOnly,
    },
    rootService.surface.id,
  );
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
  const blockRecordsRef = useRef(initialState.blockRecords);
  const blockUpdatedDisposeRef = useRef<{ dispose: () => void } | null>(null);
  const selectionUpdatedDisposeRef = useRef<{ dispose: () => void } | null>(null);
  const selectionUiFrameRef = useRef<number | null>(null);
  const selectionUiStateRef = useRef({
    count: 0,
    canUngroup: false,
  });
  const autosaveTimerRef = useRef<number | null>(null);
  const overlaySyncFrameRef = useRef<number | null>(null);
  const overlaySyncDeferredRef = useRef(false);
  const selectionUiDeferredRef = useRef(false);
  const boxSelectingObserverRef = useRef<MutationObserver | null>(null);
  const boxSelectingHostRef = useRef<HTMLElement | null>(null);
  const isBoxSelectingRef = useRef(false);
  const inflightSaveRef = useRef(false);
  const queuedSnapshotRef = useRef<string | null>(null);
  const currentSnapshotRef = useRef(initialSnapshot);
  const handledFileRequestRef = useRef<string | null>(null);
  const addMenuButtonRef = useRef<HTMLButtonElement>(null);
  const editedFilesButtonRef = useRef<HTMLButtonElement>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [status, setStatus] = useState("Initializing BlockSuite canvas…");
  const [snapshotToast, setSnapshotToast] = useState<ToastStackMessage | null>(null);
  const [isReady, setIsReady] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [selectionCount, setSelectionCount] = useState(0);
  const [canUngroupSelection, setCanUngroupSelection] = useState(false);
  const [blockRecords, setBlockRecords] = useState(initialState.blockRecords);
  const [bridgeBusy, setBridgeBusy] = useState<null | "ask" | "snapshot" | "workflow">(null);
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  const [editedFilesPickerOpen, setEditedFilesPickerOpen] = useState(false);
  const [pendingEditedFiles, setPendingEditedFiles] = useState<string[]>([]);
  const [customOverlayItems, setCustomOverlayItems] = useState<CanvasCustomBlockOverlayItem[]>([]);
  const [overlayMountElement, setOverlayMountElement] = useState<HTMLDivElement | null>(null);

  const changedFiles = useMemo(
    () => Array.from(new Set(editedFiles.map((path) => path.trim()).filter(Boolean))),
    [editedFiles],
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
    blockRecordsRef.current = initialState.blockRecords;
    setBlockRecords(initialState.blockRecords);
  }, [initialState]);

  const getEditor = useCallback(() => editorRef.current, []);
  const getDoc = useCallback(() => docRef.current, []);

  const getRootService = useCallback((): EdgelessRootService | null => {
    try {
      return getEditor()?.std?.get?.<EdgelessRootService>(EdgelessRootService) ?? null;
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
    overlaySyncDeferredRef.current = false;
    selectionUiDeferredRef.current = false;
  }, []);

  const syncCustomBlockOverlay = useCallback(() => {
    const host = hostRef.current;
    const editor = getEditor();
    const rootService = getRootService();
    const doc = getDoc();
    if (!host || !editor || !rootService || !doc) {
      setCustomOverlayItems((current) => (current.length === 0 ? current : []));
      return;
    }

    const hostRect = host.getBoundingClientRect();
    const selectedIds = new Set(getSelectedCanvasElements(editor, rootService).ids);
    const overlays: CanvasCustomBlockOverlayItem[] = [];
    const entries = [
      ...doc.getBlocksByFlavour("sessio:file-card"),
      ...doc.getBlocksByFlavour("sessio:markdown-preview"),
      ...doc.getBlocksByFlavour("sessio:workflow-card"),
    ];

    for (const entry of entries) {
      const element = editor.std?.host?.view?.getBlock?.(entry.model.id) as HTMLElement | null;
      if (!element?.isConnected) {
        continue;
      }
      const rect = element.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) {
        continue;
      }

      const transform = window.getComputedStyle(element).transform;
      const matrix = transform && transform !== "none" ? new DOMMatrixReadOnly(transform) : null;
      const scaleX = matrix ? Math.hypot(matrix.a, matrix.b) : 1;
      const scaleY = matrix ? Math.hypot(matrix.c, matrix.d) : 1;
      const scale = scaleX || scaleY ? (scaleX + scaleY) / (scaleX && scaleY ? 2 : 1) : 1;
      const geometryModel = entry.model as { xywh?: string };
      const bound = geometryModel.xywh ? Bound.deserialize(geometryModel.xywh) : null;
      const baseWidth = bound ? bound.w : rect.width / scale;
      const baseHeight = bound ? bound.h : rect.height / scale;
      const base = {
        blockId: entry.model.id,
        left: rect.left - hostRect.left,
        top: rect.top - hostRect.top,
        baseWidth,
        baseHeight,
        scale,
        selected: selectedIds.has(entry.model.id),
      };

      if (entry.model.flavour === "sessio:file-card") {
        const fileCardModel = entry.model as FileCardBlockModel;
        overlays.push({
          ...base,
          kind: "file_card",
          title: fileCardModel.title || "File card",
          sourcePath: fileCardModel.sourcePath || "",
          sourceType: fileCardModel.sourceType || "workspace_file",
          subtitle: fileCardModel.subtitle || "",
          summary: fileCardModel.summary || "",
          status: fileCardModel.status || "idle",
        });
        continue;
      }

      if (entry.model.flavour === "sessio:workflow-card") {
        const workflowCardModel = entry.model as WorkflowCardBlockModel;
        overlays.push({
          ...base,
          kind: "workflow_card",
          title: workflowCardModel.title || "Workflow",
          threadId: workflowCardModel.threadId || "",
          threadStageId: workflowCardModel.threadStageId || "",
          executionState: workflowCardModel.executionState || "idle",
          lastRunId: workflowCardModel.lastRunId || "",
          threadGoal: workflowCardModel.threadGoal || "",
          workflowSummaryMarkdown: workflowCardModel.workflowSummaryMarkdown || "",
        });
        continue;
      }

      const markdownPreviewModel = entry.model as MarkdownPreviewBlockModel;
      overlays.push({
        ...base,
        kind: "markdown_preview",
        title: markdownPreviewModel.title || "Markdown preview",
        sourcePath: markdownPreviewModel.sourcePath || "",
        excerpt: markdownPreviewModel.excerpt || "",
        contentVersion: markdownPreviewModel.contentVersion || "",
        renderMode:
          !markdownPreviewModel.collapsed &&
          markdownPreviewModel.renderMode === "preview"
            ? "preview"
            : "summary",
        workspacePath,
      });
    }

    setCustomOverlayItems(overlays);
  }, [getDoc, getEditor, getRootService, workspacePath]);

  const scheduleCustomBlockOverlaySync = useCallback((force = false) => {
    if (isBoxSelectingRef.current && !force) {
      overlaySyncDeferredRef.current = true;
      return;
    }
    if (overlaySyncFrameRef.current !== null) {
      return;
    }
    overlaySyncFrameRef.current = window.requestAnimationFrame(() => {
      overlaySyncFrameRef.current = null;
      if (isBoxSelectingRef.current && !force) {
        overlaySyncDeferredRef.current = true;
        return;
      }
      syncCustomBlockOverlay();
    });
  }, [syncCustomBlockOverlay]);

  const flushDeferredCanvasUiSync = useCallback(() => {
    if (selectionUiDeferredRef.current) {
      selectionUiDeferredRef.current = false;
      scheduleSelectionStateUpdate();
    }
    if (overlaySyncDeferredRef.current) {
      overlaySyncDeferredRef.current = false;
      scheduleCustomBlockOverlaySync(true);
    }
  }, [scheduleCustomBlockOverlaySync, scheduleSelectionStateUpdate]);

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
        if (overlaySyncFrameRef.current !== null) {
          window.cancelAnimationFrame(overlaySyncFrameRef.current);
          overlaySyncFrameRef.current = null;
        }
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
    const nextBlocks: CanvasBlockRecord["metadataJson"][] = [];
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

    void nextBlocks;
    try {
      const saved = await updateCanvasBlocks({
        sessionId,
        blocks: [...records, ...surfaceRecords],
      });
      blockRecordsRef.current = saved;
      setBlockRecords(saved);
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
      const subscription = doc.slots.blockUpdated.subscribe(() => {
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
            overlaySyncDeferredRef.current = true;
            return;
          }
          scheduleSelectionStateUpdate();
          scheduleCustomBlockOverlaySync();
        });
        selectionUpdatedDisposeRef.current = {
          dispose: () => subscription.unsubscribe(),
        };
        updateSelectionState();
        scheduleCustomBlockOverlaySync();
      });
      scheduleSyncBlocks();
      scheduleCustomBlockOverlaySync();
    });
  }, [attachBoxSelectingObserver, clearBoxSelectingObserver, scheduleAutosave, scheduleCustomBlockOverlaySync, scheduleSelectionStateUpdate, scheduleSyncBlocks, updateSelectionState, waitForRootService]);

  const openEditedFilesPicker = () => {
    if (changedFiles.length === 0) return;
    setPendingEditedFiles((current) => (
      current.length > 0
        ? current.filter((path) => changedFiles.includes(path))
        : changedFiles
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
    const uniquePaths = Array.from(new Set(paths.map((path) => path.trim()).filter(Boolean)));
    const center = rootService.viewport.center;
    let addedCount = 0;
    const insertedBlockIds: string[] = [];
    for (const [index, path] of uniquePaths.entries()) {
      try {
        const absolutePath = resolveCanvasFilePath(path, workspacePath);
        if (!absolutePath) throw new Error("file path is unavailable");
        const file = await readWorkspaceTextFile(workspacePath, absolutePath).catch(() => null);
        const title = absolutePath.split(/[/\\]/).pop() ?? absolutePath;
        const bound = Bound.fromCenter(
          [center.x + (index % 3) * 360, center.y + Math.floor(index / 3) * 156],
          340,
          144,
        );
        const blockId = insertEdgelessBlock(
          "sessio:file-card",
          {
            title,
            sourcePath: absolutePath,
            sourceType: resolveFileSourceType(path),
            subtitle: absolutePath,
            summary: summarizeText(file?.content ?? "", 260),
            status: file ? "ready" : "unavailable",
            contentVersion: file ? `${absolutePath}:${file.mtimeMs}` : absolutePath,
            xywh: bound.serialize(),
          },
        );
        insertedBlockIds.push(blockId);
        addedCount += 1;
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        onError(`Failed to add file card for ${path}: ${message}`);
      }
    }
    if (addedCount === 0) {
      return false;
    }
    focusBlocksInViewport(rootService, doc, insertedBlockIds);
    const snapshotJson = snapshotToJson(doc);
    if (!snapshotJson) {
      onError("Canvas snapshot export failed after adding file cards.");
      return false;
    }
    await flushSaveRef.current(snapshotJson);
    await syncCanvasBlocks(doc);
    setStatus(`Added ${addedCount} file card${addedCount === 1 ? "" : "s"} to canvas.`);
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
    const noteId = addEdgelessNote(rootService, editor);
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
    if (!editor) return;
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
      const files = await Promise.all(paths.map(async (path) => {
        const response = await fetch(convertFileSrc(path));
        const blob = await response.blob();
        return new File([blob], path.split(/[/\\]/).pop() ?? "image", {
          type: blob.type || "image/png",
        });
      }));
      await addImages(editor.std as never, files, {});
      scheduleSyncBlocks();
    } catch (error) {
      onError(`Failed to add image node: ${String(error)}`);
    }
  }, [getEditor, onError, scheduleSyncBlocks]);

  const addWorkflowCard = useCallback(async () => {
    const doc = getDoc();
    const rootService = getRootService();
    if (!doc || !rootService) return;
    const snapshotResult = await getThreadWorkSnapshot(sessionAgent, sessionId).catch(() => null);
    const snapshot = snapshotResult?.snapshot ?? null;
    const title = snapshot?.goal?.trim() || "Workflow";
    const summaryMarkdown = workflowSnapshotToMarkdown(snapshot);
    const center = rootService.viewport.center;
    const bound = Bound.fromCenter([center.x, center.y], 360, 196);
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

  const promoteFileCardToMarkdown = useCallback((blockId: string) => {
    const doc = getDoc();
    const rootService = getRootService();
    if (!doc || !rootService) return;
    const model = doc.getModelById(blockId) as FileCardBlockModel | null;
    if (!model) return;
    const bound = Bound.deserialize(model.xywh);
    const nextWidth = Math.max(bound.w, 420);
    const nextHeight = Math.max(bound.h + 96, 260);
    insertEdgelessBlock(
      "sessio:markdown-preview",
      {
        title: model.title || "Markdown preview",
        sourcePath: model.sourcePath || "",
        sourceType: model.sourceType || "workspace_file",
        excerpt: model.summary || "",
        renderMode: "preview",
        collapsed: false,
        contentVersion: model.contentVersion || model.sourcePath || "",
        cachedContent: "",
        xywh: Bound.fromCenter([bound.center[0] + 28, bound.center[1] + 28], nextWidth, nextHeight).serialize(),
      },
    );
    rootService.removeElement(blockId);
    scheduleSyncBlocks();
    scheduleCustomBlockOverlaySync();
    updateSelectionState();
  }, [
    getDoc,
    getRootService,
    insertEdgelessBlock,
    scheduleCustomBlockOverlaySync,
    scheduleSyncBlocks,
    updateSelectionState,
  ]);

  const dragMarkdownPreviewFromHeader = useCallback((
    blockId: string,
    event: ReactPointerEvent<HTMLDivElement>,
  ) => {
    if (event.button !== 0) return;

    const doc = getDoc();
    const rootService = getRootService();
    if (!doc || !rootService) return;

    const model = doc.getModelById(blockId) as MarkdownPreviewBlockModel | null;
    if (!model?.xywh) return;

    const startBound = Bound.deserialize(model.xywh);
    const startClientX = event.clientX;
    const startClientY = event.clientY;
    const startZoom = rootService.viewport.zoom || 1;
    let moved = false;

    rootService.selection.set({
      editing: false,
      elements: [blockId],
    });
    scheduleSelectionStateUpdate();
    event.preventDefault();
    event.stopPropagation();

    const onPointerMove = (moveEvent: PointerEvent) => {
      const dx = (moveEvent.clientX - startClientX) / startZoom;
      const dy = (moveEvent.clientY - startClientY) / startZoom;
      if (!moved && Math.abs(dx) < 1 && Math.abs(dy) < 1) {
        return;
      }
      moved = true;
      doc.updateBlock(model, {
        xywh: serializeXYWH(
          startBound.x + dx,
          startBound.y + dy,
          startBound.w,
          startBound.h,
        ),
      });
      scheduleCustomBlockOverlaySync();
    };

    const finishDrag = () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp, true);
      window.removeEventListener("pointercancel", onPointerUp, true);
      if (moved) {
        scheduleSyncBlocks();
      }
      scheduleCustomBlockOverlaySync();
      scheduleSelectionStateUpdate();
    };

    const onPointerUp = () => {
      finishDrag();
    };

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp, true);
    window.addEventListener("pointercancel", onPointerUp, true);
  }, [
    getDoc,
    getRootService,
    scheduleCustomBlockOverlaySync,
    scheduleSelectionStateUpdate,
    scheduleSyncBlocks,
  ]);

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
    setBlockSuitePortalBridge(null);
    return () => {
      setBlockSuitePortalBridge(null);
    };
  }, []);

  useEffect(() => {
    currentSnapshotRef.current = initialSnapshot;
  }, [initialSnapshot, sessionId]);

  useEffect(() => {
    return () => {
      if (autosaveTimerRef.current !== null) {
        window.clearTimeout(autosaveTimerRef.current);
      }
      if (overlaySyncFrameRef.current !== null) {
        window.cancelAnimationFrame(overlaySyncFrameRef.current);
        overlaySyncFrameRef.current = null;
      }
      if (selectionUiFrameRef.current !== null) {
        window.cancelAnimationFrame(selectionUiFrameRef.current);
        selectionUiFrameRef.current = null;
      }
      clearBoxSelectingObserver();
    };
  }, [clearBoxSelectingObserver]);

  useEffect(() => {
    if (!isReady) {
      setCustomOverlayItems((current) => (current.length === 0 ? current : []));
      return;
    }

    scheduleCustomBlockOverlaySync();

    const editor = getEditor();
    const rootService = getRootService();
    if (!editor || !rootService) {
      return;
    }

    const onWindowUpdate = () => {
      scheduleCustomBlockOverlaySync();
    };

    window.addEventListener("resize", onWindowUpdate);
    window.addEventListener("scroll", onWindowUpdate, true);

    const viewportSubscription = rootService.viewport.viewportUpdated?.subscribe(() => {
      scheduleCustomBlockOverlaySync();
    });

    const doc = getDoc();
    const blockSubscription = doc?.slots.blockUpdated.subscribe(() => {
      scheduleCustomBlockOverlaySync();
    });

    return () => {
      window.removeEventListener("resize", onWindowUpdate);
      window.removeEventListener("scroll", onWindowUpdate, true);
      viewportSubscription?.unsubscribe();
      blockSubscription?.unsubscribe();
    };
  }, [getDoc, getEditor, getRootService, isReady, scheduleCustomBlockOverlaySync]);

  useLayoutEffect(() => {
    const editor = editorRef.current;
    if (!editor) {
      setOverlayMountElement(null);
      return;
    }

    const syncOverlayMount = () => {
      const mountPoint = editor
        .querySelector("affine-edgeless-root .edgeless-mount-point") as HTMLDivElement | null;
      if (mountPoint) {
        mountPoint.style.position = "absolute";
        mountPoint.style.inset = "0";
        mountPoint.style.pointerEvents = "auto";
      }
      setOverlayMountElement(mountPoint);
    };

    syncOverlayMount();
    const frameId = window.requestAnimationFrame(syncOverlayMount);

    return () => {
      window.cancelAnimationFrame(frameId);
      setOverlayMountElement((current) => (current?.isConnected ? current : null));
    };
  }, [isReady, initialState.document.id, initialSnapshot]);

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

  const handleRestoreSavedRevision = async () => {
    const savedSnapshot = initialState.savedSnapshot;
    if (!savedSnapshot || !hostRef.current) return;
    try {
      setIsReady(false);
      setStatus("Restoring the last saved revision…");
      const snapshot = JSON.parse(savedSnapshot);
      const restored = await importDocSnapshot(snapshot);
      if (!restored) {
        throw new Error("snapshot import returned null");
      }
      ensureEdgelessRoot(restored);
      attachDoc(hostRef.current, restored);
      const ready = await finishCanvasInitialization("Restored the last saved revision.");
      if (!ready) return;
      currentSnapshotRef.current = savedSnapshot;
      setSaveError(null);
      latestStateRef.current = {
        ...latestStateRef.current,
        draftSnapshot: savedSnapshot,
      };
      onStateLoaded(latestStateRef.current);
      await syncCanvasBlocks(restored);
    } catch (error) {
      const message = `Failed to restore the last saved revision: ${String(error)}`;
      setSaveError(message);
      onError(message);
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
    if (overlayMountElement?.isConnected && overlayMountElement !== shell) {
      overlayRoots.push(overlayMountElement);
    }
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
      hasOverlayMount: Boolean(overlayMountElement?.isConnected),
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
  }, [getEditor, getRootService, overlayMountElement]);

  const getSelectedCanvasContext = useCallback((): CanvasSelectionContext => {
    const rootService = getRootService();
    const editor = getEditor();
    if (!rootService && !editor) {
      return {
        refs: [],
        elementIds: [],
      };
    }
    const selectedIds = getSelectedCanvasElements(editor, rootService).ids;
    const selectedSet = new Set(selectedIds);
    const refsById = new Map(blockRecords.map((ref) => [ref.blockId, ref]));
    const refs = selectedIds.map((id: string) => {
      const ref = refsById.get(id);
      const meta = readCanvasMeta(id, getDoc(), blockRecords);
      const title =
        (typeof meta?.title === "string" && meta.title.trim() ? meta.title.trim() : null)
        ?? ref?.sourceKey
        ?? fallbackTitle(meta?.kind, id);
      const kind = normalizeBlockKind(
        typeof meta?.kind === "string" ? meta.kind : ref?.blockKind ?? "note",
      );
      const sourcePath =
        typeof meta?.sourcePath === "string" && meta.sourcePath.trim()
          ? meta.sourcePath
          : ref?.sourcePath ?? null;
      if (!selectedSet.has(id)) return null;
      return {
        blockId: id,
        title,
        sourcePath,
        blockKind: kind,
        meta,
      };
    }).filter((item: CanvasSelectionRef | null): item is CanvasSelectionRef => Boolean(item));
    return {
      refs,
      elementIds: selectedIds,
    };
  }, [blockRecords, getDoc, getEditor, getRootService]);

  const buildSelectionContext = useCallback(async () => {
    const { refs, elementIds } = getSelectedCanvasContext();
    if (refs.length === 0) return null;
    const selectionSummary = renderSelectionSummaryMarkdown(refs);
    const summaryPath = await createCanvasContextFile({
      sessionId,
      kind: "selection",
      fileNamePrefix: "canvas-selection",
      content: selectionSummary,
    });
    const attachments: ComposerAttachment[] = [
      {
        path: summaryPath,
        kind: "file",
        mimeType: "text/markdown",
        previewDataUrl: null,
        displayName: "Canvas selection summary",
        name: "Canvas selection summary",
      },
    ];
    const workflowRefs = refs.filter((ref) => ref.blockKind === "workflow_card");
    for (const workflow of workflowRefs) {
      const workflowMarkdown = renderWorkflowSummaryMarkdown(workflow.meta ?? {}, workflow.title);
      const workflowPath = await createCanvasContextFile({
        sessionId,
        kind: "workflow",
        fileNamePrefix: safeFilePrefix(workflow.title),
        content: workflowMarkdown,
      });
      attachments.push({
        path: workflowPath,
        kind: "file",
        mimeType: "text/markdown",
        previewDataUrl: null,
        displayName: `${workflow.title} workflow summary`,
        name: `${workflow.title} workflow summary`,
      });
    }
    const snapshotResult = await exportSelectionSnapshot();
    if (snapshotResult.ok) attachments.push(snapshotResult.snapshot.attachment);
    const canvasContext: CanvasContextOption = {
      canvasId: initialState.document.id,
      scope: "selection",
      blockIds: refs.map((ref) => ref.blockId),
      elementIds,
      snapshotAttachmentPath: snapshotResult.ok ? snapshotResult.snapshot.path : null,
      refs: refs.map((ref) => {
        const record = blockRecords.find((item) => item.blockId === ref.blockId);
        return record
          ? canvasBlockRecordToContextRef(record)
          : {
              blockId: ref.blockId,
              blockKind: ref.blockKind,
              sourceType: "note",
              sourcePath: ref.sourcePath,
              sourceKey: ref.title,
              summary: ref.title,
            };
      }),
    };
    return {
      refs,
      attachments,
      canvasContext,
    };
  }, [blockRecords, exportSelectionSnapshot, getSelectedCanvasContext, initialState.document.id, sessionId]);

  const attachSelectionSnapshot = useCallback(async (elementIds?: string[] | null) => {
    setBridgeBusy("snapshot");
    showSnapshotToast("Exporting BlockSuite snapshot...");
    try {
      const snapshotResult = await exportSelectionSnapshot(elementIds, (message) => {
        showSnapshotToast(message);
      });
      if (!snapshotResult.ok) {
        const message = snapshotExportFailureMessage(snapshotResult.reason);
        showSnapshotToast(message, "error");
        onError(message);
        return;
      }
      if (composer.supportsImageAttachments) {
        await composer.appendAttachments([snapshotResult.snapshot.attachment]);
        showSnapshotToast(`Attached snapshot: ${snapshotResult.snapshot.path}`);
      } else {
        showSnapshotToast(`Saved snapshot: ${snapshotResult.snapshot.path}`);
        onError("Snapshot PNG was saved locally, but the selected agent does not support image attachments.");
      }
    } catch (error) {
      const message = `Failed to attach selection snapshot: ${String(error)}`;
      showSnapshotToast(message, "error");
      onError(message);
    } finally {
      setBridgeBusy(null);
    }
  }, [composer, exportSelectionSnapshot, onError, showSnapshotToast]);

  const askSelection = async () => {
    setBridgeBusy("ask");
    try {
      const payload = await buildSelectionContext();
      if (!payload) {
        onError("Select one or more canvas items before asking about the canvas.");
        return;
      }
      const prompt =
        composer.text.trim() ||
        `Help me reason about these ${payload.refs.length} selected canvas item${payload.refs.length === 1 ? "" : "s"}.`;
      const sent = await composer.sendWithContext(prompt, {
        clearComposer: true,
        attachments: [...composer.attachments, ...payload.attachments],
        runtimeOptions: {
          canvasContext: payload.canvasContext,
        },
      });
      if (!sent.ok) {
        onError("Failed to send the canvas selection to the agent.");
        return;
      }
      const turnId = sent.turnId ?? `canvas-selection:${Date.now()}`;
      const anchor = await createCanvasAnchor({
        sessionId,
        anchorBlockId: payload.refs[0]?.blockId ?? null,
        selectionBlockIdsJson: JSON.stringify(payload.refs.map((ref) => ref.blockId)),
        selectionElementIdsJson: JSON.stringify(payload.canvasContext.elementIds),
        turnId,
        summary: payload.refs.map((ref) => ref.title).join(", ").slice(0, 180),
      });
      const nextState = {
        ...latestStateRef.current,
        anchors: [anchor, ...latestStateRef.current.anchors],
      };
      latestStateRef.current = nextState;
      onStateLoaded(nextState);
    } catch (error) {
      onError(`Failed to ask about the current selection: ${String(error)}`);
    } finally {
      setBridgeBusy(null);
    }
  };

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
      <div className="flex h-9 shrink-0 items-center justify-between gap-3 border-b border-ink/8 bg-surface-panel/90 px-4">
        <div className="flex items-center gap-3 text-caption text-ink/50">
          <button
            ref={addMenuButtonRef}
            type="button"
            onClick={() => setAddMenuOpen((value) => !value)}
            className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/12 px-3 text-ink/72 transition hover:bg-ink/5"
          >
            <Layers3 className="h-3.5 w-3.5" />
            Add to canvas
          </button>
          <button
            ref={editedFilesButtonRef}
            type="button"
            onClick={openEditedFilesPicker}
            disabled={changedFiles.length === 0}
            className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-3 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <FilePlus2 className="h-3.5 w-3.5" />
            Add edited files
          </button>
        </div>
        <div className="flex items-center gap-2 text-caption">
          <span className="inline-flex h-6 items-center rounded-md border border-ink/10 px-2.5 text-ink/55">
            {selectionCount > 0 ? `${selectionCount} selected` : isReady ? status : "Canvas"}
          </span>
          <button
            type="button"
            onClick={() => void askSelection()}
            disabled={selectionCount === 0 || bridgeBusy !== null}
            className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-2.5 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <MessageCircleQuestionMark className="h-3.5 w-3.5" />
            Ask selection
          </button>
          <button
            type="button"
            onClick={() => void attachSelectionSnapshot()}
            disabled={selectionCount === 0 || bridgeBusy !== null}
            className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-2.5 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <Camera className="h-3.5 w-3.5" />
            Attach snapshot
          </button>
          <button
            type="button"
            onClick={groupSelection}
            disabled={selectionCount < 2}
            className="inline-flex h-6 items-center rounded-md border border-ink/10 px-2.5 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Group
          </button>
          <button
            type="button"
            onClick={ungroupSelection}
            disabled={!canUngroupSelection}
            className="inline-flex h-6 items-center rounded-md border border-ink/10 px-2.5 text-ink/60 transition hover:bg-ink/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Ungroup
          </button>
          {initialState.savedSnapshot && (
            <button
              type="button"
              onClick={() => void handleRestoreSavedRevision()}
              className="inline-flex h-6 items-center gap-1.5 rounded-md border border-ink/10 px-2.5 text-ink/65 transition hover:bg-ink/5"
            >
              <RefreshCcw className="h-3.5 w-3.5" />
              Restore
            </button>
          )}
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
      {overlayMountElement && createPortal(
        <CanvasCustomBlockOverlay
          items={customOverlayItems}
          onPromoteFileCardToMarkdown={promoteFileCardToMarkdown}
          onRunWorkflow={(blockId) => {
            void runWorkflowBlock(blockId);
          }}
          onOpenWorkflowThread={openWorkflowThread}
          onOpenFile={onOpenProjectFile}
          onDragMarkdownPreviewFromHeader={dragMarkdownPreviewFromHeader}
          onUpdateMarkdownRenderMode={(blockId, nextMode) => {
            const doc = getDoc();
            const model = doc?.getModelById(blockId) ?? null;
            if (!doc || !model) return;
            doc.updateBlock(model, { renderMode: nextMode });
            scheduleSyncBlocks();
            scheduleCustomBlockOverlaySync();
          }}
        />,
        overlayMountElement,
      )}
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
          files={changedFiles}
          selectedFiles={pendingEditedFiles}
          onToggleFile={togglePendingEditedFile}
          onAdd={addPendingEditedFiles}
          onClose={closeEditedFilesPicker}
        />
      )}
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

function normalizeBlockKind(value: string): CanvasBlockKind {
  switch (value) {
    case "markdown_preview":
    case "file_card":
    case "workflow_card":
    case "note":
    case "group":
    case "image":
      return value;
    default:
      return "note";
  }
}

function fallbackTitle(kind: unknown, blockId: string): string {
  if (kind === "workflow_card") return "Workflow";
  if (kind === "file_card") return "File";
  if (kind === "markdown_preview") return "Markdown preview";
  if (kind === "image") return "Image";
  if (kind === "group") return "Group";
  return blockId ? `Block ${blockId.slice(0, 6)}` : "Canvas note";
}

function readCanvasMeta(
  blockId: string,
  doc: ReturnType<typeof createBlockSuiteDoc>["doc"] | null,
  blockRecords: CanvasBlockRecord[],
): Record<string, unknown> | null {
  const model = doc?.getModelById(blockId) as (Record<string, unknown> & { flavour?: string; caption?: string }) | null;
  if (model) {
    if (model.flavour === "sessio:markdown-preview") {
      return {
        kind: "markdown_preview",
        title: model.title,
        sourcePath: model.sourcePath,
        sourceType: model.sourceType,
        excerpt: model.excerpt,
        renderMode: model.renderMode,
      };
    }
    if (model.flavour === "sessio:file-card") {
      return {
        kind: "file_card",
        title: model.title,
        sourcePath: model.sourcePath,
        sourceType: model.sourceType,
        subtitle: model.subtitle,
        summary: model.summary,
        status: model.status,
      };
    }
    if (model.flavour === "sessio:workflow-card") {
      return {
        kind: "workflow_card",
        title: model.title,
        threadId: model.threadId,
        threadStageId: model.threadStageId,
        workflowSummaryMarkdown: model.workflowSummaryMarkdown,
        executionState: model.executionState,
        lastRunId: model.lastRunId,
        threadGoal: model.threadGoal,
        workflowSnapshotJson: model.workflowSnapshotJson,
      };
    }
    if (model.flavour === "affine:note") {
      return {
        kind: "note",
        title: blockRecords.find(item => item.blockId === blockId)?.sourceKey ?? "New note",
      };
    }
    if (model.flavour === "affine:image") {
      return {
        kind: "image",
        title: typeof model.caption === "string" && model.caption.trim() ? model.caption.trim() : "Image",
      };
    }
  }
  const record = blockRecords.find((item) => item.blockId === blockId);
  return tryParseJson(record?.metadataJson);
}

function safeFilePrefix(value: string): string {
  const cleaned = value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  return cleaned || "canvas";
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
