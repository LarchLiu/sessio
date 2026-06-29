import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { emit } from "@tauri-apps/api/event";
import {
  Check,
  LoaderCircle,
  Minus,
  Save,
  Square,
  Undo2,
  X,
} from "lucide-react";
import {
  completeScreenshotOverlayCapture,
  finishScreenshotOverlay,
  getScreenshotOverlaySource,
  readLocalImageDataUrl,
  savePastedAttachment,
  type ScreenshotOverlayWindowCandidate,
  type ScreenshotOverlaySource,
} from "../api";
import { useI18n } from "../i18n";
import {
  canSelectWindows,
  selectableWindowCandidateAtPoint,
  windowCandidateAtPoint,
  windowCandidateRect,
} from "./screenshotOverlayGeometry";

type EditorTool = "rect" | "line" | "mosaic";

type Point = {
  x: number;
  y: number;
};

type Rect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

type ResizeHandle = "n" | "s" | "e" | "w" | "nw" | "ne" | "sw" | "se";

type Annotation = {
  id: number | string;
  tool: EditorTool;
  start: Point;
  end: Point;
};

type OverlaySavedPayload = {
  requestId: string;
  path: string;
  previewDataUrl: string;
};

type OverlayCancelledPayload = {
  requestId: string;
};

export default function ScreenshotOverlayWindow() {
  const { t } = useI18n();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const [source, setSource] = useState<ScreenshotOverlaySource | null>(null);
  const [imageReady, setImageReady] = useState(false);
  const [selection, setSelection] = useState<Annotation | null>(null);
  const [selectionDraft, setSelectionDraft] = useState<Annotation | null>(null);
  const [hoverWindow, setHoverWindow] = useState<ScreenshotOverlayWindowCandidate | null>(null);
  const [tool, setTool] = useState<EditorTool>("rect");
  const [annotations, setAnnotations] = useState<Annotation[]>([]);
  const [annotationDraft, setAnnotationDraft] = useState<Annotation | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [canvasCursor, setCanvasCursor] = useState("crosshair");
  const pointerStartRef = useRef<{
    point: Point;
    window: ScreenshotOverlayWindowCandidate | null;
  } | null>(null);
  const resizeGestureRef = useRef<{
    handle: ResizeHandle;
    initialRect: Rect;
  } | null>(null);

  useEffect(() => {
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
    let disposed = false;
    getScreenshotOverlaySource()
      .then(async (overlaySource) => {
        if (disposed) return;
        setSource(overlaySource);
        if (overlaySource.initialSelection) {
          setSelection(rectToAnnotation(overlaySource.initialSelection, "initial-selection"));
        }
        const dataUrl = await readLocalImageDataUrl(overlaySource.sourcePath);
        if (disposed) return;
        const image = new Image();
        image.onload = () => {
          if (disposed) return;
          imageRef.current = image;
          setImageReady(true);
        };
        image.onerror = () => {
          if (disposed) return;
          setError(t("screenshot.capture_failed", { error: "Could not load screenshot" }));
        };
        image.src = dataUrl;
      })
      .catch((err) => {
        if (disposed) return;
        setError(String(err));
      });
    return () => {
      disposed = true;
    };
  }, [t]);

  const redraw = useCallback(() => {
    redrawOverlayCanvas(
      canvasRef.current,
      imageRef.current,
      selectionDraft ?? selection,
      selection || selectionDraft ? null : hoverWindow,
      annotations,
      annotationDraft,
    );
  }, [annotationDraft, annotations, hoverWindow, selection, selectionDraft]);

  useEffect(() => {
    if (!imageReady) return;
    const resize = () => redraw();
    resize();
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  }, [imageReady, redraw]);

  useEffect(() => {
    redraw();
  }, [redraw]);

  const cancel = useCallback(() => {
    void (async () => {
      try {
        if (source) {
          void emit<OverlayCancelledPayload>("screenshot_overlay_cancelled", {
            requestId: source.requestId,
          });
          await completeScreenshotOverlayCapture({
            requestId: source.requestId,
            cancelled: true,
          });
        }
      } finally {
        await finishScreenshotOverlay();
      }
    })();
  }, [source]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        cancel();
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "z") {
        event.preventDefault();
        setAnnotations((items) => items.slice(0, -1));
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [cancel]);

  const imagePoint = (event: ReactPointerEvent<HTMLCanvasElement>): Point | null => {
    const canvas = canvasRef.current;
    const image = imageRef.current;
    if (!canvas || !image) return null;
    const rect = canvas.getBoundingClientRect();
    const x = ((event.clientX - rect.left) / rect.width) * image.naturalWidth;
    const y = ((event.clientY - rect.top) / rect.height) * image.naturalHeight;
    return {
      x: clamp(x, 0, image.naturalWidth),
      y: clamp(y, 0, image.naturalHeight),
    };
  };

  const pointerDown = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (!imageReady) return;
    const point = imagePoint(event);
    if (!point) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    const resizeHandle = selection
      ? resizeHandleAtPoint(point, selection, imageRef.current)
      : null;
    if (resizeHandle && selection) {
      resizeGestureRef.current = {
        handle: resizeHandle,
        initialRect: normalizedRect(selection),
      };
      pointerStartRef.current = null;
      setCanvasCursor(cursorForResizeHandle(resizeHandle));
      setSelectionDraft(selection);
      setAnnotationDraft(null);
      setHoverWindow(null);
      return;
    }
    const hitWindow = selectableWindowCandidateAtPoint(source, point);
    pointerStartRef.current = { point, window: hitWindow };
    if (selection && pointInRect(point, normalizedRect(selection))) {
      setAnnotationDraft({ id: Date.now(), tool, start: point, end: point });
      return;
    }
    if (hitWindow) {
      setSelection(null);
      setSelectionDraft(null);
      setAnnotations([]);
      setAnnotationDraft(null);
      setHoverWindow(hitWindow);
      return;
    }
    setSelection(null);
    setAnnotations([]);
    setAnnotationDraft(null);
    setSelectionDraft({ id: Date.now(), tool: "rect", start: point, end: point });
  };

  const pointerMove = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    const point = imagePoint(event);
    if (!point) return;
    const resizeGesture = resizeGestureRef.current;
    if (resizeGesture) {
      setSelectionDraft(rectToAnnotation(
        resizedSelectionRect(resizeGesture, point, imageRef.current),
        selection?.id ?? Date.now(),
      ));
      return;
    }
    if (annotationDraft) {
      setAnnotationDraft({ ...annotationDraft, end: point });
      return;
    }
    const pointerStart = pointerStartRef.current;
    if (pointerStart?.window && distance(pointerStart.point, point) > 6) {
      setHoverWindow(null);
      setSelectionDraft({ id: Date.now(), tool: "rect", start: pointerStart.point, end: point });
      pointerStartRef.current = { point: pointerStart.point, window: null };
      return;
    }
    if (selectionDraft) {
      setSelectionDraft({ ...selectionDraft, end: point });
      return;
    }
    if (selection) {
      setCanvasCursor(cursorForResizeHandle(
        resizeHandleAtPoint(point, selection, imageRef.current),
      ));
      return;
    }
    if (!selection && source && canSelectWindows(source)) {
      const hitWindow = windowCandidateAtPoint(source.windows, point);
      setHoverWindow(hitWindow);
      setCanvasCursor(hitWindow ? "pointer" : "crosshair");
    }
  };

  const pointerUp = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    const point = imagePoint(event);
    const resizeGesture = resizeGestureRef.current;
    if (resizeGesture) {
      const next = rectToAnnotation(
        resizedSelectionRect(resizeGesture, point ?? {
          x: resizeGesture.initialRect.x,
          y: resizeGesture.initialRect.y,
        }, imageRef.current),
        selection?.id ?? Date.now(),
      );
      setSelection(next);
      setSelectionDraft(null);
      resizeGestureRef.current = null;
      pointerStartRef.current = null;
      setCanvasCursor(cursorForResizeHandle(
        resizeHandleAtPoint(point ?? next.end, next, imageRef.current),
      ));
      return;
    }
    if (annotationDraft) {
      const next = { ...annotationDraft, end: point ?? annotationDraft.end };
      if (annotationLength(next) > 4) {
        setAnnotations((items) => [...items, next]);
      }
      setAnnotationDraft(null);
      pointerStartRef.current = null;
      return;
    }
    if (selectionDraft) {
      const next = { ...selectionDraft, end: point ?? selectionDraft.end };
      if (annotationLength(next) > 8) {
        setSelection(next);
      }
      setSelectionDraft(null);
      pointerStartRef.current = null;
      return;
    }
    const pointerStart = pointerStartRef.current;
    if (pointerStart?.window && point && distance(pointerStart.point, point) <= 6) {
      setSelection(windowCandidateToAnnotation(pointerStart.window));
      setHoverWindow(null);
      pointerStartRef.current = null;
      return;
    }
    pointerStartRef.current = null;
  };

  const save = async () => {
    if (!source || !imageRef.current || saving) return;
    setSaving(true);
    try {
      const output = renderFinalScreenshot(imageRef.current, selection, annotations);
      const dataUrl = output.toDataURL("image/png");
      const saved = await savePastedAttachment({
        fileName: source.fileName || "Screenshot.png",
        mimeType: "image/png",
        dataBase64: dataUrlToBase64(dataUrl),
      });
      await completeScreenshotOverlayCapture({
        requestId: source.requestId,
        path: saved.path,
      });
      await emit<OverlaySavedPayload>("screenshot_overlay_saved", {
        requestId: source.requestId,
        path: saved.path,
        previewDataUrl: dataUrl,
      });
    } catch (err) {
      setError(t("screenshot.capture_failed", { error: String(err) }));
      setSaving(false);
      return;
    }
    await finishScreenshotOverlay();
  };

  const activeSelection = selectionDraft ?? selection;
  const toolbarRect = activeSelection
    ? imageRectToViewportRect(normalizedRect(activeSelection), imageRef.current)
    : null;

  return (
    <div className="fixed inset-0 cursor-crosshair overflow-hidden bg-transparent text-white">
      <canvas
        ref={canvasRef}
        onPointerDown={pointerDown}
        onPointerMove={pointerMove}
        onPointerUp={pointerUp}
        onPointerCancel={() => {
          setSelectionDraft(null);
          setAnnotationDraft(null);
          resizeGestureRef.current = null;
          pointerStartRef.current = null;
          setCanvasCursor("crosshair");
        }}
        className="block h-screen w-screen"
        style={{ cursor: canvasCursor }}
      />
      {!imageReady && !error && (
        <div className="pointer-events-none fixed inset-0 flex items-center justify-center bg-black/72">
          <LoaderCircle className="h-6 w-6 animate-spin text-white/70" />
        </div>
      )}
      {toolbarRect && (
        <ScreenshotOverlayToolbar
          rect={toolbarRect}
          tool={tool}
          saving={saving}
          canUndo={annotations.length > 0}
          labels={{
            rect: t("screenshot.tool_rect"),
            line: t("screenshot.tool_line"),
            mosaic: t("screenshot.tool_mosaic"),
            undo: t("screenshot.undo"),
            cancel: t("screenshot.cancel"),
            save: t("screenshot.save"),
          }}
          onToolChange={setTool}
          onUndo={() => setAnnotations((items) => items.slice(0, -1))}
          onCancel={cancel}
          onSave={() => void save()}
        />
      )}
      {error && (
        <div className="fixed left-1/2 top-1/2 max-w-[520px] -translate-x-1/2 -translate-y-1/2 rounded-lg bg-white px-4 py-3 text-sm text-neutral-900 shadow-2xl">
          {error}
          <button
            type="button"
            onClick={cancel}
            className="ml-3 rounded-md bg-neutral-900 px-2.5 py-1 text-white"
          >
            {t("screenshot.cancel")}
          </button>
        </div>
      )}
    </div>
  );
}

