import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

const IS_MAC =
  typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);

export default function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (IS_MAC) return;
    const win = getCurrentWindow();
    let unlisten: (() => void) | null = null;
    win.isMaximized().then(setMaximized).catch(() => {});
    win
      .onResized(() => {
        win.isMaximized().then(setMaximized).catch(() => {});
      })
      .then((f) => {
        unlisten = f;
      })
      .catch(() => {});
    return () => {
      unlisten?.();
    };
  }, []);

  if (IS_MAC) return null;

  const win = getCurrentWindow();
  return (
    <div className="flex h-12 shrink-0" data-tauri-drag-region="false">
      <CtrlButton onClick={() => win.minimize()} aria-label="Minimize">
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M0 5 H10" stroke="currentColor" strokeWidth="1" fill="none" />
        </svg>
      </CtrlButton>
      <CtrlButton
        onClick={() => win.toggleMaximize()}
        aria-label={maximized ? "Restore" : "Maximize"}
      >
        {maximized ? (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
            <path
              d="M2.5 0.5 H9.5 V7.5 M0.5 2.5 H7.5 V9.5 H0.5 Z"
              stroke="currentColor"
              strokeWidth="1"
              fill="none"
            />
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
            <rect
              x="0.5"
              y="0.5"
              width="9"
              height="9"
              stroke="currentColor"
              strokeWidth="1"
              fill="none"
            />
          </svg>
        )}
      </CtrlButton>
      <button
        type="button"
        aria-label="Close"
        onClick={() => win.close()}
        className="w-[46px] h-12 flex items-center justify-center text-ink/70 hover:bg-status-error hover:text-white transition-colors"
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path
            d="M0.5 0.5 L9.5 9.5 M9.5 0.5 L0.5 9.5"
            stroke="currentColor"
            strokeWidth="1"
            fill="none"
          />
        </svg>
      </button>
    </div>
  );
}

function CtrlButton({
  onClick,
  children,
  ...rest
}: {
  onClick: () => void;
  children: React.ReactNode;
  "aria-label": string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="w-[46px] h-12 flex items-center justify-center text-ink/70 hover:bg-ink/10 hover:text-ink transition-colors"
      {...rest}
    >
      {children}
    </button>
  );
}
