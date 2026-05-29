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
    expect(prompt).toContain("[thinking]\nneed to inspect files");
    expect(prompt).toContain("[assistant]\ndone");
    expect(prompt).not.toContain("tool-1");
  });
});
