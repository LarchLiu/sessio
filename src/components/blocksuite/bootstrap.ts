import { effects as registerAffineEffects } from "@blocksuite/integration-test/effects";
import { getTestStoreManager } from "@blocksuite/integration-test/store";
import { getInternalViewExtensions } from "@blocksuite/affine/extensions/view";
import { ViewExtensionManager } from "@blocksuite/affine/ext-loader";
import { SignalWatcher, WithDisposable } from "@blocksuite/affine/global/lit";
import { noop } from "@blocksuite/global/utils";
import { ThemeProvider } from "@blocksuite/affine/shared/services";
import { BlockStdScope, EditorHost, ShadowlessElement } from "@blocksuite/affine/std";
import type { ExtensionType, Store, DocSnapshot } from "@blocksuite/store";
import { Schema, Text, nanoid } from "@blocksuite/store";
import {
  TestWorkspace,
} from "@blocksuite/store/test";
import { MarkdownTransformer } from "@blocksuite/affine/widgets/linked-doc";
import { AffineSchemas } from "@blocksuite/affine/schemas";
import { CommunityCanvasTextFonts, FontConfigExtension } from "@blocksuite/affine/shared/services";
import { css, html, nothing } from "lit";
import { property, state } from "lit/decorators.js";
import { guard } from "lit/directives/guard.js";

import {
  SessioBlockSuiteSchemas,
  SessioCustomBlockViewExtensions,
  SessioEdgelessSpecs,
  SessioStoreExtensions,
} from "../../lib/blocksuite/specs";

const BLOCKSUITE_EFFECTS_REGISTERED_KEY = "__sessio_blocksuite_effects_registered__";
const blockSuiteSchema = new Schema().register([
  ...AffineSchemas,
  ...SessioBlockSuiteSchemas,
]);
const storeManager = getTestStoreManager();
const baseViewExtensions = [
  ...getInternalViewExtensions(),
];
const viewManager = new ViewExtensionManager([
  ...baseViewExtensions,
  ...SessioCustomBlockViewExtensions,
]);
const nativeOnlyViewManager = new ViewExtensionManager(baseViewExtensions);

noop(EditorHost);

class SessioPageEditor extends SignalWatcher(
  WithDisposable(ShadowlessElement),
) {
  static override styles = css`
    page-editor {
      font-family: var(--affine-font-family);
      background: var(--affine-background-primary-color);
      display: block;
      height: 100%;
    }

    page-editor * {
      box-sizing: border-box;
    }

    .page-editor-container {
      display: block;
      height: 100%;
    }
  `;

  get host() {
    try {
      return this.std.host;
    } catch {
      return null;
    }
  }

  override connectedCallback() {
    super.connectedCallback();
    this._disposables.add(
      this.doc.slots.rootAdded.subscribe(() => this.requestUpdate()),
    );
    this.std = new BlockStdScope({
      store: this.doc,
      extensions: this.specs,
    });
  }

  override async getUpdateComplete(): Promise<boolean> {
    const result = await super.getUpdateComplete();
    await this.host?.updateComplete;
    return result;
  }

  override render() {
    if (!this.doc.root) return nothing;

    const std = this.std;
    const theme = std.get(ThemeProvider).app$.value;
    return html`
      <div data-theme=${theme} class="page-editor-container">
        ${guard([std], () => std.render())}
      </div>
    `;
  }

  override willUpdate(
    changedProperties: Map<string | number | symbol, unknown>,
  ) {
    super.willUpdate(changedProperties);
    if (this.hasUpdated && changedProperties.has("doc")) {
      this.std = new BlockStdScope({
        store: this.doc,
        extensions: this.specs,
      });
    }
  }

  @property({ attribute: false })
  accessor doc!: Store;

  @property({ attribute: false })
  accessor specs: ExtensionType[] = [];

  @state()
  accessor std!: BlockStdScope;
}

class SessioEdgelessEditor extends SignalWatcher(
  WithDisposable(ShadowlessElement),
) {
  static override styles = css`
    edgeless-editor {
      font-family: var(--affine-font-family);
      background: var(--affine-background-primary-color);
      display: block;
      height: 100%;
    }

    edgeless-editor * {
      box-sizing: border-box;
    }

    .affine-edgeless-viewport {
      display: block;
      height: 100%;
      position: relative;
      overflow: clip;
      container-name: viewport;
      container-type: inline-size;
    }
  `;

  get host() {
    try {
      return this.std.host;
    } catch {
      return null;
    }
  }

  override connectedCallback() {
    super.connectedCallback();
    this._disposables.add(
      this.doc.slots.rootAdded.subscribe(() => this.requestUpdate()),
    );
    this.std = new BlockStdScope({
      store: this.doc,
      extensions: this.specs,
    });
  }

  override async getUpdateComplete(): Promise<boolean> {
    const result = await super.getUpdateComplete();
    await this.host?.updateComplete;
    return result;
  }

  override render() {
    if (!this.doc.root) return nothing;

    const std = this.std;
    const theme = std.get(ThemeProvider).edgeless$.value;
    return html`
      <div class="affine-edgeless-viewport" data-theme=${theme}>
        ${guard([std], () => std.render())}
      </div>
    `;
  }

  override willUpdate(
    changedProperties: Map<string | number | symbol, unknown>,
  ) {
    super.willUpdate(changedProperties);
    if (this.hasUpdated && changedProperties.has("doc")) {
      this.std = new BlockStdScope({
        store: this.doc,
        extensions: this.specs,
      });
    }
  }

  @property({ attribute: false })
  accessor doc!: Store;

  @property({ attribute: false })
  accessor specs: ExtensionType[] = [];

  @state()
  accessor std!: BlockStdScope;
}

