import { EdgelessEditorBlockSpecs, SurfaceBlockSchema } from "@blocksuite/blocks";
import type { ExtensionType } from "@blocksuite/block-std";

import { MarkdownPreviewBlockSchema } from "./blocks/markdown-preview";
import { MarkdownPreviewEdgelessSpec } from "./blocks/markdown-preview";
import { FileCardBlockSchema } from "./blocks/file-card";
import { FileCardEdgelessSpec } from "./blocks/file-card";
import { WorkflowCardBlockSchema } from "./blocks/workflow-card";
import { WorkflowCardEdgelessSpec } from "./blocks/workflow-card";

const SessioSurfaceBlockSchema = {
  ...SurfaceBlockSchema,
  model: {
    ...SurfaceBlockSchema.model,
    children: Array.from(new Set([
      ...(SurfaceBlockSchema.model.children ?? []),
      "sessio:markdown-preview",
      "sessio:file-card",
      "sessio:workflow-card",
    ])),
  },
};

export const SessioBlockSuiteSchemas = [
  SessioSurfaceBlockSchema,
  MarkdownPreviewBlockSchema,
  FileCardBlockSchema,
  WorkflowCardBlockSchema,
];

export const SessioEdgelessSpecs: ExtensionType[] = [
  ...EdgelessEditorBlockSpecs,
  ...MarkdownPreviewEdgelessSpec,
  ...FileCardEdgelessSpec,
  ...WorkflowCardEdgelessSpec,
];
