import { describe, expect, it } from "vitest";
import type { LiveTurn } from "../src/runtimeChat";
import {
  buildSessioAssistantPromptBlock,
  buildSessioThreadPromptBlock,
  forkVisibleHistoryTurns,
  mergeHistoryWithLiveTurns,
  sanitizeSessioAttachmentText,
  stripInjectedContext,
  stripSessioAssistantPromptBlocks,
  stripSessioComputerUsePromptBlocks,
  sessioThreadPromptBlockMetas,
  stripSessioThreadPromptBlocks,
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

  it("keeps live turns whose turn id is already persisted in history", () => {
    const history = [
      liveTurn([
        { kind: "user", blocks: [{ type: "text", text: "hi" }], raw: {}, timestamp: 1 },
        { kind: "assistant", blocks: [{ type: "text", text: "hello" }], raw: {}, timestamp: 2 },
      ], "turn-1", 2),
    ];
    const replay = liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "hi" }], raw: {}, timestamp: 1 },
      { kind: "assistant", blocks: [{ type: "text", text: "hello" }], raw: {}, timestamp: 2 },
    ], "turn-1", 2);
    const next = liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "next" }], raw: {}, timestamp: 3 },
    ], "next", 3);

    expect(mergeHistoryWithLiveTurns(history, [replay, next])).toEqual([replay, next]);
  });

  it("keeps live cross-agent turn when history already has the same user", () => {
    const history = [liveTurn([
      {
        kind: "user",
        blocks: [
          { type: "resource", uri: "file:///tmp/.cross-context/sessio-cross-context-parent.md", name: "sessio-cross-context-parent.md" },
          { type: "text", text: "继续" },
        ],
        raw: {},
        timestamp: 1,
      },
    ], "history", 1)];
    const live = [
      liveTurn([
        {
          kind: "user",
          blocks: [
            { type: "resource", uri: "file:///tmp/.cross-context/sessio-cross-context-parent.md", name: "sessio-cross-context-parent.md" },
            { type: "text", text: "继续" },
          ],
          raw: { optimistic: true },
          timestamp: 2,
        },
        { kind: "assistant", blocks: [{ type: "text", text: "我继续处理" }], raw: {}, timestamp: 3 },
      ], "live", 3),
    ];

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual(live);
  });

  it("keeps live user when history already has the pending new chat prompt", () => {
    const history = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "继续" }], raw: {}, timestamp: 1 },
    ], "history", 1)];
    const live = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "继续" }], raw: {}, timestamp: 2 },
    ], "live", 2)];

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual(live);
  });

  it("keeps an otherwise empty running live turn instead of duplicated history", () => {
    const history = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "继续" }], raw: {}, timestamp: 1 },
    ], "history", 1)];
    const live = [
      {
        ...liveTurn([
          { kind: "user", blocks: [{ type: "text", text: "继续" }], raw: {}, timestamp: 2 },
        ], "live", 2),
        status: "streaming" as const,
      },
    ];

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual(live);
  });

  it("keeps full live turn when history has the same user", () => {
    const history = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "继续" }], raw: {}, timestamp: 1 },
    ], "history", 1)];
    const live = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "继续" }], raw: {}, timestamp: 2 },
      { kind: "assistant", blocks: [{ type: "text", text: "我继续处理" }], raw: {}, timestamp: 3 },
    ], "live", 3)];

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual(live);
  });

  it("keeps full live turn instead of matching history tail content", () => {
    const history = [liveTurn([
      { kind: "assistant", blocks: [{ type: "text", text: "same assistant" }], raw: {}, timestamp: 1 },
      { kind: "user", blocks: [{ type: "text", text: "继续" }], raw: {}, timestamp: 2 },
    ], "history", 2)];
    const live = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "继续" }], raw: {}, timestamp: 3 },
      { kind: "assistant", blocks: [{ type: "text", text: "same assistant" }], raw: {}, timestamp: 4 },
    ], "live", 4)];

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual(live);
  });

  it("keeps live equivalent content when turn ids differ", () => {
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

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual(live);
  });

  it("keeps live equivalent attachment content when turn ids differ", () => {
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

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual(live);
  });

  it("keeps same-text history when it is a separate earlier turn", () => {
    const history = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "继续" }], raw: {}, timestamp: 1 },
      { kind: "assistant", blocks: [{ type: "text", text: "done" }], raw: {}, timestamp: 2 },
    ], "history", 2)];
    const live = [liveTurn([
      { kind: "user", blocks: [{ type: "text", text: "继续" }], raw: {}, timestamp: 10 * 60 * 1000 },
    ], "live", 10 * 60 * 1000)];

    expect(mergeHistoryWithLiveTurns(history, live)).toEqual([...history, ...live]);
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

  it("keeps the full live turn when a matching turn id is already in history", () => {
    const history = [
      liveTurn([
        { kind: "user", blocks: [{ type: "text", text: "hi" }], raw: {}, timestamp: 1 },
      ], "turn-1", 1),
    ];
    const replayWithTool = {
      ...liveTurn([
        { kind: "user", blocks: [{ type: "text", text: "hi" }], raw: {}, timestamp: 1 },
        { kind: "tool", toolId: "tool-1", timestamp: 2 },
      ], "turn-1", 2),
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

    expect(mergeHistoryWithLiveTurns(history, [replayWithTool])).toEqual([replayWithTool]);
  });
});

