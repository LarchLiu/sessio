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
const MIN_THUMB_SIZE = 24;

type Axis = "x" | "y";

type DragState = {
  axis: Axis;
  startPointer: number;
  startScroll: number;
};

type Props = {
  className?: string;
  viewportClassName?: string;
  children: ReactNode;
  persistScrollbars?: boolean;
  orientation?: "vertical" | "horizontal" | "both";
};

const ScrollArea = forwardRef<HTMLDivElement, Props>(function ScrollArea(
  {
    className,
    viewportClassName,
    children,
    persistScrollbars = false,
    orientation = "vertical",
  },
  ref,
) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const verticalThumbRef = useRef<HTMLDivElement>(null);
  const horizontalThumbRef = useRef<HTMLDivElement>(null);
  const hideTimerRef = useRef<number | undefined>(undefined);
  const dragStateRef = useRef<DragState | null>(null);

  const [visible, setVisible] = useState(false);
  const [hasVerticalOverflow, setHasVerticalOverflow] = useState(false);
  const [hasHorizontalOverflow, setHasHorizontalOverflow] = useState(false);

  const enableVertical = orientation === "vertical" || orientation === "both";
  const enableHorizontal = orientation === "horizontal" || orientation === "both";

  const updateThumbs = useCallback(() => {
    const vp = viewportRef.current;
    if (!vp) return;

    const verticalOverflow = vp.scrollHeight > vp.clientHeight + 1;
    const horizontalOverflow = vp.scrollWidth > vp.clientWidth + 1;
    setHasVerticalOverflow(enableVertical && verticalOverflow);
    setHasHorizontalOverflow(enableHorizontal && horizontalOverflow);

    const verticalThumb = verticalThumbRef.current;
    if (enableVertical && verticalOverflow && verticalThumb) {
      const trackHeight = vp.clientHeight;
      const ratio = vp.clientHeight / vp.scrollHeight;
      const thumbHeight = Math.max(trackHeight * ratio, MIN_THUMB_SIZE);
      const maxThumbTop = trackHeight - thumbHeight;
      const maxScrollTop = vp.scrollHeight - vp.clientHeight;
      const top =
        maxScrollTop > 0 ? (vp.scrollTop / maxScrollTop) * maxThumbTop : 0;
      verticalThumb.style.height = `${thumbHeight}px`;
      verticalThumb.style.transform = `translateY(${top}px)`;
    }

    const horizontalThumb = horizontalThumbRef.current;
    if (enableHorizontal && horizontalOverflow && horizontalThumb) {
      const trackWidth = vp.clientWidth;
      const ratio = vp.clientWidth / vp.scrollWidth;
      const thumbWidth = Math.max(trackWidth * ratio, MIN_THUMB_SIZE);
      const maxThumbLeft = trackWidth - thumbWidth;
      const maxScrollLeft = vp.scrollWidth - vp.clientWidth;
      const left =
        maxScrollLeft > 0 ? (vp.scrollLeft / maxScrollLeft) * maxThumbLeft : 0;
      horizontalThumb.style.width = `${thumbWidth}px`;
      horizontalThumb.style.transform = `translateX(${left}px)`;
    }
  }, [enableHorizontal, enableVertical]);

  const flashVisible = useCallback(() => {
    setVisible(true);
    if (hideTimerRef.current !== undefined)
      window.clearTimeout(hideTimerRef.current);
    hideTimerRef.current = window.setTimeout(() => {
      if (!dragStateRef.current) setVisible(false);
    }, HIDE_DELAY);
  }, []);

  const handleScroll = useCallback(() => {
    updateThumbs();
    flashVisible();
  }, [updateThumbs, flashVisible]);

  useLayoutEffect(() => {
    updateThumbs();
  });

  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;
    const ro = new ResizeObserver(() => updateThumbs());
    ro.observe(vp);
    for (const child of Array.from(vp.children)) ro.observe(child);
    return () => ro.disconnect();
  }, [updateThumbs]);

  useEffect(() => {
    return () => {
      if (hideTimerRef.current !== undefined)
        window.clearTimeout(hideTimerRef.current);
    };
  }, []);

  const onThumbPointerDown =
    (axis: Axis) => (e: React.PointerEvent<HTMLDivElement>) => {
      const vp = viewportRef.current;
      if (!vp) return;
      e.preventDefault();
      (e.target as Element).setPointerCapture(e.pointerId);
      dragStateRef.current = {
        axis,
        startPointer: axis === "y" ? e.clientY : e.clientX,
        startScroll: axis === "y" ? vp.scrollTop : vp.scrollLeft,
      };
      setVisible(true);
    };

  const onThumbPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragStateRef.current;
    const vp = viewportRef.current;
    if (!drag || !vp) return;

    if (drag.axis === "y") {
      const thumb = verticalThumbRef.current;
      if (!thumb) return;
      const maxThumbTop = vp.clientHeight - thumb.offsetHeight;
      const maxScrollTop = vp.scrollHeight - vp.clientHeight;
      if (maxThumbTop <= 0) return;
      const dy = e.clientY - drag.startPointer;
      vp.scrollTop = drag.startScroll + (dy / maxThumbTop) * maxScrollTop;
      return;
    }

    const thumb = horizontalThumbRef.current;
    if (!thumb) return;
    const maxThumbLeft = vp.clientWidth - thumb.offsetWidth;
    const maxScrollLeft = vp.scrollWidth - vp.clientWidth;
    if (maxThumbLeft <= 0) return;
    const dx = e.clientX - drag.startPointer;
    vp.scrollLeft = drag.startScroll + (dx / maxThumbLeft) * maxScrollLeft;
  };

  const onThumbPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragStateRef.current) return;
    (e.target as Element).releasePointerCapture(e.pointerId);
    dragStateRef.current = null;
    flashVisible();
  };

  useImperativeHandle(ref, () => viewportRef.current as HTMLDivElement, []);

  const idleClass = persistScrollbars
    ? "opacity-35 duration-300"
    : "opacity-0 pointer-events-none duration-700";
  const visibilityClass = visible ? "opacity-100 duration-150" : idleClass;
  const overflowClass =
    orientation === "both"
      ? "overflow-scroll"
      : orientation === "horizontal"
        ? "overflow-x-scroll overflow-y-hidden"
        : "overflow-y-scroll overflow-x-hidden";

  return (
    <div className={"relative flex flex-col " + (className ?? "")}>
      <div
        ref={viewportRef}
        onScroll={handleScroll}
        className={
          "flex-1 min-h-0 w-full hide-native-scrollbar " +
          overflowClass +
          " " +
          (viewportClassName ?? "")
        }
      >
        {children}
      </div>
      {hasVerticalOverflow && (
        <div
          aria-hidden
          className={
            "absolute top-0 right-0.5 w-1.5 rounded-full bg-ink/30 hover:bg-ink/50 cursor-pointer transition-opacity " +
            visibilityClass
          }
          ref={verticalThumbRef}
          style={{ height: MIN_THUMB_SIZE }}
          onPointerDown={onThumbPointerDown("y")}
          onPointerMove={onThumbPointerMove}
          onPointerUp={onThumbPointerUp}
          onPointerCancel={onThumbPointerUp}
        />
      )}
      {hasHorizontalOverflow && (
        <div
          aria-hidden
          className={
            "absolute bottom-0.5 left-0 h-1.5 rounded-full bg-ink/30 hover:bg-ink/50 cursor-pointer transition-opacity " +
            visibilityClass
          }
          ref={horizontalThumbRef}
          style={{ width: MIN_THUMB_SIZE }}
          onPointerDown={onThumbPointerDown("x")}
          onPointerMove={onThumbPointerMove}
          onPointerUp={onThumbPointerUp}
          onPointerCancel={onThumbPointerUp}
        />
      )}
    </div>
  );
});

export default ScrollArea;
