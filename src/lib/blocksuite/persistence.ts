import type { CanvasBlockKind, CanvasBlockSourceType, UpsertCanvasBlockRecordInput } from "../../canvasTypes";
import type { MarkdownPreviewBlockModel } from "./blocks/markdown-preview";

export function markdownPreviewModelToCanvasBlock(
  model: MarkdownPreviewBlockModel,
): UpsertCanvasBlockRecordInput {
  return {
    blockId: model.id,
    blockKind: "markdown_preview" satisfies CanvasBlockKind,
    sourceType: normalizeSourceType(model.sourceType),
    sourcePath: model.sourcePath || null,
    sourceKey: model.title || null,
    metadataJson: JSON.stringify({
      kind: "markdown_preview",
      title: model.title,
      sourcePath: model.sourcePath,
      sourceType: model.sourceType,
      excerpt: model.excerpt,
      renderMode: model.renderMode,
      collapsed: model.collapsed,
      contentVersion: model.contentVersion,
    }),
  };
}

function normalizeSourceType(value: string): CanvasBlockSourceType {
  switch (value) {
    case "edited_file":
    case "workspace_file":
    case "attachment_image":
    case "workflow_definition":
    case "inline_markdown":
    case "note":
    case "group":
      return value;
    default:
      return "workspace_file";
  }
}
