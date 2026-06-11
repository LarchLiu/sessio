import { describe, expect, it } from "vitest";
import { buildSessioThreadPromptBlock } from "../src/historyMerge";
import { threadPromptDisplayContentBlocks } from "../src/threadPromptDisplay";

describe("threadPromptDisplayContentBlocks", () => {
  it("shows a kind placeholder when a hidden thread prompt text block is empty", () => {
    const blocks = threadPromptDisplayContentBlocks(
      [{ type: "text", text: "", meta: { sessioThreadPromptKinds: ["astra_plan_task"] } }],
      {},
      true,
    );

    expect(blocks).toEqual([{ type: "text", text: "Thread prompt: astra_plan_task" }]);
  });

  it("recovers a kind placeholder from raw prompt data when user blocks are empty", () => {
    const prompt = buildSessioThreadPromptBlock(
      "astra_planner",
      JSON.stringify({ userPrompt: "写个笑话" }),
      { thread_id: "thread-1" },
    );
    const blocks = threadPromptDisplayContentBlocks([], { prompt }, true);

    expect(blocks).toEqual([{ type: "text", text: "Thread prompt: astra_planner" }]);
  });

  it("keeps thread prompts hidden outside ThreadChatPage placeholder mode", () => {
    const prompt = buildSessioThreadPromptBlock(
      "astra_teamwork_task",
      "internal",
      { thread_id: "thread-1" },
    );
    const blocks = threadPromptDisplayContentBlocks([{ type: "text", text: prompt }], {}, false);

    expect(blocks).toEqual([]);
  });
});
