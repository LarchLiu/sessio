import { BlockViewExtension, FlavourExtension, type ExtensionType } from "@blocksuite/block-std";
import { literal } from "lit/static-html.js";

export const MarkdownPreviewEdgelessSpec: ExtensionType[] = [
  FlavourExtension("sessio:markdown-preview"),
  BlockViewExtension("sessio:markdown-preview", literal`sessio-edgeless-markdown-preview`),
];
