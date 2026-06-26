import type {
  ScreenshotOverlaySource,
  ScreenshotOverlayWindowCandidate,
} from "../api";

export type OverlayPoint = {
  x: number;
  y: number;
};

export type OverlayRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export function canSelectWindows(source: Pick<ScreenshotOverlaySource, "mode"> | null): boolean {
  return !source || source.mode !== "selection";
}

export function windowCandidateAtPoint(
  windows: ScreenshotOverlayWindowCandidate[],
  point: OverlayPoint,
): ScreenshotOverlayWindowCandidate | null {
  for (const candidate of windows) {
    const rect = windowCandidateRect(candidate);
    if (pointInOverlayRect(point, rect)) return candidate;
  }
  return null;
}

export function selectableWindowCandidateAtPoint(
  source: Pick<ScreenshotOverlaySource, "mode" | "windows"> | null,
  point: OverlayPoint,
): ScreenshotOverlayWindowCandidate | null {
  if (!source || !canSelectWindows(source)) return null;
  return windowCandidateAtPoint(source.windows, point);
}

export function windowCandidateRect(candidate: ScreenshotOverlayWindowCandidate): OverlayRect {
  return {
    x: candidate.x,
    y: candidate.y,
    width: candidate.width,
    height: candidate.height,
  };
}

export function pointInOverlayRect(point: OverlayPoint, rect: OverlayRect): boolean {
  return (
    point.x >= rect.x &&
    point.x <= rect.x + rect.width &&
    point.y >= rect.y &&
    point.y <= rect.y + rect.height
  );
}
