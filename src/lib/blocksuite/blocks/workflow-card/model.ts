import type { GfxElementGeometry, GfxCompatibleProps } from "@blocksuite/block-std/gfx";
import { GfxCompatible } from "@blocksuite/block-std/gfx";
import { defineBlockSchema, BlockModel } from "@blocksuite/store";

export interface WorkflowCardBlockProps extends GfxCompatibleProps {
  title: string;
  threadId: string;
  threadStageId: string;
  sourceType: string;
  workflowSummaryMarkdown: string;
  executionState: string;
  lastRunId: string;
  workflowSnapshotJson: string;
  threadGoal: string;
  status: string;
}

export const DEFAULT_WORKFLOW_CARD_WIDTH = 360;
export const DEFAULT_WORKFLOW_CARD_HEIGHT = 196;

export const WorkflowCardBlockSchema = defineBlockSchema({
  flavour: "sessio:workflow-card",
  props: (): WorkflowCardBlockProps => ({
    title: "Workflow",
    threadId: "",
    threadStageId: "",
    sourceType: "workflow_definition",
    workflowSummaryMarkdown: "",
    executionState: "idle",
    lastRunId: "",
    workflowSnapshotJson: "",
    threadGoal: "",
    status: "idle",
    index: "a0",
    xywh: `[0,0,${DEFAULT_WORKFLOW_CARD_WIDTH},${DEFAULT_WORKFLOW_CARD_HEIGHT}]`,
    lockedBySelf: false,
  }),
  metadata: {
    version: 1,
    role: "hub",
    parent: ["affine:surface"],
    children: [],
  },
  toModel: () => new WorkflowCardBlockModel(),
});

const WorkflowCardBlockBase = GfxCompatible<
  WorkflowCardBlockProps,
  typeof BlockModel<WorkflowCardBlockProps>
>(BlockModel as typeof BlockModel<WorkflowCardBlockProps>);

export class WorkflowCardBlockModel
  extends WorkflowCardBlockBase
  implements GfxElementGeometry {}

declare global {
  namespace BlockSuite {
    interface BlockModels {
      "sessio:workflow-card": WorkflowCardBlockModel;
    }

    interface EdgelessBlockModelMap {
      "sessio:workflow-card": WorkflowCardBlockModel;
    }
  }
}
