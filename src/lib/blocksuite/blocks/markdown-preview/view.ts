import { BlockViewExtension, FlavourExtension } from "@blocksuite/std";
import type { ExtensionType } from "@blocksuite/store";
import { literal } from "lit/static-html.js";

export const MarkdownPreviewEdgelessSpec: ExtensionType[] = [
  FlavourExtension("sessio:markdown-preview"),
  BlockViewExtension("sessio:markdown-preview", literal`sessio-edgeless-markdown-preview`),
];
