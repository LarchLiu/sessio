import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type ThemeMode = "light" | "dark" | "system";

const STORAGE_KEY = "sessio.theme";

function syncWindowAppearance(effective: "light" | "dark") {
  // Force NSWindow.effectiveAppearance to match the webview theme so macOS
  // draws inactive traffic lights with a tone that contrasts the window
  // background instead of blending in. Tauri's `setTheme` does not touch
  // NSAppearance on macOS, so we have to go through a Rust command that calls
  // AppKit directly.
  invoke("set_window_appearance", { theme: effective }).catch(() => {});
}

function readStored(): ThemeMode {
  if (typeof localStorage === "undefined") return "system";
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

// Used only for the initial React state, before any NSWindow override has been
// applied — at that point webview matchMedia still reflects the real system.
function bootstrapEffective(mode: ThemeMode): "light" | "dark" {
  if (mode !== "system") return mode;
  if (typeof window === "undefined" || !window.matchMedia) return "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

// Runtime resolution: once we've overridden the window's NSAppearance, webview
// matchMedia mirrors the window, not the system, so we must ask AppKit.
async function resolveEffective(mode: ThemeMode): Promise<"light" | "dark"> {
  if (mode !== "system") return mode;
  try {
    const v = await invoke<string>("get_system_appearance");
    return v === "dark" ? "dark" : "light";
  } catch {
    return bootstrapEffective("system");
  }
}

function applyTheme(effective: "light" | "dark") {
  const el = document.documentElement;
  el.setAttribute("data-theme", effective);
  syncWindowAppearance(effective);
}

type ViewTransitionDocument = Document & {
  startViewTransition?: (cb: () => void) => { finished: Promise<void> };
};

function applyThemeAnimated(
  next: "light" | "dark",
  prev: "light" | "dark",
  animate: boolean,
) {
  const doc = document as ViewTransitionDocument;
  if (!animate || next === prev || !doc.startViewTransition) {
    applyTheme(next);
    return;
  }
  const root = document.documentElement;
  root.setAttribute("data-theme-transition", next === "light" ? "to-light" : "to-dark");
  const t = doc.startViewTransition(() => applyTheme(next));
  t.finished.finally(() => root.removeAttribute("data-theme-transition"));
}

export function useTheme() {
  const [mode, setMode] = useState<ThemeMode>(() => readStored());
  const [effective, setEffective] = useState<"light" | "dark">(() =>
    bootstrapEffective(readStored()),
  );
  const prevEffectiveRef = useRef<"light" | "dark">(effective);
  const initializedRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    resolveEffective(mode).then((eff) => {
      if (cancelled) return;
      const prev = prevEffectiveRef.current;
      setEffective(eff);
      applyThemeAnimated(eff, prev, initializedRef.current);
      prevEffectiveRef.current = eff;
      initializedRef.current = true;
      localStorage.setItem(STORAGE_KEY, mode);
    });
    return () => {
      cancelled = true;
    };
  }, [mode]);

  useEffect(() => {
    if (mode !== "system") return;
    let cancelled = false;

    const apply = (eff: "light" | "dark") => {
      if (cancelled) return;
      const prev = prevEffectiveRef.current;
      if (eff === prev) return;
      setEffective(eff);
      applyThemeAnimated(eff, prev, true);
      prevEffectiveRef.current = eff;
    };

    // macOS: matchMedia inside the webview tracks our pinned NSWindow
    // appearance, not the real system, so we rely on a Rust-side poll that
    // emits this event when AppleInterfaceStyle flips.
    const unlisten = listen<string>("system_appearance_changed", (event) => {
      apply(event.payload === "dark" ? "dark" : "light");
    });

    // Windows/Linux: we don't pin webview appearance there, so matchMedia is
    // still the right signal. Keep it as a fallback alongside the Rust event.
    const mql = window.matchMedia?.("(prefers-color-scheme: light)") ?? null;
    const onMqlChange = () => {
      if (!mql) return;
      apply(mql.matches ? "light" : "dark");
    };
    mql?.addEventListener("change", onMqlChange);

    return () => {
      cancelled = true;
      unlisten.then((f) => f()).catch(() => {});
      mql?.removeEventListener("change", onMqlChange);
    };
  }, [mode]);

  return { mode, setMode, effective };
}
