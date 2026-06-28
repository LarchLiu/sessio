import { describe, expect, it } from "vitest";
import { desktopControlPermissionPresentation } from "../src/desktopControlPermissionPresentation";
import type { DesktopControlPermissionStatus } from "../src/api";

function status(
  patch: Partial<DesktopControlPermissionStatus>,
): DesktopControlPermissionStatus {
  return {
    platform: "macos",
    requiresPermission: true,
    screenshots: { granted: false, supported: true },
    accessibility: { granted: false, supported: true },
    canObserve: false,
    canInspect: false,
    canControl: false,
    ...patch,
  };
}

describe("desktop control permission presentation", () => {
  it("treats accessibility as required (not optional) when missing", () => {
    const p = desktopControlPermissionPresentation(status({
      screenshots: { granted: true, supported: true },
      accessibility: { granted: false, supported: true },
      canObserve: true,
      canInspect: false,
    }));
    expect(p.accessibilityKey).toBe(
      "settings.desktop_control_accessibility_required",
    );
    // Not ready: inspection is a hard dependency.
    expect(p.ready).toBe(false);
    expect(p.descriptionKey).toBe("settings.desktop_control_needed");
    expect(p.showManageButton).toBe(true);
  });

  it("is ready only when both observe and inspect are granted", () => {
    const p = desktopControlPermissionPresentation(status({
      screenshots: { granted: true, supported: true },
      accessibility: { granted: true, supported: true },
      canObserve: true,
      canInspect: true,
      canControl: false,
    }));
    expect(p.ready).toBe(true);
    expect(p.descriptionKey).toBe("settings.desktop_control_ready");
    expect(p.accessibilityKey).toBe(
      "settings.desktop_control_accessibility_granted",
    );
  });

  it("keeps readiness based on observe and inspect even when control differs", () => {
    const granted = desktopControlPermissionPresentation(status({
      screenshots: { granted: true, supported: true },
      accessibility: { granted: true, supported: true },
      canObserve: true,
      canInspect: true,
      canControl: true,
    }));
    expect(granted.ready).toBe(true);
    expect(granted.descriptionKey).toBe("settings.desktop_control_ready");

    const noControl = desktopControlPermissionPresentation(status({
      canObserve: true,
      canInspect: true,
      canControl: false,
    }));
    expect(noControl.ready).toBe(true);
    expect(noControl.descriptionKey).toBe("settings.desktop_control_ready");
  });

  it("reports unsupported platforms without permission management", () => {
    const p = desktopControlPermissionPresentation(status({
      platform: "linux",
      requiresPermission: false,
      screenshots: { granted: false, supported: false },
      accessibility: { granted: false, supported: false },
      canObserve: true,
      canInspect: true,
      canControl: false,
    }));
    expect(p.requiresPermission).toBe(false);
    expect(p.showManageButton).toBe(false);
    expect(p.descriptionKey).toBe("settings.desktop_control_not_supported");
  });
});
