import { useEffect, type ReactNode } from "react";
import { Group, Panel, Separator, usePanelRef } from "react-resizable-panels";

interface AppLayoutProps {
  sidebar: ReactNode;
  header: ReactNode;
  sidebarOpen: boolean;
  onSidebarOpenChange: (open: boolean) => void;
  rightSidebar?: ReactNode;
  overlays?: ReactNode;
  children: ReactNode;
}

export default function AppLayout({
  sidebar,
  header,
  sidebarOpen,
  onSidebarOpenChange,
  rightSidebar,
  overlays,
  children,
}: AppLayoutProps) {
  const sidebarPanelRef = usePanelRef();

  useEffect(() => {
    const sidebarPanel = sidebarPanelRef.current;
    if (!sidebarPanel) return;

    if (sidebarOpen) {
      if (sidebarPanel.isCollapsed()) sidebarPanel.expand();
    } else if (!sidebarPanel.isCollapsed()) {
      sidebarPanel.collapse();
    }
  }, [sidebarOpen, sidebarPanelRef]);

  return (
    <Group className="h-screen text-body" orientation="horizontal">
      <Panel
        id="app-sidebar"
        panelRef={sidebarPanelRef}
        className="min-w-0"
        collapsible
        collapsedSize="0px"
        defaultSize="300px"
        minSize="240px"
        maxSize="520px"
        groupResizeBehavior="preserve-pixel-size"
        onResize={(size) => onSidebarOpenChange(size.inPixels > 1)}
      >
        {sidebar}
      </Panel>
      <Separator
        id="app-sidebar-resize-handle"
        aria-label="Resize sidebar"
        className="app-sidebar-resize-handle"
      />
      <Panel id="app-main" minSize="360px" className="min-w-0">
        <main className="relative flex h-full min-w-0 flex-col">
          {header}
          {children}
          {overlays}
        </main>
      </Panel>
      {rightSidebar && (
        <>
          <Separator
            id="app-right-sidebar-resize-handle"
            aria-label="Resize details sidebar"
            className="app-sidebar-resize-handle"
          />
          <Panel
            id="app-right-sidebar"
            defaultSize="320px"
            minSize="260px"
            maxSize="520px"
            groupResizeBehavior="preserve-pixel-size"
            className="min-w-0"
          >
            <aside className="h-full border-l border-ink/5 bg-surface-sidebar">
              {rightSidebar}
            </aside>
          </Panel>
        </>
      )}
    </Group>
  );
}
