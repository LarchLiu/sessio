import { describe, expect, it } from "vitest";
import {
  canSelectWindows,
  pointInOverlayRect,
  selectableWindowCandidateAtPoint,
  windowCandidateAtPoint,
  windowCandidateRect,
} from "../src/components/screenshotOverlayGeometry";
import type { ScreenshotOverlayWindowCandidate } from "../src/api";

function candidate(
  id: string,
  x: number,
  y: number,
  width: number,
  height: number,
): ScreenshotOverlayWindowCandidate {
  return {
    id,
    appName: id,
    title: null,
    x,
    y,
    width,
    height,
  };
}

describe("screenshot overlay geometry", () => {
  it("disables window selection in selected portion mode", () => {
    expect(canSelectWindows({ mode: "selection" })).toBe(false);
    expect(canSelectWindows({ mode: "interactive" })).toBe(true);
    expect(canSelectWindows({})).toBe(true);
    expect(canSelectWindows(null)).toBe(true);
  });

  it("does not return window candidates while selecting a screen portion", () => {
    const app = candidate("app", 0, 0, 320, 240);

    expect(selectableWindowCandidateAtPoint({
      mode: "selection",
      windows: [app],
    }, { x: 20, y: 20 })).toBeNull();
    expect(selectableWindowCandidateAtPoint({
      mode: "interactive",
      windows: [app],
    }, { x: 20, y: 20 })).toBe(app);
  });

  it("returns the first matching window candidate for overlapping windows", () => {
    const large = candidate("large", 0, 0, 600, 400);
    const medium = candidate("medium", 100, 90, 240, 180);
    const small = candidate("small", 120, 100, 80, 60);

    expect(windowCandidateAtPoint([large, medium, small], { x: 130, y: 110 })).toBe(large);
    expect(windowCandidateAtPoint([small, medium, large], { x: 130, y: 110 })).toBe(small);
  });

  it("ignores candidates that do not contain the pointer", () => {
    const left = candidate("left", 0, 0, 120, 120);
    const right = candidate("right", 180, 0, 120, 120);

    expect(windowCandidateAtPoint([left, right], { x: 200, y: 60 })).toBe(right);
    expect(windowCandidateAtPoint([left, right], { x: 150, y: 60 })).toBeNull();
  });

  it("treats rectangle edges as part of the selectable target", () => {
    const rect = { x: 20, y: 30, width: 100, height: 80 };

    expect(pointInOverlayRect({ x: 20, y: 30 }, rect)).toBe(true);
    expect(pointInOverlayRect({ x: 120, y: 110 }, rect)).toBe(true);
    expect(pointInOverlayRect({ x: 121, y: 110 }, rect)).toBe(false);
  });

  it("maps a window candidate to its image-space rectangle", () => {
    expect(windowCandidateRect(candidate("app", 12, 18, 320, 180))).toEqual({
      x: 12,
      y: 18,
      width: 320,
      height: 180,
    });
  });
});
