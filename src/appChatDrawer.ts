import type { LiveTurn } from "./runtimeChat";

const DEFAULT_HEIGHT = 300;
const MIN_HEIGHT = 168;
const MAX_HEIGHT = 560;
const VIEWPORT_MARGIN = 300;
const STORAGE_KEY_PREFIX = "sessio.appChatDrawerHeight.";

export const appChatDrawerDimensions = {
  defaultHeight: DEFAULT_HEIGHT,
  minHeight: MIN_HEIGHT,
  maxHeight: MAX_HEIGHT,
} as const;

export function clampAppChatDrawerHeight(height: number): number {
  const viewportMax =
    typeof window === "undefined"
      ? MAX_HEIGHT
      : Math.max(
          MIN_HEIGHT,
          Math.min(MAX_HEIGHT, Math.floor(window.innerHeight - VIEWPORT_MARGIN)),
        );
  return Math.min(viewportMax, Math.max(MIN_HEIGHT, height));
}

export function readAppChatDrawerHeight(appId: string): number {
  if (typeof localStorage === "undefined") return DEFAULT_HEIGHT;
  const stored = Number(localStorage.getItem(`${STORAGE_KEY_PREFIX}${appId}`));
  return Number.isFinite(stored) && stored > 0
    ? clampAppChatDrawerHeight(stored)
    : DEFAULT_HEIGHT;
}

export function storeAppChatDrawerHeight(appId: string, height: number): number {
  const next = clampAppChatDrawerHeight(height);
  localStorage.setItem(`${STORAGE_KEY_PREFIX}${appId}`, String(next));
  return next;
}

export function mergeAppHistoryTurns(groups: LiveTurn[][]): LiveTurn[] {
  const byId = new Map<string, LiveTurn>();
  for (const turns of groups) {
    for (const turn of turns) {
      const previous = byId.get(turn.turnId);
      if (!previous || turn.updatedAt >= previous.updatedAt) {
        byId.set(turn.turnId, turn);
      }
    }
  }
  return Array.from(byId.values()).sort(
    (left, right) => left.startedAt - right.startedAt || left.updatedAt - right.updatedAt,
  );
}
