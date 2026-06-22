import type { GfxElementGeometry, GfxCompatibleProps } from "@blocksuite/std/gfx";
import { GfxCompatible } from "@blocksuite/std/gfx";
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
  implements GfxElementGeometry
{
  get title() {
    return this.props.title;
  }

  set title(value: string) {
    this.props.title = value;
  }

  get threadId() {
    return this.props.threadId;
  }

  set threadId(value: string) {
    this.props.threadId = value;
  }

  get threadStageId() {
    return this.props.threadStageId;
  }

  set threadStageId(value: string) {
    this.props.threadStageId = value;
  }

  get sourceType() {
    return this.props.sourceType;
  }

  set sourceType(value: string) {
    this.props.sourceType = value;
  }

  get workflowSummaryMarkdown() {
    return this.props.workflowSummaryMarkdown;
  }

  set workflowSummaryMarkdown(value: string) {
    this.props.workflowSummaryMarkdown = value;
  }

  get executionState() {
    return this.props.executionState;
  }

  set executionState(value: string) {
    this.props.executionState = value;
  }

  get lastRunId() {
    return this.props.lastRunId;
  }

  set lastRunId(value: string) {
    this.props.lastRunId = value;
  }

  get workflowSnapshotJson() {
    return this.props.workflowSnapshotJson;
  }

  set workflowSnapshotJson(value: string) {
    this.props.workflowSnapshotJson = value;
  }

  get threadGoal() {
    return this.props.threadGoal;
  }

  set threadGoal(value: string) {
    this.props.threadGoal = value;
  }

  get status() {
    return this.props.status;
  }

  set status(value: string) {
    this.props.status = value;
  }
}

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
