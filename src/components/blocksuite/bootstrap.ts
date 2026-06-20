import "@blocksuite/affine/effects";

import type { ExtensionType } from "@blocksuite/block-std";
import { AffineEditorContainer } from "@blocksuite/presets";
import { AffineSchemas } from "@blocksuite/affine/blocks/schemas";
import {
  DocCollection,
  Job,
  Schema,
  type DocSnapshot,
} from "@blocksuite/store";
import { SessioBlockSuiteSchemas, SessioEdgelessSpecs } from "../../lib/blocksuite/specs";

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
  if (doc.root) return;
  const rootId = doc.addBlock("affine:page", {});
  doc.addBlock("affine:surface", {}, rootId);
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
  const editor = new AffineEditorContainer();
  editor.doc = doc;
  editor.mode = "edgeless";
  editor.edgelessSpecs = specs && specs.length > 0 ? specs : SessioEdgelessSpecs;
  return editor;
}

export function exportDocSnapshot(doc: BlockSuiteDocHandle["doc"]): DocSnapshot | null {
  const snapshot = new Job({ collection: doc.collection }).docToSnapshot(doc);
  return snapshot ?? null;
}

export async function importDocSnapshot(snapshot: DocSnapshot): Promise<BlockSuiteDocHandle["doc"] | null> {
  const collection = new DocCollection({ schema: blockSuiteSchema });
  collection.meta.initialize();
  const restored = await new Job({ collection }).snapshotToDoc(snapshot);
  return restored ?? null;
}