function ensureSessioEditorElementsRegistered() {
  if (!customElements.get("page-editor")) {
    customElements.define("page-editor", SessioPageEditor);
  }
  if (!customElements.get("edgeless-editor")) {
    customElements.define("edgeless-editor", SessioEdgelessEditor);
  }
}

function ensureBlockSuiteEffectsRegistered() {
  const globalScope = globalThis as typeof globalThis & {
    [BLOCKSUITE_EFFECTS_REGISTERED_KEY]?: boolean;
  };
  if (globalScope[BLOCKSUITE_EFFECTS_REGISTERED_KEY]) return;
  registerAffineEffects();
  globalScope[BLOCKSUITE_EFFECTS_REGISTERED_KEY] = true;
}

function createWorkspace(docId?: string) {
  const workspace = new TestWorkspace({
    id: docId,
    idGenerator: nanoid,
  });
  workspace.storeExtensions = [
    ...storeManager.get("store"),
    ...SessioStoreExtensions,
  ];
  workspace.meta.initialize();
  return workspace;
}

function createDocStore(workspace: TestWorkspace, docId?: string) {
  const doc = workspace.createDoc(docId);
  return doc.getStore({ id: doc.id });
}

function createEditorSpecs(
  scope: "page" | "edgeless",
  extraSpecs?: ExtensionType[],
  options?: {
    includeSessioCustomViews?: boolean;
  },
) {
  const manager = options?.includeSessioCustomViews === false
    ? nativeOnlyViewManager
    : viewManager;
  return [
    ...manager.get(scope),
    FontConfigExtension(CommunityCanvasTextFonts),
    ...(scope === "edgeless" ? SessioEdgelessSpecs : []),
    ...(extraSpecs ?? []),
  ];
}

ensureBlockSuiteEffectsRegistered();
ensureSessioEditorElementsRegistered();

export interface BlockSuiteDocHandle {
  collection: TestWorkspace;
  doc: Store;
}

export function createBlockSuiteDoc(docId?: string): BlockSuiteDocHandle {
  const collection = createWorkspace(docId);
  const doc = createDocStore(collection, docId);
  doc.load();
  return { collection, doc };
}

export function ensureEdgelessRoot(doc: BlockSuiteDocHandle["doc"]) {
  let rootId = doc.root?.id ?? null;
  if (!rootId) {
    rootId = doc.addBlock("affine:page", { title: new Text() });
  }
  const hasSurface = doc.getBlocksByFlavour("affine:surface").length > 0;
  if (!hasSurface) {
    doc.addBlock("affine:surface", {}, rootId);
  }
}

export function ensurePageRoot(doc: BlockSuiteDocHandle["doc"]) {
  if (doc.root) return;
  const rootId = doc.addBlock("affine:page", { title: new Text() });
  const noteId = doc.addBlock("affine:note", {}, rootId);
  doc.addBlock("affine:paragraph", {}, noteId);
}

export function createEdgelessEditor(doc: BlockSuiteDocHandle["doc"]) {
  return createEdgelessEditorWithSpecs(doc);
}

export function createEdgelessEditorWithSpecs(
  doc: BlockSuiteDocHandle["doc"],
  specs?: ExtensionType[],
  options?: {
    includeSessioCustomViews?: boolean;
  },
) {
  const editor = document.createElement("edgeless-editor") as HTMLElement & {
    doc: Store;
    specs: ExtensionType[];
    std?: BlockStdScope;
    host?: HTMLElement;
  };
  editor.doc = doc;
  editor.specs = createEditorSpecs("edgeless", specs, options);
  editor.style.display = "block";
  editor.style.height = "100%";
  return editor;
}

export function createPageEditor(
  doc: BlockSuiteDocHandle["doc"],
  specs?: ExtensionType[],
) {
  const editor = document.createElement("page-editor") as HTMLElement & {
    doc: Store;
    specs: ExtensionType[];
    std?: BlockStdScope;
    host?: HTMLElement;
  };
  editor.doc = doc;
  editor.specs = createEditorSpecs("page", specs);
  editor.style.display = "block";
  editor.style.height = "100%";
  return editor;
}

export function exportDocSnapshot(doc: BlockSuiteDocHandle["doc"]): DocSnapshot | null {
  const snapshot = doc.getTransformer().docToSnapshot(doc);
  return snapshot ?? null;
}

export async function createPageDocFromMarkdown(
  markdown: string,
  title?: string,
): Promise<BlockSuiteDocHandle> {
  const collection = createWorkspace();
  const normalizedMarkdown = markdown.trim();
  if (!normalizedMarkdown) {
    const doc = createDocStore(collection);
    doc.load();
    ensurePageRoot(doc);
    return { collection, doc };
  }
  const docId = await MarkdownTransformer.importMarkdownToDoc({
    collection,
    schema: blockSuiteSchema,
    markdown: normalizedMarkdown,
    fileName: title?.trim() || "Structured preview",
    extensions: createEditorSpecs("page"),
  });
  const doc = (docId ? collection.getDoc(docId)?.getStore({ id: docId }) : null) ?? createDocStore(collection);
  doc.load();
  ensurePageRoot(doc);
  return { collection, doc };
}

export async function importDocSnapshot(snapshot: DocSnapshot): Promise<BlockSuiteDocHandle["doc"] | null> {
  const collection = createWorkspace();
  const templateDoc = createDocStore(collection);
  const restored = await templateDoc.getTransformer().snapshotToDoc(snapshot);
  return restored ?? null;
}
