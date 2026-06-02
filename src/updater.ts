import { useCallback, useEffect, useRef, useState } from "react";
import type { Update } from "@tauri-apps/plugin-updater";

const REPO = "LarchLiu/sessio";
const API_URL = `https://api.github.com/repos/${REPO}/releases/latest`;
const RELEASES_PAGE = `https://github.com/${REPO}/releases/latest`;
const CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000;

const FORCE_UPDATE_PREVIEW_INFO = {
  latestVersion: "0.4.1",
  releaseUrl: "https://github.com/LarchLiu/sessio/releases/tag/v0.4.1",
  releaseNotes: String.raw`### 🚀 Features

- Enhance buildCrossPrompt to accept source metadata - by **Alex.Liu** [<samp>(0dd75)</samp>](https://github.com/LarchLiu/sessio/commit/0dd75dc)
- Add qmd-backed project memory indexing - by **Alex.Liu** [<samp>(cd8c4)</samp>](https://github.com/LarchLiu/sessio/commit/cd8c471)
- Add source excerpt resolution for memory cards - by **Alex.Liu** [<samp>(a8453)</samp>](https://github.com/LarchLiu/sessio/commit/a845350)
- Add memory source deduplication - by **Alex.Liu** [<samp>(b9a44)</samp>](https://github.com/LarchLiu/sessio/commit/b9a442d)
- Add same-project continuation dedupe with structured provenance - by **Alex.Liu** [<samp>(148e7)</samp>](https://github.com/LarchLiu/sessio/commit/148e703)
- Add continuation provenance CLI commands and gemini per-item offsets - by **Alex.Liu** [<samp>(bd2ea)</samp>](https://github.com/LarchLiu/sessio/commit/bd2eacf)
- Requeue dependent sources when base session changes - by **Alex.Liu** [<samp>(70f82)</samp>](https://github.com/LarchLiu/sessio/commit/70f8264)
- Add title field to session info and update related components - by **Alex.Liu** [<samp>(7c8a9)</samp>](https://github.com/LarchLiu/sessio/commit/7c8a9b2)
- Add session ID copy button - by **Alex.Liu** [<samp>(1461f)</samp>](https://github.com/LarchLiu/sessio/commit/1461fa7)
- Add sessions delete flow and gemini log pruning - by **Alex.Liu** [<samp>(7ede3)</samp>](https://github.com/LarchLiu/sessio/commit/7ede3a4)
- Persist backend config to ~/.sessio/config.toml - by **Alex.Liu** [<samp>(f2749)</samp>](https://github.com/LarchLiu/sessio/commit/f27497e)
- Add backend-aware search UI and reusable project picker - by **Alex.Liu** [<samp>(ccda8)</samp>](https://github.com/LarchLiu/sessio/commit/ccda8bf)
- Add copy path actions in session list/detail - by **Alex.Liu** [<samp>(be514)</samp>](https://github.com/LarchLiu/sessio/commit/be5143d)

### 🐞 Bug Fixes

- Hide unavailable sessions from the list view - by **Alex.Liu** [<samp>(48f09)</samp>](https://github.com/LarchLiu/sessio/commit/48f0999)
- Correct binary names in release workflow for consistency - by **Alex.Liu** [<samp>(9d961)</samp>](https://github.com/LarchLiu/sessio/commit/9d961b1)

### 🛠 Refactors

- Improve memory card flow summaries - by **Alex.Liu** [<samp>(87f09)</samp>](https://github.com/LarchLiu/sessio/commit/87f09b0)
- Introduce memory backend abstraction layer - by **Alex.Liu** [<samp>(73c7f)</samp>](https://github.com/LarchLiu/sessio/commit/73c7fea)
- Finish memory backend abstraction cleanup (Phase 8H) - by **Alex.Liu** [<samp>(82621)</samp>](https://github.com/LarchLiu/sessio/commit/8262115)
- Consolidate record schema and naming - by **Alex.Liu** [<samp>(46ece)</samp>](https://github.com/LarchLiu/sessio/commit/46eced2)

##### [View changes on GitHub](https://github.com/LarchLiu/sessio/compare/v0.3.2...v0.4.1)`,
};

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

export interface UpdateState {
  latestVersion: string | null;
  releaseUrl: string | null;
  releaseNotes: string | null;
  hasUpdate: boolean;
  canInstall: boolean;
  checking: boolean;
  installing: boolean;
  updateReady: boolean;
  downloadedBytes: number;
  totalBytes: number | null;
  lastCheckedAt: number | null;
  check: () => void;
  install: () => Promise<void>;
  restart: () => Promise<void>;
}

function stripV(tag: string): string {
  return tag.replace(/^v/i, "").trim();
}

export function formatVersionLabel(version: string): string {
  const normalized = version.trim();
  return normalized.toLowerCase().startsWith("v") ? normalized : `v${normalized}`;
}

function compareSemver(a: string, b: string): number {
  const pa = stripV(a).split(/[.+-]/).map((x) => Number.parseInt(x, 10));
  const pb = stripV(b).split(/[.+-]/).map((x) => Number.parseInt(x, 10));
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const va = Number.isFinite(pa[i]) ? pa[i] : 0;
    const vb = Number.isFinite(pb[i]) ? pb[i] : 0;
    if (va !== vb) return va - vb;
  }
  return 0;
}

