import { BlockViewExtension, FlavourExtension, type ExtensionType } from "@blocksuite/block-std";
import { literal } from "lit/static-html.js";

export const FileCardEdgelessSpec: ExtensionType[] = [
  FlavourExtension("sessio:file-card"),
  BlockViewExtension("sessio:file-card", literal`sessio-edgeless-file-card`),
];
