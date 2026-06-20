import { useEffect, useRef, useState } from "react";
import {
  createBlockSuiteDoc,
  createEdgelessEditor,
  ensureEdgelessRoot,
  exportDocSnapshot,
  importDocSnapshot,
} from "./bootstrap";

export interface BlockSuiteSpikeProps {
  docId: string;
}

const STORAGE_PREFIX = "sessio:blocksuite-spike:";

export default function BlockSuiteSpike({ docId }: BlockSuiteSpikeProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const [status, setStatus] = useState("Initializing BlockSuite spike…");

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const storageKey = `${STORAGE_PREFIX}${docId}`;
    let editor: ReturnType<typeof createEdgelessEditor> | null = null;
    let activeDoc = createBlockSuiteDoc(docId).doc;

    const mountDoc = (doc: typeof activeDoc) => {
      activeDoc = doc;
      ensureEdgelessRoot(doc);
      const nextEditor = createEdgelessEditor(doc);
      editor?.remove();
      editor = nextEditor;
      host.replaceChildren(nextEditor);
    };

    const restore = async () => {
      const raw = window.localStorage.getItem(storageKey);
      if (!raw) {
        mountDoc(activeDoc);
        setStatus("Created an empty BlockSuite edgeless doc.");
        return;
      }
      try {
        const snapshot = JSON.parse(raw);
        const restored = await importDocSnapshot(snapshot);
        mountDoc(restored ?? activeDoc);
        setStatus("Restored BlockSuite snapshot from localStorage.");
      } catch (error) {
        console.error("Failed to restore BlockSuite spike snapshot", error);
        mountDoc(activeDoc);
        setStatus("Snapshot restore failed. Loaded a fresh edgeless doc.");
      }
    };

    void restore();

    const save = () => {
      const snapshot = exportDocSnapshot(activeDoc);
      if (!snapshot) return;
      window.localStorage.setItem(storageKey, JSON.stringify(snapshot));
      setStatus("Snapshot saved to localStorage.");
    };

    const handleBeforeUnload = () => save();
    window.addEventListener("beforeunload", handleBeforeUnload);

    return () => {
      save();
      window.removeEventListener("beforeunload", handleBeforeUnload);
      editor?.remove();
    };
  }, [docId]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-9 items-center justify-between border-b border-ink/8 px-3 text-caption text-ink/55">
        <span>{status}</span>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="rounded-md border border-ink/10 px-2 py-1 text-ink/70 transition hover:bg-ink/5"
            onClick={() => {
              window.localStorage.setItem(
                `${STORAGE_PREFIX}${docId}:meta`,
                JSON.stringify({
                  demoBlock: {
                    flavour: "sessio:spike-card",
                    title: "BlockSuite spike metadata",
                    savedAt: Date.now(),
                  },
                }),
              );
              setStatus("Saved a custom Sessio spike payload alongside the doc snapshot.");
            }}
          >
            Save custom payload
          </button>
          <button
            type="button"
            className="rounded-md border border-ink/10 px-2 py-1 text-ink/70 transition hover:bg-ink/5"
            onClick={() => {
              window.localStorage.removeItem(`${STORAGE_PREFIX}${docId}`);
              window.localStorage.removeItem(`${STORAGE_PREFIX}${docId}:meta`);
              window.location.reload();
            }}
          >
            Reset spike doc
          </button>
        </div>
      </div>
      <div ref={hostRef} className="min-h-0 flex-1" />
    </div>
  );
}
