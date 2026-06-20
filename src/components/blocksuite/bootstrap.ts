import { effects as registerAffineEffects } from "@blocksuite/affine/effects";

import type { ExtensionType } from "@blocksuite/block-std";
import { MarkdownTransformer } from "@blocksuite/blocks";
import { AffineEditorContainer, PageEditor } from "@blocksuite/presets";
import { AffineSchemas } from "@blocksuite/affine/blocks/schemas";
import {
  DocCollection,
  Job,
  Schema,
  type DocSnapshot,
} from "@blocksuite/store";
import { SessioBlockSuiteSchemas, SessioEdgelessSpecs } from "../../lib/blocksuite/specs";

const BLOCKSUITE_EFFECTS_REGISTERED_KEY = "__sessio_blocksuite_effects_registered__";

function ensureBlockSuiteEffectsRegistered() {
  const globalScope = globalThis as typeof globalThis & {
    [BLOCKSUITE_EFFECTS_REGISTERED_KEY]?: boolean;
  };
  if (globalScope[BLOCKSUITE_EFFECTS_REGISTERED_KEY]) return;
  registerAffineEffects();
  globalScope[BLOCKSUITE_EFFECTS_REGISTERED_KEY] = true;
}

ensureBlockSuiteEffectsRegistered();

const blockSuiteSchema = new Schema().register([
  ...AffineSchemas,
  ...SessioBlockSuiteSchemas,
]);

export interface BlockSuiteDocHandle {
  collection: DocCollection;
  doc: ReturnType<DocCollection["createDoc"]>;
}

export function createBlockSuiteDoc(docId?: string): BlockSuiteDocHandle {
  const collection = new DocCollection({ schema: blockSuiteSchema });
  collection.meta.initialize();
  const doc = collection.createDoc(docId ? { id: docId } : undefined);
  doc.load();
  return { collection, doc };
}

export function ensureEdgelessRoot(doc: BlockSuiteDocHandle["doc"]) {
  let rootId = doc.root?.id ?? null;
  if (!rootId) {
    rootId = doc.addBlock("affine:page", {});
  }
  const hasSurface = doc.getBlocksByFlavour("affine:surface").length > 0;
  if (!hasSurface) {
    doc.addBlock("affine:surface", {}, rootId);
  }
}

export function ensurePageRoot(doc: BlockSuiteDocHandle["doc"]) {
  if (doc.root) return;
  const rootId = doc.addBlock("affine:page", {});
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
  const editor = document.createElement("affine-editor-container") as AffineEditorContainer;
  editor.doc = doc;
  editor.mode = "edgeless";
  editor.edgelessSpecs = specs && specs.length > 0 ? specs : SessioEdgelessSpecs;
  return editor;
}

export function createPageEditor(
  doc: BlockSuiteDocHandle["doc"],
  specs?: ExtensionType[],
) {
  const editor = document.createElement("page-editor") as PageEditor;
  editor.doc = doc;
  editor.specs = specs && specs.length > 0 ? specs : editor.specs;
  editor.style.display = "block";
  editor.style.height = "100%";
  return editor;
}

export function exportDocSnapshot(doc: BlockSuiteDocHandle["doc"]): DocSnapshot | null {
  const snapshot = new Job({ collection: doc.collection }).docToSnapshot(doc);
  return snapshot ?? null;
}

export async function createPageDocFromMarkdown(
  markdown: string,
  title?: string,
): Promise<BlockSuiteDocHandle> {
  const collection = new DocCollection({ schema: blockSuiteSchema });
  collection.meta.initialize();
  const normalizedMarkdown = markdown.trim();
  if (!normalizedMarkdown) {
    const doc = collection.createDoc();
    doc.load();
    ensurePageRoot(doc);
    return { collection, doc };
  }
  const docId = await MarkdownTransformer.importMarkdownToDoc({
    collection,
    markdown: normalizedMarkdown,
    fileName: title?.trim() || "Structured preview",
  });
  const doc = (docId ? collection.getDoc(docId) : null) ?? collection.createDoc();
  doc.load();
  ensurePageRoot(doc);
  return { collection, doc };
}

export async function importDocSnapshot(snapshot: DocSnapshot): Promise<BlockSuiteDocHandle["doc"] | null> {
  const collection = new DocCollection({ schema: blockSuiteSchema });
  collection.meta.initialize();
  const restored = await new Job({ collection }).snapshotToDoc(snapshot);
  return restored ?? null;
}
