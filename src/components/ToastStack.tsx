import { useCallback, useLayoutEffect, useRef, useState } from "react";

type Toast = { id: number; message: string; entering: boolean };

function ToastItem({
  id,
  message,
  entering,
  durationMs,
  onDismiss,
}: {
  id: number;
  message: string;
  entering: boolean;
  durationMs: number;
  onDismiss: (id: number) => void;
}) {
  useLayoutEffect(() => {
    const timeout = window.setTimeout(() => onDismiss(id), durationMs);
    return () => window.clearTimeout(timeout);
  }, [durationMs, id, onDismiss]);

  return (
    <div
      data-toast-id={id}
      className={
        "flex items-start gap-2 rounded-md border border-status-error/25 bg-surface-panel px-3 py-2 text-body-sm text-status-error shadow-xl transition-[opacity,transform] duration-180 ease-out " +
        (entering ? "translate-y-2 opacity-0" : "translate-y-0 opacity-100")
      }
    >
      <div className="min-w-0 flex-1 whitespace-pre-wrap break-words">{message}</div>
      <button
        type="button"
        onClick={() => onDismiss(id)}
        className="pointer-events-auto rounded px-1 text-caption text-status-error/70 hover:bg-status-error/10 hover:text-status-error"
      >
        x
      </button>
    </div>
  );
}

export default function ToastStack({
  message,
  onMessageConsumed,
  durationMs = 6000,
}: {
  message: string | null;
  onMessageConsumed: () => void;
  durationMs?: number;
}) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const toastIdRef = useRef(0);
  const stackRef = useRef<HTMLDivElement | null>(null);
  const previousRectsRef = useRef<Map<number, DOMRect>>(new Map());
  const shouldAnimateLayoutRef = useRef(false);

  useLayoutEffect(() => {
    if (!message) return;
    const id = ++toastIdRef.current;
    setToasts((prev) => [...prev, { id, message, entering: true }]);
    onMessageConsumed();
  }, [message, onMessageConsumed]);

  useLayoutEffect(() => {
    if (!toasts.some((toast) => toast.entering)) return;
    const frame = window.requestAnimationFrame(() => {
      setToasts((prev) => prev.map((toast) => ({ ...toast, entering: false })));
    });
    return () => window.cancelAnimationFrame(frame);
  }, [toasts]);

  useLayoutEffect(() => {
    if (!shouldAnimateLayoutRef.current) return;
    shouldAnimateLayoutRef.current = false;
    const stack = stackRef.current;
    if (!stack) return;
    const previousRects = previousRectsRef.current;
    const elements = Array.from(stack.querySelectorAll<HTMLElement>("[data-toast-id]"));

    for (const element of elements) {
      const id = Number(element.dataset.toastId);
      const previous = previousRects.get(id);
      if (!previous) continue;
      const next = element.getBoundingClientRect();
      const deltaY = previous.top - next.top;
      if (deltaY === 0) continue;
      element.animate(
        [
          { transform: `translateY(${deltaY}px)` },
          { transform: "translateY(0)" },
        ],
        { duration: 180, easing: "ease-out" },
      );
    }

    previousRectsRef.current = new Map(
      elements.map((element) => [Number(element.dataset.toastId), element.getBoundingClientRect()]),
    );
  }, [toasts]);

  const dismiss = useCallback((id: number) => {
    const stack = stackRef.current;
    if (stack) {
      previousRectsRef.current = new Map(
        Array.from(stack.querySelectorAll<HTMLElement>("[data-toast-id]")).map((element) => [
          Number(element.dataset.toastId),
          element.getBoundingClientRect(),
        ]),
      );
      shouldAnimateLayoutRef.current = true;
    }
    setToasts((prev) => prev.filter((toast) => toast.id !== id));
  }, []);

  if (toasts.length === 0) return null;

  return (
    <div ref={stackRef} className="pointer-events-none absolute left-1/2 top-4 z-50 flex w-[min(520px,calc(100%-2rem))] -translate-x-1/2 flex-col gap-2">
      {toasts.map((toast) => (
        <ToastItem
          key={toast.id}
          id={toast.id}
          message={toast.message}
          entering={toast.entering}
          durationMs={durationMs}
          onDismiss={dismiss}
        />
      ))}
    </div>
  );
}
