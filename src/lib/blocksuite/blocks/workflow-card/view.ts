import { BlockViewExtension, FlavourExtension } from "@blocksuite/std";
import type { ExtensionType } from "@blocksuite/store";
import { literal } from "lit/static-html.js";

export const WorkflowCardEdgelessSpec: ExtensionType[] = [
  FlavourExtension("sessio:workflow-card"),
  BlockViewExtension("sessio:workflow-card", literal`sessio-edgeless-workflow-card`),
];
