import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { listen } from "@tauri-apps/api/event";

type PointerAction =
  | "click"
  | "secondary_click"
  | "double_click"
  | "drag"
  | "semantic";

interface PointerEventPayload {
  action: PointerAction;
  sessionId: string;
  x?: number | null;
  y?: number | null;
  toX?: number | null;
  toY?: number | null;
  label?: string | null;
}

interface OverlayGeometry {
  originX: number;
  originY: number;
  width: number;
  height: number;
}

interface LocalPoint {
  x: number;
  y: number;
}

interface PointerVisualState {
  visible: boolean;
  point: LocalPoint;
  action: PointerAction;
  label: string | null;
  pulseId: number;
  dragTo: LocalPoint | null;
}

const EVENT_NAME = "computer_use_pointer_event";
const HIDE_DELAY_MS = 1200;

function numberParam(params: URLSearchParams, key: string, fallback: number): number {
  const raw = params.get(key);
  if (!raw) return fallback;
  const value = Number(raw);
  return Number.isFinite(value) ? value : fallback;
}

function readOverlayGeometry(): OverlayGeometry {
  const params = new URLSearchParams(window.location.search);
  return {
    originX: numberParam(params, "originX", 0),
    originY: numberParam(params, "originY", 0),
    width: numberParam(params, "width", window.innerWidth),
    height: numberParam(params, "height", window.innerHeight),
  };
}

function pointInside(point: LocalPoint, geometry: OverlayGeometry): boolean {
  return point.x >= 0 && point.y >= 0 && point.x <= geometry.width && point.y <= geometry.height;
}

function toLocalPoint(
  geometry: OverlayGeometry,
  x: number | null | undefined,
  y: number | null | undefined,
): LocalPoint | null {
  if (typeof x !== "number" || typeof y !== "number") return null;
  if (!Number.isFinite(x) || !Number.isFinite(y)) return null;
  return {
    x: x - geometry.originX,
    y: y - geometry.originY,
  };
}

function actionLabel(action: PointerAction, label: string | null | undefined): string {
  if (label) return label;
  switch (action) {
    case "click":
      return "click";
    case "secondary_click":
      return "right click";
    case "double_click":
      return "double click";
    case "drag":
      return "drag";
    case "semantic":
      return "computer use";
  }
}

export default function ComputerUsePointerOverlayWindow() {
  const geometry = useMemo(readOverlayGeometry, []);
  const hideTimerRef = useRef<number | null>(null);
  const [visual, setVisual] = useState<PointerVisualState>(() => ({
    visible: false,
    point: { x: Math.min(48, geometry.width / 2), y: Math.min(48, geometry.height / 2) },
    action: "semantic",
    label: null,
    pulseId: 0,
    dragTo: null,
  }));

  useEffect(() => {
    document.documentElement.classList.add("computer-use-pointer-overlay-root");
    document.body.classList.add("computer-use-pointer-overlay-body");
    return () => {
      document.documentElement.classList.remove("computer-use-pointer-overlay-root");
      document.body.classList.remove("computer-use-pointer-overlay-body");
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlistenPromise = listen<PointerEventPayload>(EVENT_NAME, (event) => {
      if (disposed) return;
      const payload = event.payload;
      const from = toLocalPoint(geometry, payload.x, payload.y);
      const to = toLocalPoint(geometry, payload.toX, payload.toY);

      if (!from) return;
      if (from && !pointInside(from, geometry) && (!to || !pointInside(to, geometry))) return;

      if (hideTimerRef.current !== null) {
        window.clearTimeout(hideTimerRef.current);
      }

      setVisual((current) => ({
        visible: true,
        point: from ?? current.point,
        action: payload.action,
        label: actionLabel(payload.action, payload.label),
        pulseId: current.pulseId + 1,
        dragTo: to && pointInside(to, geometry) ? to : null,
      }));

      hideTimerRef.current = window.setTimeout(() => {
        setVisual((current) => ({ ...current, visible: false, dragTo: null }));
      }, HIDE_DELAY_MS);
    });

    return () => {
      disposed = true;
      if (hideTimerRef.current !== null) {
        window.clearTimeout(hideTimerRef.current);
      }
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [geometry]);

  const dragLine = visual.dragTo
    ? lineStyle(visual.point, visual.dragTo)
    : null;

  return (
    <div className="computer-use-pointer-overlay" aria-hidden="true">
      {dragLine && visual.visible && (
        <div className="computer-use-pointer-drag-line" style={dragLine} />
      )}
      <div
        className={
          "computer-use-pointer " +
          (visual.visible ? "computer-use-pointer-visible" : "")
        }
        style={{
          transform: `translate3d(${visual.point.x}px, ${visual.point.y}px, 0)`,
        }}
      >
        <div className="computer-use-pointer-cursor" />
        {visual.visible && (
          <div key={visual.pulseId} className="computer-use-pointer-pulse" />
        )}
        {visual.visible && visual.label && (
          <div className="computer-use-pointer-label">{visual.label}</div>
        )}
      </div>
    </div>
  );
}

function lineStyle(from: LocalPoint, to: LocalPoint): CSSProperties {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const length = Math.max(1, Math.hypot(dx, dy));
  const angle = Math.atan2(dy, dx) * (180 / Math.PI);
  return {
    width: `${length}px`,
    transform: `translate3d(${from.x}px, ${from.y}px, 0) rotate(${angle}deg)`,
  };
}
