import { describe, expect, it } from "vitest";
import type { SessionMessage } from "../src/api";
import type { LiveRuntimeSession, LiveTurn } from "../src/runtimeChat";
import {
  crossContextMessages,
  forkVisibleHistoryMessages,
  liveSessionMessages,
  mergeHistoryWithLiveMessages,
  sanitizeSessioAttachmentText,
} from "../src/historyMerge";

function userMessage(text: string, timestamp = 1): SessionMessage {
  return { role: "user", text, timestamp };
}

function assistantMessage(text: string, timestamp = 2): SessionMessage {
  return { role: "assistant", text, timestamp };
}

function liveSession(turns: LiveTurn[]): LiveRuntimeSession {
  return {
    sessioRuntimeSessionId: "session",
    agent: "claude",
    agentRuntimeSessionId: "session",
    workspacePath: "/tmp",
    capabilities: { promptCapabilities: { audio: false, image: false, embeddedContext: false } },
    turns,
    pendingPermissions: [],
    notifications: [],
    sessionConfig: [],
    availableCommands: [],
    state: "idle",
    startedAt: 1,
    updatedAt: 1,
  } as unknown as LiveRuntimeSession;
}

function liveTurn(
  blocks: LiveTurn["blocks"],
  turnId = "t1",
  updatedAt = 100,
): LiveTurn {
  return {
    turnId,
    status: "completed",
    blocks,
    tools: [],
    permissions: [],
    protocolMessages: [],
    stopReason: null,
    error: null,
    startedAt: updatedAt,
    updatedAt,
  };
}

describe("mergeHistoryWithLiveMessages", () => {
  it("returns live when history is empty", () => {
    const live = [userMessage("hi")];
    expect(mergeHistoryWithLiveMessages([], live)).toEqual(live);
  });

  it("returns history when live is empty", () => {
    const history = [userMessage("hi")];
    expect(mergeHistoryWithLiveMessages(history, [])).toEqual(history);
  });

  it("appends live when there is no overlap", () => {
    const history = [userMessage("a"), assistantMessage("A")];
    const live = [userMessage("b")];
    expect(mergeHistoryWithLiveMessages(history, live)).toEqual([...history, ...live]);
  });

  it("drops the duplicated overlap between history tail and live head", () => {
    const history = [userMessage("a"), assistantMessage("A")];
    const live = [userMessage("a"), assistantMessage("A"), userMessage("b")];
    expect(mergeHistoryWithLiveMessages(history, live)).toEqual([
      ...history,
      userMessage("b"),
    ]);
  });

  it("ignores cosmetic whitespace and IDE-injected blocks when matching overlap", () => {
    const history = [userMessage("<ide_opened_file>foo</ide_opened_file>hi  there")];
    const live = [userMessage("hi there")];
    expect(mergeHistoryWithLiveMessages(history, live)).toEqual(history);
  });
});

describe("forkVisibleHistoryMessages", () => {
  it("returns currentMessages when no ancestors", () => {
    const current = [userMessage("hi")];
    expect(forkVisibleHistoryMessages([], current)).toEqual(current);
  });

  it("returns ancestors when current is empty", () => {
    const ancestors = [userMessage("a")];
    expect(forkVisibleHistoryMessages(ancestors, [])).toEqual(ancestors);
  });

  it("drops the duplicated last-ancestor-user when current restarts from it", () => {
    const ancestors = [
      userMessage("a"),
      assistantMessage("A"),
      userMessage("b"),
    ];
    const current = [userMessage("b"), assistantMessage("B-new")];
    expect(forkVisibleHistoryMessages(ancestors, current)).toEqual([
      userMessage("a"),
      assistantMessage("A"),
      ...current,
    ]);
  });

  it("keeps ancestors when assistant follows the last ancestor user (no fork point)", () => {
    const ancestors = [
      userMessage("a"),
      assistantMessage("A"),
    ];
    const current = [userMessage("b")];
    expect(forkVisibleHistoryMessages(ancestors, current)).toEqual([...ancestors, ...current]);
  });

  it("keeps ancestors when first current user differs from last ancestor user", () => {
    const ancestors = [
      userMessage("a"),
      userMessage("b"),
    ];
    const current = [userMessage("c")];
    expect(forkVisibleHistoryMessages(ancestors, current)).toEqual([...ancestors, ...current]);
  });
});

