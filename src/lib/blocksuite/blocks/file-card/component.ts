import { BlockComponent, toGfxBlockComponent } from "@blocksuite/std";
import { html } from "lit";
import { classMap } from "lit/directives/class-map.js";
import { styleMap } from "lit/directives/style-map.js";
import type { FileCardBlockModel } from "./model";

class FileCardPageComponent extends BlockComponent<FileCardBlockModel> {
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
          "sessio-file-card": true,
          "sessio-file-card-selected": selected,
        })}
        style=${this.containerStyleMap}
      >
        <div aria-hidden="true" style=${this.placeholderStyleMap}></div>
      </div>
    `;
  }
}

export class FileCardEdgelessComponent extends toGfxBlockComponent(FileCardPageComponent) {}

if (!customElements.get("sessio-edgeless-file-card")) {
  customElements.define("sessio-edgeless-file-card", FileCardEdgelessComponent);
}

if (import.meta.hot) {
  import.meta.hot.accept(() => {
    window.location.reload();
  });
}

declare global {
  namespace BlockSuite {
    interface BlockServices {
      "sessio:file-card": never;
    }
  }

  interface HTMLElementTagNameMap {
    "sessio-edgeless-file-card": FileCardEdgelessComponent;
  }
}
