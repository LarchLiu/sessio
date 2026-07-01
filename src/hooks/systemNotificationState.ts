import type { SessionInfo, ThreadIndexItemInfo } from "../api";
import {
  intersectsSet,
  sessionDisplayTitle,
  sessionIdentity,
  sessionIdentityKey,
  sessionUnreadKeys,
  threadUnreadKeys,
} from "../appUtils";
import type { DetailMode, PendingNewChatSession } from "../navigation";
import {
  liveSessionActivity,
  liveThreadActivity,
  type LiveRuntimeSession,
  type LiveSessionActivity,
} from "../runtimeChat";

export type NotificationRowKind = "session" | "thread";
export type NotificationTranslate = (
  key: string,
  vars?: Record<string, string | number>,
) => string;

export interface NotificationRowSnapshot {
  key: string;
  kind: NotificationRowKind;
  label: string;
  sessionIdentityKey: string | null;
  threadId: string | null;
  pendingOrigin: PendingNewChatSession["origin"] | null;
  liveActivity: LiveSessionActivity;
  unread: boolean;
  visible: boolean;
}

export interface NotificationSnapshot {
  rows: Map<string, NotificationRowSnapshot>;
  unreadCount: number;
}

export interface NotificationEvent {
  kind: "permission" | "failed" | "unread";
  row: NotificationRowSnapshot;
}

export function buildNotificationSnapshot({
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
}: {
  sessions: SessionInfo[];
  threadIndexItems: ThreadIndexItemInfo[];
  liveSessions: Record<string, LiveRuntimeSession>;
  runtimeSessionAliases: Record<string, string>;
  pendingNewChats: Record<string, PendingNewChatSession>;
  unreadSessionIds: Set<string>;
  selected: SessionInfo | null;
  selectedThreadId: string | null;
  detailMode: DetailMode;
  windowFocused: boolean;
  t: NotificationTranslate;
}): NotificationSnapshot {
  const rows = new Map<string, NotificationRowSnapshot>();
  const selectedSessionIdentity = selected ? sessionIdentityKey(selected) : null;
  const selectedRuntimeSessionId = selectedSessionIdentity
    ? runtimeSessionAliases[selectedSessionIdentity] ?? selected?.id ?? null
    : null;

  for (const thread of threadIndexItems) {
    const unread = intersectsSet(
      threadUnreadKeys(thread, runtimeSessionAliases, liveSessions),
      unreadSessionIds,
    );
    const liveActivity = liveThreadActivity(
      thread.threadId,
      thread.sessionKeys,
      liveSessions,
      runtimeSessionAliases,
    );
    rows.set(`thread:${thread.threadId}`, {
      key: `thread:${thread.threadId}`,
      kind: "thread",
      label: thread.goal || t("thread.goal_placeholder"),
      sessionIdentityKey: null,
      threadId: thread.threadId,
      pendingOrigin: null,
      liveActivity,
      unread,
      visible: isThreadVisible({
        threadId: thread.threadId,
        selectedThreadId,
        detailMode,
        windowFocused,
      }),
    });
  }

  const bestSessionByIdentity = new Map<string, SessionInfo>();
  for (const session of sessions) {
    const identity = sessionIdentityKey(session);
    const current = bestSessionByIdentity.get(identity);
    if (!current || sessionSortTime(session) > sessionSortTime(current)) {
      bestSessionByIdentity.set(identity, session);
    }
  }

  for (const session of bestSessionByIdentity.values()) {
    const identity = sessionIdentityKey(session);
    const runtimeSessionId = runtimeSessionAliases[identity] ?? session.id;
    const liveActivity = liveSessionActivity(liveSessions[runtimeSessionId]);
    const unread = intersectsSet(
      sessionUnreadKeys(session, runtimeSessionAliases),
      unreadSessionIds,
    );
    rows.set(identity, {
      key: identity,
      kind: "session",
      label: sessionDisplayTitle(session) ?? t("notification.session.fallback"),
      sessionIdentityKey: identity,
      threadId: null,
      pendingOrigin: null,
      liveActivity,
      unread,
      visible: isSessionVisible({
        sessionIdentityKey: identity,
        runtimeSessionId,
        selectedSessionIdentity,
        selectedRuntimeSessionId,
        detailMode,
        windowFocused,
      }),
    });
  }

  for (const pending of Object.values(pendingNewChats)) {
    const rowKey = pendingSessionRowKey(pending);
    if (rows.has(rowKey)) continue;

    if (pending.threadLink?.threadId) {
      const threadId = pending.threadLink.threadId;
      rows.set(rowKey, {
        key: rowKey,
        kind: "thread",
        label: pending.prompt || t("thread.goal_placeholder"),
        sessionIdentityKey: null,
        threadId,
        pendingOrigin: pending.origin ?? null,
        liveActivity: liveSessionActivity(liveSessions[pending.sessioRuntimeSessionId]),
        unread: unreadSessionIds.has(pending.sessioRuntimeSessionId),
        visible: isThreadVisible({
          threadId,
          selectedThreadId,
          detailMode,
          windowFocused,
        }),
      });
      continue;
    }

    const identity = sessionIdentity(pending.agent, pending.sessioRuntimeSessionId);
    rows.set(rowKey, {
      key: rowKey,
      kind: "session",
      label: pending.prompt || t("notification.session.fallback"),
      sessionIdentityKey: identity,
      threadId: null,
      pendingOrigin: pending.origin ?? null,
      liveActivity: liveSessionActivity(liveSessions[pending.sessioRuntimeSessionId]),
      unread: unreadSessionIds.has(pending.sessioRuntimeSessionId),
      visible: isSessionVisible({
        sessionIdentityKey: identity,
        runtimeSessionId: pending.sessioRuntimeSessionId,
        selectedSessionIdentity,
        selectedRuntimeSessionId,
        detailMode,
        windowFocused,
        pendingOrigin: pending.origin ?? null,
      }),
    });
  }

  let unreadCount = 0;
  for (const row of rows.values()) {
    if (row.unread) unreadCount += 1;
  }

  return {
    rows,
    unreadCount,
  };
}

