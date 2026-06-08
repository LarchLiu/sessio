import { describe, expect, it } from "vitest";
import type { Agent } from "../src/api";
import type { PendingNewChatSession } from "../src/navigation";
import type { LiveRuntimeSession, LiveTurn } from "../src/runtimeChat";
import {
  planTaskRuntimeCompletionForPending,
  terminalPatchForLiveSession,
} from "../src/hooks/usePlanTaskRuntimeCompletion";

describe("planTaskRuntimeCompletionForPending", () => {
  it("waits until the plan task runtime session has been linked and ended", () => {
    const pending = pendingTask({ runtimeStarted: false });
    const liveSessions = { "runtime-1": liveSession(false) };

    expect(planTaskRuntimeCompletionForPending(pending, liveSessions)).toBeNull();
    expect(planTaskRuntimeCompletionForPending(
      { ...pending, planTaskLink: { taskId: "task-1", role: "runtime", runtimeStarted: true } },
      liveSessions,
    )).toBeNull();
  });

  it("returns the task status patch once the linked runtime session ends", () => {
    const completion = planTaskRuntimeCompletionForPending(
      pendingTask({ runtimeStarted: true }),
      { "runtime-1": liveSession(true, [assistantTurn("done")]) },
    );

    expect(completion).toEqual({
      taskId: "task-1",
      patch: {
        status: "completed",
        resultSummary: "done",
      },
    });
  });
});

describe("terminalPatchForLiveSession", () => {
  it("marks failed sessions as failed with the turn error", () => {
    const patch = terminalPatchForLiveSession(liveSession(true, [{
      ...assistantTurn("ignored"),
      error: { message: "boom", code: "runtime_error", data: null },
    }]));

    expect(patch).toEqual({ status: "failed", error: "boom" });
  });

  it("marks cancelled sessions as cancelled", () => {
    const patch = terminalPatchForLiveSession(liveSession(true, [{
      ...assistantTurn("ignored"),
      status: "cancelled",
    }]));

    expect(patch).toEqual({ status: "cancelled" });
  });

  it("uses the latest assistant text as the result summary", () => {
    const patch = terminalPatchForLiveSession(liveSession(true, [
      assistantTurn("first"),
      assistantTurn("final"),
    ]));

    expect(patch).toEqual({ status: "completed", resultSummary: "final" });
  });
});

function pendingTask({ runtimeStarted }: { runtimeStarted: boolean }): PendingNewChatSession {
  return {
    sessioRuntimeSessionId: "runtime-1",
    agent: "codex",
    projectPath: "/tmp/project",
    projectName: "Project",
    prompt: "run task",
    timestamp: 1,
    planTaskLink: {
      taskId: "task-1",
      role: "runtime",
      runtimeStarted,
    },
  };
}

function liveSession(
  ended: boolean,
  turns: LiveTurn[] = [assistantTurn("ok")],
  agent: Agent = "codex",
): LiveRuntimeSession {
  return {
    sessioRuntimeSessionId: "runtime-1",
    agent,
    agentRuntimeSessionId: "session-1",
    transport: "acp",
    workspacePath: "/tmp/project",
    capabilities: runtimeCapabilities(),
    turns,
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

function runtimeCapabilities(): LiveRuntimeSession["capabilities"] {
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

function assistantTurn(text: string): LiveTurn {
  return {
    turnId: `turn-${text}`,
    status: "completed",
    blocks: [
      {
        kind: "assistant",
        blocks: [{ type: "text", text }],
        raw: {},
        timestamp: 1,
      },
    ],
    tools: [],
    permissions: [],
    protocolMessages: [],
    stopReason: null,
    error: null,
    startedAt: 1,
    updatedAt: 1,
  };
}
