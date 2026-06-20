import { createElement } from "react";
import { BlockComponent, toGfxBlockComponent } from "@blocksuite/block-std";
import { html } from "lit";
import { classMap } from "lit/directives/class-map.js";
import { styleMap } from "lit/directives/style-map.js";
import { getBlockSuitePortalBridge } from "../../portalBridge";
import type { FileCardBlockModel } from "./model";
import { FileCardHost } from "./host";

class FileCardPageComponent extends BlockComponent<FileCardBlockModel> {
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
    const selected = Boolean(this.selected?.is("block") || this.selected?.is("surface"));
    const bridge = getBlockSuitePortalBridge();
    const content = bridge
      ? bridge.reactToLit(
          () =>
            createElement(FileCardHost, {
              title: this.model.title || "File card",
              sourcePath: this.model.sourcePath || "",
              sourceType: this.model.sourceType || "workspace_file",
              subtitle: this.model.subtitle || "",
              summary: this.model.summary || "",
              status: this.model.status || "idle",
              onPromoteToMarkdown: () => {
                bridge.promoteFileCardToMarkdown?.(this.model.id);
              },
            }),
          true,
        )
      : html`<div class="sessio-file-card-fallback">File card bridge is not ready.</div>`;

    return html`
      <div
        draggable=${this.blockDraggable ? "true" : "false"}
        class=${classMap({
          "sessio-file-card": true,
          "sessio-file-card-selected": selected,
        })}
        style=${this.containerStyleMap}
      >
        ${content}
      </div>
    `;
  }
}

export class FileCardEdgelessComponent extends toGfxBlockComponent(FileCardPageComponent) {}

if (!customElements.get("sessio-edgeless-file-card")) {
  customElements.define("sessio-edgeless-file-card", FileCardEdgelessComponent);
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
