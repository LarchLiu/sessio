import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { flushSync } from "react-dom";

interface AppLayoutProps {
  sidebar: ReactNode;
  header: ReactNode;
  sidebarOpen: boolean;
  rightSidebar?: ReactNode;
  rightSidebarOpen?: boolean;
  overlays?: ReactNode;
  children: ReactNode;
}

const SIDEBAR_DEFAULT_WIDTH = 300;
const SIDEBAR_MIN_WIDTH = 240;
const SIDEBAR_MAX_WIDTH = 520;

const RIGHT_SIDEBAR_DEFAULT_WIDTH = 320;
const RIGHT_SIDEBAR_MIN_WIDTH = 260;

const MAIN_MIN_WIDTH = 360;

function clampSidebarWidth(width: number) {
  return Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, width));
}

function clampRightSidebarWidth(width: number) {
  return Math.max(RIGHT_SIDEBAR_MIN_WIDTH, width);
}

export default function AppLayout({
  sidebar,
  header,
  sidebarOpen,
  rightSidebar,
  rightSidebarOpen = false,
  overlays,
  children,
}: AppLayoutProps) {
  const [sidebarWidth, setSidebarWidth] = useState(SIDEBAR_DEFAULT_WIDTH);
  const [rightSidebarWidth, setRightSidebarWidth] = useState(
    RIGHT_SIDEBAR_DEFAULT_WIDTH,
  );
  const [dragging, setDragging] = useState(false);
  const dragStartRef = useRef<{
    side: "left" | "right";
    x: number;
    width: number;
  } | null>(null);
  const contentRowRef = useRef<HTMLDivElement | null>(null);
  const lastContentWidthRef = useRef<number | null>(null);

  useEffect(() => {
    if (!sidebarOpen) setDragging(false);
  }, [sidebarOpen]);

  useEffect(() => {
    if (!rightSidebarOpen) {
      if (dragStartRef.current?.side === "right") {
        dragStartRef.current = null;
        setDragging(false);
      }
    }
  }, [rightSidebarOpen]);

  // When the right sidebar reopens, the saved width may now exceed what the
  // current content row can give it (e.g. window shrunk while it was closed).
  // Clamp before paint so main always keeps its minimum and the right panel
  // doesn't pop in oversized and squash main to nothing.
  useLayoutEffect(() => {
    if (!rightSidebarOpen) return;
    const rowWidth =
      contentRowRef.current?.getBoundingClientRect().width ?? null;
    if (rowWidth == null) return;
    const maxAllowed = Math.max(
      RIGHT_SIDEBAR_MIN_WIDTH,
      rowWidth - MAIN_MIN_WIDTH - 1,
    );
    setRightSidebarWidth((current) =>
      current > maxAllowed ? maxAllowed : current,
    );
  }, [rightSidebarOpen]);

  // Distribute window-size changes across main and the right sidebar in the
  // same frame as the resize. window.resize fires before the browser paints,
  // and flushSync forces React to commit the new width synchronously, so there
  // is no intermediate frame where main absorbs the entire delta. We avoid
  // ResizeObserver here because it fires after layout — the user would briefly
  // see main snap to the new size before the right panel catches up.
  useEffect(() => {
    if (!rightSidebarOpen) {
      lastContentWidthRef.current = null;
      return;
    }
    const measure = () =>
      contentRowRef.current?.getBoundingClientRect().width ?? null;
    lastContentWidthRef.current = measure();
    let idleTimer: number | null = null;
    const stopSuppressing = () => {
      idleTimer = null;
      const panel = document.getElementById("app-right-sidebar");
      if (panel) panel.style.removeProperty("transition");
    };
    const onResize = () => {
      if (dragStartRef.current) return;
      const width = measure();
      if (width == null) return;
      const prev = lastContentWidthRef.current;
      lastContentWidthRef.current = width;
      if (prev == null) return;
      const delta = width - prev;
      if (Math.abs(delta) < 0.5) return;
      // Suppress the width transition for the duration of the resize burst so
      // main and the right panel update in the same frame. Inline-style is
      // used (rather than a React-controlled class) to avoid a one-frame gap
      // between state commit and DOM update.
      const panel = document.getElementById("app-right-sidebar");
      if (panel) panel.style.transition = "none";
      if (idleTimer != null) window.clearTimeout(idleTimer);
      idleTimer = window.setTimeout(stopSuppressing, 150);
      flushSync(() => {
        setRightSidebarWidth((current) => {
          // Right sidebar absorbs 60% of the window delta, main absorbs 40%.
          const target = current + delta * 0.6;
          const maxAllowed = Math.max(
            RIGHT_SIDEBAR_MIN_WIDTH,
            width - MAIN_MIN_WIDTH - 1,
          );
          return Math.min(maxAllowed, clampRightSidebarWidth(target));
        });
      });
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      if (idleTimer != null) {
        window.clearTimeout(idleTimer);
        stopSuppressing();
      }
    };
  }, [rightSidebarOpen]);

  const startSidebarResize = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (!sidebarOpen || event.button !== 0) return;
      event.currentTarget.setPointerCapture(event.pointerId);
      dragStartRef.current = {
        side: "left",
        x: event.clientX,
        width: sidebarWidth,
      };
      setDragging(true);
    },
    [sidebarOpen, sidebarWidth],
  );

  const startRightSidebarResize = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (!rightSidebarOpen || event.button !== 0) return;
      event.currentTarget.setPointerCapture(event.pointerId);
      dragStartRef.current = {
        side: "right",
        x: event.clientX,
        width: rightSidebarWidth,
      };
      setDragging(true);
    },
    [rightSidebarOpen, rightSidebarWidth],
  );

  const resizeSidebar = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const start = dragStartRef.current;
      if (!start) return;
      const delta = event.clientX - start.x;
      if (start.side === "left") {
        setSidebarWidth(clampSidebarWidth(start.width + delta));
      } else {
        // Dragging the right handle leftward should grow the right panel.
        const rowWidth =
          contentRowRef.current?.getBoundingClientRect().width ??
          Number.POSITIVE_INFINITY;
        const maxAllowed = Math.max(
          RIGHT_SIDEBAR_MIN_WIDTH,
          rowWidth - MAIN_MIN_WIDTH - 1,
        );
        const target = clampRightSidebarWidth(start.width - delta);
        setRightSidebarWidth(Math.min(maxAllowed, target));
      }
    },
    [],
  );

  const stopSidebarResize = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (!dragStartRef.current) return;
      event.currentTarget.releasePointerCapture(event.pointerId);
      dragStartRef.current = null;
      setDragging(false);
    },
    [],
  );

  const nudgeSidebarWidth = (delta: number) => {
    if (!sidebarOpen) return;
    setSidebarWidth((width) => clampSidebarWidth(width + delta));
  };

  const nudgeRightSidebarWidth = (delta: number) => {
    if (!rightSidebarOpen) return;
    setRightSidebarWidth((width) => clampRightSidebarWidth(width + delta));
  };

  const hasRight = Boolean(rightSidebar);

  return (
    <div
      className={
        "app-layout h-screen text-body " +
        (hasRight ? "app-layout-with-right-sidebar " : "") +
        (dragging ? "app-layout-resizing " : "") +
        (sidebarOpen ? "app-layout-sidebar-open " : "app-layout-sidebar-closed ") +
        (hasRight && rightSidebarOpen
          ? "app-layout-right-sidebar-open"
          : "app-layout-right-sidebar-closed")
      }
      data-sidebar-open={sidebarOpen}
      data-right-sidebar-open={rightSidebarOpen}
      style={
        {
          "--app-sidebar-width": `${sidebarWidth}px`,
          "--app-right-sidebar-width": `${rightSidebarWidth}px`,
          "--app-right-sidebar-min-width": `${RIGHT_SIDEBAR_MIN_WIDTH}px`,
          "--app-right-sidebar-content-min-width": `calc(${RIGHT_SIDEBAR_MIN_WIDTH}px - 2.5rem)`,
        } as CSSProperties
      }
    >
      <div
        id="app-sidebar"
        className="app-sidebar-panel min-h-0 min-w-0 overflow-hidden"
        aria-hidden={!sidebarOpen}
      >
        {sidebar}
      </div>
      <div
        id="app-sidebar-resize-handle"
        aria-label="Resize sidebar"
        aria-orientation="vertical"
        aria-valuemin={SIDEBAR_MIN_WIDTH}
        aria-valuemax={SIDEBAR_MAX_WIDTH}
        aria-valuenow={sidebarWidth}
        className="app-sidebar-resize-handle"
        role="separator"
        tabIndex={sidebarOpen ? 0 : -1}
        onPointerDown={startSidebarResize}
        onPointerMove={resizeSidebar}
        onPointerUp={stopSidebarResize}
        onPointerCancel={stopSidebarResize}
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft") {
            event.preventDefault();
            nudgeSidebarWidth(-16);
          } else if (event.key === "ArrowRight") {
            event.preventDefault();
            nudgeSidebarWidth(16);
          } else if (event.key === "Home") {
            event.preventDefault();
            setSidebarWidth(SIDEBAR_MIN_WIDTH);
          } else if (event.key === "End") {
            event.preventDefault();
            setSidebarWidth(SIDEBAR_MAX_WIDTH);
          }
        }}
      />
      <div
        ref={contentRowRef}
        className="app-content-row flex h-full min-h-0 min-w-0"
      >
        <main
          id="app-main"
          className="relative flex h-full min-h-0 min-w-0 flex-1 overflow-hidden flex-col"
        >
          {header}
          {children}
          {overlays}
        </main>
        {hasRight && (
          <>
            <div
              id="app-right-sidebar-resize-handle"
              aria-label="Resize details sidebar"
              aria-orientation="vertical"
              aria-valuemin={RIGHT_SIDEBAR_MIN_WIDTH}
              aria-valuenow={rightSidebarWidth}
              className="app-sidebar-resize-handle"
              role="separator"
              tabIndex={rightSidebarOpen ? 0 : -1}
              onPointerDown={startRightSidebarResize}
              onPointerMove={resizeSidebar}
              onPointerUp={stopSidebarResize}
              onPointerCancel={stopSidebarResize}
              onKeyDown={(event) => {
                if (event.key === "ArrowLeft") {
                  event.preventDefault();
                  nudgeRightSidebarWidth(16);
                } else if (event.key === "ArrowRight") {
                  event.preventDefault();
                  nudgeRightSidebarWidth(-16);
                } else if (event.key === "Home") {
                  event.preventDefault();
                  setRightSidebarWidth(RIGHT_SIDEBAR_MIN_WIDTH);
                }
              }}
            />
            <aside
              id="app-right-sidebar"
              className="app-right-sidebar-panel h-full min-h-0 min-w-0 shrink-0 overflow-hidden border-l border-ink/5 bg-surface-sidebar"
              aria-hidden={!rightSidebarOpen}
            >
              {rightSidebar}
            </aside>
          </>
        )}
      </div>
    </div>
  );
}
