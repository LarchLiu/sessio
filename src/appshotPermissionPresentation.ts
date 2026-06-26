import type { AppshotPermissionStatus } from "./api";

export type AppshotPermissionPresentation = {
  requiresPermission: boolean;
  descriptionKey:
    | "settings.appshot_permissions_not_required"
    | "settings.appshot_permissions_ready"
    | "settings.appshot_permissions_needed";
  showManageButton: boolean;
  statusKey:
    | "settings.appshot_permissions_unrestricted"
    | "settings.appshot_screenshots_granted"
    | "settings.appshot_screenshots_missing";
  accessibilityKey:
    | "settings.appshot_accessibility_granted"
    | "settings.appshot_accessibility_optional"
    | null;
};

export function appshotPermissionPresentation(
  status: AppshotPermissionStatus | null,
): AppshotPermissionPresentation {
  const requiresPermission = status?.requiresPermission ?? true;
  if (!requiresPermission) {
    return {
      requiresPermission: false,
      descriptionKey: "settings.appshot_permissions_not_required",
      showManageButton: false,
      statusKey: "settings.appshot_permissions_unrestricted",
      accessibilityKey: null,
    };
  }
  return {
    requiresPermission: true,
    descriptionKey: status?.canCapture
      ? "settings.appshot_permissions_ready"
      : "settings.appshot_permissions_needed",
    showManageButton: true,
    statusKey: status?.screenshots.granted
      ? "settings.appshot_screenshots_granted"
      : "settings.appshot_screenshots_missing",
    accessibilityKey: status?.accessibility.granted
      ? "settings.appshot_accessibility_granted"
      : "settings.appshot_accessibility_optional",
  };
}
