export type CanvasNodeKind = "file" | "image" | "video" | "workflow" | "note" | "group";

export type CanvasSourceType =
  | "workspace_file"
  | "edited_file"
  | "attachment_file"
  | "attachment_image"
  | "video_file"
  | "workflow_definition"
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

export interface CanvasShapeRef {
  id: string;
  canvasId: string;
  shapeId: string;
  kind: CanvasNodeKind;
  sourceType: CanvasSourceType;
  sourceKey: string | null;
  sourcePath: string | null;
  metadataJson: string;
  createdAt: number;
  updatedAt: number;
}

export interface CanvasAnchorInfo {
  id: string;
  canvasId: string;
  anchorShapeId: string | null;
  selectionShapeIdsJson: string;
  turnId: string;
  summary: string | null;
  createdAt: number;
}

export interface CanvasDocumentState {
  document: CanvasDocumentInfo;
  draftSnapshot: string | null;
  savedRevision: CanvasRevisionInfo | null;
  savedSnapshot: string | null;
  shapeRefs: CanvasShapeRef[];
  anchors: CanvasAnchorInfo[];
}

export interface UpsertCanvasShapeRefInput {
  shapeId: string;
  kind: CanvasNodeKind;
  sourceType: CanvasSourceType;
  sourceKey?: string | null;
  sourcePath?: string | null;
  metadataJson?: string | null;
}

export interface SaveCanvasDraftRequest {
  sessionId: string;
  title?: string | null;
  snapshotJson: string;
}

export interface SaveCanvasRevisionRequest {
  sessionId: string;
  title?: string | null;
  snapshotJson: string;
  source: string;
}

export interface UpdateCanvasShapeRefsRequest {
  sessionId: string;
  refs: UpsertCanvasShapeRefInput[];
}

export interface UpsertCanvasAnchorRequest {
  sessionId: string;
  anchorShapeId?: string | null;
  selectionShapeIdsJson: string;
  turnId: string;
  summary?: string | null;
}

export interface CanvasContextRef {
  shapeId: string;
  kind: CanvasNodeKind;
  sourceType: string;
  sourcePath?: string | null;
  sourceKey?: string | null;
  summary?: string | null;
}

export interface CanvasContextOption {
  canvasId: string;
  scope: "canvas" | "selection" | "anchor";
  shapeIds: string[];
  anchorId?: string | null;
  snapshotAttachmentPath?: string | null;
  refs: CanvasContextRef[];
}

export interface BuildCanvasContextFileRequest {
  sessionId: string;
  kind: "selection" | "workflow";
  fileNamePrefix: string;
  content: string;
}

