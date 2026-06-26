import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import {
  BadgeCheck,
  ChevronRight,
  LoaderCircle,
  MonitorUp,
  RefreshCw,
  ScanEye,
  Sparkles,
  X,
} from "lucide-react";
import type { AppshotPermissionKind, AppshotPermissionStatus } from "../api";
import {
  openAppshotPermissionSettings,
  requestAppshotPermission,
} from "../api";
import { useI18n } from "../i18n";

type AppshotPermissionDialogProps = {
  open: boolean;
  status: AppshotPermissionStatus | null;
  shortcut: string;
  onClose: () => void;
  onExited: () => void;
  onStatusChange: (status: AppshotPermissionStatus) => void;
  onError: (error: string | null) => void;
};

type PermissionActionState = {
  requesting: boolean;
  opening: boolean;
};

const inactiveActionState: PermissionActionState = {
  requesting: false,
  opening: false,
};

export default function AppshotPermissionDialog({
  open,
  status,
  shortcut,
  onClose,
  onExited,
  onStatusChange,
  onError,
}: AppshotPermissionDialogProps) {
  const { t } = useI18n();
  const [hasOpened, setHasOpened] = useState(open);
  const [actionState, setActionState] = useState<Record<AppshotPermissionKind, PermissionActionState>>({
    screenshots: inactiveActionState,
    accessibility: inactiveActionState,
  });
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    if (open) setHasOpened(true);
  }, [open]);

  const rows = useMemo(
    () => [
      {
        kind: "accessibility" as const,
        icon: ScanEye,
        tint: "from-sky-500 to-cyan-400",
        label: t("appshot.permission.accessibility"),
        description: t("appshot.permission.accessibility_description"),
        note: t("appshot.permission.accessibility_note"),
        state: status?.accessibility ?? null,
      },
      {
        kind: "screenshots" as const,
        icon: MonitorUp,
        tint: "from-amber-500 to-orange-400",
        label: t("appshot.permission.screenshots"),
        description: t("appshot.permission.screenshots_description"),
        note: t("appshot.permission.screenshots_note"),
        state: status?.screenshots ?? null,
      },
    ],
    [status, t],
  );

  const allRequiredReady = Boolean(status?.screenshots.granted);

  const refreshStatus = async () => {
    if (refreshing) return;
    setRefreshing(true);
    try {
      const next = await import("../api").then(({ getAppshotPermissionStatus }) =>
        getAppshotPermissionStatus(),
      );
      onStatusChange(next);
      onError(null);
    } catch (err) {
      onError(String(err));
    } finally {
      setRefreshing(false);
    }
  };

  const handleAllow = async (permission: AppshotPermissionKind) => {
    const current = actionState[permission];
    if (current.requesting || current.opening) return;
    setActionState((prev) => ({
      ...prev,
      [permission]: { requesting: true, opening: false },
    }));
    try {
      const next = await requestAppshotPermission(permission);
      onStatusChange(next);
      if (
        (permission === "screenshots" && !next.screenshots.granted) ||
        (permission === "accessibility" && !next.accessibility.granted)
      ) {
        setActionState((prev) => ({
          ...prev,
          [permission]: { requesting: false, opening: true },
        }));
        await openAppshotPermissionSettings(permission);
      }
      onError(null);
    } catch (err) {
      onError(String(err));
    } finally {
      setActionState((prev) => ({
        ...prev,
        [permission]: inactiveActionState,
      }));
    }
  };

  const handleOpenSettings = async (permission: AppshotPermissionKind) => {
    const current = actionState[permission];
    if (current.requesting || current.opening) return;
    setActionState((prev) => ({
      ...prev,
      [permission]: { requesting: false, opening: true },
    }));
    try {
      await openAppshotPermissionSettings(permission);
      onError(null);
    } catch (err) {
      onError(String(err));
    } finally {
      setActionState((prev) => ({
        ...prev,
        [permission]: inactiveActionState,
      }));
    }
  };

  const primaryButtonClass =
    "inline-flex h-10 items-center gap-2 rounded-full bg-blue px-4 text-body-sm font-semibold text-white shadow-[0_10px_30px_rgb(var(--color-blue)/0.28)] transition hover:bg-blue/92 disabled:opacity-45";
  const secondaryButtonClass =
    "inline-flex h-10 items-center gap-2 rounded-full border border-card-border/[0.12] bg-card-chip/[0.08] px-4 text-body-sm font-medium text-card-fg/78 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-45";

  return createPortal(
    <div
      className={
        "absolute inset-0 z-[90] flex items-center justify-center bg-black/44 px-4 backdrop-blur-md " +
        (open ? "update-confirm-dialog-in" : hasOpened ? "update-confirm-dialog-out" : "opacity-0")
      }
      onClick={onClose}
      onAnimationEnd={(event) => {
        if (!open && event.currentTarget === event.target) onExited();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="appshot-permission-title"
        className={
          "w-full max-w-[680px] overflow-hidden rounded-[28px] border border-white/35 bg-[linear-gradient(180deg,rgba(255,255,255,0.94),rgba(246,248,251,0.97))] text-ink shadow-[0_32px_120px_rgba(15,23,42,0.28)] transition-[opacity,transform] duration-150 " +
          (open ? "translate-y-0 opacity-100" : hasOpened ? "translate-y-2 opacity-0" : "opacity-0")
        }
        onClick={(event) => event.stopPropagation()}
      >
        <div className="relative overflow-hidden px-6 pb-6 pt-6">
          <div className="pointer-events-none absolute inset-x-0 top-0 h-28 bg-[radial-gradient(circle_at_top,rgba(59,130,246,0.16),transparent_68%)]" />
          <button
            type="button"
            aria-label={t("detail.close")}
            onClick={onClose}
            className="absolute right-4 top-4 rounded-full p-1.5 text-ink/42 transition hover:bg-ink/5 hover:text-ink"
          >
            <X className="h-4 w-4" />
          </button>
          <div className="relative mx-auto flex max-w-[560px] flex-col items-center text-center">
            <div className="mb-5 flex h-18 w-18 items-center justify-center rounded-[22px] bg-[linear-gradient(135deg,rgba(59,130,246,0.22),rgba(96,165,250,0.08))] shadow-[inset_0_1px_0_rgba(255,255,255,0.9)]">
              <Sparkles className="h-8 w-8 text-blue" />
            </div>
            <h2 id="appshot-permission-title" className="text-[40px] font-semibold tracking-[-0.04em] text-ink/90">
              {t("appshot.permission.title")}
            </h2>
            <p className="mt-3 max-w-[540px] text-body leading-relaxed text-ink/62">
              {t("appshot.permission.subtitle", { shortcut })}
            </p>
          </div>
          <div className="relative mx-auto mt-8 grid max-w-[560px] gap-4">
            {rows.map((row) => {
              const Icon = row.icon;
              const granted = row.state?.granted ?? false;
              const supported = row.state?.supported ?? true;
              const pending = actionState[row.kind].requesting || actionState[row.kind].opening;
              return (
                <div
                  key={row.kind}
                  className="rounded-[24px] border border-ink/8 bg-white/82 px-5 py-4 shadow-[0_12px_28px_rgba(15,23,42,0.08)] backdrop-blur"
                >
                  <div className="flex items-center gap-4">
                    <div className={"flex h-14 w-14 shrink-0 items-center justify-center rounded-[18px] bg-gradient-to-br text-white shadow-[0_8px_24px_rgba(15,23,42,0.16)] " + row.tint}>
                      <Icon className="h-7 w-7" />
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <div className="text-title font-semibold text-ink/88">{row.label}</div>
                        {granted && (
                          <span className="inline-flex items-center gap-1 rounded-full bg-emerald/12 px-2 py-0.5 text-[12px] font-medium text-emerald">
                            <BadgeCheck className="h-3.5 w-3.5" />
                            {t("appshot.permission.granted")}
                          </span>
                        )}
                        {!supported && (
                          <span className="rounded-full bg-ink/8 px-2 py-0.5 text-[12px] font-medium text-ink/55">
                            {t("appshot.permission.not_needed")}
                          </span>
                        )}
                      </div>
                      <div className="mt-1 text-body-sm text-ink/63">{row.description}</div>
                      <div className="mt-2 text-caption leading-relaxed text-ink/48">{row.note}</div>
                    </div>
                    {supported && granted ? (
                      <div className="rounded-full bg-emerald/12 px-3 py-1.5 text-body-sm font-medium text-emerald">
                        {t("appshot.permission.allowed")}
                      </div>
                    ) : (
                      <div className="flex shrink-0 items-center gap-2">
                        <button
                          type="button"
                          disabled={pending}
                          onClick={() => void handleAllow(row.kind)}
                          className={secondaryButtonClass}
                        >
                          {actionState[row.kind].requesting ? (
                            <LoaderCircle className="h-4 w-4 animate-spin" />
                          ) : null}
                          {t("appshot.permission.allow")}
                        </button>
                        <button
                          type="button"
                          disabled={pending}
                          onClick={() => void handleOpenSettings(row.kind)}
                          className={secondaryButtonClass}
                        >
                          {actionState[row.kind].opening ? (
                            <LoaderCircle className="h-4 w-4 animate-spin" />
                          ) : (
                            <ChevronRight className="h-4 w-4" />
                          )}
                          {t("appshot.permission.open_settings")}
                        </button>
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
          <div className="relative mx-auto mt-6 flex max-w-[560px] items-center justify-between gap-3 rounded-[22px] border border-ink/8 bg-white/72 px-4 py-3 text-body-sm text-ink/60">
            <div>{t("appshot.permission.footer")}</div>
            <button
              type="button"
              disabled={refreshing}
              onClick={() => void refreshStatus()}
              className="inline-flex h-9 items-center gap-2 rounded-full border border-card-border/[0.12] bg-card-chip/[0.08] px-3 font-medium text-card-fg/76 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-45"
            >
              <RefreshCw className={"h-4 w-4 " + (refreshing ? "animate-spin" : "")} />
              {t("appshot.permission.refresh")}
            </button>
          </div>
        </div>
        <div className="flex items-center justify-end gap-2 border-t border-ink/8 bg-white/70 px-6 py-4">
          <button
            type="button"
            onClick={onClose}
            className="rounded-full px-4 py-2 text-body-sm font-medium text-ink/62 transition hover:bg-ink/5 hover:text-ink"
          >
            {t("delete.cancel")}
          </button>
          <button
            type="button"
            disabled={!allRequiredReady}
            onClick={onClose}
            className={primaryButtonClass}
          >
            {t("appshot.permission.done")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
