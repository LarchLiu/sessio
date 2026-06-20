import type { ReactNode } from "react";
import { createPortal } from "react-dom";

export interface PortalMount {
  id: string;
  container: Element;
  node: ReactNode;
}

export function renderPortalMounts(mounts: PortalMount[]) {
  return mounts.map((mount) => (
    <span key={mount.id}>{createPortal(mount.node, mount.container)}</span>
  ));
}
