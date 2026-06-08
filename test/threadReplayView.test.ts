import { describe, expect, it } from "vitest";
import type {
  Agent,
  SessionInfo,
  ThreadInfo,
  ThreadReplayInfo,
  ThreadReplaySessionInfo,
  ThreadReplaySessionSourceInfo,
} from "../src/api";
import type { PendingNewChatSession } from "../src/navigation";
import type { LiveRuntimeState, LiveTurn } from "../src/runtimeChat";
import {
  buildThreadSessionLanes,
  groupReplaySessionsByThreadKind,
  replaySourceKey,
  replaySourceTitle,
} from "../src/threadReplayView";

const t = (key: string, vars?: Record<string, string | number>) => {
  if (key === "thread.pending_lane") return "Pending lane";
  if (key === "thread.replay_group.round") return `Round ${vars?.value}`;
  if (key === "thread.replay_group.round_lane") return `Round ${vars?.round} / ${vars?.lane}`;
  if (key === "thread.replay_source.stage") return "Stage";
  if (key === "thread.replay_source.plan_task") return "Task";
  if (key === "thread.replay_source.astra_internal") return "Astra";
  return key;
};

describe("buildThreadSessionLanes", () => {
  it("keeps one lane per replay session while preserving multiple sources", () => {
    const replay: ThreadReplayInfo = {
      threadId: "thread-1",
      kind: "workflow",
      sessions: [
        replaySession("codex", "session-1", session("codex", "session-1"), [
          source({ kind: "thread", label: "Thread", createdAt: 1 }),
          source({ kind: "stage", stageId: "stage-1", label: "Build", createdAt: 2 }),
        ]),
      ],
    };

    const lanes = buildThreadSessionLanes({
      thread: thread("workflow"),
      replay,
      liveState: emptyLiveState(),
      runtimeSessionAliases: {},
      pendingNewChats: {},
      t,
    });

    expect(lanes).toHaveLength(1);
    expect(lanes[0].groupKey).toBe("stage:stage-1");
    expect(lanes[0].sources.map((item) => item.kind)).toEqual(["thread", "stage"]);
    expect(lanes[0].status).toBe("history");
  });

  it("merges a live runtime alias into the matching replay lane", () => {
    const replay: ThreadReplayInfo = {
      threadId: "thread-1",
      kind: "teamwork",
      sessions: [replaySession("codex", "session-1", session("codex", "session-1"), [])],
    };

    const lanes = buildThreadSessionLanes({
      thread: thread("teamwork"),
      replay,
      liveState: {
        sessions: {
          "runtime-1": liveSession("codex", "runtime-1", "session-1", false),
        },
        lastSequence: 1,
      },
      runtimeSessionAliases: { "codex:session-1": "runtime-1" },
      pendingNewChats: {},
      t,
    });

    expect(lanes).toHaveLength(1);
    expect(lanes[0].sessioRuntimeSessionId).toBe("runtime-1");
    expect(lanes[0].status).toBe("live");
  });

  it("adds a pending lane before the real agent session id is known", () => {
    const pending: PendingNewChatSession = {
      sessioRuntimeSessionId: "runtime-pending",
      agent: "claude",
      projectPath: "/tmp/project",
      projectName: "Project",
      prompt: "continue",
      timestamp: 10,
      threadLink: { threadId: "thread-1", stageId: null },
      suppressAutoSelect: true,
      origin: "thread_multi_session",
    };

    const lanes = buildThreadSessionLanes({
      thread: thread("teamwork"),
      replay: { threadId: "thread-1", kind: "teamwork", sessions: [] },
      liveState: emptyLiveState(),
      runtimeSessionAliases: {},
      pendingNewChats: { "runtime-pending": pending },
      t,
    });

    expect(lanes).toHaveLength(1);
    expect(lanes[0].laneId).toBe("claude:runtime-pending:pending:runtime-pending");
    expect(lanes[0].status).toBe("pending");
    expect(lanes[0].groupLabel).toBe("Pending lane");
  });

  it("does not duplicate a pending lane once replay has the same runtime alias", () => {
    const replay: ThreadReplayInfo = {
      threadId: "thread-1",
      kind: "teamwork",
      sessions: [replaySession("codex", "session-1", session("codex", "session-1"), [])],
    };
    const pending: PendingNewChatSession = {
      sessioRuntimeSessionId: "runtime-1",
      agent: "codex",
      projectPath: "/tmp/project",
      projectName: "Project",
      prompt: "continue",
      timestamp: 10,
      threadLink: { threadId: "thread-1", stageId: null },
      suppressAutoSelect: true,
      origin: "thread_multi_session",
    };

    const lanes = buildThreadSessionLanes({
      thread: thread("teamwork"),
      replay,
      liveState: {
        sessions: {
          "runtime-1": liveSession("codex", "runtime-1", "session-1", false),
        },
        lastSequence: 1,
      },
      runtimeSessionAliases: { "codex:session-1": "runtime-1" },
      pendingNewChats: { "runtime-1": pending },
      t,
    });

    expect(lanes).toHaveLength(1);
    expect(lanes[0].sessionId).toBe("session-1");
  });
});

