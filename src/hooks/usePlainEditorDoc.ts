import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { PartialBlock } from "@blocknote/core";
import type { useCreateBlockNote } from "@blocknote/react";
import { writeWorkspaceTextFile } from "../api";
import {
  normalizeEditorText,
  roundTripMatches,
  serializeSourceLineBlocks,
  type PlainEditorParseMode,
} from "./plainEditorSerialization";

type BlockNoteEditor = ReturnType<typeof useCreateBlockNote>;

export interface ParsedPlainEditorDoc {
  blocks: PartialBlock[];
  usedSourceFallback: boolean;
}

export type PlainEditorSaveStatus =
  | "clean"
  | "dirty"
  | "saving"
  | "saved"
  | "readonly"
  | "conflict"
  | "error";

export interface UsePlainEditorDocOptions {
  fileKey: string;
  text: string;
  workspacePath: string | null;
  path: string | null;
  mtimeMs: number | null;
  contentVersion: string;
  editingLocked: boolean;
  onSaved: (content: string, mtimeMs: number) => void;
}

export interface UsePlainEditorDocResult {
  usedSourceFallback: boolean;
  editable: boolean;
  status: PlainEditorSaveStatus;
  messageKey: string | null;
  messageDetail: string | null;
  saveable: boolean;
  hasPendingChanges: () => boolean;
  saveNow: () => Promise<boolean>;
  canLeaveDocument: () => Promise<boolean>;
}