function ScreenshotOverlayToolbar({
  rect,
  tool,
  saving,
  canUndo,
  labels,
  onToolChange,
  onUndo,
  onCancel,
  onSave,
}: {
  rect: { left: number; top: number; width: number; height: number };
  tool: EditorTool;
  saving: boolean;
  canUndo: boolean;
  labels: Record<"rect" | "line" | "mosaic" | "undo" | "cancel" | "save", string>;
  onToolChange: (tool: EditorTool) => void;
  onUndo: () => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const width = 330;
  const left = clamp(rect.left + rect.width / 2 - width / 2, 12, window.innerWidth - width - 12);
  const top = rect.top + rect.height + 12 > window.innerHeight - 62
    ? Math.max(12, rect.top - 58)
    : rect.top + rect.height + 12;
  return (
    <div
      className="fixed z-10 flex h-12 items-center gap-1 rounded-lg border border-black/10 bg-white px-3 text-neutral-900 shadow-[0_10px_28px_rgba(0,0,0,0.28)]"
      style={{ left, top, width }}
    >
      <ToolbarIconButton active={tool === "rect"} label={labels.rect} onClick={() => onToolChange("rect")}>
        <Square className="h-5 w-5" />
      </ToolbarIconButton>
      <ToolbarIconButton active={tool === "line"} label={labels.line} onClick={() => onToolChange("line")}>
        <Minus className="h-5 w-5" />
      </ToolbarIconButton>
      <ToolbarIconButton active={tool === "mosaic"} label={labels.mosaic} onClick={() => onToolChange("mosaic")}>
        <Square className="h-5 w-5 fill-current opacity-70" />
      </ToolbarIconButton>
      <div className="mx-1 h-5 w-px bg-neutral-200" />
      <ToolbarIconButton disabled={!canUndo} label={labels.undo} onClick={onUndo}>
        <Undo2 className="h-5 w-5" />
      </ToolbarIconButton>
      <ToolbarIconButton disabled={saving} label={labels.save} onClick={onSave}>
        {saving ? <LoaderCircle className="h-5 w-5 animate-spin" /> : <Save className="h-5 w-5" />}
      </ToolbarIconButton>
      <div className="ml-auto flex items-center gap-1">
        <ToolbarIconButton disabled={saving} label={labels.cancel} tone="danger" onClick={onCancel}>
          <X className="h-5 w-5" />
        </ToolbarIconButton>
        <ToolbarIconButton disabled={saving} label={labels.save} tone="ok" onClick={onSave}>
          <Check className="h-5 w-5" />
        </ToolbarIconButton>
      </div>
    </div>
  );
}

function ToolbarIconButton({
  active = false,
  disabled = false,
  tone = "default",
  label,
  children,
  onClick,
}: {
  active?: boolean;
  disabled?: boolean;
  tone?: "default" | "danger" | "ok";
  label: string;
  children: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={active}
      disabled={disabled}
      onClick={onClick}
      className={
        "flex h-8 w-8 items-center justify-center rounded-md transition disabled:cursor-not-allowed disabled:opacity-30 " +
        (active
          ? "bg-emerald text-white"
          : tone === "danger"
            ? "text-red-500 hover:bg-red-50"
            : tone === "ok"
              ? "text-emerald hover:bg-emerald/10"
              : "text-neutral-800 hover:bg-neutral-100")
      }
    >
      {children}
    </button>
  );
}

function redrawOverlayCanvas(
  canvas: HTMLCanvasElement | null,
  image: HTMLImageElement | null,
  selection: Annotation | null,
  hoverWindow: ScreenshotOverlayWindowCandidate | null,
  annotations: Annotation[],
  draft: Annotation | null,
) {
  if (!canvas || !image) return;
  const dpr = window.devicePixelRatio || 1;
  const width = Math.max(1, Math.round(window.innerWidth * dpr));
  const height = Math.max(1, Math.round(window.innerHeight * dpr));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  canvas.style.width = `${window.innerWidth}px`;
  canvas.style.height = `${window.innerHeight}px`;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, window.innerWidth, window.innerHeight);
  ctx.drawImage(image, 0, 0, window.innerWidth, window.innerHeight);
  ctx.fillStyle = "rgba(0, 0, 0, 0.22)";
  ctx.fillRect(0, 0, window.innerWidth, window.innerHeight);

  if (!selection && hoverWindow) {
    const sourceRect = windowCandidateRect(hoverWindow);
    const viewportRect = imageRectToViewportRect(sourceRect, image);
    drawUnmaskedImageRect(ctx, image, sourceRect, viewportRect);
    drawSelectionFrame(ctx, viewportRect, true);
    return;
  }
  if (!selection) return;
  const viewportRect = imageRectToViewportRect(normalizedRect(selection), image);
  const sourceRect = normalizedRect(selection);
  drawUnmaskedImageRect(ctx, image, sourceRect, viewportRect);
  drawAnnotations(ctx, image, annotations, draft);
  drawSelectionFrame(ctx, viewportRect, false);
}

