import type { MarkdownPreviewBlockModel } from "./model";
import { BlockComponent, toGfxBlockComponent } from "@blocksuite/std";
import { html } from "lit";
import { classMap } from "lit/directives/class-map.js";
import { styleMap } from "lit/directives/style-map.js";

class MarkdownPreviewPageComponent extends BlockComponent<MarkdownPreviewBlockModel> {
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
    const collapsed = this.model.collapsed;

    return html`
      <div
        draggable=${this.blockDraggable ? "true" : "false"}
        class=${classMap({
          "sessio-markdown-preview-card": true,
          "sessio-markdown-preview-selected": selected,
          "sessio-markdown-preview-collapsed": collapsed,
        })}
        style=${this.containerStyleMap}
      >
        <div aria-hidden="true" style=${this.placeholderStyleMap}></div>
      </div>
    `;
  }
}

export class MarkdownPreviewEdgelessComponent extends toGfxBlockComponent(
  MarkdownPreviewPageComponent,
) {}

if (!customElements.get("sessio-edgeless-markdown-preview")) {
  customElements.define("sessio-edgeless-markdown-preview", MarkdownPreviewEdgelessComponent);
}

if (import.meta.hot) {
  import.meta.hot.accept(() => {
    window.location.reload();
  });
}

declare global {
  namespace BlockSuite {
    interface BlockServices {
      "sessio:markdown-preview": never;
    }
  }

  interface HTMLElementTagNameMap {
    "sessio-edgeless-markdown-preview": MarkdownPreviewEdgelessComponent;
  }
}