export function usePlainEditorDoc(
  editor: BlockNoteEditor,
  {
    fileKey,
    text,
    workspacePath,
    path,
    mtimeMs,
    contentVersion,
    editingLocked,
    onSaved,
  }: UsePlainEditorDocOptions,
): UsePlainEditorDocResult {
  const [usedSourceFallback, setUsedSourceFallback] = useState(false);
  const [roundTripSafe, setRoundTripSafe] = useState(true);
  const [saveBlocked, setSaveBlocked] = useState(false);
  const [status, setStatusState] = useState<PlainEditorSaveStatus>("clean");
  const [messageKey, setMessageKey] = useState<string | null>(null);
  const [messageDetail, setMessageDetail] = useState<string | null>(null);

  const parseModeRef = useRef<PlainEditorParseMode>("markdown");
  const expectedMtimeRef = useRef<number | null>(mtimeMs);
  const loadedVersionRef = useRef("");
  const baselineContentRef = useRef("");
  const statusRef = useRef<PlainEditorSaveStatus>("clean");
  const dirtyRef = useRef(false);
  const applyingBlocksRef = useRef(false);
  const canEditRef = useRef(false);
  const savingRef = useRef<Promise<boolean> | null>(null);
  const saveNowRef = useRef<() => Promise<boolean>>(async () => true);

  const setStatus = useCallback(
    (nextStatus: PlainEditorSaveStatus, nextMessageKey: string | null = null, detail: string | null = null) => {
      statusRef.current = nextStatus;
      setStatusState(nextStatus);
      setMessageKey(nextMessageKey);
      setMessageDetail(detail);
    },
    [],
  );

  const effectiveVersion = contentVersion || fileKey;

  const hasPendingChanges = useCallback(
    () => dirtyRef.current || savingRef.current !== null,
    [],
  );

  const applyParsedBlocks = useCallback(
    (blocks: PartialBlock[]) => {
      applyingBlocksRef.current = true;
      editor.replaceBlocks(editor.document, blocks);
      window.setTimeout(() => {
        applyingBlocksRef.current = false;
      }, 0);
    },
    [editor],
  );

  const loadDocument = useCallback(
    (nextText: string, nextVersion: string, nextMtimeMs: number | null) => {
      dirtyRef.current = false;
      setSaveBlocked(false);
      const parsed = parseMarkdownBlocksWithFallback(editor, nextText);
      parseModeRef.current = parsed.usedSourceFallback ? "source-fallback" : "markdown";
      setUsedSourceFallback(parsed.usedSourceFallback);
      applyParsedBlocks(parsed.blocks);

      const serialized = parsed.usedSourceFallback
        ? serializeSourceLineBlocks(parsed.blocks)
        : { ok: true, content: editor.blocksToMarkdownLossy(parsed.blocks) };
      const roundTrip = serialized.ok
        ? roundTripMatches(nextText, serialized.content)
        : { safe: false, serialized: "" };
      setRoundTripSafe(roundTrip.safe);
      loadedVersionRef.current = nextVersion;
      baselineContentRef.current = normalizeEditorText(nextText);
      expectedMtimeRef.current = nextMtimeMs;
      if (!roundTrip.safe) {
        setStatus("readonly", "chat.files.editor_readonly_lossy");
        return;
      }
      setStatus("clean");
    },
    [applyParsedBlocks, editor, setStatus],
  );

  useEffect(() => {
    if (!effectiveVersion) return;
    if (loadedVersionRef.current === effectiveVersion) {
      expectedMtimeRef.current = mtimeMs;
      return;
    }
    if (hasPendingChanges()) {
      setStatus("conflict", "chat.files.editor_external_change_pending");
      return;
    }
    loadDocument(text, effectiveVersion, mtimeMs);
  }, [effectiveVersion, hasPendingChanges, loadDocument, mtimeMs, setStatus, text]);

  const serializeCurrentDocument = useCallback(() => {
    if (parseModeRef.current === "source-fallback") {
      const serialized = serializeSourceLineBlocks(editor.document);
      if (!serialized.ok) {
        setSaveBlocked(true);
        dirtyRef.current = true;
        setStatus("error", "chat.files.editor_source_fallback_non_linear");
        return null;
      }
      return normalizeEditorText(serialized.content);
    }
    return normalizeEditorText(editor.blocksToMarkdownLossy(editor.document));
  }, [editor, setStatus]);

  const saveNow = useCallback(async () => {
    if (savingRef.current) {
      return savingRef.current.then(() => {
        if (dirtyRef.current) return saveNowRef.current();
        return statusRef.current !== "conflict" && statusRef.current !== "error";
      });
    }
    if (!dirtyRef.current) return statusRef.current !== "conflict" && statusRef.current !== "error";
    const serialized = serializeCurrentDocument();
    if (serialized === null) return false;
    if (serialized === baselineContentRef.current) {
      dirtyRef.current = false;
      setStatus("clean");
      return true;
    }
    if (!workspacePath || !path || expectedMtimeRef.current === null) {
      setStatus("error", "chat.files.editor_save_missing_path");
      return false;
    }

    const expectedMtimeMs = expectedMtimeRef.current;
    const savePromise = writeWorkspaceTextFile(
      workspacePath,
      path,
      serialized,
      expectedMtimeMs,
    )
      .then((result) => {
        expectedMtimeRef.current = result.mtimeMs;
        baselineContentRef.current = serialized;
        onSaved(serialized, result.mtimeMs);
        const latest = serializeCurrentDocument();
        if (latest === null) return false;
        if (latest !== serialized) {
          dirtyRef.current = true;
          setStatus("dirty", "chat.files.editor_unsaved");
          return false;
        }
        dirtyRef.current = false;
        setStatus("saved", "chat.files.editor_saved");
        return true;
      })
      .catch((err) => {
        const detail = String(err);
        if (detail.toLowerCase().includes("changed on disk")) {
          setStatus("conflict", "chat.files.editor_mtime_conflict", detail);
        } else {
          setStatus("error", "chat.files.editor_save_failed", detail);
        }
        return false;
      })
      .finally(() => {
        savingRef.current = null;
      });

    savingRef.current = savePromise;
    setStatus("saving", "chat.files.editor_saving");
    return savePromise;
  }, [
    onSaved,
    path,
    serializeCurrentDocument,
    setStatus,
    workspacePath,
  ]);

  const canLeaveDocument = useCallback(async () => {
    if (savingRef.current) return savingRef.current;
    if (!dirtyRef.current) {
      return statusRef.current !== "conflict" && statusRef.current !== "error";
    }
    setStatus("dirty", "chat.files.editor_leave_blocked_unsaved");
    return false;
  }, [setStatus]);

  const saveable = useMemo(
    () =>
      roundTripSafe &&
      !saveBlocked &&
      status !== "readonly" &&
      status !== "conflict" &&
      status !== "error" &&
      workspacePath !== null &&
      path !== null &&
      mtimeMs !== null,
    [mtimeMs, path, roundTripSafe, saveBlocked, status, workspacePath],
  );

  const editable = useMemo(
    () => !editingLocked && saveable,
    [editingLocked, saveable],
  );

  useEffect(() => {
    canEditRef.current = editable;
  }, [editable]);

  useEffect(() => {
    const unsubscribe = editor.onChange(() => {
      if (applyingBlocksRef.current || !canEditRef.current) return;
      dirtyRef.current = true;
      setStatus("dirty", "chat.files.editor_unsaved");
    });
    return () => unsubscribe();
  }, [editor, setStatus]);

  useEffect(() => {
    saveNowRef.current = saveNow;
  }, [saveNow]);

  return {
    usedSourceFallback,
    editable,
    status,
    messageKey,
    messageDetail,
    saveable,
    hasPendingChanges,
    saveNow,
    canLeaveDocument,
  };
}

export function parseMarkdownBlocksWithFallback(
  editor: BlockNoteEditor,
  text: string,
): ParsedPlainEditorDoc {
  try {
    const blocks = editor.tryParseMarkdownToBlocks(text);
    if (blocks.length > 0 || text.length === 0) {
      return {
        blocks: blocks.length > 0 ? blocks : [buildSourceLineBlock("")],
        usedSourceFallback: false,
      };
    }
  } catch {
    // Fall through to source-line blocks below.
  }

  return {
    blocks: buildSourceLineBlocks(text),
    usedSourceFallback: true,
  };
}

function buildSourceLineBlocks(text: string): PartialBlock[] {
  const lines = text.split("\n");
  return lines.map(buildSourceLineBlock);
}

function buildSourceLineBlock(line: string): PartialBlock {
  return {
    type: "paragraph",
    content: line
      ? [
          {
            type: "text",
            text: line,
            styles: {},
          },
        ]
      : [],
    children: [],
  };
}
