import { EdgelessEditorBlockSpecs } from "@blocksuite/blocks";
import type { ExtensionType } from "@blocksuite/block-std";

import { MarkdownPreviewBlockSchema } from "./blocks/markdown-preview";
import { MarkdownPreviewEdgelessSpec } from "./blocks/markdown-preview";
import { FileCardBlockSchema } from "./blocks/file-card";
import { FileCardEdgelessSpec } from "./blocks/file-card";
import { WorkflowCardBlockSchema } from "./blocks/workflow-card";
import { WorkflowCardEdgelessSpec } from "./blocks/workflow-card";

export const SessioBlockSuiteSchemas = [
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
