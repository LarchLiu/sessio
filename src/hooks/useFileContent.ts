import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { readLocalTextFile, unwatchPreviewFile, watchPreviewFile } from "../api";
import type { FileEditItem } from "../acpRenderItems";

export interface FileContentResult {
  loading: boolean;
  text: string | null;
  error: string | null;
}

const EMPTY_RESULT: FileContentResult = {
  loading: false,
  text: null,
  error: null,
};

const MAX_FILE_CACHE_ENTRIES = 32;
const fileContentCache = new Map<string, string>();

function getCachedFileContent(path: string): string | null {
  const cached = fileContentCache.get(path);
  if (cached === undefined) return null;
  fileContentCache.delete(path);
  fileContentCache.set(path, cached);
  return cached;
}

function setCachedFileContent(path: string, text: string) {
  if (fileContentCache.has(path)) fileContentCache.delete(path);
  fileContentCache.set(path, text);
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
): FileContentResult {
  const [result, setResult] = useState<FileContentResult>(EMPTY_RESULT);
  const invalidation = editInvalidationKey(edit);
  const path = edit?.path || edit?.displayPath || "";

  useEffect(() => {
    if (!edit) {
      setResult(EMPTY_RESULT);
      return;
    }

    const absolute = resolveAbsolutePath(path, workspacePath);
    if (!absolute) {
      setResult({
        loading: false,
        text: null,
        error: "no-path",
      });
      return;
    }

    let cancelled = false;
    let removeListener: (() => void) | null = null;
    let unwatchRequested = false;
    const cachedText = getCachedFileContent(absolute);
    setResult({
      loading: cachedText === null,
      text: cachedText,
      error: null,
    });

    const refresh = (showLoading: boolean) => {
      if (showLoading) {
        setResult((prev) => ({
          ...prev,
          loading: true,
          error: null,
        }));
      }
      readLocalTextFile(absolute)
        .then((text) => {
          if (cancelled) return;
          setCachedFileContent(absolute, text);
          setResult({
            loading: false,
            text,
            error: null,
          });
        })
        .catch((err) => {
          if (cancelled) return;
          setResult({
            loading: false,
            text: null,
            error: String(err),
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
  }, [invalidation, path, workspacePath]);

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
