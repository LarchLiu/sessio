import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import { respondAgentPermission } from "../api";
import { acpViewModelToRenderItems, renderItemKeys } from "../acpRenderItems";
import {
  appChatDrawerDimensions,
  clampAppChatDrawerHeight,
  readAppChatDrawerHeight,
  storeAppChatDrawerHeight,
} from "../appChatDrawer";
import { useI18n } from "../i18n";
import type { AcpViewModel } from "../runtimeChat";
import { AcpRenderItems } from "./AcpTranscriptPanel";
import ScrollArea from "./ScrollArea";
import Tooltip from "./Tooltip";

export default function AppChatTranscriptDrawer({
  appId,
  viewModel,
  runtimeSessionId,
  liveTurnIds,
  workingTurnId,
  onCollapse,
  onError,
}: {
  appId: string;
  viewModel: AcpViewModel;
  runtimeSessionId: string;
  liveTurnIds: string[];
  workingTurnId: string | null;
  onCollapse: () => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [height, setHeight] = useState(() => readAppChatDrawerHeight(appId));
  const [dragging, setDragging] = useState(false);
  const dragStartRef = useRef<{ y: number; height: number } | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const pinnedToBottomRef = useRef(true);
  const bubbleRefs = useRef<(HTMLDivElement | null)[]>([]);
  const liveTurnIdSet = useMemo(
    () => new Set(liveTurnIds),
    [liveTurnIds],
  );
  const items = useMemo(
    () => acpViewModelToRenderItems(viewModel, liveTurnIdSet, workingTurnId ?? ""),
    [liveTurnIdSet, viewModel, workingTurnId],
  );
  const itemKeys = useMemo(() => renderItemKeys(items), [items]);
  const activityKey = useMemo(
    () =>
      viewModel.turns
        .map((turn) => `${turn.turnId}:${turn.status}:${turn.updatedAt}:${turn.blocks.length}`)
        .join("|"),
    [viewModel.turns],
  );

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || !pinnedToBottomRef.current) return;
    viewport.scrollTop = viewport.scrollHeight;
  }, [activityKey, height]);

  useEffect(() => {
    const clampToViewport = () => {
      setHeight((current) => clampAppChatDrawerHeight(current));
    };
    window.addEventListener("resize", clampToViewport);
    return () => window.removeEventListener("resize", clampToViewport);
  }, []);

  const setStoredHeight = useCallback(
    (nextHeight: number) => {
      setHeight(storeAppChatDrawerHeight(appId, nextHeight));
    },
    [appId],
  );

  const startResize = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      event.preventDefault();
      event.currentTarget.setPointerCapture(event.pointerId);
      dragStartRef.current = { y: event.clientY, height };
      setDragging(true);
    },
    [height],
  );

  const resize = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const start = dragStartRef.current;
      if (!start) return;
      setStoredHeight(start.height + start.y - event.clientY);
    },
    [setStoredHeight],
  );

  const stopResize = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragStartRef.current) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    dragStartRef.current = null;
    setDragging(false);
  }, []);

  const handlePermissionResponse = useCallback(
    async (sessionId: string, requestId: string, optionId: string) => {
      try {
        await respondAgentPermission(sessionId, requestId, optionId);
        onError(null);
      } catch (error) {
        onError(String(error));
      }
    },
    [onError],
  );

  return (
    <div
      className={"flex min-h-0 flex-col " + (dragging ? "select-none" : "")}
      style={{ height: clampAppChatDrawerHeight(height) }}
    >
      <div
        role="separator"
        tabIndex={0}
        aria-label={t("apps.chat_resize")}
        aria-orientation="horizontal"
        aria-valuemin={appChatDrawerDimensions.minHeight}
        aria-valuemax={appChatDrawerDimensions.maxHeight}
        aria-valuenow={clampAppChatDrawerHeight(height)}
        className="group flex h-2 shrink-0 touch-none cursor-row-resize items-center justify-center outline-none"
        onPointerDown={startResize}
        onPointerMove={resize}
        onPointerUp={stopResize}
        onPointerCancel={stopResize}
        onKeyDown={(event) => {
          if (event.key === "ArrowUp") {
            event.preventDefault();
            setStoredHeight(height + 16);
          } else if (event.key === "ArrowDown") {
            event.preventDefault();
            setStoredHeight(height - 16);
          } else if (event.key === "Home") {
            event.preventDefault();
            setStoredHeight(appChatDrawerDimensions.minHeight);
          } else if (event.key === "End") {
            event.preventDefault();
            setStoredHeight(appChatDrawerDimensions.maxHeight);
          }
        }}
      >
        <span className="h-0.5 w-10 rounded-full bg-ink/10 transition group-hover:bg-ink/25 group-focus-visible:bg-ink/25" />
      </div>
      <div className="flex h-8 shrink-0 items-center justify-between border-b border-ink/[0.06] px-3">
        <span className="text-caption font-medium text-ink/55">{t("apps.chat_history")}</span>
        <Tooltip content={t("detail.collapse")} placement="top">
          <button
            type="button"
            aria-label={t("detail.collapse")}
            onClick={onCollapse}
            className="flex h-6 w-6 items-center justify-center rounded-md text-ink/40 transition hover:bg-ink/[0.06] hover:text-ink/70"
          >
            <ChevronDown className="h-3.5 w-3.5" />
          </button>
        </Tooltip>
      </div>
      <ScrollArea
        ref={viewportRef}
        className="min-h-0 flex-1"
        viewportClassName="px-3 py-2"
        onScroll={(viewport) => {
          pinnedToBottomRef.current =
            viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight < 48;
        }}
      >
        {items.length > 0 ? (
          <div className="flex flex-col gap-2">
            <AcpRenderItems
              items={items}
              itemKeys={itemKeys}
              bubbleRefs={bubbleRefs}
              sessioRuntimeSessionId={runtimeSessionId}
              defaultMessageExpanded
              onPreviewImage={() => {}}
              onPreviewFile={() => {}}
              onFilePreviewError={onError}
              onPermissionResponse={handlePermissionResponse}
            />
          </div>
        ) : (
          <div className="flex h-full items-center justify-center text-body-sm text-ink/35">
            {t("detail.no_messages")}
          </div>
        )}
      </ScrollArea>
    </div>
  );
}
