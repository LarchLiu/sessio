import { createElement } from "react";
import { BlockComponent, toGfxBlockComponent } from "@blocksuite/std";
import { Bound } from "@blocksuite/global/gfx";
import { html } from "lit";
import { classMap } from "lit/directives/class-map.js";
import { styleMap } from "lit/directives/style-map.js";
import { getBlockSuitePortalBridge } from "../../portalBridge";
import { FileCardHost } from "./host";
import type { FileCardBlockModel } from "./model";

function normalizeBridgeFileKey(
  path: string,
  workspacePath: string | null,
): string | null {
  if (!path) return null;
  const trimmed = path.trim();
  if (!trimmed) return null;
  const resolved = /^([a-zA-Z]:[\\/]|\/)/.test(trimmed)
    ? trimmed
    : workspacePath
      ? `${workspacePath.replace(/[\\/]+$/, "")}${workspacePath.includes("\\") ? "\\" : "/"}${trimmed.replace(/^[\\/]+/, "")}`
      : trimmed;
  const normalized = resolved.replace(/\\/g, "/").replace(/\/+$/, "");
  return /^[a-zA-Z]:\//.test(normalized) ? normalized.toLowerCase() : normalized;
}

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
    const selected = this.selected$.value;
    const bridge = getBlockSuitePortalBridge();
    const isLatestEditedFile = (() => {
      const key = normalizeBridgeFileKey(this.model.sourcePath || "", bridge?.workspacePath ?? null);
      return key ? bridge?.latestEditedFileKeys?.has(key) ?? false : false;
    })();
    const rerenderToken = [
      selected ? "1" : "0",
      this.model.title || "File card",
      this.model.sourcePath || "",
      this.model.sourceType || "workspace_file",
      this.model.subtitle || "",
      this.model.status || "idle",
      this.model.contentVersion || "",
      this.model.previewCollapsed ? "1" : "0",
      bridge?.workspacePath ?? "",
      isLatestEditedFile ? "1" : "0",
    ].join("\u001f");
    const content = bridge
      ? bridge.reactToLit(
          () =>
            createElement(FileCardHost, {
              workspacePath: bridge.workspacePath,
              blockId: this.model.id,
              selected,
              title: this.model.title || "File card",
              sourcePath: this.model.sourcePath || "",
              subtitle: this.model.subtitle || "",
              contentVersion: this.model.contentVersion || this.model.sourcePath || "",
              previewCollapsed: this.model.previewCollapsed,
              isLatestEditedFile,
              onTogglePreviewCollapsed: (nextCollapsed) => {
                bridge.updateBlock(this.model.id, { previewCollapsed: nextCollapsed });
              },
              onOpenFile: bridge.openProjectFile,
            }),
          rerenderToken,
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

export class FileCardEdgelessComponent extends toGfxBlockComponent(FileCardPageComponent) {
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
