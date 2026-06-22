import type { GfxElementGeometry, GfxCompatibleProps } from "@blocksuite/std/gfx";
import { GfxCompatible } from "@blocksuite/std/gfx";
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
    renderMode: "preview",
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
  implements GfxElementGeometry
{
  get title() {
    return this.props.title;
  }

  set title(value: string) {
    this.props.title = value;
  }

  get sourcePath() {
    return this.props.sourcePath;
  }

  set sourcePath(value: string) {
    this.props.sourcePath = value;
  }

  get sourceType() {
    return this.props.sourceType;
  }

  set sourceType(value: string) {
    this.props.sourceType = value;
  }

  get excerpt() {
    return this.props.excerpt;
  }

  set excerpt(value: string) {
    this.props.excerpt = value;
  }

  get renderMode() {
    return this.props.renderMode;
  }

  set renderMode(value: MarkdownPreviewRenderMode) {
    this.props.renderMode = value;
  }

  get collapsed() {
    return this.props.collapsed;
  }

  set collapsed(value: boolean) {
    this.props.collapsed = value;
  }

  get contentVersion() {
    return this.props.contentVersion;
  }

  set contentVersion(value: string) {
    this.props.contentVersion = value;
  }

  get cachedContent() {
    return this.props.cachedContent;
  }

  set cachedContent(value: string) {
    this.props.cachedContent = value;
  }
}

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
