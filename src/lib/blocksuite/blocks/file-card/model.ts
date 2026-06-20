import type { GfxElementGeometry, GfxCompatibleProps } from "@blocksuite/block-std/gfx";
import { GfxCompatible } from "@blocksuite/block-std/gfx";
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
    role: "content",
    parent: ["affine:surface"],
  },
  toModel: () => new FileCardBlockModel(),
});

const FileCardBlockBase = GfxCompatible<
  FileCardBlockProps,
  typeof BlockModel<FileCardBlockProps>
>(BlockModel as typeof BlockModel<FileCardBlockProps>);

export class FileCardBlockModel
  extends FileCardBlockBase
  implements GfxElementGeometry {}

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
