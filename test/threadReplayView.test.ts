import { describe, expect, it } from "vitest";
import type {
  Agent,
  AstraHandle,
  PlanRoundInfo,
  PlanTaskInfo,
  SessionInfo,
  ThreadInfo,
  ThreadReplayInfo,
  ThreadReplaySessionInfo,
  ThreadReplaySessionSourceInfo,
} from "../src/api";
import type { PendingNewChatSession } from "../src/navigation";
import type { LiveRuntimeState, LiveTurn } from "../src/runtimeChat";
import {
  buildThreadTimelineRows,
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
      kind: "process",
      sessions: [
        replaySession("codex", "session-1", session("codex", "session-1"), [
          source({ kind: "thread", label: "Thread", createdAt: 1 }),
          source({ kind: "stage", stageId: "stage-1", label: "Build", createdAt: 2 }),
        ]),
      ],
    };

    const lanes = buildThreadSessionLanes({
      thread: thread("process"),
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

  it("marks an idle live runtime lane as history once its turns are completed", () => {
    const replay: ThreadReplayInfo = {
      threadId: "thread-1",
      kind: "teamwork",
      sessions: [replaySession("codex", "session-1", session("codex", "session-1"), [])],
    };
    const completedLiveSession = liveSession("codex", "runtime-1", "session-1", false);
    completedLiveSession.turns = [liveTurn("turn-1", "completed")];

    const lanes = buildThreadSessionLanes({
      thread: thread("teamwork"),
      replay,
      liveState: {
        sessions: { "runtime-1": completedLiveSession },
        lastSequence: 1,
      },
      runtimeSessionAliases: { "codex:session-1": "runtime-1" },
      pendingNewChats: {},
      t,
    });

    expect(lanes).toHaveLength(1);
    expect(lanes[0].status).toBe("history");
  });

  it("connects a replay lane whose session id is already the runtime id", () => {
    const replay: ThreadReplayInfo = {
      threadId: "thread-1",
      kind: "teamwork",
      sessions: [
        replaySession("codex", "runtime-1", null, [
          source({ kind: "plan_task", planRoundId: "round-1", planTaskId: "task-1" }),
        ]),
      ],
    };

    const lanes = buildThreadSessionLanes({
      thread: thread("teamwork"),
      replay,
      liveState: {
        sessions: {
          "runtime-1": liveSession("codex", "runtime-1", "pending", false),
        },
        lastSequence: 1,
      },
      runtimeSessionAliases: {},
      pendingNewChats: {},
      t,
    });

    expect(lanes).toHaveLength(1);
    expect(lanes[0].sessioRuntimeSessionId).toBe("runtime-1");
    expect(lanes[0].liveSession?.sessioRuntimeSessionId).toBe("runtime-1");
  });

  it("adds live-only planner lanes from runtime metadata", () => {
    const lanes = buildThreadSessionLanes({
      thread: thread("teamwork"),
      replay: { threadId: "thread-1", kind: "teamwork", sessions: [] },
      liveState: {
        sessions: {
          "runtime-planner": liveSession("codex", "runtime-planner", "planner-session", false, {
            astraInternal: true,
            astraPurpose: "orchestration",
            astraRunId: "run-1",
            astraThreadId: "thread-1",
          }),
        },
        lastSequence: 1,
      },
      runtimeSessionAliases: {},
      pendingNewChats: {},
      t,
    });

    expect(lanes).toHaveLength(1);
    expect(lanes[0].sources[0].kind).toBe("astra_internal");
    expect(lanes[0].sources[0].astraRunId).toBe("run-1");
    expect(lanes[0].sessioRuntimeSessionId).toBe("runtime-planner");
  });

  it("keeps deterministic orchestrator pseudo sessions in lanes for orchestration summaries", () => {
    const replay: ThreadReplayInfo = {
      threadId: "thread-1",
      kind: "process",
      sessions: [
        replaySession("astra-pi", "deterministic-orchestrator-astra-run-1-0", null, [
          source({
            kind: "astra_internal",
            astraRunId: "run-1",
            role: "planner",
            label: "Astra planner: deterministic",
            createdAt: 20,
          }),
        ]),
        replaySession("astra-pi", "deterministic-orchestrator-astra-run-1-1", null, [
          source({
            kind: "astra_internal",
            astraRunId: "run-1",
            role: "planner",
            label: "Astra planner: deterministic",
            createdAt: 40,
          }),
        ]),
        replaySession("codex", "stage-session-1", session("codex", "stage-session-1"), [
          source({ kind: "stage", stageId: "stage-1", label: "Writing", createdAt: 30 }),
        ]),
      ],
    };

    const lanes = buildThreadSessionLanes({
      thread: thread("process"),
      replay,
      liveState: emptyLiveState(),
      runtimeSessionAliases: {},
      pendingNewChats: {},
      t,
    });

    expect(new Set(lanes.map((lane) => lane.sessionId))).toEqual(new Set([
      "deterministic-orchestrator-astra-run-1-0",
      "deterministic-orchestrator-astra-run-1-1",
      "stage-session-1",
    ]));
    const rows = buildThreadTimelineRows(
      lanes,
      [planRound({
        id: "round-1",
        astraRunId: "run-1",
        createdAt: 20,
        updatedAt: 30,
        tasks: [planTask({ id: "task-1", roundId: "round-1", threadStageId: "stage-1" })],
      })],
      [astraRun({
        runId: "run-1",
        plannerBackend: "deterministic",
        status: "completed",
        terminalReason: "done",
      })],
      "process",
    );

    expect(rows.map((row) => row.kind)).toEqual(["orchestration", "sessions", "orchestration"]);
    expect(rows[0].lanes).toHaveLength(0);
    expect(rows[1].lanes.map((lane) => lane.sessionId)).toEqual(["stage-session-1"]);
    expect(rows[2].run?.terminalReason).toBe("done");
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
  it("groups process sessions by stage and debate sessions by round lane", () => {
    const process = groupReplaySessionsByThreadKind({
      threadId: "thread-1",
      kind: "process",
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

    expect(process).toHaveLength(1);
    expect(process[0].sessions.map((item) => item.sessionId)).toEqual(["session-2", "session-1"]);
    expect(debate[0].key).toBe("debate:round-123456789:task-1");
    expect(debate[0].label).toContain("Pro");
  });

  it("filters deterministic orchestrator pseudo sessions from replay groups", () => {
    const groups = groupReplaySessionsByThreadKind({
      threadId: "thread-1",
      kind: "process",
      sessions: [
        replaySession("astra-pi", "deterministic-orchestrator-astra-run-1-0", null, [
          source({
            kind: "astra_internal",
            astraRunId: "run-1",
            role: "planner",
            label: "Astra planner: deterministic",
            createdAt: 20,
          }),
        ]),
        replaySession("codex", "stage-session-1", session("codex", "stage-session-1"), [
          source({ kind: "stage", stageId: "stage-1", label: "Writing", createdAt: 30 }),
        ]),
      ],
    }, t);

    expect(groups).toHaveLength(1);
    expect(groups[0].sessions.map((item) => item.sessionId)).toEqual(["stage-session-1"]);
  });
});

describe("buildThreadTimelineRows", () => {
  it("keeps orchestration before task lanes and hides missing internal planner placeholders", () => {
    const round = planRound({
      id: "round-1",
      astraRunId: "run-1",
      roundIndex: 0,
      createdAt: 100,
      updatedAt: 130,
      tasks: [planTask({ id: "task-1", roundId: "round-1", sortOrder: 0 })],
    });
    const replay: ThreadReplayInfo = {
      threadId: "thread-1",
      kind: "brainstorm",
      sessions: [
        replaySession("astra-pi", "brainstorm-backend-run-1-0", null, [
          source({
            kind: "astra_internal",
            astraRunId: "run-1",
            role: "planner",
            label: "Astra planner: brainstorm_backend",
            createdAt: 95,
          }),
        ]),
        replaySession("codex", "task-session", session("codex", "task-session"), [
          source({
            kind: "plan_task",
            planRoundId: "round-1",
            planTaskId: "task-1",
            astraRunId: "run-1",
            label: "Opinion",
            createdAt: 90,
          }),
        ]),
      ],
    };
    const lanes = buildThreadSessionLanes({
      thread: thread("brainstorm"),
      replay,
      liveState: emptyLiveState(),
      runtimeSessionAliases: {},
      pendingNewChats: {},
      t,
    });

    const rows = buildThreadTimelineRows(lanes, [round], [astraRun({ runId: "run-1" })], "brainstorm");

    expect(rows.map((row) => row.kind)).toEqual(["orchestration", "sessions"]);
    expect(rows[0].round?.id).toBe("round-1");
    expect(rows[0].lanes).toHaveLength(0);
    expect(rows[1].lanes.map((lane) => lane.sessionId)).toEqual(["task-session"]);
    expect(rows.flatMap((row) => row.lanes).some((lane) => lane.sessionId.startsWith("brainstorm-backend"))).toBe(false);
  });

  it("pairs debate task lanes two at a time inside a round", () => {
    const round = planRound({
      id: "round-1",
      astraRunId: "run-1",
      tasks: [
        planTask({ id: "task-1", roundId: "round-1", sortOrder: 0 }),
        planTask({ id: "task-2", roundId: "round-1", sortOrder: 1 }),
        planTask({ id: "task-3", roundId: "round-1", sortOrder: 2 }),
      ],
    });
    const replay: ThreadReplayInfo = {
      threadId: "thread-1",
      kind: "debate",
      sessions: [
        debateReplaySession("codex", "session-1", "task-1", 10),
        debateReplaySession("claude", "session-2", "task-2", 11),
        debateReplaySession("gemini", "session-3", "task-3", 12),
      ],
    };
    const lanes = buildThreadSessionLanes({
      thread: thread("debate"),
      replay,
      liveState: emptyLiveState(),
      runtimeSessionAliases: {},
      pendingNewChats: {},
      t,
    });

    const rows = buildThreadTimelineRows(lanes, [round], [astraRun({ runId: "run-1" })], "debate");
    const sessionRows = rows.filter((row) => row.kind === "sessions");

    expect(sessionRows).toHaveLength(2);
    expect(sessionRows[0].debatePair).toBe(true);
    expect(sessionRows[0].lanes.map((lane) => lane.sessionId)).toEqual(["session-1", "session-2"]);
    expect(sessionRows[1].lanes.map((lane) => lane.sessionId)).toEqual(["session-3"]);
  });

  it("orders standalone session rows from oldest to newest", () => {
    const replay: ThreadReplayInfo = {
      threadId: "thread-1",
      kind: "teamwork",
      sessions: [
        replaySession("codex", "session-new", sessionWithTimes("codex", "session-new", 10, 30), [
          source({ kind: "thread", createdAt: 10 }),
        ]),
        replaySession("claude", "session-old", sessionWithTimes("claude", "session-old", 5, 20), [
          source({ kind: "thread", createdAt: 5 }),
        ]),
      ],
    };
    const lanes = buildThreadSessionLanes({
      thread: thread("teamwork"),
      replay,
      liveState: emptyLiveState(),
      runtimeSessionAliases: {},
      pendingNewChats: {},
      t,
    });

    const rows = buildThreadTimelineRows(lanes, [], [], "teamwork");

    expect(rows.flatMap((row) => row.lanes.map((lane) => lane.sessionId))).toEqual([
      "session-old",
      "session-new",
    ]);
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

  it("prefers agent participant snapshot detail for agent-based task sources", () => {
    const item = source({
      kind: "plan_task",
      planRoundId: "round-1",
      planTaskId: "task-1",
      label: "Brainstorm opinion",
      agentSnapshotJson: JSON.stringify({
        agent: "codex",
        participant: {
          participantId: "participant-1",
          agent: "codex",
          model: "gpt-5.3-codex",
          effort: "high",
          permissionMode: "workspace-write",
        },
        agentInfo: { displayName: "Codex", model: "fallback-model" },
      }),
    });

    expect(replaySourceTitle(item)).toContain(
      "Participant snapshot: Codex / gpt-5.3-codex / high / workspace-write",
    );
  });
});

function thread(kind: ThreadInfo["kind"]): Pick<ThreadInfo, "id" | "kind"> {
  return { id: "thread-1", kind };
}

function planRound(patch: Partial<PlanRoundInfo>): PlanRoundInfo {
  return {
    id: "round-1",
    threadId: "thread-1",
    astraRunId: "run-1",
    roundIndex: 0,
    summary: "Plan this round",
    mode: "parallel",
    source: "astra",
    status: "planned",
    createdAt: 100,
    updatedAt: 100,
    tasks: [],
    ...patch,
  };
}

function planTask(patch: Partial<PlanTaskInfo>): PlanTaskInfo {
  return {
    id: "task-1",
    roundId: "round-1",
    threadStageId: null,
    assistantId: null,
    agentParticipantId: null,
    targetAgent: "codex",
    stageSnapshotJson: null,
    assistantSnapshotJson: null,
    agentSnapshotJson: "{}",
    title: "Task",
    prompt: "Do the task",
    expectedOutput: null,
    risk: "low",
    sortOrder: 0,
    status: "planned",
    resultSummary: null,
    error: null,
    startedAt: null,
    completedAt: null,
    createdAt: 100,
    updatedAt: 100,
    sessions: [],
    ...patch,
  };
}

function astraRun(patch: Partial<AstraHandle>): AstraHandle {
  return {
    runId: "run-1",
    threadId: "thread-1",
    projectId: "project-1",
    status: "running",
    mode: "automatic",
    plannerBackend: "brainstorm_backend",
    roundIndex: 0,
    roundLimit: 3,
    terminalReason: null,
    lastErrorCode: null,
    lastErrorMessage: null,
    internalPlannerSessionIds: [],
    runDiagnostics: [],
    error: null,
    createdAt: 80,
    updatedAt: 120,
    ...patch,
  };
}

function debateReplaySession(
  agent: Agent,
  sessionId: string,
  planTaskId: string,
  createdAt: number,
): ThreadReplaySessionInfo {
  return replaySession(agent, sessionId, session(agent, sessionId), [
    source({
      kind: "plan_task",
      planRoundId: "round-1",
      planTaskId,
      astraRunId: "run-1",
      label: `${agent} debate lane`,
      createdAt,
    }),
  ]);
}

function session(agent: Agent, id: string): SessionInfo {
  return sessionWithTimes(agent, id, 1, id.endsWith("2") ? 2 : 1);
}

function sessionWithTimes(agent: Agent, id: string, startedAt: number, updatedAt: number): SessionInfo {
  return {
    id,
    agent,
    forkedFromAgent: null,
    forkedFromId: null,
    projectPath: "/tmp/project",
    projectName: "Project",
    startedAt,
    updatedAt,
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
  metadata: Record<string, unknown> = {},
): LiveRuntimeState["sessions"][string] {
  return {
    sessioRuntimeSessionId,
    agent,
    agentRuntimeSessionId,
    transport: "acp",
    workspacePath: "/tmp/project",
    capabilities: runtimeCapabilities(),
    metadata,
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

function liveTurn(turnId: string, status: LiveTurn["status"] = "streaming"): LiveTurn {
  return {
    turnId,
    status,
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