describe("liveSessionMessages", () => {
  it("converts user/assistant/thought blocks into SessionMessages, skipping empty text", () => {
    const session = liveSession([
      liveTurn([
        {
          kind: "user",
          blocks: [{ type: "text", text: "hi" }],
          raw: { source: "test" },
          timestamp: 10,
        },
        {
          kind: "assistant",
          blocks: [{ type: "text", text: "hello" }],
          raw: { source: "test" },
          timestamp: 20,
        },
        {
          kind: "thought",
          blocks: [{ type: "text", text: "" }],
          raw: { source: "test" },
          timestamp: 30,
        },
      ]),
    ]);
    expect(liveSessionMessages(session)).toEqual([
      { role: "user", text: "hi", timestamp: 10 },
      { role: "assistant", text: "hello", timestamp: 20 },
    ]);
  });
});

describe("crossContextMessages", () => {
  it("appends live turns to forked history", () => {
    const ancestors = [
      userMessage("root"),
      assistantMessage("A"),
      userMessage("forked"),
    ];
    const current = [userMessage("forked"), assistantMessage("B")];
    const session = liveSession([
      liveTurn([
        {
          kind: "user",
          blocks: [{ type: "text", text: "next" }],
          raw: { source: "test" },
          timestamp: 50,
        },
      ]),
    ]);
    expect(crossContextMessages(ancestors, current, session)).toEqual([
      userMessage("root"),
      assistantMessage("A"),
      ...current,
      { role: "user", text: "next", timestamp: 50 },
    ]);
  });

  it("returns plain forked history when there is no live session", () => {
    const ancestors = [userMessage("a"), assistantMessage("A"), userMessage("b")];
    const current = [userMessage("b"), assistantMessage("B")];
    expect(crossContextMessages(ancestors, current, null)).toEqual([
      userMessage("a"),
      assistantMessage("A"),
      ...current,
    ]);
  });
});

describe("sanitizeSessioAttachmentText", () => {
  it("keeps user-authored file links without @ prefix", () => {
    const input = "see [design doc](file:///Users/alex/Documents/design.md) for details";
    expect(sanitizeSessioAttachmentText(input)).toBe(input);
  });

  it("keeps non-file links", () => {
    const input = "open [docs](https://example.com/file.md) please";
    expect(sanitizeSessioAttachmentText(input)).toBe(input);
  });

  it("drops @-prefixed attachment links and leading bang", () => {
    const cleaned = sanitizeSessioAttachmentText("preview ![@photo.png](file:///tmp/photo.png) end");
    expect(cleaned).not.toContain("photo.png");
    expect(cleaned).not.toContain("!");
    expect(cleaned).toContain("preview");
    expect(cleaned).toContain("end");
  });

  it("drops cross-context links even without @ prefix", () => {
    const cleaned = sanitizeSessioAttachmentText(
      "carry [doc](file:///tmp/.cross-context/sessio-cross-context-abc.md) over",
    );
    expect(cleaned).not.toContain("sessio-cross-context");
    expect(cleaned).toContain("carry");
    expect(cleaned).toContain("over");
  });

  it("replaces sessio-upload-file blocks with file marker", () => {
    const cleaned = sanitizeSessioAttachmentText(
      '<sessio-upload-file uri="file:///tmp/x.md" name="x.md">body</sessio-upload-file>',
    );
    expect(cleaned).toBe("[file: x.md|file:///tmp/x.md]");
  });
});
