import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { PartialBlock } from "@blocknote/core";
import type { useCreateBlockNote } from "@blocknote/react";
import { writeWorkspaceTextFile } from "../api";
import {
  normalizeEditorText,
  roundTripMatches,
  serializeSourceLineBlocks,
  type NotionParseMode,
} from "./notionSerialization";

type BlockNoteEditor = ReturnType<typeof useCreateBlockNote>;

export interface ParsedNotionDoc {
  blocks: PartialBlock[];
  usedSourceFallback: boolean;
}

export type NotionSaveStatus =
  | "clean"
  | "dirty"
  | "saving"
  | "saved"
  | "readonly"
  | "conflict"
  | "error";

export interface UseNotionDocOptions {
  fileKey: string;
  text: string;
  workspacePath: string | null;
  path: string | null;
  mtimeMs: number | null;
  contentVersion: string;
  editingLocked: boolean;
  onSaved: (content: string, mtimeMs: number) => void;
}

export interface UseNotionDocResult {
  usedSourceFallback: boolean;
  editable: boolean;
  status: NotionSaveStatus;
  messageKey: string | null;
  messageDetail: string | null;
  hasPendingChanges: () => boolean;
  flushPendingSave: () => Promise<boolean>;
}

export function useNotionDoc(
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
  }: UseNotionDocOptions,
): UseNotionDocResult {
  const [usedSourceFallback, setUsedSourceFallback] = useState(false);
  const [roundTripSafe, setRoundTripSafe] = useState(true);
  const [saveBlocked, setSaveBlocked] = useState(false);
  const [status, setStatusState] = useState<NotionSaveStatus>("clean");
  const [messageKey, setMessageKey] = useState<string | null>(null);
  const [messageDetail, setMessageDetail] = useState<string | null>(null);

  const parseModeRef = useRef<NotionParseMode>("markdown");
  const expectedMtimeRef = useRef<number | null>(mtimeMs);
  const loadedVersionRef = useRef("");
  const baselineContentRef = useRef("");
  const statusRef = useRef<NotionSaveStatus>("clean");
  const dirtyRef = useRef(false);
  const applyingBlocksRef = useRef(false);
  const canEditRef = useRef(false);
  const debounceRef = useRef<number | null>(null);
  const savingRef = useRef<Promise<boolean> | null>(null);
  const flushPendingSaveRef = useRef<() => Promise<boolean>>(async () => true);

  const setStatus = useCallback(
    (nextStatus: NotionSaveStatus, nextMessageKey: string | null = null, detail: string | null = null) => {
      statusRef.current = nextStatus;
      setStatusState(nextStatus);
      setMessageKey(nextMessageKey);
      setMessageDetail(detail);
    },
    [],
  );

  const clearDebounce = useCallback(() => {
    if (debounceRef.current !== null) {
      window.clearTimeout(debounceRef.current);
      debounceRef.current = null;
    }
  }, []);

  const scheduleSave = useCallback(() => {
    clearDebounce();
    debounceRef.current = window.setTimeout(() => {
      debounceRef.current = null;
      void flushPendingSaveRef.current();
    }, 500);
  }, [clearDebounce]);

  const effectiveVersion = contentVersion || fileKey;

  const hasPendingChanges = useCallback(
    () => dirtyRef.current || debounceRef.current !== null || savingRef.current !== null,
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
      clearDebounce();
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
    [applyParsedBlocks, clearDebounce, editor, setStatus],
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

  const flushPendingSave = useCallback(async () => {
    clearDebounce();
    if (savingRef.current) {
      return savingRef.current.then(() => {
        if (dirtyRef.current) return flushPendingSaveRef.current();
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
          scheduleSave();
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
    clearDebounce,
    onSaved,
    path,
    serializeCurrentDocument,
    setStatus,
    scheduleSave,
    workspacePath,
  ]);

  const editable = useMemo(
    () =>
      !editingLocked &&
      roundTripSafe &&
      !saveBlocked &&
      status !== "readonly" &&
      status !== "conflict" &&
      status !== "error" &&
      workspacePath !== null &&
      path !== null &&
      mtimeMs !== null,
    [editingLocked, mtimeMs, path, roundTripSafe, saveBlocked, status, workspacePath],
  );

  useEffect(() => {
    canEditRef.current = editable;
  }, [editable]);

  useEffect(() => {
    const unsubscribe = editor.onChange(() => {
      if (applyingBlocksRef.current || !canEditRef.current) return;
      dirtyRef.current = true;
      setStatus("dirty", "chat.files.editor_unsaved");
      scheduleSave();
    });
    return () => unsubscribe();
  }, [editor, scheduleSave, setStatus]);

  useEffect(() => {
    if (editingLocked && hasPendingChanges()) {
      void flushPendingSave();
    }
  }, [editingLocked, flushPendingSave, hasPendingChanges]);

  useEffect(() => {
    flushPendingSaveRef.current = flushPendingSave;
  }, [flushPendingSave]);

  useEffect(
    () => () => {
      void flushPendingSaveRef.current();
    },
    [],
  );

  return {
    usedSourceFallback,
    editable,
    status,
    messageKey,
    messageDetail,
    hasPendingChanges,
    flushPendingSave,
  };
}

export function parseMarkdownBlocksWithFallback(
  editor: BlockNoteEditor,
  text: string,
): ParsedNotionDoc {
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
