// @vitest-environment happy-dom

import { afterEach, describe, expect, it } from "vitest";
import {
  clampAppChatDrawerHeight,
  mergeAppHistoryTurns,
} from "../src/appChatDrawer";
import type { LiveTurn } from "../src/runtimeChat";

describe("clampAppChatDrawerHeight", () => {
  const originalHeight = window.innerHeight;

  afterEach(() => {
    Object.defineProperty(window, "innerHeight", {
      configurable: true,
      value: originalHeight,
    });
  });

  it("keeps the transcript usable while preserving app viewport space", () => {
    Object.defineProperty(window, "innerHeight", {
      configurable: true,
      value: 900,
    });

    expect(clampAppChatDrawerHeight(80)).toBe(168);
    expect(clampAppChatDrawerHeight(320)).toBe(320);
    expect(clampAppChatDrawerHeight(900)).toBe(560);
  });

  it("reduces the maximum height for a short window", () => {
    Object.defineProperty(window, "innerHeight", {
      configurable: true,
      value: 640,
    });

    expect(clampAppChatDrawerHeight(500)).toBe(340);
  });
});

describe("mergeAppHistoryTurns", () => {
  const turn = (turnId: string, startedAt: number, updatedAt = startedAt): LiveTurn => ({
    turnId,
    status: "completed",
    blocks: [],
    tools: [],
    permissions: [],
    protocolMessages: [],
    stopReason: null,
    error: null,
    startedAt,
    updatedAt,
  });

  it("orders linked session history and keeps the newest copy of a turn", () => {
    const olderCopy = turn("shared", 20, 21);
    const newerCopy = turn("shared", 20, 25);

    expect(
      mergeAppHistoryTurns([
        [turn("later", 30), newerCopy],
        [turn("earlier", 10), olderCopy],
      ]),
    ).toEqual([turn("earlier", 10), newerCopy, turn("later", 30)]);
  });
});