function drawUnmaskedImageRect(
  ctx: CanvasRenderingContext2D,
  image: HTMLImageElement,
  sourceRect: { x: number; y: number; width: number; height: number },
  viewportRect: { left: number; top: number; width: number; height: number },
) {
  ctx.drawImage(
    image,
    sourceRect.x,
    sourceRect.y,
    sourceRect.width,
    sourceRect.height,
    viewportRect.left,
    viewportRect.top,
    viewportRect.width,
    viewportRect.height,
  );
}

function drawSelectionFrame(
  ctx: CanvasRenderingContext2D,
  rect: { left: number; top: number; width: number; height: number },
  hover: boolean,
) {
  ctx.save();
  ctx.strokeStyle = hover ? "rgba(16, 209, 122, 0.78)" : "#10d17a";
  ctx.lineWidth = hover ? 2 : 3;
  if (hover) ctx.setLineDash([10, 7]);
  ctx.strokeRect(rect.left, rect.top, rect.width, rect.height);
  if (hover) {
    ctx.restore();
    return;
  }
  ctx.fillStyle = "#10d17a";
  const handles = [
    [rect.left, rect.top],
    [rect.left + rect.width / 2, rect.top],
    [rect.left + rect.width, rect.top],
    [rect.left, rect.top + rect.height / 2],
    [rect.left + rect.width, rect.top + rect.height / 2],
    [rect.left, rect.top + rect.height],
    [rect.left + rect.width / 2, rect.top + rect.height],
    [rect.left + rect.width, rect.top + rect.height],
  ];
  for (const [x, y] of handles) {
    ctx.fillRect(x - 4, y - 4, 8, 8);
  }
  ctx.restore();
}

