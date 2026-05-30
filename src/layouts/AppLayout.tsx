import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";

interface AppLayoutProps {
  sidebar: ReactNode;
  header: ReactNode;
  sidebarOpen: boolean;
  rightSidebar?: ReactNode;
  overlays?: ReactNode;
  children: ReactNode;
}

const SIDEBAR_DEFAULT_WIDTH = 300;
const SIDEBAR_MIN_WIDTH = 240;
const SIDEBAR_MAX_WIDTH = 520;

function clampSidebarWidth(width: number) {
  return Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, width));
}

export default function AppLayout({
  sidebar,
  header,
  sidebarOpen,
  rightSidebar,
  overlays,
  children,
}: AppLayoutProps) {
  const [sidebarWidth, setSidebarWidth] = useState(SIDEBAR_DEFAULT_WIDTH);
  const [dragging, setDragging] = useState(false);
  const dragStartRef = useRef<{ x: number; width: number } | null>(null);

  useEffect(() => {
    if (!sidebarOpen) setDragging(false);
  }, [sidebarOpen]);

  const startSidebarResize = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (!sidebarOpen || event.button !== 0) return;
      event.currentTarget.setPointerCapture(event.pointerId);
      dragStartRef.current = { x: event.clientX, width: sidebarWidth };
      setDragging(true);
    },
    [sidebarOpen, sidebarWidth],
  );

  const resizeSidebar = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    const start = dragStartRef.current;
    if (!start) return;
    setSidebarWidth(clampSidebarWidth(start.width + event.clientX - start.x));
  }, []);

  const stopSidebarResize = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragStartRef.current) return;
    event.currentTarget.releasePointerCapture(event.pointerId);
    dragStartRef.current = null;
    setDragging(false);
  }, []);

  const nudgeSidebarWidth = (delta: number) => {
    if (!sidebarOpen) return;
    setSidebarWidth((width) => clampSidebarWidth(width + delta));
  };

  return (
    <div
      className={
        "app-layout h-screen text-body " +
        (rightSidebar ? "app-layout-with-right-sidebar " : "") +
        (dragging ? "app-layout-resizing " : "") +
        (sidebarOpen ? "app-layout-sidebar-open" : "app-layout-sidebar-closed")
      }
      data-sidebar-open={sidebarOpen}
      style={
        {
          "--app-sidebar-width": `${sidebarWidth}px`,
        } as CSSProperties
      }
    >
      <div
        id="app-sidebar"
        className="app-sidebar-panel min-w-0 overflow-hidden"
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
      <main id="app-main" className="relative flex h-full min-w-0 flex-col">
        {header}
        {children}
        {overlays}
      </main>
      {rightSidebar && (
        <>
          <div
            id="app-right-sidebar-resize-handle"
            aria-label="Resize details sidebar"
            aria-orientation="vertical"
            className="app-sidebar-resize-handle"
            role="separator"
          />
          <aside
            id="app-right-sidebar"
            className="h-full min-w-0 border-l border-ink/5 bg-surface-sidebar"
          >
            {rightSidebar}
          </aside>
        </>
      )}
    </div>
  );
}
