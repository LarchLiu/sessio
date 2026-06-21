import { createElement } from "react";
import type { MarkdownPreviewBlockModel } from "./model";
import { MarkdownPreviewHost } from "./host";
import { BlockComponent, toGfxBlockComponent } from "@blocksuite/std";
import { html } from "lit";
import { classMap } from "lit/directives/class-map.js";
import { styleMap } from "lit/directives/style-map.js";
import { getBlockSuitePortalBridge } from "../../portalBridge";

class MarkdownPreviewPageComponent extends BlockComponent<MarkdownPreviewBlockModel> {
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
    const title = this.model.title || "Markdown preview";
    const path = this.model.sourcePath || "";
    const excerpt = this.model.excerpt || "";
    const collapsed = this.model.collapsed;
    const renderMode =
      selected && !collapsed && this.model.renderMode === "preview"
        ? "preview"
        : "summary";
    const bridge = getBlockSuitePortalBridge();
    const content = bridge
      ? bridge.reactToLit(
          () =>
            createElement(MarkdownPreviewHost, {
              workspacePath: bridge.workspacePath,
              blockId: this.model.id,
              title,
              sourcePath: path,
              excerpt,
              contentVersion: this.model.contentVersion,
              renderMode,
              focused: selected,
              onToggleRenderMode: (nextMode) => {
                bridge.updateBlock(this.model.id, { renderMode: nextMode });
              },
            }),
          true,
        )
      : html`<div class="sessio-markdown-preview-excerpt">${excerpt || "Markdown portal bridge is not ready."}</div>`;

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
        <div class="sessio-markdown-preview-header">
          <div class="sessio-markdown-preview-title">${title}</div>
          <div class="sessio-markdown-preview-path">${path}</div>
        </div>
        <div class="sessio-markdown-preview-body">${content}</div>
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