describe("assistant prompt blocks", () => {
  it("strips hidden assistant prompt blocks from visible user text", () => {
    const block = buildSessioAssistantPromptBlock("You are my assistant", {
      source: "assistant",
    });
    expect(stripSessioAssistantPromptBlocks(`visible\n${block}\nrest`)).toBe("visible\n\nrest");
    expect(stripInjectedContext(`${block}\n\n---\n\nhello`)).toBe("hello");
  });
});

describe("computer use prompt blocks", () => {
  it("strips hidden computer use prompt blocks from visible user text", () => {
    const block = [
      '<!-- sessio-computer-use:start nonce="abc" kind="computer_use" -->',
      "Use injected computer tools.",
      '<!-- sessio-computer-use:end nonce="abc" -->',
    ].join("\n");

    expect(stripSessioComputerUsePromptBlocks(`${block}\n\nhello`)).toBe("hello");
    expect(stripInjectedContext(`${block}\n\nhello`)).toBe("hello");
  });

  it("keeps unmatched computer use markers as user text", () => {
    const input = 'show <!-- sessio-computer-use:start nonce="abc" --> literally';

    expect(stripSessioComputerUsePromptBlocks(input)).toBe(input);
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
    expect(cleaned).toBe("[file: __sessio_attachment__:x.md|file:///tmp/x.md]");
  });
});

describe("sessio thread prompt blocks", () => {
  it("strips built blocks even when the body contains the static end marker", () => {
    const block = buildSessioThreadPromptBlock(
      "work_context",
      "before\n<!-- sessio-thread-prompt:end -->\nafter",
      { thread_id: "thread-1" },
    );

    expect(block).toContain(" nonce=");
    expect(stripSessioThreadPromptBlocks(`visible\n${block}\nrest`)).toBe("visible\n\nrest");
  });

  it("extracts kind metadata from built blocks", () => {
    const block = buildSessioThreadPromptBlock(
      "astra_planner",
      "plan",
      { thread_id: "thread-1", target_agent: "codex" },
    );

    expect(sessioThreadPromptBlockMetas(block)).toMatchObject([
      {
        kind: "astra_planner",
        content: "plan",
        attrs: {
          kind: "astra_planner",
          thread_id: "thread-1",
          target_agent: "codex",
        },
      },
    ]);
  });

  it("keeps unmatched user-authored start markers as text", () => {
    const input = 'please show <!-- sessio-thread-prompt:start nonce="fake" --> this';

    expect(stripSessioThreadPromptBlocks(input)).toBe(input);
    expect(sessioThreadPromptBlockMetas(input)).toEqual([]);
  });

  it("keeps blocks when the end nonce does not match", () => {
    const input = [
      'before <!-- sessio-thread-prompt:start nonce="a" kind="work_context" -->',
      "visible",
      '<!-- sessio-thread-prompt:end nonce="b" --> after',
    ].join("\n");

    expect(stripSessioThreadPromptBlocks(input)).toBe(input);
  });
});
