import {
  forwardRef,
  ReactNode,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

const HIDE_DELAY = 600;
const MIN_THUMB_HEIGHT = 24;

type Props = {
  className?: string;
  viewportClassName?: string;
  children: ReactNode;
};

const ScrollArea = forwardRef<HTMLDivElement, Props>(function ScrollArea(
  { className, viewportClassName, children },
  ref,
) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const thumbRef = useRef<HTMLDivElement>(null);
  const hideTimerRef = useRef<number | undefined>(undefined);
  const dragStateRef = useRef<{
    startY: number;
    startScrollTop: number;
  } | null>(null);

  const [visible, setVisible] = useState(false);
  const [hasOverflow, setHasOverflow] = useState(false);

  const updateThumb = useCallback(() => {
    const vp = viewportRef.current;
    if (!vp) return;
    const { scrollTop, scrollHeight, clientHeight } = vp;
    const overflow = scrollHeight > clientHeight + 1;
    setHasOverflow(overflow);
    if (!overflow) return;
    const thumb = thumbRef.current;
    if (!thumb) return;
    const trackHeight = clientHeight;
    const ratio = clientHeight / scrollHeight;
    const thumbHeight = Math.max(trackHeight * ratio, MIN_THUMB_HEIGHT);
    const maxThumbTop = trackHeight - thumbHeight;
    const maxScrollTop = scrollHeight - clientHeight;
    const top =
      maxScrollTop > 0 ? (scrollTop / maxScrollTop) * maxThumbTop : 0;
    thumb.style.height = `${thumbHeight}px`;
    thumb.style.transform = `translateY(${top}px)`;
  }, []);

  const flashVisible = useCallback(() => {
    setVisible(true);
    if (hideTimerRef.current !== undefined)
      window.clearTimeout(hideTimerRef.current);
    hideTimerRef.current = window.setTimeout(() => {
      if (!dragStateRef.current) setVisible(false);
    }, HIDE_DELAY);
  }, []);

  const handleScroll = useCallback(() => {
    updateThumb();
    flashVisible();
  }, [updateThumb, flashVisible]);

  useLayoutEffect(() => {
    updateThumb();
  });

  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;
    const ro = new ResizeObserver(() => updateThumb());
    ro.observe(vp);
    for (const child of Array.from(vp.children)) ro.observe(child);
    return () => ro.disconnect();
  }, [updateThumb]);

  useEffect(() => {
    return () => {
      if (hideTimerRef.current !== undefined)
        window.clearTimeout(hideTimerRef.current);
    };
  }, []);

  const onThumbPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    const vp = viewportRef.current;
    if (!vp) return;
    e.preventDefault();
    (e.target as Element).setPointerCapture(e.pointerId);
    dragStateRef.current = {
      startY: e.clientY,
      startScrollTop: vp.scrollTop,
    };
    setVisible(true);
  };

  const onThumbPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragStateRef.current;
    const vp = viewportRef.current;
    const thumb = thumbRef.current;
    if (!drag || !vp || !thumb) return;
    const trackHeight = vp.clientHeight;
    const thumbHeight = thumb.offsetHeight;
    const maxThumbTop = trackHeight - thumbHeight;
    const maxScrollTop = vp.scrollHeight - vp.clientHeight;
    if (maxThumbTop <= 0) return;
    const dy = e.clientY - drag.startY;
    const scrollDelta = (dy / maxThumbTop) * maxScrollTop;
    vp.scrollTop = drag.startScrollTop + scrollDelta;
  };

  const onThumbPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragStateRef.current) return;
    (e.target as Element).releasePointerCapture(e.pointerId);
    dragStateRef.current = null;
    flashVisible();
  };

  useImperativeHandle(ref, () => viewportRef.current as HTMLDivElement, []);

  return (
    <div className={"relative flex flex-col " + (className ?? "")}>
      <div
        ref={viewportRef}
        onScroll={handleScroll}
        className={
          "flex-1 min-h-0 w-full overflow-y-scroll hide-native-scrollbar " +
          (viewportClassName ?? "")
        }
      >
        {children}
      </div>
      {hasOverflow && (
        <div
          aria-hidden
          className={
            "absolute top-0 right-0.5 w-1.5 rounded-full bg-ink/30 hover:bg-ink/50 cursor-pointer transition-opacity " +
            (visible
              ? "opacity-100 duration-150"
              : "opacity-0 pointer-events-none duration-700")
          }
          ref={thumbRef}
          style={{ height: MIN_THUMB_HEIGHT }}
          onPointerDown={onThumbPointerDown}
          onPointerMove={onThumbPointerMove}
          onPointerUp={onThumbPointerUp}
          onPointerCancel={onThumbPointerUp}
        />
      )}
    </div>
  );
});

export default ScrollArea;
