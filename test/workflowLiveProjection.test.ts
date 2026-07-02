import { describe, expect, it } from "vitest";
import type { ThreadWorkSnapshot } from "../src/api";
import type { LiveRuntimeSession, LiveRuntimeState } from "../src/runtimeChat";
import {
  buildSessionThreadStageMap,
  createWorkflowOverlayCardContext,
  projectWorkflowLiveOverlays,
} from "../src/lib/blocksuite/workflowLiveProjection";

function snapshot(): ThreadWorkSnapshot {
  return {
    threadId: "thread-1",
    projectId: "project-1",
    goal: "Live workflow",
    description: null,
    focusedStageId: "stage-build",
    activeStageId: "stage-build",
    stages: [
      {
        threadStageId: "stage-build",
        projectStageId: "project-stage-build",
        name: "Build",
        kind: "build",
        icon: null,
        status: "not_started",
        summary: null,
        outcome: null,
        assistants: [
          {
            assistantId: "assistant-codex",
            name: "Builder",
            color: null,
            agent: {
              id: "codex",
              name: "Codex",
              model: "gpt-5.3-codex",
              mode: "read-write",
              effort: "medium",
            },
            systemPrompt: null,
            order: 0,
          },
        ],
        issues: [],
        sessionRefs: [{
          agent: "codex",
          sessionId: "child-build",
          title: "Build session",
          sourceKind: "stage",
        }],
      },
      {
        threadStageId: "stage-review",
        projectStageId: "project-stage-review",
        name: "Review",
        kind: "review",
        icon: null,
        status: "not_started",
        summary: null,
        outcome: null,
        assistants: [],
        issues: [],
        sessionRefs: [],
      },
    ],
    threadSessionRefs: [{
      agent: "codex",
      sessionId: "child-thread",
      title: "Thread session",
      sourceKind: "thread",
    }],
    detailRefs: {
      threadId: "thread-1",
      focusedStageId: "stage-build",
      stageIds: ["stage-build", "stage-review"],
      issueIds: [],
      sessionRefs: [],
    },
    rollup: {
      completed: 0,
      incomplete: 2,
      blocked: 0,
      openIssues: 0,
      currentStage: "Build",
      total: 2,
    },
    capturedAt: 1,
  };
}

function card(
  blockId: string,
  threadStageId: string | null,
) {
  const context = createWorkflowOverlayCardContext({
    blockId,
    threadId: "thread-1",
    threadStageId,
    workflowSnapshotJson: JSON.stringify(snapshot()),
  });
  if (!context) throw new Error("expected workflow card context");
  return context;
}

function liveSession(
  sessioRuntimeSessionId: string,
  agentRuntimeSessionId: string,
  updatedAt: number,
  action = "Running tests",
  metadata: Record<string, unknown> = {},
): LiveRuntimeSession {
  return {
    sessioRuntimeSessionId,
    agent: "codex",
    agentRuntimeSessionId,
    transport: "fake",
    workspacePath: "/tmp/project",
    capabilities: {
      supportsCancel: true,
      supportsPermissions: true,
      supportsToolDeltas: true,
      supportsLoadSession: true,
      supportsModes: true,
      supportsResume: true,
      supportsFork: true,
      supportsImageAttachments: true,
      supportsAudioAttachments: false,
      supportsEmbeddedContext: true,
      supportsAttachments: true,
    },
    metadata,
    turns: [{
      turnId: `turn-${sessioRuntimeSessionId}`,
      status: "streaming",
      blocks: [],
      tools: [{
        toolId: "tool-1",
        title: action,
        kind: "shell",
        status: "running",
        content: [],
        locations: [],
        rawInput: null,
        rawOutput: null,
        meta: null,
        raw: null,
        updatedAt,
      }],
      permissions: [],
      protocolMessages: [],
      stopReason: null,
      error: null,
      startedAt: updatedAt - 1,
      updatedAt,
    }],
    sessionState: {
      plan: null,
      availableCommands: [],
      currentModeId: null,
      configOptions: [],
      sessionInfo: null,
    },
    protocolMessages: [],
    ended: false,
  };
}

describe("workflow live projection", () => {
  it("routes live stage sessions to thread and matching stage cards", () => {
    const cards = [
      card("thread-card", null),
      card("build-card", "stage-build"),
      card("review-card", "stage-review"),
    ];
    const liveState: LiveRuntimeState = {
      sessions: {
        "runtime-build": liveSession("runtime-build", "child-build", 10),
      },
      lastSequence: 1,
    };

    const overlays = projectWorkflowLiveOverlays({
      cards,
      runtimeSessionAliases: { "codex:child-build": "runtime-build" },
      liveState,
    });

    expect(overlays.get("thread-card")?.stages["stage-build"]).toMatchObject({
      active: true,
      status: "in_progress",
      activeAssistantIds: ["assistant-codex"],
      currentAction: "Running tests",
    });
    expect(overlays.get("build-card")?.stages["stage-build"]).toMatchObject({
      active: true,
      status: "in_progress",
    });
    expect(overlays.has("review-card")).toBe(false);
  });

  it("keeps thread-level sessions on thread cards only", () => {
    const cards = [
      card("thread-card", null),
      card("build-card", "stage-build"),
    ];
    const liveState: LiveRuntimeState = {
      sessions: {
        "runtime-thread": liveSession("runtime-thread", "child-thread", 12, "Planning next stage"),
      },
      lastSequence: 1,
    };

    const overlays = projectWorkflowLiveOverlays({
      cards,
      runtimeSessionAliases: { "codex:child-thread": "runtime-thread" },
      liveState,
    });

    expect(overlays.get("thread-card")).toMatchObject({
      activeCount: 1,
      currentAction: "Planning next stage",
      stages: {},
    });
    expect(overlays.has("build-card")).toBe(false);
  });

  it("routes Astra planner live sessions by thread metadata to thread cards", () => {
    const cards = [
      card("thread-card", null),
      card("build-card", "stage-build"),
    ];
    const liveState: LiveRuntimeState = {
      sessions: {
        "runtime-astra": liveSession("runtime-astra", "astra-session", 14, "Planning delegated work", {
          astraThreadId: "thread-1",
          astraPurpose: "orchestration",
          astraRunId: "run-1",
        }),
      },
      lastSequence: 1,
    };

    const overlays = projectWorkflowLiveOverlays({
      cards,
      runtimeSessionAliases: {},
      liveState,
    });

    expect(overlays.get("thread-card")).toMatchObject({
      activeCount: 1,
      currentAction: "Planning delegated work",
      stages: {},
    });
    expect(overlays.has("build-card")).toBe(false);
  });

  it("builds reverse runtime and thread fan-out indexes", () => {
    const sessionMap = buildSessionThreadStageMap(
      [card("thread-card", null), card("build-card", "stage-build")],
      { "codex:child-build": "runtime-build" },
    );

    expect(sessionMap.bySessioRuntimeId.get("runtime-build")).toMatchObject({
      childSessionId: "child-build",
      threadId: "thread-1",
      stageId: "stage-build",
    });
    expect([...sessionMap.blockIdsByThread.get("thread-1") ?? []]).toEqual([
      "thread-card",
      "build-card",
    ]);
  });
});
