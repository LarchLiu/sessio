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

    expect(blocks).toEqual([{ type: "text", text: "astra_plan_task" }]);
  });

  it("recovers a kind placeholder from raw prompt data when user blocks are empty", () => {
    const prompt = buildSessioThreadPromptBlock(
      "astra_planner",
      JSON.stringify({ userPrompt: "写个笑话" }),
      { thread_id: "thread-1", prompt_summary: "写个笑话" },
    );
    const blocks = threadPromptDisplayContentBlocks([], { prompt }, true);

    expect(blocks).toEqual([{ type: "text", text: "astra_planner · 写个笑话" }]);
  });

  it("uses task attrs for hidden task prompt placeholders", () => {
    const prompt = buildSessioThreadPromptBlock(
      "astra_teamwork_task",
      "internal",
      {
        thread_id: "thread-1",
        task_title: "Write the joke",
        assistant_name: "Writer",
        target_agent: "codex",
      },
    );
    const blocks = threadPromptDisplayContentBlocks([{ type: "text", text: prompt }], {}, true);

    expect(blocks).toEqual([
      { type: "text", text: "astra_teamwork_task · Write the joke · Writer" },
    ]);
  });

  it("does not show target agent as the only placeholder context", () => {
    const prompt = buildSessioThreadPromptBlock(
      "astra_plan_task",
      "internal",
      { thread_id: "thread-1", target_agent: "codex" },
    );
    const blocks = threadPromptDisplayContentBlocks([{ type: "text", text: prompt }], {}, true);

    expect(blocks).toEqual([{ type: "text", text: "astra_plan_task" }]);
  });

  it("uses snapshot fallback when old prompt attrs only contain agent", () => {
    const prompt = buildSessioThreadPromptBlock(
      "astra_plan_task",
      "internal",
      { thread_id: "thread-1", target_agent: "codex" },
    );
    const blocks = threadPromptDisplayContentBlocks(
      [{ type: "text", text: prompt }],
      {},
      true,
      [{
        kind: null,
        attrs: {
          task_title: "Draft a new Chinese joke",
          assistant_name: "Writer",
        },
      }],
    );

    expect(blocks).toEqual([
      { type: "text", text: "astra_plan_task · Draft a new Chinese joke · Writer" },
    ]);
  });

  it("does not show fallback target agent as placeholder context", () => {
    const prompt = buildSessioThreadPromptBlock(
      "astra_plan_task",
      "internal",
      { thread_id: "thread-1", target_agent: "codex" },
    );
    const blocks = threadPromptDisplayContentBlocks(
      [{ type: "text", text: prompt }],
      {},
      true,
      [{
        kind: null,
        attrs: {
          target_agent: "codex",
        },
      }],
    );

    expect(blocks).toEqual([{ type: "text", text: "astra_plan_task" }]);
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
