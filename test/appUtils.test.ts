import { describe, expect, it } from "vitest";
import { betterSessionCandidate, mergeRuntimeSessionAliases, sessionDisplayTitle } from "../src/appUtils";
import type { SessionInfo } from "../src/api";

describe("mergeRuntimeSessionAliases", () => {
  it("maps a real agent session id back to its live runtime session", () => {
    const aliases = mergeRuntimeSessionAliases({}, {
      "runtime-1": {
        agent: "codex",
        agentRuntimeSessionId: "agent-session-1",
        sessioRuntimeSessionId: "runtime-1",
      },
    });

    expect(aliases).toEqual({ "codex:agent-session-1": "runtime-1" });
  });

  it("ignores startup placeholders that are not selectable local sessions", () => {
    const aliases = mergeRuntimeSessionAliases({}, {
      "runtime-1": {
        agent: "codex",
        agentRuntimeSessionId: "fake-agent-session-1",
        sessioRuntimeSessionId: "runtime-1",
      },
      "runtime-2": {
        agent: "claude",
        agentRuntimeSessionId: "pending",
        sessioRuntimeSessionId: "runtime-2",
      },
    });

    expect(aliases).toEqual({});
  });

  it("returns the same alias object when nothing changes", () => {
    const existing = { "codex:agent-session-1": "runtime-1" };
    const aliases = mergeRuntimeSessionAliases(existing, {
      "runtime-1": {
        agent: "codex",
        agentRuntimeSessionId: "agent-session-1",
        sessioRuntimeSessionId: "runtime-1",
      },
    });

    expect(aliases).toBe(existing);
  });
});

describe("betterSessionCandidate", () => {
  const base: SessionInfo = {
    id: "session-1",
    agent: "codex",
    forkedFromAgent: null,
    forkedFromId: null,
    projectPath: "/tmp/project",
    projectName: "project",
    startedAt: 1,
    updatedAt: 1,
    messageCount: 0,
    renameTitle: null,
    title: "pending",
    firstUserMessage: "pending",
    filePath: "",
    fileSize: 0,
    partial: true,
    available: true,
    archived: false,
    subagents: [],
  };

  it("prefers the real file row over a placeholder row for the same identity", () => {
    expect(betterSessionCandidate({
      ...base,
      title: "# Sessio stage task",
      firstUserMessage: "# Sessio stage task",
      filePath: "/tmp/project/session.jsonl",
      fileSize: 128,
      partial: false,
    }, base)).toBe(true);
  });

  it("prefers a real file row over an astra virtual placeholder path", () => {
    const virtual = {
      ...base,
      filePath: "astra://run/stage/session",
      partial: true,
      updatedAt: 100,
    };

    expect(betterSessionCandidate({
      ...base,
      filePath: "/tmp/project/session.jsonl",
      fileSize: 128,
      partial: false,
      updatedAt: 10,
    }, virtual)).toBe(true);
  });
});

describe("sessionDisplayTitle", () => {
  it("prefers rename title over indexed title", () => {
    const session: SessionInfo = {
      id: "session-1",
      agent: "codex",
      forkedFromAgent: null,
      forkedFromId: null,
      projectPath: "/tmp/project",
      projectName: "project",
      startedAt: 1,
      updatedAt: 1,
      messageCount: 0,
      renameTitle: "Renamed",
      title: "Indexed",
      firstUserMessage: "Prompt",
      filePath: "/tmp/project/session.jsonl",
      fileSize: 128,
      partial: false,
      available: true,
      archived: false,
      subagents: [],
    };

    expect(sessionDisplayTitle(session)).toBe("Renamed");
  });
});
