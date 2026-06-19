import { describe, expect, it } from "vitest";
import type { SessionInfo, ThreadIndexItemInfo } from "../src/api";
import type { PendingNewChatSession } from "../src/navigation";
import {
  buildNotificationSnapshot,
  diffNotificationSnapshots,
  notificationSound,
} from "../src/hooks/systemNotificationState";
import { emptyAcpSessionState, type LiveRuntimeSession } from "../src/runtimeChat";

const t = (key: string) => key;

function makeSession(overrides: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id: "session-1",
    agent: "codex",
    forkedFromAgent: null,
    forkedFromId: null,
    projectPath: "/workspace",
    projectName: "Workspace",
    startedAt: 1,
    updatedAt: 1,
    messageCount: 0,
    renameTitle: "Ship beta",
    title: "Ship beta",
    firstUserMessage: "Ship beta",
    filePath: "/workspace/session.jsonl",
    fileSize: 1,
    partial: false,
    available: true,
    archived: false,
    origin: "chat",
    scheduledTaskId: null,
    isAuxiliary: false,
    subagents: [],
    ...overrides,
  };
}

function makeThread(overrides: Partial<ThreadIndexItemInfo> = {}): ThreadIndexItemInfo {
  return {
    threadId: "thread-1",
    projectId: "project-1",
    goal: "Finish release",
    kind: "process",
    origin: "manual",
    scheduledTaskId: null,
    createdAt: 1,
    updatedAt: 1,
    time: 1,
    sessionKeys: ["codex:session-1"],
    ...overrides,
  };
}

function makeLiveSession(
  sessioRuntimeSessionId: string,
  {
    agentRuntimeSessionId = "session-1",
    status = "streaming",
    ended = false,
    permission = false,
    metadata,
  }: {
    agentRuntimeSessionId?: string;
    status?: "pending" | "streaming" | "completed" | "failed" | "cancelled";
    ended?: boolean;
    permission?: boolean;
    metadata?: Record<string, unknown>;
  } = {},
): LiveRuntimeSession {
  return {
    sessioRuntimeSessionId,
    agent: "codex",
    agentRuntimeSessionId,
    transport: "acp",
    workspacePath: "/workspace",
    capabilities: {
      supportsCancel: true,
      supportsPermissions: true,
      supportsToolDeltas: true,
      supportsLoadSession: true,
      supportsResume: false,
      supportsFork: false,
      supportsImageAttachments: false,
      supportsAudioAttachments: false,
      supportsEmbeddedContext: false,
      supportsAttachments: false,
      supportsModes: false,
    },
    metadata,
    turns: [{
      turnId: "turn-1",
      status,
      blocks: [],
      tools: [],
      permissions: permission ? [{
        requestId: "perm-1",
        toolCall: null,
        toolName: "apply_patch",
        input: null,
        options: [],
        selectedOptionId: null,
        cancelled: false,
        raw: null,
      }] : [],
      protocolMessages: [],
      stopReason: null,
      error: null,
      startedAt: 1,
      updatedAt: 1,
    }],
    sessionState: emptyAcpSessionState(),
    protocolMessages: [],
    ended,
  };
}

function snapshot(input: {
  sessions?: SessionInfo[];
  threadIndexItems?: ThreadIndexItemInfo[];
  liveSessions?: Record<string, LiveRuntimeSession>;
  runtimeSessionAliases?: Record<string, string>;
  pendingNewChats?: Record<string, PendingNewChatSession>;
  unreadSessionIds?: Set<string>;
  selected?: SessionInfo | null;
  selectedThreadId?: string | null;
  detailMode?: "chat" | "project" | "threadChat" | "threadMultiSessionChat";
  windowFocused?: boolean;
}) {
  return buildNotificationSnapshot({
    sessions: input.sessions ?? [],
    threadIndexItems: input.threadIndexItems ?? [],
    liveSessions: input.liveSessions ?? {},
    runtimeSessionAliases: input.runtimeSessionAliases ?? {},
    pendingNewChats: input.pendingNewChats ?? {},
    unreadSessionIds: input.unreadSessionIds ?? new Set(),
    selected: input.selected ?? null,
    selectedThreadId: input.selectedThreadId ?? null,
    detailMode: input.detailMode ?? "chat",
    windowFocused: input.windowFocused ?? false,
    t,
  });
}

