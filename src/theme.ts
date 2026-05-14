import { useEffect, useRef, useState } from "react";

export type ThemeMode = "light" | "dark" | "system";

const STORAGE_KEY = "sessio.theme";

function readStored(): ThemeMode {
  if (typeof localStorage === "undefined") return "system";
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

function resolveEffective(mode: ThemeMode): "light" | "dark" {
  if (mode !== "system") return mode;
  if (typeof window === "undefined" || !window.matchMedia) return "dark";
  return window.matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

function applyTheme(effective: "light" | "dark") {
  const el = document.documentElement;
  if (effective === "light") el.setAttribute("data-theme", "light");
  else el.removeAttribute("data-theme");
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
    resolveEffective(readStored()),
  );
  const prevEffectiveRef = useRef<"light" | "dark">(effective);
  const initializedRef = useRef(false);

  useEffect(() => {
    const eff = resolveEffective(mode);
    const prev = prevEffectiveRef.current;
    setEffective(eff);
    applyThemeAnimated(eff, prev, initializedRef.current);
    prevEffectiveRef.current = eff;
    initializedRef.current = true;
    localStorage.setItem(STORAGE_KEY, mode);
  }, [mode]);

  useEffect(() => {
    if (mode !== "system" || !window.matchMedia) return;
    const mql = window.matchMedia("(prefers-color-scheme: light)");
    const onChange = () => {
      const eff = mql.matches ? "light" : "dark";
      const prev = prevEffectiveRef.current;
      setEffective(eff);
      applyThemeAnimated(eff, prev, true);
      prevEffectiveRef.current = eff;
    };
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, [mode]);

  return { mode, setMode, effective };
}
