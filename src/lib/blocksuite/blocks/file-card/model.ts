import type { GfxElementGeometry, GfxCompatibleProps } from "@blocksuite/std/gfx";
import { GfxCompatible } from "@blocksuite/std/gfx";
import { defineBlockSchema, BlockModel } from "@blocksuite/store";

export interface FileCardBlockProps extends GfxCompatibleProps {
  title: string;
  sourcePath: string;
  sourceType: string;
  subtitle: string;
  summary: string;
  status: string;
  contentVersion: string;
}

export const DEFAULT_FILE_CARD_WIDTH = 340;
export const DEFAULT_FILE_CARD_HEIGHT = 144;

export const FileCardBlockSchema = defineBlockSchema({
  flavour: "sessio:file-card",
  props: (): FileCardBlockProps => ({
    title: "File card",
    sourcePath: "",
    sourceType: "workspace_file",
    subtitle: "",
    summary: "",
    status: "idle",
    contentVersion: "",
    index: "a0",
    xywh: `[0,0,${DEFAULT_FILE_CARD_WIDTH},${DEFAULT_FILE_CARD_HEIGHT}]`,
    lockedBySelf: false,
  }),
  metadata: {
    version: 1,
    role: "hub",
    parent: ["affine:surface"],
    children: [],
  },
  toModel: () => new FileCardBlockModel(),
});

const FileCardBlockBase = GfxCompatible<
  FileCardBlockProps,
  typeof BlockModel<FileCardBlockProps>
>(BlockModel as typeof BlockModel<FileCardBlockProps>);

export class FileCardBlockModel
  extends FileCardBlockBase
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

  get subtitle() {
    return this.props.subtitle;
  }

  set subtitle(value: string) {
    this.props.subtitle = value;
  }

  get summary() {
    return this.props.summary;
  }

  set summary(value: string) {
    this.props.summary = value;
  }

  get status() {
    return this.props.status;
  }

  set status(value: string) {
    this.props.status = value;
  }

  get contentVersion() {
    return this.props.contentVersion;
  }

  set contentVersion(value: string) {
    this.props.contentVersion = value;
  }
}

declare global {
  namespace BlockSuite {
    interface BlockModels {
      "sessio:file-card": FileCardBlockModel;
    }

    interface EdgelessBlockModelMap {
      "sessio:file-card": FileCardBlockModel;
    }
  }
}