async function fetchLatest(): Promise<{ tag: string; url: string; body: string | null } | null> {
  try {
    const res = await fetch(API_URL, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) return null;
    const data: { tag_name?: string; html_url?: string; body?: string | null } = await res.json();
    if (!data.tag_name) return null;
    return {
      tag: data.tag_name,
      url: data.html_url ?? RELEASES_PAGE,
      body: data.body?.trim() || null,
    };
  } catch {
    return null;
  }
}

async function checkInstallableUpdate(): Promise<Update | null> {
  if (import.meta.env.DEV) return null;
  try {
    const { check: checkTauriUpdate } = await import("@tauri-apps/plugin-updater");
    return await checkTauriUpdate();
  } catch (error) {
    console.warn("tauri updater check failed", error);
    return null;
  }
}

export function useUpdateCheck(current: string, updatePreview = false): UpdateState {
  const previewEnabled = import.meta.env.DEV && updatePreview;
  const [info, setInfo] = useState<{
    latestVersion: string | null;
    releaseUrl: string | null;
    releaseNotes: string | null;
    hasUpdate: boolean;
    canInstall: boolean;
  }>({
    latestVersion: previewEnabled ? FORCE_UPDATE_PREVIEW_INFO.latestVersion : null,
    releaseUrl: previewEnabled ? FORCE_UPDATE_PREVIEW_INFO.releaseUrl : null,
    releaseNotes: previewEnabled ? FORCE_UPDATE_PREVIEW_INFO.releaseNotes : null,
    hasUpdate: previewEnabled,
    canInstall: previewEnabled,
  });
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [updateReady, setUpdateReady] = useState(false);
  const [downloadedBytes, setDownloadedBytes] = useState(0);
  const [totalBytes, setTotalBytes] = useState<number | null>(null);
  const [lastCheckedAt, setLastCheckedAt] = useState<number | null>(null);
  const runningRef = useRef(false);
  const installRef = useRef(false);
  const updateRef = useRef<Update | null>(null);

  const check = useCallback(async () => {
    if (previewEnabled) {
      updateRef.current = null;
      setInfo({
        ...FORCE_UPDATE_PREVIEW_INFO,
        hasUpdate: true,
        canInstall: true,
      });
      setLastCheckedAt(Date.now());
      return;
    }
    if (runningRef.current) return;
    runningRef.current = true;
    setChecking(true);
    try {
      const tauriUpdate = await checkInstallableUpdate();
      if (tauriUpdate) {
        updateRef.current = tauriUpdate;
        setInfo({
          latestVersion: stripV(tauriUpdate.version),
          releaseUrl: null,
          releaseNotes: tauriUpdate.body?.trim() || null,
          hasUpdate: true,
          canInstall: true,
        });
        return;
      }
      updateRef.current = null;

      const latest = await fetchLatest();
      const newer = latest ? compareSemver(latest.tag, current) > 0 : false;
      setInfo({
        latestVersion: latest ? stripV(latest.tag) : null,
        releaseUrl: latest?.url ?? RELEASES_PAGE,
        releaseNotes: latest?.body ?? null,
        hasUpdate: newer,
        canInstall: false,
      });
    } finally {
      setLastCheckedAt(Date.now());
      runningRef.current = false;
      setChecking(false);
    }
  }, [current, previewEnabled]);

  useEffect(() => {
    if (previewEnabled) check();
  }, [check, previewEnabled]);

  useEffect(() => {
    if (import.meta.env.DEV) return;
    check();
    const id = window.setInterval(check, CHECK_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [check]);

  const install = useCallback(async () => {
    if (installRef.current) return;
    installRef.current = true;
    setInstalling(true);
    setUpdateReady(false);
    setDownloadedBytes(0);
    setTotalBytes(null);
    try {
      if (previewEnabled) {
        const total = 100;
        setTotalBytes(total);
        for (let downloaded = 0; downloaded <= total; downloaded += 3) {
          setDownloadedBytes(downloaded);
          await delay(80);
        }
        setDownloadedBytes(total);
        setUpdateReady(true);
        return;
      }
      let update = updateRef.current;
      if (!update) {
        update = await checkInstallableUpdate();
        updateRef.current = update;
      }
      if (!update) {
        await openReleasePage(info.releaseUrl);
        return;
      }
      let downloaded = 0;
      let total: number | null = null;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          downloaded = 0;
          total = event.data.contentLength ?? null;
          setDownloadedBytes(0);
          setTotalBytes(total);
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          setDownloadedBytes(downloaded);
        } else if (event.event === "Finished") {
          setDownloadedBytes((value) => total ?? value);
        }
      });
      setUpdateReady(true);
    } finally {
      installRef.current = false;
      setInstalling(false);
    }
  }, [info.releaseUrl, previewEnabled]);

  const restart = useCallback(async () => {
    if (previewEnabled) return;
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  }, [previewEnabled]);

  return {
    ...info,
    checking,
    installing,
    updateReady,
    downloadedBytes,
    totalBytes,
    lastCheckedAt,
    check,
    install,
    restart,
  };
}

export async function openReleasePage(url: string | null): Promise<void> {
  const target = url ?? RELEASES_PAGE;
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(target);
}
