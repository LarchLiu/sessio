import {
  cloneElement,
  isValidElement,
  ReactElement,
  ReactNode,
  Ref,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

export type Placement = "top" | "bottom" | "left" | "right";

interface TooltipProps {
  content: ReactNode;
  placement?: Placement;
  offset?: number;
  delayMs?: number;
  children: ReactElement<any>;
}

const VIEWPORT_MARGIN = 8;

export default function Tooltip({
  content,
  placement = "top",
  offset = 8,
  delayMs = 500,
  children,
}: TooltipProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const anchorRef = useRef<HTMLElement | null>(null);
  const tipRef = useRef<HTMLDivElement>(null);
  const openTimerRef = useRef<number | undefined>(undefined);

  const updatePosition = useCallback(() => {
    const anchor = anchorRef.current;
    const tip = tipRef.current;
    if (!anchor || !tip) return;
    const ar = anchor.getBoundingClientRect();
    const tw = tip.offsetWidth;
    const th = tip.offsetHeight;
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    let top = 0;
    let left = 0;
    switch (placement) {
      case "top":
        top = ar.top - th - offset;
        left = ar.left + ar.width / 2 - tw / 2;
        break;
      case "bottom":
        top = ar.bottom + offset;
        left = ar.left + ar.width / 2 - tw / 2;
        break;
      case "left":
        top = ar.top + ar.height / 2 - th / 2;
        left = ar.left - tw - offset;
        break;
      case "right":
        top = ar.top + ar.height / 2 - th / 2;
        left = ar.right + offset;
        break;
    }

    if (left < VIEWPORT_MARGIN) left = VIEWPORT_MARGIN;
    if (left + tw > vw - VIEWPORT_MARGIN) left = vw - VIEWPORT_MARGIN - tw;
    if (top < VIEWPORT_MARGIN) top = VIEWPORT_MARGIN;
    if (top + th > vh - VIEWPORT_MARGIN) top = vh - VIEWPORT_MARGIN - th;

    setPos({ top, left });
  }, [placement, offset]);

  useLayoutEffect(() => {
    if (open) updatePosition();
  }, [open, content, updatePosition]);

  useEffect(() => {
    if (!open) return;
    const f = () => updatePosition();
    window.addEventListener("scroll", f, true);
    window.addEventListener("resize", f);
    return () => {
      window.removeEventListener("scroll", f, true);
      window.removeEventListener("resize", f);
    };
  }, [open, updatePosition]);

  useEffect(() => {
    return () => {
      if (openTimerRef.current !== undefined) {
        window.clearTimeout(openTimerRef.current);
      }
    };
  }, []);

  if (!isValidElement(children)) return children;

  const setAnchor = (el: HTMLElement | null) => {
    anchorRef.current = el;
    const orig = (children as { ref?: Ref<HTMLElement> }).ref;
    if (typeof orig === "function") orig(el);
    else if (orig && typeof orig === "object" && "current" in orig) {
      (orig as { current: HTMLElement | null }).current = el;
    }
  };

  const childProps = children.props as {
    onMouseEnter?: (e: React.MouseEvent) => void;
    onMouseLeave?: (e: React.MouseEvent) => void;
    onFocus?: (e: React.FocusEvent) => void;
    onBlur?: (e: React.FocusEvent) => void;
  };

  const clearOpenTimer = () => {
    if (openTimerRef.current === undefined) return;
    window.clearTimeout(openTimerRef.current);
    openTimerRef.current = undefined;
  };

  const openWithDelay = () => {
    clearOpenTimer();
    if (delayMs <= 0) {
      setOpen(true);
      return;
    }
    openTimerRef.current = window.setTimeout(() => {
      openTimerRef.current = undefined;
      setOpen(true);
    }, delayMs);
  };

  const closeNow = () => {
    clearOpenTimer();
    setOpen(false);
  };

  const merged = cloneElement(children, {
    ref: setAnchor,
    onMouseEnter: (e: React.MouseEvent) => {
      childProps.onMouseEnter?.(e);
      openWithDelay();
    },
    onMouseLeave: (e: React.MouseEvent) => {
      childProps.onMouseLeave?.(e);
      closeNow();
    },
    onFocus: (e: React.FocusEvent) => {
      childProps.onFocus?.(e);
      setOpen(true);
    },
    onBlur: (e: React.FocusEvent) => {
      childProps.onBlur?.(e);
      closeNow();
    },
  } as Partial<typeof children.props>);

  return (
    <>
      {merged}
      {open &&
        createPortal(
          <div
            ref={tipRef}
            style={{
              position: "fixed",
              top: pos?.top ?? -9999,
              left: pos?.left ?? -9999,
              visibility: pos ? "visible" : "hidden",
            }}
            className="z-50 w-max max-w-[calc(100vw-16px)] bg-tooltip-bg border border-ink/10 text-tooltip-fg text-body-sm px-2 py-1 rounded-md shadow-lg leading-snug pointer-events-none"
          >
            {content}
          </div>,
          document.body,
        )}
    </>
  );
}
