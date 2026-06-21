import { BlockViewExtension, FlavourExtension } from "@blocksuite/std";
import type { ExtensionType } from "@blocksuite/store";
import { literal } from "lit/static-html.js";

export const FileCardEdgelessSpec: ExtensionType[] = [
  FlavourExtension("sessio:file-card"),
  BlockViewExtension("sessio:file-card", literal`sessio-edgeless-file-card`),
];