function drawAnnotations(
  ctx: CanvasRenderingContext2D,
  image: HTMLImageElement,
  annotations: Annotation[],
  draft: Annotation | null,
) {
  for (const annotation of annotations) drawAnnotation(ctx, image, annotation, false);
  if (draft) drawAnnotation(ctx, image, draft, true);
}

function drawAnnotation(
  ctx: CanvasRenderingContext2D,
  image: HTMLImageElement,
  annotation: Annotation,
  draft: boolean,
) {
  const start = imagePointToViewportPoint(annotation.start, image);
  const end = imagePointToViewportPoint(annotation.end, image);
  const x = Math.min(start.x, end.x);
  const y = Math.min(start.y, end.y);
  const width = Math.abs(end.x - start.x);
  const height = Math.abs(end.y - start.y);
  ctx.save();
  if (annotation.tool === "mosaic") {
    if (width >= 2 && height >= 2) {
      drawViewportMosaic(ctx, x, y, width, height);
    }
    ctx.restore();
    return;
  }
  ctx.strokeStyle = "#10d17a";
  ctx.lineWidth = 3;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  if (draft) ctx.setLineDash([9, 6]);
  if (annotation.tool === "rect") {
    ctx.strokeRect(x, y, width, height);
  } else {
    ctx.beginPath();
    ctx.moveTo(start.x, start.y);
    ctx.lineTo(end.x, end.y);
    ctx.stroke();
  }
  ctx.restore();
}

