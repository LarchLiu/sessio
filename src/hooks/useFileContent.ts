import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  readWorkspaceTextFile,
  unwatchPreviewFile,
  watchPreviewFile,
  type WorkspaceTextFile,
} from "../api";
import type { FileEditItem } from "../acpRenderItems";

export interface FileContentResult {
  loading: boolean;
  text: string | null;
  mtimeMs: number | null;
  path: string | null;
  contentVersion: string;
  error: string | null;
  applyLocalSave: (content: string, mtimeMs: number) => void;
}

const EMPTY_RESULT: FileContentResult = {
  loading: false,
  text: null,
  mtimeMs: null,
  path: null,
  contentVersion: "",
  error: null,
  applyLocalSave: () => {},
};

const MAX_FILE_CACHE_ENTRIES = 32;
const fileContentCache = new Map<string, WorkspaceTextFile>();

function fileContentVersion(path: string, mtimeMs: number): string {
  return `${path}:${mtimeMs}`;
}

function getCachedFileContent(path: string): WorkspaceTextFile | null {
  const cached = fileContentCache.get(path);
  if (cached === undefined) return null;
  fileContentCache.delete(path);
  fileContentCache.set(path, cached);
  return cached;
}

function setCachedFileContent(path: string, content: WorkspaceTextFile) {
  if (fileContentCache.has(path)) fileContentCache.delete(path);
  fileContentCache.set(path, content);
  while (fileContentCache.size > MAX_FILE_CACHE_ENTRIES) {
    const oldest = fileContentCache.keys().next().value;
    if (!oldest) break;
    fileContentCache.delete(oldest);
  }
}

function editInvalidationKey(edit: FileEditItem | null | undefined): string {
  if (!edit) return "";
  return [edit.path ?? "", edit.displayPath ?? ""].join(":");
}

/**
 * Resolve the editable file content for the file edit picked in Files view.
 */
export function useFileContent(
  edit: FileEditItem | null,
  workspacePath: string | null,
  reloadKey = 0,
): FileContentResult {
  const [result, setResult] = useState<FileContentResult>(EMPTY_RESULT);
  const invalidation = editInvalidationKey(edit);
  const path = edit?.path || edit?.displayPath || "";

  const applyLocalSave = useCallback((content: string, mtimeMs: number) => {
    setResult((prev) => {
      if (!prev.path) return prev;
      const file = { content, mtimeMs };
      setCachedFileContent(prev.path, file);
      return {
        ...prev,
        loading: false,
        text: content,
        mtimeMs,
        contentVersion: prev.contentVersion || fileContentVersion(prev.path, mtimeMs),
        error: null,
      };
    });
  }, []);

  useEffect(() => {
    if (!edit) {
      setResult({ ...EMPTY_RESULT, applyLocalSave });
      return;
    }

    const absolute = resolveAbsolutePath(path, workspacePath);
    if (!absolute || !workspacePath) {
      setResult({
        loading: false,
        text: null,
        mtimeMs: null,
        path: null,
        contentVersion: "",
        error: "no-path",
        applyLocalSave,
      });
      return;
    }

    let cancelled = false;
    let removeListener: (() => void) | null = null;
    let unwatchRequested = false;
    const cachedText = getCachedFileContent(absolute);
    setResult({
      loading: cachedText === null,
      text: cachedText?.content ?? null,
      mtimeMs: cachedText?.mtimeMs ?? null,
      path: absolute,
      contentVersion: cachedText ? fileContentVersion(absolute, cachedText.mtimeMs) : "",
      error: null,
      applyLocalSave,
    });

    const refresh = (showLoading: boolean) => {
      if (showLoading) {
        setResult((prev) => ({
          ...prev,
          loading: true,
          error: null,
        }));
      }
      readWorkspaceTextFile(workspacePath, absolute)
        .then((file) => {
          if (cancelled) return;
          setCachedFileContent(absolute, file);
          setResult((prev) => {
            const unchangedContent = prev.path === absolute && prev.text === file.content;
            return {
              loading: false,
              text: file.content,
              mtimeMs: file.mtimeMs,
              path: absolute,
              contentVersion: unchangedContent
                ? prev.contentVersion
                : fileContentVersion(absolute, file.mtimeMs),
              error: null,
              applyLocalSave,
            };
          });
        })
        .catch((err) => {
          if (cancelled) return;
          setResult({
            loading: false,
            text: null,
            mtimeMs: null,
            path: absolute,
            contentVersion: "",
            error: String(err),
            applyLocalSave,
          });
        });
    };

    refresh(cachedText === null);

    watchPreviewFile(absolute)
      .then(() =>
        listen<{ path: string }>("preview_file_changed", (event) => {
          if (event.payload.path !== absolute) return;
          refresh(false);
        }),
      )
      .then((unlisten) => {
        if (cancelled) {
          unlisten();
          if (!unwatchRequested) {
            unwatchRequested = true;
            void unwatchPreviewFile(absolute).catch(() => {});
          }
          return;
        }
        removeListener = unlisten;
      })
      .catch((err) => {
        if (cancelled) return;
        console.warn("watch preview file failed", err);
      });

    return () => {
      cancelled = true;
      removeListener?.();
      if (!unwatchRequested) {
        unwatchRequested = true;
        void unwatchPreviewFile(absolute).catch(() => {});
      }
    };
  }, [applyLocalSave, invalidation, path, reloadKey, workspacePath]);

  return result;
}

function resolveAbsolutePath(path: string, workspacePath: string | null): string | null {
  if (!path) return null;
  if (/^([a-zA-Z]:[\\/]|\/)/.test(path)) return path;
  if (!workspacePath) return null;
  const sep = workspacePath.includes("\\") ? "\\" : "/";
  const trimmedRoot = workspacePath.replace(/[\\/]+$/, "");
  const trimmedPath = path.replace(/^[\\/]+/, "");
  return `${trimmedRoot}${sep}${trimmedPath}`;
}

export function languageFromPath(path: string): string {
  const lower = path.toLowerCase();
  const dot = lower.lastIndexOf(".");
  if (dot < 0) return "";
  const ext = lower.slice(dot + 1);
  return ext;
}