describe("system notification state", () => {
  it("picks portable notification sounds", () => {
    expect(notificationSound("MacIntel")).toBe("Ping");
    expect(notificationSound("Linux x86_64")).toBe("message-new-instant");
    expect(notificationSound("Win32")).toBeUndefined();
  });

  it("suppresses session notifications when the focused window is already showing that session", () => {
    const session = makeSession();
    const next = snapshot({
      sessions: [session],
      selected: session,
      detailMode: "chat",
      windowFocused: true,
      unreadSessionIds: new Set([session.id]),
    });
    expect(diffNotificationSnapshots(snapshot({}), next)).toEqual([]);
  });

  it("emits a permission notification when a background session enters permission state", () => {
    const session = makeSession();
    const previous = snapshot({
      sessions: [session],
      runtimeSessionAliases: { "codex:session-1": "runtime-1" },
      liveSessions: {
        "runtime-1": makeLiveSession("runtime-1", { status: "streaming" }),
      },
      windowFocused: false,
    });
    const next = snapshot({
      sessions: [session],
      runtimeSessionAliases: { "codex:session-1": "runtime-1" },
      liveSessions: {
        "runtime-1": makeLiveSession("runtime-1", { permission: true }),
      },
      windowFocused: false,
    });
    const events = diffNotificationSnapshots(previous, next);
    expect(events).toHaveLength(1);
    expect(events[0]?.kind).toBe("permission");
    expect(events[0]?.row.kind).toBe("session");
  });

  it("emits a failed notification when a background thread run fails", () => {
    const thread = makeThread();
    const previous = snapshot({
      threadIndexItems: [thread],
      runtimeSessionAliases: { "codex:session-1": "runtime-1" },
      liveSessions: {
        "runtime-1": makeLiveSession("runtime-1", { status: "streaming" }),
      },
      windowFocused: false,
    });
    const next = snapshot({
      threadIndexItems: [thread],
      runtimeSessionAliases: { "codex:session-1": "runtime-1" },
      liveSessions: {
        "runtime-1": makeLiveSession("runtime-1", { status: "failed", ended: true }),
      },
      windowFocused: false,
    });
    const events = diffNotificationSnapshots(previous, next);
    expect(events).toHaveLength(1);
    expect(events[0]?.kind).toBe("failed");
    expect(events[0]?.row.kind).toBe("thread");
  });

  it("emits an unread notification for a background pending thread row", () => {
    const oldSession = makeSession();
    const pending: PendingNewChatSession = {
      sessioRuntimeSessionId: "runtime-2",
      agent: "codex",
      projectPath: "/workspace",
      projectName: "Workspace",
      prompt: "New lane",
      timestamp: 2,
      suppressAutoSelect: true,
      origin: "thread_multi_session",
      threadLink: {
        threadId: "thread-1",
        stageId: null,
      },
    };
    const previous = snapshot({
      sessions: [oldSession],
      selected: oldSession,
      detailMode: "chat",
      windowFocused: true,
    });
    const next = snapshot({
      sessions: [oldSession],
      pendingNewChats: {
        "runtime-2": pending,
      },
      liveSessions: {
        "runtime-2": makeLiveSession("runtime-2", {
          agentRuntimeSessionId: "pending",
          status: "completed",
          ended: true,
          metadata: { astraThreadId: "thread-1", astraInternal: true, astraRunId: "run-1" },
        }),
      },
      unreadSessionIds: new Set(["runtime-2"]),
      selected: oldSession,
      detailMode: "chat",
      windowFocused: true,
    });
    const events = diffNotificationSnapshots(previous, next);
    expect(events).toHaveLength(1);
    expect(events[0]?.kind).toBe("unread");
    expect(events[0]?.row.kind).toBe("thread");
    expect(events[0]?.row.threadId).toBe("thread-1");
  });
});