function drawViewportMosaic(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
) {
  const block = Math.max(8, Math.round(Math.max(width, height) / 20));
  const smallWidth = Math.max(1, Math.ceil(width / block));
  const smallHeight = Math.max(1, Math.ceil(height / block));
  const tmp = document.createElement("canvas");
  tmp.width = smallWidth;
  tmp.height = smallHeight;
  const tmpCtx = tmp.getContext("2d");
  if (!tmpCtx) return;
  tmpCtx.drawImage(ctx.canvas, x, y, width, height, 0, 0, smallWidth, smallHeight);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(tmp, 0, 0, smallWidth, smallHeight, x, y, width, height);
  ctx.imageSmoothingEnabled = true;
}

function renderFinalScreenshot(
  image: HTMLImageElement,
  selection: Annotation | null,
  annotations: Annotation[],
) {
  const crop = selection
    ? normalizedRect(selection)
    : { x: 0, y: 0, width: image.naturalWidth, height: image.naturalHeight };
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(crop.width));
  canvas.height = Math.max(1, Math.round(crop.height));
  const ctx = canvas.getContext("2d");
  if (!ctx) return canvas;
  ctx.drawImage(image, crop.x, crop.y, crop.width, crop.height, 0, 0, canvas.width, canvas.height);
  for (const annotation of annotations) {
    drawFinalAnnotation(ctx, annotation, crop);
  }
  return canvas;
}

