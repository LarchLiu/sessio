import { useEffect, useRef, useState } from "react";
import { Skull } from "lucide-react";
import type { MemoryBackendStatus } from "../api";
import { useI18n } from "../i18n";
import Tooltip from "./Tooltip";

type MemoryBackendMissingButtonProps = {
  status: MemoryBackendStatus | null;
  placement?: "top" | "bottom";
  onRefresh?: () => Promise<void> | void;
};

export default function MemoryBackendMissingButton({
  status,
  placement = "top",
  onRefresh,
}: MemoryBackendMissingButtonProps) {
  const { t } = useI18n();
  const [state, setState] = useState<"idle" | "copied" | "error">("idle");
  const timerRef = useRef<number | null>(null);
  const installCommand =
    (status?.details as { installCommand?: string } | undefined)?.installCommand ??
    "";
  const backendName = status?.backend ?? "memory backend";

  useEffect(
    () => () => {
      if (timerRef.current) window.clearTimeout(timerRef.current);
    },
    [],
  );

  const resetSoon = (next: "copied" | "error") => {
    setState(next);
    if (timerRef.current) window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => setState("idle"), 1500);
  };

  const handleClick = async () => {
    let nextState: "copied" | "error" = "copied";
    try {
      await navigator.clipboard.writeText(installCommand);
    } catch (err) {
      console.error("qmd install command copy failed", err);
      nextState = "error";
    } finally {
      resetSoon(nextState);
      void onRefresh?.();
    }
  };

  const tip =
    state === "copied" ? (
      t("list.copied")
    ) : state === "error" ? (
      t("list.copy_failed")
    ) : (
      <div className="flex max-w-full flex-col gap-1.5 py-0.5">
        <span>{t("sidebar.memory_backend_required", { backend: backendName })}</span>
        <code className="block max-w-full truncate whitespace-nowrap font-mono text-[11px] text-ink/85">
          {installCommand}
        </code>
        <span className="text-ink/55">
          {t("sidebar.click_to_copy", { backend: backendName })}
        </span>
        <span className="text-ink/55">
          {t("sidebar.click_to_check_backend", { backend: backendName })}
        </span>
      </div>
    );

  return (
    <Tooltip content={tip} placement={placement}>
      <button
        type="button"
        onClick={handleClick}
        aria-label={t("sidebar.copy_memory_backend_install")}
        className="inline-flex p-1 text-ink/55 hover:text-status-error hover:bg-status-error/10 focus-visible:text-status-error focus-visible:bg-status-error/10 focus-visible:outline-none transition rounded-md"
      >
        <Skull className="w-4 h-4" />
      </button>
    </Tooltip>
  );
}
