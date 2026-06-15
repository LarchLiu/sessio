import { describe, expect, it } from "vitest";
import type { AgentRuntimeEvent, RuntimeCapabilitySet, RuntimeTurnStatus } from "../src/api";
import {
  aggregateLiveSessionActivity,
  applyRuntimeAction,
  emptyAcpSessionState,
  emptyLiveRuntimeState,
  liveSessionActivity,
  normalizeRuntimeTurnSnapshot,
  type AcpPermissionRequest,
  type LiveRuntimeSession,
  type LiveRuntimeTurnSnapshotEvent,
} from "../src/runtimeChat";

const capabilities: RuntimeCapabilitySet = {
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
};

function session(
  sessioRuntimeSessionId: string,
  {
    agentRuntimeSessionId = `${sessioRuntimeSessionId}-agent`,
    turnId = `${sessioRuntimeSessionId}-turn`,
    text = "hello",
    ended = false,
    status = "streaming",
    permissions = [],
  }: {
    agentRuntimeSessionId?: string;
    turnId?: string;
    text?: string;
    ended?: boolean;
    status?: RuntimeTurnStatus;
    permissions?: AcpPermissionRequest[];
  } = {},
): LiveRuntimeSession {
  return {
    sessioRuntimeSessionId,
    agent: "codex",
    agentRuntimeSessionId,
    transport: "acp",
    workspacePath: "/workspace",
    capabilities,
    turns: [
      {
        turnId,
        status,
        blocks: [
          {
            kind: "assistant",
            blocks: [{ type: "text", text }],
            raw: {},
            timestamp: 1,
          },
        ],
        tools: [],
        permissions,
        protocolMessages: [],
        stopReason: null,
        error: null,
        startedAt: 1,
        updatedAt: 1,
      },
    ],
    sessionState: emptyAcpSessionState(),
    protocolMessages: [],
    ended,
  };
}

function snapshot(sequence: number, runtimeSessionId: string, text: string): LiveRuntimeTurnSnapshotEvent {
  return {
    sequence,
    timestamp: sequence,
    session: session(runtimeSessionId, { text }),
  };
}

describe("runtimeChat", () => {
  it("keeps throttled snapshots ordered per session instead of by global sequence", () => {
    const withSecondSession = applyRuntimeAction(emptyLiveRuntimeState, {
      type: "runtime-turn-snapshot",
      event: snapshot(20, "session-b", "newer session"),
    });

    const withInterleavedSession = applyRuntimeAction(withSecondSession, {
      type: "runtime-turn-snapshot",
      event: snapshot(10, "session-a", "older but different session"),
    });

    expect(withInterleavedSession.sessions["session-a"]?.turns[0]?.blocks[0]).toMatchObject({
      kind: "assistant",
      blocks: [{ type: "text", text: "older but different session" }],
    });
    expect(withInterleavedSession.lastSequence).toBe(20);
    expect(withInterleavedSession.sessionSequences).toEqual({
      "session-a": 10,
      "session-b": 20,
    });
  });

  it("drops stale snapshots only for the same session", () => {
    const current = applyRuntimeAction(emptyLiveRuntimeState, {
      type: "runtime-turn-snapshot",
      event: snapshot(20, "session-a", "latest"),
    });

    const stale = applyRuntimeAction(current, {
      type: "runtime-turn-snapshot",
      event: snapshot(19, "session-a", "stale"),
    });

    expect(stale.sessions["session-a"]?.turns[0]?.blocks[0]).toMatchObject({
      kind: "assistant",
      blocks: [{ type: "text", text: "latest" }],
    });
  });

  it("normalizes already-camel turn snapshots without cloning deep payloads", () => {
    const event = snapshot(1, "session-a", "camel");

    expect(normalizeRuntimeTurnSnapshot(event)).toBe(event);
  });

  it("keeps session start events from regressing an existing session sequence", () => {
    const current = applyRuntimeAction(emptyLiveRuntimeState, {
      type: "runtime-turn-snapshot",
      event: snapshot(20, "session-a", "latest"),
    });
    const event: AgentRuntimeEvent = {
      kind: "sessionStarted",
      sequence: 19,
      timestamp: 19,
      agent: "codex",
      sessioRuntimeSessionId: "session-a",
      agentRuntimeSessionId: "agent-a",
      transport: "acp",
      workspacePath: "/workspace",
      capabilities,
      metadata: {},
    };

    expect(applyRuntimeAction(current, { type: "runtime-event", event })).toBe(current);
  });

  it("does not report a running activity for an ended session whose last turn never finalized", () => {
    expect(liveSessionActivity(session("session-a", { ended: false }))).toBe("running");
    expect(liveSessionActivity(session("session-a", { ended: true }))).toBe("updated");
  });

  it("aggregates the most attention-worthy activity across a thread's sessions", () => {
    const pendingPermission: AcpPermissionRequest = {
      requestId: "perm-1",
      toolCall: null,
      toolName: "bash",
      input: null,
      options: [],
      selectedOptionId: null,
      cancelled: false,
      raw: {},
    };

    expect(aggregateLiveSessionActivity([])).toBe("idle");
    expect(aggregateLiveSessionActivity([null, undefined])).toBe("idle");

    // A finished lane next to a running one still reads as running.
    expect(
      aggregateLiveSessionActivity([
        session("done", { ended: true }),
        session("busy"),
      ]),
    ).toBe("running");

    // A running lane outranks a failed one.
    expect(
      aggregateLiveSessionActivity([
        session("broken", { status: "failed" }),
        session("busy"),
      ]),
    ).toBe("running");

    // A pending permission on any lane wins over everything else.
    expect(
      aggregateLiveSessionActivity([
        session("busy"),
        session("broken", { status: "failed" }),
        session("waiting", { permissions: [pendingPermission] }),
      ]),
    ).toBe("permission");
  });
});
