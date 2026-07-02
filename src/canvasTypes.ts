export type CanvasBlockKind =
  | "file_card"
  | "workflow_card"
  | "note"
  | "image"
  | "group";

export type CanvasBlockSourceType =
  | "workspace_file"
  | "edited_file"
  | "attachment_image"
  | "workflow_definition"
  | "inline_markdown"
  | "note"
  | "group";

export interface CanvasDocumentInfo {
  id: string;
  sessionId: string;
  title: string;
  currentSavedRevision: number | null;
  draftSnapshotPath: string | null;
  draftSnapshotHash: string | null;
  draftUpdatedAt: number | null;
  createdAt: number;
  updatedAt: number;
}

export interface CanvasRevisionInfo {
  id: string;
  canvasId: string;
  revision: number;
  snapshotPath: string;
  snapshotHash: string;
  snapshotSizeBytes: number;
  source: string;
  createdAt: number;
}

export interface CanvasBlockRecord {
  id: string;
  canvasId: string;
  blockId: string;
  blockKind: CanvasBlockKind;
  sourceType: CanvasBlockSourceType;
  sourceKey: string | null;
  sourcePath: string | null;
  metadataJson: string;
  createdAt: number;
  updatedAt: number;
}

export interface CanvasAnchorInfo {
  id: string;
  canvasId: string;
  anchorBlockId: string | null;
  selectionBlockIdsJson: string;
  selectionElementIdsJson: string;
  turnId: string;
  summary: string | null;
  createdAt: number;
}

export interface CanvasDocumentState {
  document: CanvasDocumentInfo;
  draftSnapshot: string | null;
  savedRevision: CanvasRevisionInfo | null;
  savedSnapshot: string | null;
  blockRecords: CanvasBlockRecord[];
  anchors: CanvasAnchorInfo[];
}

export type CanvasKeyKind = "session" | "thread";

export interface CanvasKey {
  kind: CanvasKeyKind;
  id: string;
}

export interface UpsertCanvasBlockRecordInput {
  blockId: string;
  blockKind: CanvasBlockKind;
  sourceType: CanvasBlockSourceType;
  sourceKey?: string | null;
  sourcePath?: string | null;
  metadataJson?: string | null;
}

export interface SaveCanvasDraftRequest {
  canvasKey: CanvasKey;
  title?: string | null;
  snapshotJson: string;
}

export interface SaveCanvasRevisionRequest {
  canvasKey: CanvasKey;
  title?: string | null;
  snapshotJson: string;
  source: string;
}

export interface UpdateCanvasBlocksRequest {
  canvasKey: CanvasKey;
  blocks: UpsertCanvasBlockRecordInput[];
}

export interface UpsertCanvasAnchorRequest {
  canvasKey: CanvasKey;
  anchorBlockId?: string | null;
  selectionBlockIdsJson: string;
  selectionElementIdsJson: string;
  turnId: string;
  summary?: string | null;
}

export interface CanvasContextRef {
  blockId: string;
  blockKind: CanvasBlockKind;
  sourceType: string;
  sourcePath?: string | null;
  sourceKey?: string | null;
  summary?: string | null;
}

export interface CanvasContextOption {
  canvasId: string;
  scope: "canvas" | "selection" | "anchor";
  blockIds: string[];
  elementIds: string[];
  anchorId?: string | null;
  snapshotAttachmentPath?: string | null;
  refs: CanvasContextRef[];
}

export interface BuildCanvasContextFileRequest {
  canvasKey: CanvasKey;
  kind: "selection" | "workflow";
  fileNamePrefix: string;
  content: string;
}
