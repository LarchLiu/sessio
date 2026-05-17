import { useCallback, useEffect, useRef, useState } from "react";

const REPO = "LarchLiu/sessio";
const API_URL = `https://api.github.com/repos/${REPO}/releases/latest`;
const RELEASES_PAGE = `https://github.com/${REPO}/releases/latest`;
const CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000;

export interface UpdateState {
  latestVersion: string | null;
  releaseUrl: string | null;
  hasUpdate: boolean;
  checking: boolean;
  check: () => void;
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

async function fetchLatest(): Promise<{ tag: string; url: string } | null> {
  try {
    const res = await fetch(API_URL, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) return null;
    const data: { tag_name?: string; html_url?: string } = await res.json();
    if (!data.tag_name) return null;
    return { tag: data.tag_name, url: data.html_url ?? RELEASES_PAGE };
  } catch {
    return null;
  }
}

export function useUpdateCheck(current: string): UpdateState {
  const [info, setInfo] = useState<{
    latestVersion: string | null;
    releaseUrl: string | null;
    hasUpdate: boolean;
  }>({ latestVersion: null, releaseUrl: null, hasUpdate: false });
  const [checking, setChecking] = useState(false);
  const runningRef = useRef(false);

  const check = useCallback(async () => {
    if (runningRef.current) return;
    runningRef.current = true;
    setChecking(true);
    try {
      const latest = await fetchLatest();
      if (!latest) return;
      const newer = compareSemver(latest.tag, current) > 0;
      setInfo({
        latestVersion: stripV(latest.tag),
        releaseUrl: latest.url,
        hasUpdate: newer,
      });
    } finally {
      runningRef.current = false;
      setChecking(false);
    }
  }, [current]);

  useEffect(() => {
    check();
    const id = window.setInterval(check, CHECK_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [check]);

  return { ...info, checking, check };
}

export async function openReleasePage(url: string | null): Promise<void> {
  const target = url ?? RELEASES_PAGE;
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(target);
}
