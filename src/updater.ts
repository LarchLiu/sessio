import { useCallback, useEffect, useRef, useState } from "react";
import type { Update } from "@tauri-apps/plugin-updater";

const REPO = "LarchLiu/sessio";
const API_URL = `https://api.github.com/repos/${REPO}/releases/latest`;
const RELEASES_PAGE = `https://github.com/${REPO}/releases/latest`;
const CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000;

export interface UpdateState {
  latestVersion: string | null;
  releaseUrl: string | null;
  releaseNotes: string | null;
  hasUpdate: boolean;
  canInstall: boolean;
  checking: boolean;
  installing: boolean;
  downloadedBytes: number;
  totalBytes: number | null;
  check: () => void;
  install: () => Promise<void>;
}

function stripV(tag: string): string {
  return tag.replace(/^v/i, "").trim();
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

export function useUpdateCheck(current: string): UpdateState {
  const [info, setInfo] = useState<{
    latestVersion: string | null;
    releaseUrl: string | null;
    releaseNotes: string | null;
    hasUpdate: boolean;
    canInstall: boolean;
  }>({
    latestVersion: null,
    releaseUrl: null,
    releaseNotes: null,
    hasUpdate: false,
    canInstall: false,
  });
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [downloadedBytes, setDownloadedBytes] = useState(0);
  const [totalBytes, setTotalBytes] = useState<number | null>(null);
  const runningRef = useRef(false);
  const installRef = useRef(false);
  const updateRef = useRef<Update | null>(null);

  const check = useCallback(async () => {
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
      runningRef.current = false;
      setChecking(false);
    }
  }, [current]);

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
    setDownloadedBytes(0);
    setTotalBytes(null);
    try {
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
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } finally {
      installRef.current = false;
      setInstalling(false);
    }
  }, [info.releaseUrl]);

  return { ...info, checking, installing, downloadedBytes, totalBytes, check, install };
}

export async function openReleasePage(url: string | null): Promise<void> {
  const target = url ?? RELEASES_PAGE;
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(target);
}
