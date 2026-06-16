import { PanelRightOpen, Info } from "lucide-react";
import { useI18n } from "../i18n";
import Tooltip from "./Tooltip";

interface AppRightSidebarProps {
  onClose: () => void;
}

export default function AppRightSidebar({ onClose }: AppRightSidebarProps) {
  const { t } = useI18n();
  return (
    <div className="flex h-full min-h-0 w-full flex-col">
      <div
        data-tauri-drag-region
        className="relative flex h-12 shrink-0 items-center justify-between border-b border-ink/10 px-3 select-none"
      >
        <span
          data-tauri-drag-region
          className="text-body-sm font-medium leading-none text-ink/72"
        >
          {t("sidebar.right_empty_title")}
        </span>
        <Tooltip content={t("sidebar.right_close")} placement="bottom">
          <button
            type="button"
            aria-label={t("sidebar.right_close")}
            data-tauri-drag-region="false"
            onClick={onClose}
            className="rounded-md p-1 text-ink/55 transition hover:bg-ink/5 hover:text-ink"
          >
            <PanelRightOpen className="h-4 w-4" />
          </button>
        </Tooltip>
      </div>
      <div className="flex flex-1 min-h-0 flex-col items-center justify-center gap-2 px-6 text-center text-ink/45">
        <Info className="h-5 w-5 text-ink/35" />
        <p className="text-body-sm leading-snug">
          {t("sidebar.right_empty_hint")}
        </p>
      </div>
    </div>
  );
}
