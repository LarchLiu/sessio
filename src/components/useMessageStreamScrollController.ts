import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

interface ScrollCacheEntry {
  scrollTop: number;
  anchor: ScrollAnchor | null;
  atBottom: boolean;
  userMovedAwayFromBottom: boolean;
  restoreScrollTop: boolean;
}

interface ScrollAnchor {
  key: string;
  offset: number;
}

type InitialPositionMode = "bottom" | "restore" | null;

interface UseMessageStreamScrollControllerArgs {
  sourceKey: string;
  available: boolean;
  filePath: string;
  skipHistoryLoad: boolean;
  loading: boolean;
  visibleDisplayItemCount: number;
  visibleDisplayItemKeys: string[];
  liveActiveKey: string;
  liveCacheKey: string;
  initialPositionMode: InitialPositionMode;
  keepInitialBottomLock: boolean;
}

interface SnapshotContext {
  sourceKey: string;
  available: boolean;
  filePath: string;
  skipHistoryLoad: boolean;
  liveActiveKey: string;
  visibleDisplayItemKeys: string[];
}

const scrollCache = new Map<string, ScrollCacheEntry>();
// Keep the broader threshold for affordances that should preserve the old
// "near bottom" behavior, such as expanding file-edit details.
const BOTTOM_FOLLOW_THRESHOLD_PX = 24;
// Chat restoration needs a stricter pin check than the old near-bottom
// affordance, while still tolerating subpixel/layout jitter at the bottom.
const BOTTOM_PIN_THRESHOLD_PX = 16;
const PROGRAMMATIC_SCROLL_SETTLE_MS = 120;
const LAYOUT_BOTTOM_STICK_MS = 360;
const INITIAL_BOTTOM_SCROLL_RETRY_MS = 1200;

export function hasMessageStreamScrollSnapshot(sourceKey: string): boolean {
  return scrollCache.has(sourceKey);
}

export function isNearScrollBottom(vp: HTMLDivElement): boolean {
  return (
    vp.scrollTop + vp.clientHeight >=
    vp.scrollHeight - BOTTOM_FOLLOW_THRESHOLD_PX
  );
}

function getScrollBottomDistance(vp: HTMLDivElement): number {
  return Math.max(0, vp.scrollHeight - vp.clientHeight - vp.scrollTop);
}

function isPinnedToScrollBottom(vp: HTMLDivElement): boolean {
  return getScrollBottomDistance(vp) <= BOTTOM_PIN_THRESHOLD_PX;
}

function shouldRestoreToBottom(snapshot: ScrollCacheEntry): boolean {
  return !snapshot.restoreScrollTop && snapshot.atBottom;
}

