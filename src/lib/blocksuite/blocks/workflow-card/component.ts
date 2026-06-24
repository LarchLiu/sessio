import { createElement } from "react";
import { BlockComponent, toGfxBlockComponent } from "@blocksuite/std";
import { Bound } from "@blocksuite/global/gfx";
import { html } from "lit";
import { classMap } from "lit/directives/class-map.js";
import { styleMap } from "lit/directives/style-map.js";
import { getBlockSuitePortalBridge } from "../../portalBridge";
import { WorkflowCardHost } from "./host";
import type { WorkflowCardBlockModel } from "./model";

class WorkflowCardPageComponent extends BlockComponent<WorkflowCardBlockModel> {
  blockDraggable = true;

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
    const rerenderToken = [
      selected ? "1" : "0",
      this.model.title || "Workflow",
      this.model.threadId || "",
      this.model.threadStageId || "",
      this.model.executionState || "idle",
      this.model.lastRunId || "",
      this.model.threadGoal || "",
      this.model.workflowSummaryMarkdown || "",
    ].join("\u001f");
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
          rerenderToken,
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

export class WorkflowCardEdgelessComponent extends toGfxBlockComponent(WorkflowCardPageComponent) {
  override blockDraggable = false;

  override connectedCallback(): void {
    super.connectedCallback();
    this._disposables.add(
      this.gfx.viewport.viewportUpdated.subscribe(() => {
        this.requestUpdate();
      }),
    );
  }

  override getCSSTransform(): string {
    return "";
  }

  override getRenderingRect() {
    const viewport = this.gfx.viewport;
    const { translateX, translateY, zoom } = viewport;
    const bound = Bound.deserialize(this.model.xywh);

    return {
      x: bound.x * zoom + translateX,
      y: bound.y * zoom + translateY,
      w: bound.w * zoom,
      h: bound.h * zoom,
      zIndex: this.toZIndex(),
    };
  }
}

if (!customElements.get("sessio-edgeless-workflow-card")) {
  customElements.define("sessio-edgeless-workflow-card", WorkflowCardEdgelessComponent);
}

if (import.meta.hot) {
  import.meta.hot.accept(() => {
    window.location.reload();
  });
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