function drawFinalAnnotation(
  ctx: CanvasRenderingContext2D,
  annotation: Annotation,
  crop: { x: number; y: number; width: number; height: number },
) {
  const translated: Annotation = {
    ...annotation,
    start: { x: annotation.start.x - crop.x, y: annotation.start.y - crop.y },
    end: { x: annotation.end.x - crop.x, y: annotation.end.y - crop.y },
  };
  const { x, y, width, height } = normalizedRect(translated);
  ctx.save();
  if (translated.tool === "mosaic") {
    drawFinalMosaic(ctx, x, y, width, height);
    ctx.restore();
    return;
  }
  ctx.strokeStyle = "#10d17a";
  ctx.lineWidth = Math.max(3, Math.round(ctx.canvas.width / 360));
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  if (translated.tool === "rect") {
    ctx.strokeRect(x, y, width, height);
  } else {
    ctx.beginPath();
    ctx.moveTo(translated.start.x, translated.start.y);
    ctx.lineTo(translated.end.x, translated.end.y);
    ctx.stroke();
  }
  ctx.restore();
}

function drawFinalMosaic(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
) {
  if (width < 2 || height < 2) return;
  const block = Math.max(8, Math.round(Math.max(width, height) / 24));
  const smallWidth = Math.max(1, Math.ceil(width / block));
  const smallHeight = Math.max(1, Math.ceil(height / block));
  const tmp = document.createElement("canvas");
  tmp.width = smallWidth;
  tmp.height = smallHeight;
  const tmpCtx = tmp.getContext("2d");
  if (!tmpCtx) return;
  tmpCtx.drawImage(ctx.canvas, x, y, width, height, 0, 0, smallWidth, smallHeight);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(tmp, 0, 0, smallWidth, smallHeight, x, y, width, height);
  ctx.imageSmoothingEnabled = true;
}

function imagePointToViewportPoint(point: Point, image: HTMLImageElement): Point {
  return {
    x: (point.x / image.naturalWidth) * window.innerWidth,
    y: (point.y / image.naturalHeight) * window.innerHeight,
  };
}

function imageRectToViewportRect(
  rect: Rect,
  image: HTMLImageElement | null,
) {
  if (!image) return { left: 0, top: 0, width: 0, height: 0 };
  const topLeft = imagePointToViewportPoint({ x: rect.x, y: rect.y }, image);
  const bottomRight = imagePointToViewportPoint(
    { x: rect.x + rect.width, y: rect.y + rect.height },
    image,
  );
  return {
    left: topLeft.x,
    top: topLeft.y,
    width: bottomRight.x - topLeft.x,
    height: bottomRight.y - topLeft.y,
  };
}

function windowCandidateToAnnotation(candidate: ScreenshotOverlayWindowCandidate): Annotation {
  return {
    id: candidate.id,
    tool: "rect",
    start: { x: candidate.x, y: candidate.y },
    end: { x: candidate.x + candidate.width, y: candidate.y + candidate.height },
  };
}

function normalizedRect(annotation: Annotation): Rect {
  const x = Math.min(annotation.start.x, annotation.end.x);
  const y = Math.min(annotation.start.y, annotation.end.y);
  return {
    x,
    y,
    width: Math.abs(annotation.end.x - annotation.start.x),
    height: Math.abs(annotation.end.y - annotation.start.y),
  };
}

