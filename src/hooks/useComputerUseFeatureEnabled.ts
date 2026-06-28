import { useEffect, useState } from "react";
import type { ComputerUseSettings } from "../api";
import { getComputerUseSettings } from "../api";
import { COMPUTER_USE_SETTINGS_CHANGED_EVENT } from "../computerUseSettingsEvents";

export function useComputerUseFeatureEnabled(): boolean {
  const [enabled, setEnabled] = useState(false);

  useEffect(() => {
    let cancelled = false;

    getComputerUseSettings()
      .then((settings) => {
        if (!cancelled) setEnabled(settings.enabled);
      })
      .catch((err) => {
        if (!cancelled) {
          setEnabled(false);
          console.warn("load computer use settings failed", err);
        }
      });

    const handleSettingsChanged = (event: Event) => {
      const detail = (event as CustomEvent<ComputerUseSettings>).detail;
      if (!detail) return;
      setEnabled(detail.enabled);
    };

    window.addEventListener(COMPUTER_USE_SETTINGS_CHANGED_EVENT, handleSettingsChanged);
    return () => {
      cancelled = true;
      window.removeEventListener(COMPUTER_USE_SETTINGS_CHANGED_EVENT, handleSettingsChanged);
    };
  }, []);

  return enabled;
}
