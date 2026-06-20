import { EdgelessEditorBlockSpecs } from "@blocksuite/blocks";
import type { ExtensionType } from "@blocksuite/block-std";

import { MarkdownPreviewBlockSchema } from "./blocks/markdown-preview";
import { MarkdownPreviewEdgelessSpec } from "./blocks/markdown-preview";

export const SessioBlockSuiteSchemas = [MarkdownPreviewBlockSchema];

export const SessioEdgelessSpecs: ExtensionType[] = [
  ...EdgelessEditorBlockSpecs,
  ...MarkdownPreviewEdgelessSpec,
];
