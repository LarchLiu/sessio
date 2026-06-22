import {
  BlockSchemaExtension,
  BlockSchemaIdentifier,
  type ExtensionType,
} from "@blocksuite/store";
import { ViewExtensionProvider, type ViewExtensionContext } from "@blocksuite/affine/ext-loader";
import { BlockViewExtension, FlavourExtension } from "@blocksuite/std";
import { SurfaceBlockSchema } from "@blocksuite/affine/blocks/surface";
import { literal } from "lit/static-html.js";

import { FileCardBlockSchema } from "./blocks/file-card";
import { MarkdownPreviewBlockSchema } from "./blocks/markdown-preview";
import { WorkflowCardBlockSchema } from "./blocks/workflow-card";

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

class SessioCustomBlocksViewExtension extends ViewExtensionProvider {
  override name = "sessio-custom-blocks";

  override setup(context: ViewExtensionContext) {
    super.setup(context);
    context.register([
      FlavourExtension("sessio:file-card"),
      FlavourExtension("sessio:markdown-preview"),
      FlavourExtension("sessio:workflow-card"),
    ]);

    if (this.isEdgeless(context.scope)) {
      context.register([
        BlockViewExtension("sessio:file-card", literal`sessio-edgeless-file-card`),
        BlockViewExtension("sessio:markdown-preview", literal`sessio-edgeless-markdown-preview`),
        BlockViewExtension("sessio:workflow-card", literal`sessio-edgeless-workflow-card`),
      ]);
      return;
    }

    context.register([
      BlockViewExtension("sessio:file-card", literal`sessio-edgeless-file-card`),
      BlockViewExtension("sessio:markdown-preview", literal`sessio-edgeless-markdown-preview`),
      BlockViewExtension("sessio:workflow-card", literal`sessio-edgeless-workflow-card`),
    ]);
  }
}

export const SessioBlockSuiteSchemas = [
  SessioSurfaceBlockSchema,
  MarkdownPreviewBlockSchema,
  FileCardBlockSchema,
  WorkflowCardBlockSchema,
];

export const SessioStoreExtensions: ExtensionType[] = [
  {
    setup(di) {
      di.override(
        BlockSchemaIdentifier(SessioSurfaceBlockSchema.model.flavour),
        () => SessioSurfaceBlockSchema,
      );
    },
  },
  BlockSchemaExtension(MarkdownPreviewBlockSchema),
  BlockSchemaExtension(FileCardBlockSchema),
  BlockSchemaExtension(WorkflowCardBlockSchema),
];

export const SessioCustomBlockViewExtensions = [
  SessioCustomBlocksViewExtension,
];

export const SessioEdgelessSpecs: ExtensionType[] = [];
