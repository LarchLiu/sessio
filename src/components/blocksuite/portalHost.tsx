import type { ReactPortal } from "react";

export function PortalHost({ portals }: { portals: Array<{ id: string; portal: ReactPortal }> }) {
  return (
    <>
      {portals.map((entry) => (
        <span key={entry.id}>{entry.portal}</span>
      ))}
    </>
  );
}
