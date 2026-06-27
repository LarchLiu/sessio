import type { ComputerUseStatus } from "../api";
import { useI18n } from "../i18n";

export default function ComputerUseTakeoverOverlay({
  status,
  onAbort,
}: {
  status: ComputerUseStatus | null;
  onAbort: () => void;
}) {
  const { t } = useI18n();
  if (!status?.foregroundActive) return null;

  const targetLabel = status.activeAppId
    ? t("computer_use.controlling_app", { app: status.activeAppId })
    : t("computer_use.controlling_app_generic");

  return (
    <div className="pointer-events-none absolute inset-x-0 top-4 z-30 flex justify-center px-4">
      <div className="pointer-events-auto flex w-full max-w-[680px] items-center justify-between gap-4 rounded-2xl border border-status-warn/30 bg-status-warn/[0.16] px-4 py-3 shadow-lg backdrop-blur">
        <div className="min-w-0">
          <div className="text-body-sm font-semibold text-status-warn">
            {t("computer_use.takeover_title")}
          </div>
          <div className="mt-1 truncate text-caption text-ink/70">{targetLabel}</div>
        </div>
        <button
          type="button"
          onClick={onAbort}
          className="inline-flex h-9 shrink-0 items-center rounded-full bg-status-error px-4 text-body-sm font-medium text-white transition hover:bg-status-error/92"
        >
          {t("computer_use.takeover_stop")}
        </button>
      </div>
    </div>
  );
}
