import type { ComputerUseSettings } from "./api";

export const COMPUTER_USE_SETTINGS_CHANGED_EVENT = "sessio:computer-use-settings-changed";

export function emitComputerUseSettingsChanged(settings: ComputerUseSettings): void {
  window.dispatchEvent(
    new CustomEvent<ComputerUseSettings>(COMPUTER_USE_SETTINGS_CHANGED_EVENT, {
      detail: settings,
    }),
  );
}
