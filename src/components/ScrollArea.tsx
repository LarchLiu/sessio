import {
  type CSSProperties,
  forwardRef,
  type ReactNode,
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
  style?: CSSProperties;
  viewportClassName?: string;
  children: ReactNode;
  persistScrollbars?: boolean;
  orientation?: "vertical" | "horizontal" | "both";
  interactionMode?: "default" | "thumbs-only" | "capture-wheel";
  scrollbarInset?: "default" | "flush";
  onScroll?: (viewport: HTMLDivElement) => void;
};

const ScrollArea = forwardRef<HTMLDivElement, Props>(function ScrollArea(
  {
    className,
    style,
    viewportClassName,
    children,
    persistScrollbars = false,
    orientation = "vertical",
    interactionMode = "default",
    scrollbarInset = "default",
    onScroll,
  },
  ref,
) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const verticalThumbRef = useRef<HTMLDivElement>(null);
  const horizontalThumbRef = useRef<HTMLDivElement>(null);
  const hideTimerRef = useRef<number | undefined>(undefined);
  const dragStateRef = useRef<DragState | null>(null);
  const updateFrameRef = useRef<number | null>(null);
  const hoverRef = useRef(false);

  const [visible, setVisible] = useState(false);
  const [hasVerticalOverflow, setHasVerticalOverflow] = useState(false);
  const [hasHorizontalOverflow, setHasHorizontalOverflow] = useState(false);

  const enableVertical = orientation === "vertical" || orientation === "both";
  const enableHorizontal = orientation === "horizontal" || orientation === "both";
  const thumbsOnlyInteraction = interactionMode === "thumbs-only";
  const captureWheelInteraction = interactionMode === "capture-wheel";

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

  const showVisible = useCallback(() => {
    setVisible(true);
    if (hideTimerRef.current !== undefined)
      window.clearTimeout(hideTimerRef.current);
  }, []);

  const flashVisible = useCallback(() => {
    showVisible();
    hideTimerRef.current = window.setTimeout(() => {
      if (!dragStateRef.current && !hoverRef.current) setVisible(false);
    }, HIDE_DELAY);
  }, [showVisible]);

  const handleScroll = useCallback(() => {
    updateThumbs();
    flashVisible();
    const vp = viewportRef.current;
    if (vp) onScroll?.(vp);
  }, [updateThumbs, flashVisible, onScroll]);

  const scheduleUpdateThumbs = useCallback(() => {
    if (updateFrameRef.current !== null) return;
    updateFrameRef.current = window.requestAnimationFrame(() => {
      updateFrameRef.current = null;
      updateThumbs();
    });
  }, [updateThumbs]);

  useLayoutEffect(() => {
    updateThumbs();
  });

  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;
    const ro = new ResizeObserver(() => scheduleUpdateThumbs());
    ro.observe(vp);
    for (const child of Array.from(vp.children)) ro.observe(child);
    return () => ro.disconnect();
  }, [scheduleUpdateThumbs]);

  useEffect(() => {
    return () => {
      if (hideTimerRef.current !== undefined)
        window.clearTimeout(hideTimerRef.current);
      if (updateFrameRef.current !== null)
        window.cancelAnimationFrame(updateFrameRef.current);
    };
  }, []);

  const onThumbPointerDown =
    (axis: Axis) => (e: React.PointerEvent<HTMLDivElement>) => {
      const vp = viewportRef.current;
      if (!vp) return;
      e.preventDefault();
      e.stopPropagation();
      (e.target as Element).setPointerCapture(e.pointerId);
      dragStateRef.current = {
        axis,
        startPointer: axis === "y" ? e.clientY : e.clientX,
        startScroll: axis === "y" ? vp.scrollTop : vp.scrollLeft,
      };
      showVisible();
    };

  const onThumbPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragStateRef.current;
    const vp = viewportRef.current;
    if (!drag || !vp) return;
    e.stopPropagation();

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
    e.stopPropagation();
    (e.target as Element).releasePointerCapture(e.pointerId);
    dragStateRef.current = null;
    flashVisible();
  };

  const onRootPointerEnter = () => {
    hoverRef.current = true;
    showVisible();
  };

  const onRootPointerLeave = () => {
    hoverRef.current = false;
    if (!dragStateRef.current) flashVisible();
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
  const viewportFlexClass =
    orientation === "horizontal" ? "flex-none" : "flex-1";
  const verticalThumbEdgeClass =
    scrollbarInset === "flush" ? "right-0" : "right-0.5";
  const horizontalThumbEdgeClass =
    scrollbarInset === "flush" ? "bottom-0" : "bottom-0.5";
  const stopWheelPropagation = captureWheelInteraction
    ? (event: React.WheelEvent<HTMLDivElement>) => {
        event.stopPropagation();
      }
    : undefined;

  return (
    <div
      style={style}
      className={
        "relative flex min-h-0 flex-col overflow-hidden " +
        (thumbsOnlyInteraction ? "pointer-events-none " : "") +
        (className ?? "")
      }
      onPointerEnter={onRootPointerEnter}
      onPointerLeave={onRootPointerLeave}
    >
      <div
        ref={viewportRef}
        onScroll={handleScroll}
        style={{ maxHeight: "inherit" }}
        className={
          viewportFlexClass +
          " min-h-0 w-full hide-native-scrollbar " +
          overflowClass +
          (thumbsOnlyInteraction ? " pointer-events-none " : " ") +
          " " +
          (viewportClassName ?? "")
        }
        onWheelCapture={stopWheelPropagation}
        onWheel={stopWheelPropagation}
      >
        {children}
      </div>
      {hasVerticalOverflow && (
        <div
          aria-hidden
          className={
            "pointer-events-auto absolute top-0 z-30 w-2 rounded-full bg-ink/30 hover:bg-ink/50 cursor-pointer transition-opacity " +
            verticalThumbEdgeClass +
            " " +
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
            "pointer-events-auto absolute left-0 z-30 h-2 rounded-full bg-ink/30 hover:bg-ink/50 cursor-pointer transition-opacity " +
            horizontalThumbEdgeClass +
            " " +
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
