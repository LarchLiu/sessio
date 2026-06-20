import type { GfxElementGeometry, GfxCompatibleProps } from "@blocksuite/block-std/gfx";
import { GfxCompatible } from "@blocksuite/block-std/gfx";
import { defineBlockSchema, BlockModel } from "@blocksuite/store";

export type MarkdownPreviewRenderMode = "summary" | "preview";

export interface MarkdownPreviewBlockProps extends GfxCompatibleProps {
  title: string;
  sourcePath: string;
  sourceType: string;
  excerpt: string;
  renderMode: MarkdownPreviewRenderMode;
  collapsed: boolean;
  contentVersion: string;
  cachedContent: string;
}

export const DEFAULT_MARKDOWN_PREVIEW_WIDTH = 420;
export const DEFAULT_MARKDOWN_PREVIEW_HEIGHT = 260;

export const MarkdownPreviewBlockSchema = defineBlockSchema({
  flavour: "sessio:markdown-preview",
  props: (): MarkdownPreviewBlockProps => ({
    title: "Markdown preview",
    sourcePath: "",
    sourceType: "workspace_file",
    excerpt: "",
    renderMode: "summary",
    collapsed: false,
    contentVersion: "",
    cachedContent: "",
    index: "a0",
    xywh: `[0,0,${DEFAULT_MARKDOWN_PREVIEW_WIDTH},${DEFAULT_MARKDOWN_PREVIEW_HEIGHT}]`,
    lockedBySelf: false,
  }),
  metadata: {
    version: 1,
    role: "hub",
    parent: ["affine:surface"],
    children: [],
  },
  toModel: () => new MarkdownPreviewBlockModel(),
});

const MarkdownPreviewBlockBase = GfxCompatible<
  MarkdownPreviewBlockProps,
  typeof BlockModel<MarkdownPreviewBlockProps>
>(BlockModel as typeof BlockModel<MarkdownPreviewBlockProps>);

export class MarkdownPreviewBlockModel
  extends MarkdownPreviewBlockBase
  implements GfxElementGeometry {}

declare global {
  namespace BlockSuite {
    interface BlockModels {
      "sessio:markdown-preview": MarkdownPreviewBlockModel;
    }

    interface EdgelessBlockModelMap {
      "sessio:markdown-preview": MarkdownPreviewBlockModel;
    }
  }
}