export function diffNotificationSnapshots(
  previous: NotificationSnapshot | null,
  next: NotificationSnapshot,
): NotificationEvent[] {
  if (!previous) return [];
  const events: NotificationEvent[] = [];

  for (const row of next.rows.values()) {
    const prev = previous.rows.get(row.key);
    if (!shouldNotifyRow(row)) continue;

    if (row.liveActivity === "permission" && prev?.liveActivity !== "permission") {
      events.push({ kind: "permission", row });
      continue;
    }

    if (row.liveActivity === "failed" && prev?.liveActivity !== "failed") {
      events.push({ kind: "failed", row });
      continue;
    }

    if (row.unread && !prev?.unread) {
      events.push({ kind: "unread", row });
    }
  }

  return dedupeNotificationEvents(events);
}

export function notificationTitle(
  event: NotificationEvent,
  t: NotificationTranslate,
): string {
  if (event.kind === "permission") return t("notification.permission.title");
  if (event.kind === "failed") return t("notification.failed.title");
  return t("notification.unread.title");
}

export function notificationBody(
  event: NotificationEvent,
  t: NotificationTranslate,
): string {
  if (event.kind === "permission") {
    return t("notification.permission.body", {
      target: event.row.label,
      tool: "tool",
    });
  }
  if (event.kind === "failed") {
    return t("notification.failed.body", { target: event.row.label });
  }
  return t("notification.unread.body", { target: event.row.label });
}

export function notificationSound(platform: string): string | undefined {
  if (/Mac/i.test(platform)) return "Ping";
  if (/Linux/i.test(platform)) return "message-new-instant";
  return undefined;
}

export function isSessionVisible({
  sessionIdentityKey,
  runtimeSessionId,
  selectedSessionIdentity,
  selectedRuntimeSessionId,
  detailMode,
  windowFocused,
  pendingOrigin = null,
}: {
  sessionIdentityKey: string;
  runtimeSessionId: string | null;
  selectedSessionIdentity: string | null;
  selectedRuntimeSessionId: string | null;
  detailMode: DetailMode;
  windowFocused: boolean;
  pendingOrigin?: PendingNewChatSession["origin"] | null;
}): boolean {
  if (!windowFocused) return false;
  if (pendingOrigin === "new_chat" && detailMode === "chat") return true;
  if (detailMode !== "chat" && detailMode !== "threadChat") return false;
  if (selectedSessionIdentity && selectedSessionIdentity === sessionIdentityKey) return true;
  if (selectedRuntimeSessionId && runtimeSessionId && selectedRuntimeSessionId === runtimeSessionId) {
    return true;
  }
  return false;
}

export function isThreadVisible({
  threadId,
  selectedThreadId,
  detailMode,
  windowFocused,
}: {
  threadId: string;
  selectedThreadId: string | null;
  detailMode: DetailMode;
  windowFocused: boolean;
}): boolean {
  if (!windowFocused) return false;
  if (selectedThreadId !== threadId) return false;
  return detailMode === "threadMultiSessionChat" || detailMode === "project";
}

export function pendingSessionRowKey(pending: PendingNewChatSession): string {
  if (pending.threadLink?.threadId) return `thread:${pending.threadLink.threadId}`;
  return `pending:${pending.sessioRuntimeSessionId}`;
}

function shouldNotifyRow(row: NotificationRowSnapshot): boolean {
  return !row.visible;
}

function dedupeNotificationEvents(events: NotificationEvent[]): NotificationEvent[] {
  const bestByKey = new Map<string, NotificationEvent>();
  for (const event of events) {
    const current = bestByKey.get(event.row.key);
    if (!current || eventPriority(event) > eventPriority(current)) {
      bestByKey.set(event.row.key, event);
    }
  }
  return Array.from(bestByKey.values());
}

function eventPriority(event: NotificationEvent): number {
  switch (event.kind) {
    case "permission":
      return 3;
    case "failed":
      return 2;
    case "unread":
      return 1;
  }
}

function sessionSortTime(session: SessionInfo): number {
  return session.updatedAt ?? session.startedAt ?? 0;
}