function resizeHandleAtPoint(
  point: Point,
  selection: Annotation,
  image: HTMLImageElement | null,
): ResizeHandle | null {
  if (!image) return null;
  const rect = normalizedRect(selection);
  const tolerance = resizeToleranceInImagePx(image);
  const nearLeft = Math.abs(point.x - rect.x) <= tolerance;
  const nearRight = Math.abs(point.x - (rect.x + rect.width)) <= tolerance;
  const nearTop = Math.abs(point.y - rect.y) <= tolerance;
  const nearBottom = Math.abs(point.y - (rect.y + rect.height)) <= tolerance;
  const withinX = point.x >= rect.x - tolerance && point.x <= rect.x + rect.width + tolerance;
  const withinY = point.y >= rect.y - tolerance && point.y <= rect.y + rect.height + tolerance;
  if (!withinX || !withinY) return null;
  if (nearLeft && nearTop) return "nw";
  if (nearRight && nearTop) return "ne";
  if (nearLeft && nearBottom) return "sw";
  if (nearRight && nearBottom) return "se";
  if (nearTop) return "n";
  if (nearBottom) return "s";
  if (nearLeft) return "w";
  if (nearRight) return "e";
  return null;
}

function resizeToleranceInImagePx(image: HTMLImageElement): number {
  const imagePerViewportX = image.naturalWidth / Math.max(1, window.innerWidth);
  const imagePerViewportY = image.naturalHeight / Math.max(1, window.innerHeight);
  return Math.max(8, 10 * Math.max(imagePerViewportX, imagePerViewportY));
}

function resizedSelectionRect(
  gesture: { handle: ResizeHandle; initialRect: Rect },
  point: Point,
  image: HTMLImageElement | null,
): Rect {
  const bounds = image
    ? { width: image.naturalWidth, height: image.naturalHeight }
    : { width: Number.POSITIVE_INFINITY, height: Number.POSITIVE_INFINITY };
  const minSize = image ? Math.max(12, resizeToleranceInImagePx(image)) : 12;
  const initial = gesture.initialRect;
  let left = initial.x;
  let right = initial.x + initial.width;
  let top = initial.y;
  let bottom = initial.y + initial.height;
  if (gesture.handle.includes("w")) left = point.x;
  if (gesture.handle.includes("e")) right = point.x;
  if (gesture.handle.includes("n")) top = point.y;
  if (gesture.handle.includes("s")) bottom = point.y;
  if (right - left < minSize) {
    if (gesture.handle.includes("w")) left = right - minSize;
    else right = left + minSize;
  }
  if (bottom - top < minSize) {
    if (gesture.handle.includes("n")) top = bottom - minSize;
    else bottom = top + minSize;
  }
  left = clamp(left, 0, bounds.width - minSize);
  top = clamp(top, 0, bounds.height - minSize);
  right = clamp(right, left + minSize, bounds.width);
  bottom = clamp(bottom, top + minSize, bounds.height);
  return {
    x: left,
    y: top,
    width: right - left,
    height: bottom - top,
  };
}

function rectToAnnotation(rect: Rect, id: number | string): Annotation {
  return {
    id,
    tool: "rect",
    start: { x: rect.x, y: rect.y },
    end: { x: rect.x + rect.width, y: rect.y + rect.height },
  };
}

function cursorForResizeHandle(handle: ResizeHandle | null): string {
  switch (handle) {
    case "n":
    case "s":
      return "ns-resize";
    case "e":
    case "w":
      return "ew-resize";
    case "nw":
    case "se":
      return "nwse-resize";
    case "ne":
    case "sw":
      return "nesw-resize";
    default:
      return "crosshair";
  }
}

function pointInRect(point: Point, rect: Rect) {
  return (
    point.x >= rect.x &&
    point.x <= rect.x + rect.width &&
    point.y >= rect.y &&
    point.y <= rect.y + rect.height
  );
}

function annotationLength(annotation: Annotation): number {
  return distance(annotation.start, annotation.end);
}

function distance(a: Point, b: Point): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  return Math.sqrt(dx * dx + dy * dy);
}

function dataUrlToBase64(dataUrl: string): string {
  const marker = ";base64,";
  const index = dataUrl.indexOf(marker);
  return index >= 0 ? dataUrl.slice(index + marker.length) : dataUrl;
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}
