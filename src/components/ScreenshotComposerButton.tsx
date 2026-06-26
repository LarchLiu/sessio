import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { listen } from "@tauri-apps/api/event";
import { ChevronDown, LoaderCircle, Scissors } from "lucide-react";
import { openScreenshotOverlayCapture } from "../api";
import { useI18n } from "../i18n";
import type { ChatComposerController } from "../hooks/useChatComposer";
import Tooltip from "./Tooltip";

type OverlaySavedPayload = {
  requestId: string;
  path: string;
  previewDataUrl: string;
};

type OverlayCancelledPayload = {
  requestId: string;
};

export default function ScreenshotComposerButton({
  composer,
  disabled = false,
}: {
  composer: ChatComposerController;
  disabled?: boolean;
}) {
  const { t } = useI18n();
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const pendingRequestRef = useRef<string | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [hideSelf, setHideSelf] = useState(false);
  const [capturing, setCapturing] = useState(false);
  const [overlayActive, setOverlayActive] = useState(false);
  const unavailable =
    disabled ||
    !composer.supportsAttachments ||
    !composer.supportsImageAttachments ||
    capturing ||
    overlayActive;

  useEffect(() => {
    let disposed = false;
    const unlistenSavedPromise = listen<OverlaySavedPayload>(
      "screenshot_overlay_saved",
      (event) => {
        if (disposed || pendingRequestRef.current !== event.payload.requestId) return;
        pendingRequestRef.current = null;
        setOverlayActive(false);
        void (async () => {
          try {
            await composer.appendAttachments([
              {
                kind: "image",
                path: event.payload.path,
                mimeType: "image/png",
                name: "Screenshot.png",
                displayName: t("screenshot.attachment_name"),
                previewDataUrl: event.payload.previewDataUrl,
              },
            ]);
            window.requestAnimationFrame(() => composer.textareaRef.current?.focus());
          } catch (err) {
            composer.setComposerError(t("screenshot.capture_failed", { error: String(err) }));
          }
        })();
      },
    );
    const unlistenCancelledPromise = listen<OverlayCancelledPayload>(
      "screenshot_overlay_cancelled",
      (event) => {
        if (disposed || pendingRequestRef.current !== event.payload.requestId) return;
        pendingRequestRef.current = null;
        setOverlayActive(false);
        window.requestAnimationFrame(() => composer.textareaRef.current?.focus());
      },
    );
    return () => {
      disposed = true;
      void unlistenSavedPromise.then((unlisten) => unlisten());
      void unlistenCancelledPromise.then((unlisten) => unlisten());
    };
  }, [composer, t]);

  const beginCapture = async () => {
    if (unavailable) return;
    setMenuOpen(false);
    setCapturing(true);
    composer.setComposerError(null);
    const requestId =
      typeof crypto !== "undefined" && "randomUUID" in crypto
        ? crypto.randomUUID()
        : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    pendingRequestRef.current = requestId;
    setOverlayActive(true);
    try {
      await openScreenshotOverlayCapture({
        requestId,
        hideSelf,
        fileName: "Screenshot.png",
      });
    } catch (err) {
      pendingRequestRef.current = null;
      setOverlayActive(false);
      composer.setComposerError(t("screenshot.capture_failed", { error: String(err) }));
    } finally {
      setCapturing(false);
    }
  };

  return (
    <>
      <div className="flex shrink-0 items-center rounded-full">
        <Tooltip content={t("screenshot.button")} placement="top">
          <button
            type="button"
            disabled={unavailable}
            onClick={() => void beginCapture()}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-l-full rounded-r-md text-ink/55 transition hover:bg-ink/8 hover:text-ink disabled:cursor-not-allowed disabled:text-ink/28 disabled:hover:bg-transparent disabled:hover:text-ink/28"
            aria-label={t("screenshot.button")}
          >
            {capturing ? (
              <LoaderCircle className="h-4 w-4 animate-spin" />
            ) : (
              <Scissors className="h-4 w-4" />
            )}
          </button>
        </Tooltip>
        <Tooltip content={t("screenshot.options")} placement="top">
          <button
            ref={menuButtonRef}
            type="button"
            disabled={unavailable}
            onClick={() => setMenuOpen((open) => !open)}
            className="flex h-7 w-5 shrink-0 items-center justify-center rounded-l-md rounded-r-full text-ink/42 transition hover:bg-ink/8 hover:text-ink disabled:cursor-not-allowed disabled:text-ink/24 disabled:hover:bg-transparent disabled:hover:text-ink/24"
            aria-label={t("screenshot.options")}
            aria-expanded={menuOpen}
            aria-haspopup="menu"
          >
            <ChevronDown className="h-3.5 w-3.5" />
          </button>
        </Tooltip>
      </div>
      {menuOpen && menuButtonRef.current && (
        <ScreenshotCaptureMenu
          anchor={menuButtonRef.current}
          hideSelf={hideSelf}
          onHideSelfChange={setHideSelf}
          onClose={() => setMenuOpen(false)}
        />
      )}
    </>
  );
}

function ScreenshotCaptureMenu({
  anchor,
  hideSelf,
  onHideSelfChange,
  onClose,
}: {
  anchor: HTMLElement;
  hideSelf: boolean;
  onHideSelfChange: (value: boolean) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const menuRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  useLayoutEffect(() => {
    const rect = anchor.getBoundingClientRect();
    const width = menuRef.current?.offsetWidth ?? 260;
    const height = menuRef.current?.offsetHeight ?? 170;
    const margin = 10;
    setPos({
      top: Math.max(margin, rect.top - height - 10),
      left: Math.max(margin, Math.min(rect.left, window.innerWidth - width - margin)),
    });
  }, [anchor]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return createPortal(
    <>
      <div className="fixed inset-0 z-[39] bg-transparent" onMouseDown={onClose} />
      <div
        ref={menuRef}
        className="fixed z-40 w-[238px] rounded-2xl border border-ink/10 bg-surface-panel p-2 shadow-[0_20px_60px_rgba(0,0,0,0.22)]"
        style={{
          top: pos?.top ?? -9999,
          left: pos?.left ?? -9999,
          visibility: pos ? "visible" : "hidden",
        }}
        role="menu"
      >
        <label className="flex items-center justify-between rounded-xl border border-ink/8 bg-ink/[0.035] px-3 py-2 text-body-sm text-ink/68">
          <span>{t("screenshot.hide_self")}</span>
          <input
            type="checkbox"
            checked={hideSelf}
            onChange={(event) => onHideSelfChange(event.target.checked)}
            className="h-4 w-4 accent-ink"
          />
        </label>
      </div>
    </>,
    document.body,
  );
}
