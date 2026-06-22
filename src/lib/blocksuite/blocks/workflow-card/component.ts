import { BlockComponent, toGfxBlockComponent } from "@blocksuite/std";
import { html } from "lit";
import { classMap } from "lit/directives/class-map.js";
import { styleMap } from "lit/directives/style-map.js";
import type { WorkflowCardBlockModel } from "./model";

class WorkflowCardPageComponent extends BlockComponent<WorkflowCardBlockModel> {
  blockDraggable = false;

  protected containerStyleMap = styleMap({
    position: "relative",
    width: "100%",
    height: "100%",
  });

  protected placeholderStyleMap = styleMap({
    width: "100%",
    height: "100%",
    borderRadius: "20px",
    border: "1px solid rgb(var(--color-ink) / 0.10)",
    background: "transparent",
    pointerEvents: "none",
  });

  override connectedCallback() {
    super.connectedCallback();
    this.contentEditable = "false";
  }

  override renderBlock() {
    const selected = this.selected$.value;

    return html`
      <div
        draggable=${this.blockDraggable ? "true" : "false"}
        class=${classMap({
          "sessio-workflow-card": true,
          "sessio-workflow-card-selected": selected,
        })}
        style=${this.containerStyleMap}
      >
        <div aria-hidden="true" style=${this.placeholderStyleMap}></div>
      </div>
    `;
  }
}

export class WorkflowCardEdgelessComponent extends toGfxBlockComponent(WorkflowCardPageComponent) {}

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
