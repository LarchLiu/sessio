import { describe, expect, it } from "vitest";
import { buildCrossPromptFromTurns, CROSS_PROMPT_MAX } from "../src/cross";

function messageTurn(role: "user" | "assistant" | "thought", text: string) {
  return {
    blocks: [
      {
        kind: role,
        blocks: [{ type: "text", text }],
      },
    ],
  };
}

function userTurn(text: string) {
  return messageTurn("user", text);
}

function assistantTurn(text: string) {
  return messageTurn("assistant", text);
}

describe("buildCrossPromptFromTurns", () => {
  it("keeps the latest oversized user turn instead of returning empty", () => {
    const prompt = buildCrossPromptFromTurns([userTurn("x".repeat(CROSS_PROMPT_MAX + 100))]);
    expect(prompt).toContain("[user]");
    expect(prompt).toContain("[...truncated...]");
    expect(prompt).toContain("x".repeat(32));
    expect(prompt).toContain("<!-- sessio-cross:end -->");
  });

  it("falls back to the latest user when only assistant tail fits", () => {
    const prompt = buildCrossPromptFromTurns([
      userTurn("important question"),
      assistantTurn("y".repeat(CROSS_PROMPT_MAX + 100)),
    ]);
    expect(prompt).toContain("[user]\nimportant question");
    expect(prompt).toContain("[assistant]");
    expect(prompt).toContain("[...truncated...]");
    expect(prompt.length).toBeLessThan(CROSS_PROMPT_MAX + 512);
  });

  it("keeps latest user and truncates oversized assistant when both cannot fully fit", () => {
    const prompt = buildCrossPromptFromTurns([
      userTurn("important question"),
      assistantTurn("y".repeat(CROSS_PROMPT_MAX * 2)),
    ]);
    expect(prompt).toContain("[user]\nimportant question");
    expect(prompt).toContain("[assistant]");
    expect(prompt).toContain("[...truncated...]");
    expect(prompt.length).toBeLessThan(CROSS_PROMPT_MAX + 512);
  });

  it("preserves earlier turns when a mid-conversation assistant is too long to fit fully", () => {
    // Earlier assistant chunks should not be erased just because a later
    // assistant block is oversized — that was the regression that left
    // cross-context with only "继续" and "[Request interrupted by user]".
    const turns = [
      userTurn("first question"),
      assistantTurn("first answer"),
      userTurn("second question"),
      // Big enough to be tail-truncated but should still leave the earlier
      // turns and the final user in the output.
      assistantTurn("z".repeat(CROSS_PROMPT_MAX - 2000)),
      userTurn("继续"),
      userTurn("[Request interrupted by user]"),
    ];
    const prompt = buildCrossPromptFromTurns(turns);
    expect(prompt).toContain("[user]\nfirst question");
    expect(prompt).toContain("[assistant]\nfirst answer");
    expect(prompt).toContain("[user]\nsecond question");
    expect(prompt).toContain("[user]\n继续");
    expect(prompt).toContain("[user]\n[Request interrupted by user]");
    expect(prompt.length).toBeLessThan(CROSS_PROMPT_MAX + 512);
  });

  it("formats structured content blocks when present", () => {
    const prompt = buildCrossPromptFromTurns([
      {
        blocks: [
          {
            kind: "user",
            blocks: [
              { type: "text", text: "review these" },
              { type: "resource", name: "spec.md", uri: "file:///tmp/spec.md" },
              { type: "image", uri: "file:///tmp/screen.png", mimeType: "image/png" },
            ],
          },
        ],
      },
    ]);
    expect(prompt).toContain("[user]\nreview these");
    expect(prompt).toContain("[file: __sessio_attachment__:spec.md|file:///tmp/spec.md]");
    expect(prompt).toContain("![__sessio_attachment__:image/png](file:///tmp/screen.png)");
    expect(prompt).not.toContain("[user]\nfallback");
  });

  it("builds cross context from ACP-like turns", () => {
    const prompt = buildCrossPromptFromTurns([
      {
        blocks: [
          {
            kind: "user",
            blocks: [
              { type: "text", text: "review these" },
              { type: "resource", name: "spec.md", uri: "file:///tmp/spec.md" },
              { type: "image", uri: "file:///tmp/screen.png", mimeType: "image/png" },
            ],
          },
          {
            kind: "tool",
            toolId: "tool-1",
          },
          {
            kind: "thought",
            blocks: [{ type: "text", text: "need to inspect files" }],
          },
          {
            kind: "assistant",
            blocks: [{ type: "text", text: "done" }],
          },
        ],
      },
    ]);

    expect(prompt).toContain("[user]\nreview these");
    expect(prompt).toContain("[file: __sessio_attachment__:spec.md|file:///tmp/spec.md]");
    expect(prompt).toContain("![__sessio_attachment__:image/png](file:///tmp/screen.png)");
    // Thought blocks are intentionally excluded — they dominate the budget
    // and the receiving agent does its own reasoning.
    expect(prompt).not.toContain("[thinking]");
    expect(prompt).not.toContain("need to inspect files");
    expect(prompt).toContain("[assistant]\ndone");
    expect(prompt).not.toContain("tool-1");
  });

  it("backfills the most recent user message above an assistant-led head", () => {
    // Build a long conversation where, after the budget pass, the top of the
    // selection would otherwise be an assistant turn — the receiver needs to
    // know what question that assistant was answering, so the most recent
    // user message preceding it should be prepended.
    const turns = [
      userTurn("original topic: refactor the parser"),
      assistantTurn("a".repeat(2000)),
      userTurn("nope try again"),
      assistantTurn("b".repeat(CROSS_PROMPT_MAX - 4000)),
      userTurn("continue"),
    ];
    const prompt = buildCrossPromptFromTurns(turns);
    // The final user is always present (it's an anchor).
    expect(prompt).toContain("[user]\ncontinue");
    // The receiver must see *some* user message at the top of the dialogue,
    // not an orphaned assistant turn.
    const body = prompt.split("<!-- sessio-cross:start")[1] ?? prompt;
    const firstRoleMarker = body.match(/\[(user|assistant)\]/);
    expect(firstRoleMarker?.[1]).toBe("user");
  });

  it("renders a [Todos] block from a turn's TodoWrite tool call", () => {
    const prompt = buildCrossPromptFromTurns([
      {
        blocks: [
          { kind: "user", blocks: [{ type: "text", text: "make a plan" }] },
          { kind: "assistant", blocks: [{ type: "text", text: "on it" }] },
        ],
        tools: [
          {
            title: "TodoWrite",
            kind: "todo",
            rawInput: {
              entries: [
                { content: "design schema", status: "completed" },
                { content: "wire backend", status: "in_progress" },
                { content: "ship UI", status: "pending" },
              ],
            },
          },
        ],
      },
    ]);
    expect(prompt).toContain("[Todos]");
    expect(prompt).toContain("[x] design schema");
    expect(prompt).toContain("[~] wire backend");
    expect(prompt).toContain("[ ] ship UI");
  });

  it("renders a [Plan] block from update_plan / TaskUpdate snapshots", () => {
    const prompt = buildCrossPromptFromTurns([
      {
        blocks: [{ kind: "user", blocks: [{ type: "text", text: "outline approach" }] }],
        tools: [
          {
            title: "update_plan",
            kind: "task_list",
            rawInput: {
              entries: [
                { content: "investigate", status: "completed" },
                { content: "implement", status: "in_progress" },
              ],
            },
          },
        ],
      },
    ]);
    expect(prompt).toContain("[Plan]");
    expect(prompt).toContain("[x] investigate");
    expect(prompt).toContain("[~] implement");
  });

  it("uses the latest todo snapshot when the same turn updates it twice", () => {
    const prompt = buildCrossPromptFromTurns([
      {
        blocks: [{ kind: "user", blocks: [{ type: "text", text: "plan" }] }],
        tools: [
          {
            title: "TodoWrite",
            kind: "todo",
            rawInput: { entries: [{ content: "draft", status: "in_progress" }] },
          },
          {
            title: "TodoWrite",
            kind: "todo",
            rawInput: { entries: [{ content: "draft", status: "completed" }] },
          },
        ],
      },
    ]);
    expect(prompt).toContain("[x] draft");
    expect(prompt).not.toContain("[~] draft");
  });
});
