import type { ReactToLit } from "./reactToLit";

type BridgeState = {
  reactToLit: ReactToLit;
  workspacePath: string | null;
  updateBlock: (blockId: string, props: Record<string, unknown>) => void;
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
