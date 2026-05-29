import { describe, expect, it } from "vitest";
import { buildCrossPrompt, CROSS_PROMPT_MAX } from "../src/cross";
import type { SessionMessage } from "../src/api";

function userMessage(text: string): SessionMessage {
  return { role: "user", text, timestamp: 1 };
}

function assistantMessage(text: string): SessionMessage {
  return { role: "assistant", text, timestamp: 2 };
}

describe("buildCrossPrompt", () => {
  it("keeps the latest oversized user turn instead of returning empty", () => {
    const prompt = buildCrossPrompt([userMessage("x".repeat(CROSS_PROMPT_MAX + 100))]);
    expect(prompt).toContain("[user]");
    expect(prompt).toContain("[...truncated...]");
    expect(prompt).toContain("x".repeat(32));
    expect(prompt).toContain("<!-- sessio-cross:end -->");
  });

  it("falls back to the latest user when only assistant tail fits", () => {
    const prompt = buildCrossPrompt([
      userMessage("important question"),
      assistantMessage("y".repeat(CROSS_PROMPT_MAX + 100)),
    ]);
    expect(prompt).toContain("[user]\nimportant question");
    expect(prompt).toContain("[assistant]");
    expect(prompt).toContain("[...truncated...]");
    expect(prompt.length).toBeLessThan(CROSS_PROMPT_MAX + 512);
  });

  it("keeps latest user and truncates oversized assistant when both cannot fully fit", () => {
    const prompt = buildCrossPrompt([
      userMessage("important question"),
      assistantMessage("y".repeat(CROSS_PROMPT_MAX * 2)),
    ]);
    expect(prompt).toContain("[user]\nimportant question");
    expect(prompt).toContain("[assistant]");
    expect(prompt).toContain("[...truncated...]");
    expect(prompt.length).toBeLessThan(CROSS_PROMPT_MAX + 512);
  });
});
