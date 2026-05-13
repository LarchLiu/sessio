import { useEffect, useState } from "react";

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

export function useTheme() {
  const [mode, setMode] = useState<ThemeMode>(() => readStored());
  const [effective, setEffective] = useState<"light" | "dark">(() =>
    resolveEffective(readStored()),
  );

  useEffect(() => {
    const eff = resolveEffective(mode);
    setEffective(eff);
    applyTheme(eff);
    localStorage.setItem(STORAGE_KEY, mode);
  }, [mode]);

  useEffect(() => {
    if (mode !== "system" || !window.matchMedia) return;
    const mql = window.matchMedia("(prefers-color-scheme: light)");
    const onChange = () => {
      const eff = mql.matches ? "light" : "dark";
      setEffective(eff);
      applyTheme(eff);
    };
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, [mode]);

  return { mode, setMode, effective };
}
