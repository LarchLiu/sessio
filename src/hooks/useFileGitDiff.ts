import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { FileEditItem } from "../acpRenderItems";
import { getFileGitDiff, type FileGitDiff, unwatchPreviewFile, watchPreviewFile } from "../api";

export interface FileGitDiffResult {
  loading: boolean;
  diff: FileGitDiff | null;
  error: string | null;
}

const EMPTY_RESULT: FileGitDiffResult = {
  loading: false,
  diff: null,
  error: null,
};

const MAX_DIFF_CACHE_ENTRIES = 32;
const diffCache = new Map<string, FileGitDiff>();

function getCachedDiff(path: string): FileGitDiff | null {
  const cached = diffCache.get(path);
  if (!cached) return null;
  diffCache.delete(path);
  diffCache.set(path, cached);
  return cached;
}

function setCachedDiff(path: string, diff: FileGitDiff) {
  if (diffCache.has(path)) diffCache.delete(path);
  diffCache.set(path, diff);
  while (diffCache.size > MAX_DIFF_CACHE_ENTRIES) {
    const oldest = diffCache.keys().next().value;
    if (!oldest) break;
    diffCache.delete(oldest);
  }
}

function editInvalidationKey(edit: FileEditItem | null | undefined): string {
  if (!edit) return "";
  return [edit.path ?? "", edit.displayPath ?? ""].join(":");
}

export function useFileGitDiff(
  edit: FileEditItem | null,
  workspacePath: string | null,
  reloadKey = 0,
): FileGitDiffResult {
  const [result, setResult] = useState<FileGitDiffResult>(EMPTY_RESULT);
  const invalidation = editInvalidationKey(edit);
  const path = edit?.path || edit?.displayPath || "";

  useEffect(() => {
    if (!edit || !workspacePath) {
      setResult(EMPTY_RESULT);
      return;
    }

    const absolute = resolveAbsolutePath(path, workspacePath);
    if (!absolute) {
      setResult({
        loading: false,
        diff: null,
        error: "no-path",
      });
      return;
    }

    let cancelled = false;
    let removeListener: (() => void) | null = null;
    let unwatchRequested = false;
    const cachedDiff = getCachedDiff(absolute);
    setResult({
      loading: cachedDiff === null,
      diff: cachedDiff,
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
      getFileGitDiff(workspacePath, absolute)
        .then((diff) => {
          if (cancelled) return;
          setCachedDiff(absolute, diff);
          setResult({
            loading: false,
            diff,
            error: null,
          });
        })
        .catch((err) => {
          if (cancelled) return;
          setResult({
            loading: false,
            diff: null,
            error: String(err),
          });
        });
    };

    refresh(cachedDiff === null || reloadKey > 0);

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
        console.warn("watch preview file for git diff failed", err);
      });

    return () => {
      cancelled = true;
      removeListener?.();
      if (!unwatchRequested) {
        unwatchRequested = true;
        void unwatchPreviewFile(absolute).catch(() => {});
      }
    };
  }, [edit, invalidation, path, reloadKey, workspacePath]);

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
