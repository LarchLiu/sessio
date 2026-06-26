import { describe, expect, it } from "vitest";
import {
  appshotPermissionPresentation,
} from "../src/appshotPermissionPresentation";
import type { AppshotPermissionStatus } from "../src/api";

function status(
  patch: Partial<AppshotPermissionStatus>,
): AppshotPermissionStatus {
  return {
    platform: "macos",
    requiresPermission: true,
    screenshots: {
      granted: false,
      supported: true,
    },
    accessibility: {
      granted: false,
      supported: true,
    },
    canCapture: false,
    ...patch,
  };
}

describe("appshot permission presentation", () => {
  it("shows Windows screenshot capture as ready without permission management", () => {
    expect(appshotPermissionPresentation(status({
      platform: "windows",
      requiresPermission: false,
      screenshots: { granted: true, supported: false },
      accessibility: { granted: true, supported: false },
      canCapture: true,
    }))).toEqual({
      requiresPermission: false,
      descriptionKey: "settings.appshot_permissions_not_required",
      showManageButton: false,
      statusKey: "settings.appshot_permissions_unrestricted",
      accessibilityKey: null,
    });
  });

  it("keeps macOS permission management visible when screen capture is missing", () => {
    expect(appshotPermissionPresentation(status({
      platform: "macos",
      requiresPermission: true,
      screenshots: { granted: false, supported: true },
      accessibility: { granted: false, supported: true },
      canCapture: false,
    }))).toEqual({
      requiresPermission: true,
      descriptionKey: "settings.appshot_permissions_needed",
      showManageButton: true,
      statusKey: "settings.appshot_screenshots_missing",
      accessibilityKey: "settings.appshot_accessibility_optional",
    });
  });

  it("shows macOS as ready after screenshot permission is granted", () => {
    expect(appshotPermissionPresentation(status({
      platform: "macos",
      requiresPermission: true,
      screenshots: { granted: true, supported: true },
      accessibility: { granted: true, supported: true },
      canCapture: true,
    }))).toMatchObject({
      descriptionKey: "settings.appshot_permissions_ready",
      statusKey: "settings.appshot_screenshots_granted",
      accessibilityKey: "settings.appshot_accessibility_granted",
    });
  });
});
