import { describe, expect, it } from "vitest";
import type { AgentRuntimeEvent, RuntimeCapabilitySet } from "../src/api";
import {
  applyRuntimeAction,
  emptyAcpSessionState,
  emptyLiveRuntimeState,
  normalizeRuntimeTurnSnapshot,
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
  }: {
    agentRuntimeSessionId?: string;
    turnId?: string;
    text?: string;
    ended?: boolean;
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
        status: "streaming",
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
});