export function useMessageStreamScrollController({
  sourceKey,
  available,
  filePath,
  skipHistoryLoad,
  loading,
  visibleDisplayItemCount,
  visibleDisplayItemKeys,
  liveActiveKey,
  liveCacheKey,
  initialPositionMode,
  keepInitialBottomLock,
}: UseMessageStreamScrollControllerArgs) {
  const [showScrollToBottom, setShowScrollToBottom] = useState(false);
  const [positionReady, setPositionReady] = useState(false);
  const bubbleRefs = useRef<(HTMLDivElement | null)[]>([]);
  const viewportRef = useRef<HTMLDivElement>(null);
  const chatContentRef = useRef<HTMLDivElement>(null);
  const followLiveStreamRef = useRef(false);
  const programmaticScrollUntilRef = useRef(0);
  const pendingInitialPositionRef = useRef<InitialPositionMode>(null);
  const initialPositionAppliedRef = useRef(false);
  const keepInitialBottomLockRef = useRef(false);
  const hasUserScrollIntentRef = useRef(false);
  const hasUserMovedAwayFromBottomRef = useRef(false);
  const showScrollToBottomRef = useRef(false);
  const hasShownScrollToBottomRef = useRef(false);
  const restoringScrollTopRef = useRef(false);
  const scrollToBottomTokenRef = useRef(0);
  const positionReadyFrameRef = useRef<number | null>(null);
  const lastScrollStateRef = useRef({
    scrollTop: 0,
    scrollHeight: 0,
    clientHeight: 0,
    atBottom: false,
  });
  const snapshotContextRef = useRef<SnapshotContext>({
    sourceKey,
    available,
    filePath,
    skipHistoryLoad,
    liveActiveKey,
    visibleDisplayItemKeys,
  });

  useLayoutEffect(() => {
    snapshotContextRef.current = {
      sourceKey,
      available,
      filePath,
      skipHistoryLoad,
      liveActiveKey,
      visibleDisplayItemKeys,
    };
  });

  const recordScrollState = useCallback((vp: HTMLDivElement | null = viewportRef.current) => {
    if (!vp) return;
    lastScrollStateRef.current = {
      scrollTop: vp.scrollTop,
      scrollHeight: vp.scrollHeight,
      clientHeight: vp.clientHeight,
      atBottom: isPinnedToScrollBottom(vp),
    };
  }, []);

  const beginFollowingLiveStream = useCallback(() => {
    followLiveStreamRef.current = true;
  }, []);

  const shouldShowScrollToBottomButtonForViewport = useCallback((vp: HTMLDivElement) => {
    const suppressProgrammaticScrollButton =
      performance.now() < programmaticScrollUntilRef.current &&
      !restoringScrollTopRef.current;
    const suppressBottomButton =
      (!initialPositionAppliedRef.current && !hasUserScrollIntentRef.current) ||
      keepInitialBottomLockRef.current ||
      suppressProgrammaticScrollButton;
    return !suppressBottomButton && !isPinnedToScrollBottom(vp);
  }, []);

  const scrollChatToBottom = useCallback((settleMs = PROGRAMMATIC_SCROLL_SETTLE_MS, clearUserAway = true) => {
    const token = scrollToBottomTokenRef.current;
    const scroll = () => {
      if (token !== scrollToBottomTokenRef.current) return;
      const vp = viewportRef.current;
      if (!vp) return;
      programmaticScrollUntilRef.current = performance.now() + settleMs;
      vp.scrollTop = Math.max(0, vp.scrollHeight - vp.clientHeight);
      showScrollToBottomRef.current = false;
      if (clearUserAway) {
        hasUserMovedAwayFromBottomRef.current = false;
        hasShownScrollToBottomRef.current = false;
        restoringScrollTopRef.current = false;
        scrollCache.set(snapshotContextRef.current.sourceKey, {
          scrollTop: vp.scrollTop,
          anchor: null,
          atBottom: true,
          userMovedAwayFromBottom: false,
          restoreScrollTop: false,
        });
      }
      recordScrollState(vp);
      setShowScrollToBottom(false);
    };
    scroll();
    window.requestAnimationFrame(() => {
      scroll();
      window.requestAnimationFrame(scroll);
    });
    window.setTimeout(scroll, 80);
  }, [recordScrollState]);

  const stickToBottomAfterLayoutChange = useCallback((vp: HTMLDivElement) => {
    if (restoringScrollTopRef.current) return false;
    const previous = lastScrollStateRef.current;
    const layoutChanged =
      previous.scrollHeight !== vp.scrollHeight ||
      previous.clientHeight !== vp.clientHeight;
    const isProgrammaticScroll =
      performance.now() < programmaticScrollUntilRef.current;
    const userMovedAwayFromBottom =
      hasUserScrollIntentRef.current &&
      !isPinnedToScrollBottom(vp) &&
      !isProgrammaticScroll &&
      !keepInitialBottomLockRef.current;
    if (userMovedAwayFromBottom) {
      followLiveStreamRef.current = false;
      return false;
    }
    const allowPreviousBottomStick =
      previous.atBottom &&
      (!hasUserScrollIntentRef.current || isPinnedToScrollBottom(vp));
    const shouldStickToBottom =
      initialPositionAppliedRef.current &&
      layoutChanged &&
      (allowPreviousBottomStick ||
        followLiveStreamRef.current ||
        keepInitialBottomLockRef.current ||
        isProgrammaticScroll);
    if (!shouldStickToBottom) return false;
    scrollChatToBottom(LAYOUT_BOTTOM_STICK_MS, false);
    return true;
  }, [scrollChatToBottom]);

  const setScrollToBottomButtonVisibility = useCallback((vp: HTMLDivElement | null = viewportRef.current) => {
    if (!vp) return;
    const visible = shouldShowScrollToBottomButtonForViewport(vp);
    showScrollToBottomRef.current = visible;
    if (visible) {
      hasShownScrollToBottomRef.current = true;
    }
    setShowScrollToBottom(visible);
  }, [shouldShowScrollToBottomButtonForViewport]);

  const updateScrollToBottomButton = useCallback(() => {
    const vp = viewportRef.current;
    if (!vp) return;
    if (stickToBottomAfterLayoutChange(vp)) return;
    setScrollToBottomButtonVisibility(vp);
  }, [setScrollToBottomButtonVisibility, stickToBottomAfterLayoutChange]);

  const revealPositionedContent = useCallback((afterFrame = false) => {
    if (positionReadyFrameRef.current !== null) {
      window.cancelAnimationFrame(positionReadyFrameRef.current);
      positionReadyFrameRef.current = null;
    }
    if (!afterFrame) {
      setPositionReady(true);
      return;
    }
    positionReadyFrameRef.current = window.requestAnimationFrame(() => {
      positionReadyFrameRef.current = null;
      setPositionReady(true);
      updateScrollToBottomButton();
    });
  }, [updateScrollToBottomButton]);

  const scrollChatToBottomUntilSettled = useCallback(() => {
    scrollChatToBottom(PROGRAMMATIC_SCROLL_SETTLE_MS, false);
    const start = performance.now();
    let cancelled = false;
    let frameId: number | null = null;
    let timeoutId: number | null = null;

    const tick = () => {
      if (cancelled) return;
      if (!keepInitialBottomLockRef.current) return;
      scrollChatToBottom(PROGRAMMATIC_SCROLL_SETTLE_MS, false);
      if (performance.now() - start >= INITIAL_BOTTOM_SCROLL_RETRY_MS) {
        keepInitialBottomLockRef.current = false;
        updateScrollToBottomButton();
        return;
      }
      frameId = window.requestAnimationFrame(tick);
    };

    frameId = window.requestAnimationFrame(tick);
    timeoutId = window.setTimeout(() => {
      if (cancelled) return;
      keepInitialBottomLockRef.current = false;
      updateScrollToBottomButton();
    }, INITIAL_BOTTOM_SCROLL_RETRY_MS + 120);

    return () => {
      cancelled = true;
      if (frameId !== null) window.cancelAnimationFrame(frameId);
      if (timeoutId !== null) window.clearTimeout(timeoutId);
    };
  }, [scrollChatToBottom, updateScrollToBottomButton]);

  const saveScrollSnapshot = useCallback(
    (
      vp: HTMLDivElement | null = viewportRef.current,
      cacheKey = snapshotContextRef.current.sourceKey,
    ) => {
      const snapshotContext = snapshotContextRef.current;
      setScrollToBottomButtonVisibility(vp);
      if (
        !vp ||
        !snapshotContext.available ||
        !snapshotContext.filePath ||
        snapshotContext.skipHistoryLoad ||
        (!initialPositionAppliedRef.current && !hasUserScrollIntentRef.current)
      ) {
        return;
      }
      const bottomDistance = getScrollBottomDistance(vp);
      const isProgrammaticScroll =
        performance.now() < programmaticScrollUntilRef.current;
      if (restoringScrollTopRef.current && !hasUserScrollIntentRef.current) {
        recordScrollState(vp);
        return;
      }
      const existingSnapshot = scrollCache.get(cacheKey);
      const scrollToBottomButtonVisible =
        showScrollToBottomRef.current ||
        shouldShowScrollToBottomButtonForViewport(vp);
      const movedUpSinceLastRecord =
        vp.scrollTop < lastScrollStateRef.current.scrollTop - 0.5;
      if (
        !isProgrammaticScroll &&
        bottomDistance > 0 &&
        (movedUpSinceLastRecord || hasUserScrollIntentRef.current)
      ) {
        hasUserMovedAwayFromBottomRef.current = true;
      }
      if (!isProgrammaticScroll && bottomDistance <= 0) {
        hasUserMovedAwayFromBottomRef.current = false;
      }
      const userMovedAwayFromBottom =
        bottomDistance > 0 &&
        (scrollToBottomButtonVisible ||
          hasUserMovedAwayFromBottomRef.current ||
          hasUserScrollIntentRef.current ||
          existingSnapshot?.userMovedAwayFromBottom === true);
      const atBottom =
        !userMovedAwayFromBottom &&
        bottomDistance <= BOTTOM_PIN_THRESHOLD_PX;
      const restoreScrollTop =
        hasShownScrollToBottomRef.current ||
        scrollToBottomButtonVisible ||
        (!atBottom && existingSnapshot?.restoreScrollTop === true);
      if (
        isProgrammaticScroll &&
        (existingSnapshot?.restoreScrollTop ||
          existingSnapshot?.userMovedAwayFromBottom ||
          hasUserMovedAwayFromBottomRef.current)
      ) {
        return;
      }
      if (!isProgrammaticScroll) {
        if (atBottom) {
          followLiveStreamRef.current = true;
        } else if (!snapshotContext.liveActiveKey) {
          keepInitialBottomLockRef.current = false;
          followLiveStreamRef.current = false;
        }
      }
      let anchor: ScrollAnchor | null = null;
      if (restoreScrollTop) {
        const vpRect = vp.getBoundingClientRect();
        let bestIdx = -1;
        let bestOffset = Number.NEGATIVE_INFINITY;
        let fallbackIdx = -1;
        let fallbackOffset = Number.POSITIVE_INFINITY;
        for (let i = 0; i < snapshotContext.visibleDisplayItemKeys.length; i += 1) {
          const el = bubbleRefs.current[i];
          if (!el) continue;
          const offset = el.getBoundingClientRect().top - vpRect.top;
          if (offset <= 0 && offset > bestOffset) {
            bestOffset = offset;
            bestIdx = i;
          }
          if (offset >= 0 && offset < fallbackOffset) {
            fallbackOffset = offset;
            fallbackIdx = i;
          }
        }
        const idx = bestIdx >= 0 ? bestIdx : fallbackIdx;
        if (idx >= 0) {
          const el = bubbleRefs.current[idx];
          if (el) {
            anchor = {
              key: snapshotContext.visibleDisplayItemKeys[idx],
              offset: el.getBoundingClientRect().top - vpRect.top,
            };
          }
        }
      }
      recordScrollState(vp);
      scrollCache.set(cacheKey, {
        scrollTop: vp.scrollTop,
        anchor,
        atBottom,
        userMovedAwayFromBottom,
        restoreScrollTop,
      });
    },
    [
      recordScrollState,
      setScrollToBottomButtonVisibility,
      shouldShowScrollToBottomButtonForViewport,
    ],
  );

  bubbleRefs.current.length = visibleDisplayItemCount;

  useLayoutEffect(() => {
    const cacheKey = sourceKey;
    return () => {
      // Capture the old key so session switches save the outgoing viewport
      // before the layout effect above publishes the next session context.
      if (initialPositionAppliedRef.current || hasUserScrollIntentRef.current) {
        saveScrollSnapshot(viewportRef.current, cacheKey);
      }
    };
  }, [sourceKey]);

  useLayoutEffect(() => {
    if (positionReadyFrameRef.current !== null) {
      window.cancelAnimationFrame(positionReadyFrameRef.current);
      positionReadyFrameRef.current = null;
    }
    scrollToBottomTokenRef.current += 1;
    pendingInitialPositionRef.current = initialPositionMode;
    initialPositionAppliedRef.current = false;
    setPositionReady(initialPositionMode === null);
    hasUserScrollIntentRef.current = false;
    const snapshot = scrollCache.get(sourceKey);
    hasUserMovedAwayFromBottomRef.current =
      snapshot?.userMovedAwayFromBottom === true;
    showScrollToBottomRef.current = false;
    setShowScrollToBottom(false);
    hasShownScrollToBottomRef.current = snapshot?.restoreScrollTop === true;
    restoringScrollTopRef.current =
      initialPositionMode === "restore" && snapshot?.restoreScrollTop === true;
    keepInitialBottomLockRef.current = keepInitialBottomLock;
    followLiveStreamRef.current = initialPositionMode === "bottom";
    if (!keepInitialBottomLock) {
      programmaticScrollUntilRef.current = 0;
    }
  }, [initialPositionMode, keepInitialBottomLock, sourceKey]);

  const restoreSnapshotPosition = useCallback(
    (vp: HTMLDivElement, snapshot: ScrollCacheEntry) => {
      let nextScrollTop = Math.max(
        0,
        Math.min(snapshot.scrollTop, vp.scrollHeight - vp.clientHeight),
      );
      if (snapshot.anchor) {
        const idx = visibleDisplayItemKeys.indexOf(snapshot.anchor.key);
        const el = idx >= 0 ? bubbleRefs.current[idx] : null;
        if (el) {
          const vpRect = vp.getBoundingClientRect();
          const top = el.getBoundingClientRect().top - vpRect.top + vp.scrollTop;
          nextScrollTop = Math.max(0, top - snapshot.anchor.offset);
        }
      }
      programmaticScrollUntilRef.current =
        performance.now() + PROGRAMMATIC_SCROLL_SETTLE_MS;
      vp.scrollTop = nextScrollTop;
    },
    [visibleDisplayItemKeys],
  );

  useEffect(() => {
    return () => {
      if (positionReadyFrameRef.current !== null) {
        window.cancelAnimationFrame(positionReadyFrameRef.current);
      }
    };
  }, []);

  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;
    let frameId: number | null = null;
    const syncScrollPosition = () => {
      frameId = null;
      const nextVp = viewportRef.current;
      if (!nextVp) return;
      if (hasUserScrollIntentRef.current) {
        followLiveStreamRef.current = isPinnedToScrollBottom(nextVp);
      }
      saveScrollSnapshot(nextVp);
    };
    const requestScrollSync = () => {
      if (frameId !== null) return;
      frameId = window.requestAnimationFrame(syncScrollPosition);
    };
    const handleUserScrollIntent = () => {
      hasUserScrollIntentRef.current = true;
      restoringScrollTopRef.current = false;
      hasUserMovedAwayFromBottomRef.current = !isPinnedToScrollBottom(vp);
      scrollToBottomTokenRef.current += 1;
      keepInitialBottomLockRef.current = false;
      programmaticScrollUntilRef.current = 0;
      followLiveStreamRef.current = false;
      requestScrollSync();
      window.requestAnimationFrame(() => {
        const nextVp = viewportRef.current;
        if (nextVp) setScrollToBottomButtonVisibility(nextVp);
      });
    };
    vp.addEventListener("scroll", requestScrollSync, { passive: true });
    vp.addEventListener("wheel", handleUserScrollIntent, { passive: true });
    vp.addEventListener("touchmove", handleUserScrollIntent, { passive: true });
    vp.addEventListener("keydown", handleUserScrollIntent);
    return () => {
      if (frameId !== null) window.cancelAnimationFrame(frameId);
      vp.removeEventListener("scroll", requestScrollSync);
      vp.removeEventListener("wheel", handleUserScrollIntent);
      vp.removeEventListener("touchmove", handleUserScrollIntent);
      vp.removeEventListener("keydown", handleUserScrollIntent);
    };
  }, [saveScrollSnapshot, setScrollToBottomButtonVisibility]);

  useLayoutEffect(() => {
    const vp = viewportRef.current;
    const content = chatContentRef.current;
    if (!vp || !content) return;
    recordScrollState(vp);
    let frameId: number | null = null;
    const handleResize = () => {
      if (frameId !== null) return;
      frameId = window.requestAnimationFrame(() => {
        frameId = null;
        const nextVp = viewportRef.current;
        if (!nextVp) return;
        const snapshot = scrollCache.get(sourceKey);
        if (restoringScrollTopRef.current && snapshot?.restoreScrollTop) {
          restoreSnapshotPosition(nextVp, snapshot);
          recordScrollState(nextVp);
          updateScrollToBottomButton();
          return;
        }
        if (stickToBottomAfterLayoutChange(nextVp)) return;
        recordScrollState(nextVp);
        updateScrollToBottomButton();
      });
    };
    const ro = new ResizeObserver(handleResize);
    ro.observe(vp);
    ro.observe(content);
    return () => {
      if (frameId !== null) window.cancelAnimationFrame(frameId);
      ro.disconnect();
    };
  }, [
    recordScrollState,
    restoreSnapshotPosition,
    sourceKey,
    stickToBottomAfterLayoutChange,
    updateScrollToBottomButton,
  ]);

  useLayoutEffect(() => {
    const vp = viewportRef.current;
    const mode = pendingInitialPositionRef.current;
    if (!vp || mode === null || loading) return;
    if (visibleDisplayItemCount === 0) {
      pendingInitialPositionRef.current = null;
      initialPositionAppliedRef.current = true;
      recordScrollState(vp);
      setScrollToBottomButtonVisibility(vp);
      revealPositionedContent(true);
      return;
    }
    const snapshot = scrollCache.get(sourceKey);
    if (hasUserScrollIntentRef.current) {
      followLiveStreamRef.current = isPinnedToScrollBottom(vp);
      keepInitialBottomLockRef.current = false;
      pendingInitialPositionRef.current = null;
      initialPositionAppliedRef.current = true;
      recordScrollState(vp);
      setScrollToBottomButtonVisibility(vp);
      revealPositionedContent(true);
      return;
    }
    if (mode === "restore" && snapshot?.restoreScrollTop) {
      restoringScrollTopRef.current = true;
      followLiveStreamRef.current = false;
      keepInitialBottomLockRef.current = false;
      restoreSnapshotPosition(vp, snapshot);
      pendingInitialPositionRef.current = null;
      initialPositionAppliedRef.current = true;
      recordScrollState(vp);
      setScrollToBottomButtonVisibility(vp);
      revealPositionedContent(true);
      return;
    }
    if (mode === "restore" && snapshot && shouldRestoreToBottom(snapshot)) {
      followLiveStreamRef.current = true;
      keepInitialBottomLockRef.current = true;
      scrollChatToBottom();
      pendingInitialPositionRef.current = null;
      initialPositionAppliedRef.current = true;
      recordScrollState(vp);
      revealPositionedContent(true);
      return;
    }
    if (mode === "restore" && snapshot) {
      vp.scrollTop = Math.max(
        0,
        Math.min(snapshot.scrollTop, vp.scrollHeight - vp.clientHeight),
      );
      keepInitialBottomLockRef.current = false;
      recordScrollState(vp);
    } else {
      followLiveStreamRef.current = true;
      keepInitialBottomLockRef.current = true;
      scrollChatToBottom();
    }
    pendingInitialPositionRef.current = null;
    initialPositionAppliedRef.current = true;
    recordScrollState(vp);
    revealPositionedContent(true);
  }, [
    loading,
    recordScrollState,
    revealPositionedContent,
    restoreSnapshotPosition,
    scrollChatToBottom,
    sourceKey,
    visibleDisplayItemCount,
  ]);

  useLayoutEffect(() => {
    const vp = viewportRef.current;
    if (!vp || visibleDisplayItemCount === 0 || !initialPositionAppliedRef.current) {
      return;
    }
    const snapshot = scrollCache.get(sourceKey);
    if (restoringScrollTopRef.current && snapshot?.restoreScrollTop) {
      restoreSnapshotPosition(vp, snapshot);
      recordScrollState(vp);
      setScrollToBottomButtonVisibility(vp);
      return;
    }
    if (hasUserScrollIntentRef.current && !isPinnedToScrollBottom(vp)) return;
    if (
      snapshot &&
      !shouldRestoreToBottom(snapshot) &&
      !followLiveStreamRef.current
    ) {
      return;
    }
    scrollChatToBottom();
  }, [
    recordScrollState,
    restoreSnapshotPosition,
    scrollChatToBottom,
    setScrollToBottomButtonVisibility,
    sourceKey,
    visibleDisplayItemCount,
  ]);

  useEffect(() => {
    if (!keepInitialBottomLockRef.current) return;
    if (loading || visibleDisplayItemCount === 0) return;
    return scrollChatToBottomUntilSettled();
  }, [
    loading,
    scrollChatToBottomUntilSettled,
    visibleDisplayItemCount,
  ]);

  useLayoutEffect(() => {
    if (!liveCacheKey || !followLiveStreamRef.current) return;
    keepInitialBottomLockRef.current = true;
    return scrollChatToBottomUntilSettled();
  }, [liveCacheKey, scrollChatToBottomUntilSettled]);

  useLayoutEffect(() => {
    updateScrollToBottomButton();
  }, [loading, updateScrollToBottomButton, visibleDisplayItemCount]);

  return {
    bubbleRefs,
    chatContentRef,
    viewportRef,
    showScrollToBottom,
    positionReady,
    beginFollowingLiveStream,
    saveScrollSnapshot,
    scrollChatToBottom,
  };
}
