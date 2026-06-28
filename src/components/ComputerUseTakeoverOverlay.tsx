import type { ComputerUseStatus } from "../api";
import { useI18n } from "../i18n";

export default function ComputerUseTakeoverOverlay({
  status,
  onApproveApp,
  onAbort,
}: {
  status: ComputerUseStatus | null;
  onApproveApp: (approved: boolean) => void;
  onAbort: () => void;
}) {
  const { t } = useI18n();
  if (!status) return null;

  const needsAppApproval = Boolean(
    status.activeAppId &&
      status.sessionApproved &&
      status.canControl &&
      !status.activeAppApproved,
  );
  if (!status.foregroundActive && !needsAppApproval) return null;

  const targetLabel = status.activeAppId
    ? t("computer_use.controlling_app", { app: status.activeAppId })
    : t("computer_use.controlling_app_generic");

  return (
    <div className="pointer-events-none absolute inset-x-0 top-4 z-30 flex justify-center px-4">
      <div className="pointer-events-auto flex w-full max-w-[760px] items-center justify-between gap-4 rounded-2xl border border-status-warn/30 bg-status-warn/[0.16] px-4 py-3 shadow-lg backdrop-blur">
        <div className="min-w-0">
          <div className="text-body-sm font-semibold text-status-warn">
            {status.foregroundActive
              ? t("computer_use.takeover_title")
              : t("computer_use.app_approval_title")}
          </div>
          <div className="mt-1 truncate text-caption text-ink/70">
            {needsAppApproval
              ? t("computer_use.app_approval_description", {
                  app: status.activeAppId ?? "",
                })
              : targetLabel}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {status.activeAppId && status.sessionApproved && status.canControl && (
            <button
              type="button"
              onClick={() => onApproveApp(!status.activeAppApproved)}
              className={
                status.activeAppApproved
                  ? "inline-flex h-9 items-center rounded-full border border-ink/15 bg-surface px-4 text-body-sm font-medium text-ink transition hover:bg-surface-muted"
                  : "inline-flex h-9 items-center rounded-full bg-accent px-4 text-body-sm font-medium text-white transition hover:bg-accent/90"
              }
            >
              {status.activeAppApproved
                ? t("computer_use.app_approval_revoke")
                : t("computer_use.app_approval_allow")}
            </button>
          )}
          {status.foregroundActive && (
            <button
              type="button"
              onClick={onAbort}
              className="inline-flex h-9 items-center rounded-full bg-status-error px-4 text-body-sm font-medium text-white transition hover:bg-status-error/92"
            >
              {t("computer_use.takeover_stop")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
