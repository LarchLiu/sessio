import type { MarkdownPreviewBlockModel } from "./model";
import { createElement } from "react";
import { BlockComponent, toGfxBlockComponent } from "@blocksuite/std";
import { Bound } from "@blocksuite/global/gfx";
import { html } from "lit";
import { classMap } from "lit/directives/class-map.js";
import { styleMap } from "lit/directives/style-map.js";
import { getBlockSuitePortalBridge } from "../../portalBridge";
import { MarkdownPreviewHost } from "./host";

class MarkdownPreviewPageComponent extends BlockComponent<MarkdownPreviewBlockModel> {
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
    const collapsed = this.model.collapsed;
    const title = this.model.title || "Markdown preview";
    const path = this.model.sourcePath || "";
    const excerpt = this.model.excerpt || "";
    const renderMode =
      this.model.renderMode === "preview" && !collapsed ? "preview" : "summary";
    const bridge = getBlockSuitePortalBridge();
    const rerenderToken = [
      selected ? "1" : "0",
      collapsed ? "1" : "0",
      title,
      path,
      excerpt,
      this.model.contentVersion,
      renderMode,
      bridge?.workspacePath ?? "",
    ].join("\u001f");
    const content = bridge
      ? bridge.reactToLit(
          () =>
            createElement(MarkdownPreviewHost, {
              workspacePath: bridge.workspacePath,
              blockId: this.model.id,
              selected,
              title,
              sourcePath: path,
              excerpt,
              contentVersion: this.model.contentVersion,
              renderMode,
              onToggleRenderMode: (nextMode) => {
                bridge.updateBlock(this.model.id, { renderMode: nextMode });
              },
              onOpenFile: bridge.openProjectFile,
            }),
          rerenderToken,
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
        ${content}
      </div>
    `;
  }
}

export class MarkdownPreviewEdgelessComponent extends toGfxBlockComponent(
  MarkdownPreviewPageComponent,
) {
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
