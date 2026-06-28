import type { DesktopControlPermissionStatus } from "./api";

/**
 * Presentation for the computer-use consumer of the shared desktop-control
 * permission layer. Unlike Appshot (where accessibility is optional decoration),
 * computer use treats accessibility as a **hard dependency** — element-tree
 * inspection and `click_element` require it — and surfaces a third `canControl`
 * tier for input injection.
 *
 * Both consumers read the same `DesktopControlPermissionStatus` source of truth;
 * they differ only in how they render its tiers.
 */
export type DesktopControlPermissionPresentation = {
  requiresPermission: boolean;
  /** Overall readiness for the full observe + inspect + control contract. */
  ready: boolean;
  descriptionKey:
    | "settings.desktop_control_not_supported"
    | "settings.desktop_control_ready"
    | "settings.desktop_control_needed";
  showManageButton: boolean;
  screenshotKey:
    | "settings.desktop_control_screenshots_granted"
    | "settings.desktop_control_screenshots_required";
  /** Accessibility is required here, not optional. */
  accessibilityKey:
    | "settings.desktop_control_accessibility_granted"
    | "settings.desktop_control_accessibility_required";
};

export function desktopControlPermissionPresentation(
  status: DesktopControlPermissionStatus | null,
): DesktopControlPermissionPresentation {
  const requiresPermission = status?.requiresPermission ?? true;
  if (!requiresPermission) {
    return {
      requiresPermission: false,
      ready: Boolean(status?.canObserve && status?.canInspect),
      descriptionKey: "settings.desktop_control_not_supported",
      showManageButton: false,
      screenshotKey: status?.screenshots.granted
        ? "settings.desktop_control_screenshots_granted"
        : "settings.desktop_control_screenshots_required",
      accessibilityKey: status?.accessibility.granted
        ? "settings.desktop_control_accessibility_granted"
        : "settings.desktop_control_accessibility_required",
    };
  }
  // Computer use needs both observe and inspect; control is reported separately
  // because it is net-new and may be unavailable even when observe/inspect are.
  const ready = Boolean(status?.canObserve && status?.canInspect);
  return {
    requiresPermission: true,
    ready,
    descriptionKey: ready
      ? "settings.desktop_control_ready"
      : "settings.desktop_control_needed",
    showManageButton: true,
    screenshotKey: status?.screenshots.granted
      ? "settings.desktop_control_screenshots_granted"
      : "settings.desktop_control_screenshots_required",
    accessibilityKey: status?.accessibility.granted
      ? "settings.desktop_control_accessibility_granted"
      : "settings.desktop_control_accessibility_required",
  };
}
