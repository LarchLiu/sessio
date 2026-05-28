import type { ReactNode } from "react";

interface AppLayoutProps {
  sidebar: ReactNode;
  header: ReactNode;
  rightSidebar?: ReactNode;
  overlays?: ReactNode;
  children: ReactNode;
}

export default function AppLayout({
  sidebar,
  header,
  rightSidebar,
  overlays,
  children,
}: AppLayoutProps) {
  return (
    <div className="flex h-screen text-body">
      {sidebar}
      <main className="relative flex-1 flex flex-col min-w-0">
        {header}
        {children}
        {overlays}
      </main>
      {rightSidebar && (
        <aside className="shrink-0 border-l border-ink/5 bg-surface-sidebar">
          {rightSidebar}
        </aside>
      )}
    </div>
  );
}
