import { effects as registerAffineEffects } from "@blocksuite/integration-test/effects";
import { getTestStoreManager } from "@blocksuite/integration-test/store";
import { getInternalViewExtensions } from "@blocksuite/affine/extensions/view";
import { ViewExtensionManager } from "@blocksuite/affine/ext-loader";
import type { ExtensionType, Store, DocSnapshot } from "@blocksuite/store";
import { Schema, Text, nanoid } from "@blocksuite/store";
import {
  TestWorkspace,
} from "@blocksuite/store/test";
import { MarkdownTransformer } from "@blocksuite/affine/widgets/linked-doc";
import { AffineSchemas } from "@blocksuite/affine/schemas";
import { CommunityCanvasTextFonts, FontConfigExtension } from "@blocksuite/affine/shared/services";

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
const viewManager = new ViewExtensionManager([
  ...getInternalViewExtensions(),
  ...SessioCustomBlockViewExtensions,
]);

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

function createEditorSpecs(scope: "page" | "edgeless", extraSpecs?: ExtensionType[]) {
  return [
    ...viewManager.get(scope),
    FontConfigExtension(CommunityCanvasTextFonts),
    ...(scope === "edgeless" ? SessioEdgelessSpecs : []),
    ...(extraSpecs ?? []),
  ];
}

ensureBlockSuiteEffectsRegistered();

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
) {
  const editor = document.createElement("affine-editor-container") as HTMLElement & {
    doc: Store;
    mode: "page" | "edgeless";
    edgelessSpecs: ExtensionType[];
  };
  editor.doc = doc;
  editor.mode = "edgeless";
  editor.edgelessSpecs = createEditorSpecs("edgeless", specs);
  return editor;
}

export function createPageEditor(
  doc: BlockSuiteDocHandle["doc"],
  specs?: ExtensionType[],
) {
  const editor = document.createElement("affine-editor-container") as HTMLElement & {
    doc: Store;
    mode: "page" | "edgeless";
    pageSpecs: ExtensionType[];
  };
  editor.doc = doc;
  editor.mode = "page";
  editor.pageSpecs = createEditorSpecs("page", specs);
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
