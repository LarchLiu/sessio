import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import type { SessionInfo, ThreadIndexItemInfo } from "../api";
import type { DetailMode, PendingNewChatSession } from "../navigation";
import type { LiveRuntimeSession } from "../runtimeChat";
import {
  buildNotificationSnapshot,
  diffNotificationSnapshots,
  notificationBody,
  notificationSound,
  notificationTitle,
  type NotificationTranslate,
  type NotificationSnapshot,
} from "./systemNotificationState";

export function useSystemNotifications({
  t,
  sessions,
  threadIndexItems,
  liveSessions,
  runtimeSessionAliases,
  pendingNewChats,
  unreadSessionIds,
  selected,
  selectedThreadId,
  detailMode,
}: {
  t: NotificationTranslate;
  sessions: SessionInfo[];
  threadIndexItems: ThreadIndexItemInfo[];
  liveSessions: Record<string, LiveRuntimeSession>;
  runtimeSessionAliases: Record<string, string>;
  pendingNewChats: Record<string, PendingNewChatSession>;
  unreadSessionIds: Set<string>;
  selected: SessionInfo | null;
  selectedThreadId: string | null;
  detailMode: DetailMode;
}) {
  const windowRef = useRef(getCurrentWindow());
  const previousSnapshotRef = useRef<NotificationSnapshot | null>(null);
  const notificationPermissionRef = useRef<boolean | null>(null);
  const [windowFocused, setWindowFocused] = useState(true);

  useEffect(() => {
    let cancelled = false;
    windowRef.current.isFocused()
      .then((focused) => {
        if (!cancelled) setWindowFocused(focused);
      })
      .catch(() => {});

    let unlisten: (() => void) | null = null;
    windowRef.current.onFocusChanged(({ payload }) => {
      setWindowFocused(Boolean(payload));
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const snapshot = useMemo(
    () =>
      buildNotificationSnapshot({
        sessions,
        threadIndexItems,
        liveSessions,
        runtimeSessionAliases,
        pendingNewChats,
        unreadSessionIds,
        selected,
        selectedThreadId,
        detailMode,
        windowFocused,
        t,
      }),
    [
      detailMode,
      liveSessions,
      pendingNewChats,
      runtimeSessionAliases,
      selected,
      selectedThreadId,
      sessions,
      t,
      threadIndexItems,
      unreadSessionIds,
      windowFocused,
    ],
  );

  useEffect(() => {
    syncBadge(snapshot.unreadCount).catch(() => {});
  }, [snapshot.unreadCount]);

  useEffect(() => {
    const events = diffNotificationSnapshots(previousSnapshotRef.current, snapshot);
    previousSnapshotRef.current = snapshot;
    if (events.length === 0) return;

    void (async () => {
      const permissionGranted = await ensureNotificationPermission(notificationPermissionRef);
      if (!permissionGranted) return;

      const sound = notificationSound(typeof navigator === "undefined" ? "" : navigator.platform);
      for (const event of events) {
        sendNotification({
          title: notificationTitle(event, t),
          body: notificationBody(event, t),
          sound,
        });
      }
    })();
  }, [snapshot, t]);
}

async function ensureNotificationPermission(
  notificationPermissionRef: React.MutableRefObject<boolean | null>,
): Promise<boolean> {
  if (notificationPermissionRef.current != null) return notificationPermissionRef.current;
  try {
    let granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
    notificationPermissionRef.current = granted;
    return granted;
  } catch {
    notificationPermissionRef.current = false;
    return false;
  }
}

async function syncBadge(unreadCount: number): Promise<void> {
  const value = unreadCount > 0 ? unreadCount : undefined;
  const label = unreadCount > 0 ? String(unreadCount) : undefined;
  await Promise.allSettled([
    getCurrentWindow().setBadgeCount(value),
    getCurrentWindow().setBadgeLabel(label),
  ]);
}
