import type { ReactToLit } from "./reactToLit";
import type { WorkflowOverlayStore } from "./workflowLiveProjection";

type BridgeState = {
  reactToLit: ReactToLit;
  workspacePath: string | null;
  latestEditedFileKeys?: ReadonlySet<string>;
  updateBlock: (blockId: string, props: Record<string, unknown>) => void;
  workflowOverlay?: WorkflowOverlayStore;
  runWorkflowBlock?: (blockId: string) => void;
  openWorkflowThread?: (blockId: string) => void;
  openProjectFile?: (path: string) => void;
};

let currentBridge: BridgeState | null = null;

export function setBlockSuitePortalBridge(state: BridgeState | null) {
  currentBridge = state;
}

export function getBlockSuitePortalBridge() {
  return currentBridge;
}
