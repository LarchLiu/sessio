import { createElement } from "react";
import { BlockComponent, toGfxBlockComponent } from "@blocksuite/std";
import { html } from "lit";
import { classMap } from "lit/directives/class-map.js";
import { styleMap } from "lit/directives/style-map.js";
import { getBlockSuitePortalBridge } from "../../portalBridge";
import type { WorkflowCardBlockModel } from "./model";
import { WorkflowCardHost } from "./host";

class WorkflowCardPageComponent extends BlockComponent<WorkflowCardBlockModel> {
  blockDraggable = false;

  protected containerStyleMap = styleMap({
    position: "relative",
    width: "100%",
    height: "100%",
  });

  override connectedCallback() {
    super.connectedCallback();
    this.contentEditable = "false";
  }

  override renderBlock() {
    const selected = this.selected$.value;
    const bridge = getBlockSuitePortalBridge();
    const content = bridge
      ? bridge.reactToLit(
          () =>
            createElement(WorkflowCardHost, {
              title: this.model.title || "Workflow",
              threadId: this.model.threadId || "",
              threadStageId: this.model.threadStageId || "",
              executionState: this.model.executionState || "idle",
              lastRunId: this.model.lastRunId || "",
              threadGoal: this.model.threadGoal || "",
              workflowSummaryMarkdown: this.model.workflowSummaryMarkdown || "",
              onRunWorkflow: () => {
                bridge.runWorkflowBlock?.(this.model.id);
              },
              onOpenThread: () => {
                bridge.openWorkflowThread?.(this.model.id);
              },
            }),
          true,
        )
      : html`<div class="sessio-workflow-card-fallback">Workflow card bridge is not ready.</div>`;

    return html`
      <div
        draggable=${this.blockDraggable ? "true" : "false"}
        class=${classMap({
          "sessio-workflow-card": true,
          "sessio-workflow-card-selected": selected,
        })}
        style=${this.containerStyleMap}
      >
        ${content}
      </div>
    `;
  }
}

export class WorkflowCardEdgelessComponent extends toGfxBlockComponent(WorkflowCardPageComponent) {}

if (!customElements.get("sessio-edgeless-workflow-card")) {
  customElements.define("sessio-edgeless-workflow-card", WorkflowCardEdgelessComponent);
}

declare global {
  namespace BlockSuite {
    interface BlockServices {
      "sessio:workflow-card": never;
    }
  }

  interface HTMLElementTagNameMap {
    "sessio-edgeless-workflow-card": WorkflowCardEdgelessComponent;
  }
}