describe("groupReplaySessionsByThreadKind", () => {
  it("groups workflow sessions by stage and debate sessions by round lane", () => {
    const workflow = groupReplaySessionsByThreadKind({
      threadId: "thread-1",
      kind: "workflow",
      sessions: [
        replaySession("codex", "session-1", null, [source({ kind: "stage", stageId: "stage-1", label: "Build" })]),
        replaySession("claude", "session-2", null, [source({ kind: "stage", stageId: "stage-1", label: "Build" })]),
      ],
    }, t);
    const debate = groupReplaySessionsByThreadKind({
      threadId: "thread-2",
      kind: "debate",
      sessions: [
        replaySession("codex", "session-3", null, [
          source({ kind: "plan_task", planRoundId: "round-123456789", planTaskId: "task-1", label: "Pro lane" }),
        ]),
      ],
    }, t);

    expect(workflow).toHaveLength(1);
    expect(workflow[0].sessions.map((item) => item.sessionId)).toEqual(["session-2", "session-1"]);
    expect(debate[0].key).toBe("debate:round-123456789:task-1");
    expect(debate[0].label).toContain("Pro");
  });
});

describe("replay sources", () => {
  it("builds stable source keys and snapshot titles", () => {
    const item = source({
      kind: "plan_task",
      planRoundId: "round-1",
      planTaskId: "task-1",
      role: "runtime",
      label: "Runtime task",
      stageSnapshotJson: JSON.stringify({ name: "Build" }),
      assistantSnapshotJson: JSON.stringify({ name: "Builder", agent: { model: "gpt" } }),
      agentSnapshotJson: JSON.stringify({ agentInfo: { displayName: "Codex", model: "gpt-5" } }),
      createdAt: 12,
    });

    expect(replaySourceKey(item)).toBe("plan_task:round-1:task-1:runtime:12");
    expect(replaySourceTitle(item)).toContain("Stage snapshot: Build");
    expect(replaySourceTitle(item)).toContain("Assistant snapshot: Builder / gpt");
    expect(replaySourceTitle(item)).toContain("Agent snapshot: Codex / gpt-5");
  });
});

function thread(kind: ThreadInfo["kind"]): Pick<ThreadInfo, "id" | "kind"> {
  return { id: "thread-1", kind };
}

function session(agent: Agent, id: string): SessionInfo {
  return {
    id,
    agent,
    forkedFromAgent: null,
    forkedFromId: null,
    projectPath: "/tmp/project",
    projectName: "Project",
    startedAt: 1,
    updatedAt: id.endsWith("2") ? 2 : 1,
    messageCount: 1,
    renameTitle: null,
    title: id,
    firstUserMessage: id,
    filePath: "/tmp/session.jsonl",
    fileSize: 1,
    partial: false,
    available: true,
    archived: false,
    subagents: [],
  };
}

function replaySession(
  agent: Agent,
  sessionId: string,
  row: SessionInfo | null,
  sources: ThreadReplaySessionSourceInfo[],
): ThreadReplaySessionInfo {
  return {
    agent,
    sessionId,
    session: row,
    sources,
    firstSeenAt: row?.startedAt ?? 1,
    lastSeenAt: row?.updatedAt ?? (sessionId.endsWith("2") ? 2 : 1),
  };
}

function source(patch: Partial<ThreadReplaySessionSourceInfo>): ThreadReplaySessionSourceInfo {
  return {
    kind: "thread",
    threadId: "thread-1",
    stageId: null,
    planRoundId: null,
    planTaskId: null,
    astraRunId: null,
    role: null,
    label: null,
    stageSnapshotJson: null,
    assistantSnapshotJson: null,
    agentSnapshotJson: null,
    createdAt: 1,
    ...patch,
  };
}

function emptyLiveState(): LiveRuntimeState {
  return { sessions: {}, lastSequence: 0 };
}

function liveSession(
  agent: Agent,
  sessioRuntimeSessionId: string,
  agentRuntimeSessionId: string,
  ended: boolean,
): LiveRuntimeState["sessions"][string] {
  return {
    sessioRuntimeSessionId,
    agent,
    agentRuntimeSessionId,
    transport: "acp",
    workspacePath: "/tmp/project",
    capabilities: runtimeCapabilities(),
    turns: [liveTurn("turn-1")],
    sessionState: {
      plan: null,
      availableCommands: [],
      currentModeId: null,
      configOptions: [],
      sessionInfo: null,
    },
    protocolMessages: [],
    ended,
  };
}

function liveTurn(turnId: string): LiveTurn {
  return {
    turnId,
    status: "streaming",
    blocks: [],
    tools: [],
    permissions: [],
    protocolMessages: [],
    stopReason: null,
    error: null,
    startedAt: 1,
    updatedAt: 1,
  };
}

function runtimeCapabilities(): LiveRuntimeState["sessions"][string]["capabilities"] {
  return {
    supportsCancel: false,
    supportsPermissions: false,
    supportsToolDeltas: false,
    supportsLoadSession: false,
    supportsResume: false,
    supportsFork: false,
    supportsImageAttachments: false,
    supportsAudioAttachments: false,
    supportsEmbeddedContext: false,
    supportsAttachments: false,
    supportsModes: false,
  };
}
