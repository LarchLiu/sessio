import { describe, expect, it } from "vitest";
import type { LiveTurn } from "../src/runtimeChat";
import {
  forkVisibleHistoryTurns,
  mergeHistoryWithLiveTurns,
  sanitizeSessioAttachmentText,
} from "../src/historyMerge";

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

describe("forkVisibleHistoryTurns", () => {
  it("returns current turns when there are no ancestors", () => {
    const current = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "hi" }], raw: {}, timestamp: 1 },
    ], "current", 1)];

    expect(forkVisibleHistoryTurns([], current)).toEqual(current);
  });

  it("returns ancestors when current is empty", () => {
    const ancestors = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "root" }], raw: {}, timestamp: 1 },
    ], "ancestor", 1)];

    expect(forkVisibleHistoryTurns(ancestors, [])).toEqual(ancestors);
  });

  it("drops the duplicated ancestor fork user turn", () => {
    const root = liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "root" }], raw: {}, timestamp: 1 },
      { kind: "assistant", blocks: [{ type: "text", text: "A" }], raw: {}, timestamp: 2 },
    ], "root", 2);
    const forkTail = liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "forked" }], raw: {}, timestamp: 3 },
    ], "fork-tail", 3);
    const current = [
      liveTurn([
        { kind: "user", blocks: [{ type: "text", text: "forked" }], raw: {}, timestamp: 4 },
        { kind: "assistant", blocks: [{ type: "text", text: "B-new" }], raw: {}, timestamp: 5 },
      ], "current", 5),
    ];

    expect(forkVisibleHistoryTurns([root, forkTail], current)).toEqual([root, ...current]);
  });

  it("matches duplicated fork users with equivalent structured attachments", () => {
    const ancestor = liveTurn([
      {
        kind: "user",
        blocks: [
          { type: "text", text: "forked" },
          { type: "resource", uri: "file:///tmp/notes.md", name: "notes.md" },
        ],
        raw: {},
        timestamp: 1,
      },
    ], "ancestor", 1);
    const current = [
      liveTurn([
        {
          kind: "user",
          blocks: [
            { type: "text", text: "forked" },
            { type: "resource_link", uri: "file:///tmp/notes.md" },
          ],
          raw: {},
          timestamp: 2,
        },
      ], "current", 2),
    ];

    expect(forkVisibleHistoryTurns([ancestor], current)).toEqual(current);
  });

  it("keeps ancestor fork user when same text points at a different attachment", () => {
    const ancestor = liveTurn([
      {
        kind: "user",
        blocks: [
          { type: "text", text: "forked" },
          { type: "resource", uri: "file:///tmp/notes.md", name: "notes.md" },
        ],
        raw: {},
        timestamp: 1,
      },
    ], "ancestor", 1);
    const current = [
      liveTurn([
        {
          kind: "user",
          blocks: [
            { type: "text", text: "forked" },
            { type: "resource_link", uri: "file:///tmp/other.md" },
          ],
          raw: {},
          timestamp: 2,
        },
      ], "current", 2),
    ];

    expect(forkVisibleHistoryTurns([ancestor], current)).toEqual([ancestor, ...current]);
  });
});

describe("mergeHistoryWithLiveTurns", () => {
  it("returns live when history is empty", () => {
    const live = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "hi" }], raw: {}, timestamp: 1 },
    ], "live", 1)];

    expect(mergeHistoryWithLiveTurns([], live)).toEqual(live);
  });

  it("returns history when live is empty", () => {
    const history = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "hi" }], raw: {}, timestamp: 1 },
    ], "history", 1)];

    expect(mergeHistoryWithLiveTurns(history, [])).toEqual(history);
  });

  it("appends live when there is no overlap", () => {
    const history = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "a" }], raw: {}, timestamp: 1 },
      { kind: "assistant", blocks: [{ type: "text", text: "A" }], raw: {}, timestamp: 2 },
    ], "history", 2)];
    const live = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "b" }], raw: {}, timestamp: 3 },
    ], "live", 3)];

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual([...history, ...live]);
  });

  it("drops completed live replay turns already persisted in history", () => {
    const history = [
      liveTurn([
        { kind: "user", blocks: [{ type: "text", text: "hi" }], raw: {}, timestamp: 1 },
        { kind: "assistant", blocks: [{ type: "text", text: "hello" }], raw: {}, timestamp: 2 },
      ], "history", 2),
    ];
    const replay = liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "hi" }], raw: {}, timestamp: 1 },
      { kind: "assistant", blocks: [{ type: "text", text: "hello" }], raw: {}, timestamp: 2 },
    ], "replay", 2);
    const next = liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "next" }], raw: {}, timestamp: 3 },
    ], "next", 3);

    expect(mergeHistoryWithLiveTurns(history, [replay, next])).toEqual([...history, next]);
  });

  it("ignores cosmetic whitespace and IDE-injected blocks when matching overlap", () => {
    const history = [liveTurn([
      {
        kind: "user",
        blocks: [{ type: "text", text: "<ide_opened_file>foo</ide_opened_file>hi  there" }],
        raw: {},
        timestamp: 1,
      },
    ], "history", 1)];
    const live = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "hi there" }], raw: {}, timestamp: 2 },
    ], "live", 2)];

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual(history);
  });

  it("matches equivalent attachment references across persisted and live block shapes", () => {
    const history = [liveTurn([
      {
        kind: "user",
        blocks: [
          { type: "text", text: "inspect this" },
          { type: "resource", uri: "file:///tmp/sketch.png", name: "sketch.png" },
        ],
        raw: {},
        timestamp: 1,
      },
    ], "history", 1)];
    const live = [liveTurn([
      {
        kind: "user",
        blocks: [
          { type: "text", text: "inspect this" },
          { type: "image", uri: "file:///tmp/sketch.png", mimeType: "image/png" },
        ],
        raw: {},
        timestamp: 2,
      },
    ], "live", 2)];

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual(history);
  });

  it("keeps same-text live replay when attachment references differ", () => {
    const history = [liveTurn([
      {
        kind: "user",
        blocks: [
          { type: "text", text: "inspect this" },
          { type: "resource", uri: "file:///tmp/sketch.png", name: "sketch.png" },
        ],
        raw: {},
        timestamp: 1,
      },
    ], "history", 1)];
    const live = [liveTurn([
      {
        kind: "user",
        blocks: [
          { type: "text", text: "inspect this" },
          { type: "image", uri: "file:///tmp/other.png", mimeType: "image/png" },
        ],
        raw: {},
        timestamp: 2,
      },
    ], "live", 2)];

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual([...history, ...live]);
  });

  it("keeps non-message live tool data when dropping replayed message blocks", () => {
    const history = [
      liveTurn([
        { kind: "user", blocks: [{ type: "text", text: "hi" }], raw: {}, timestamp: 1 },
      ], "history", 1),
    ];
    const replayWithTool = {
      ...liveTurn([
        { kind: "user", blocks: [{ type: "text", text: "hi" }], raw: {}, timestamp: 1 },
        { kind: "tool", toolId: "tool-1", timestamp: 2 },
      ], "replay", 2),
      status: "streaming" as const,
      tools: [
        {
          toolId: "tool-1",
          title: "Bash",
          kind: "tool",
          status: "running",
          content: [],
          locations: [],
          rawInput: null,
          rawOutput: null,
          meta: null,
          raw: {},
          updatedAt: 2,
        },
      ],
    };

    expect(mergeHistoryWithLiveTurns(history, [replayWithTool])).toEqual([
      ...history,
      {
        ...replayWithTool,
        blocks: [{ kind: "tool", toolId: "tool-1", timestamp: 2 }],
      },
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
