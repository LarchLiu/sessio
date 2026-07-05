import { describe, expect, it } from "vitest";
import {
  collectCreatedThreadIdsFromTexts,
  isSessioThreadCreateCommand,
} from "../src/lib/chatCreatedThreads";

describe("collectCreatedThreadIdsFromTexts", () => {
  it("collects created top-level process, teamwork, brainstorm, and debate threads", () => {
    const result = collectCreatedThreadIdsFromTexts([
      "Created process thread thread-process-1 with thread-stage-plan ignored.",
      "Created teamwork thread thread-teamwork-1 and assistant thread-agent-builder ignored.",
      "Created brainstorm thread thread-brainstorm-1.",
      "Created debate thread thread-debate-1. Duplicate thread-teamwork-1.",
    ]);

    expect(result.threadIds).toEqual([
      "thread-process-1",
      "thread-teamwork-1",
      "thread-brainstorm-1",
      "thread-debate-1",
    ]);
    expect(result.refreshKey).toContain("thread-process-1|thread-teamwork-1|thread-brainstorm-1|thread-debate-1");
  });

  it("recognizes only Sessio thread creation commands as creation evidence", () => {
    expect(isSessioThreadCreateCommand("~/.sessio/bin/sessio thread create --kind process --json")).toBe(true);
    expect(isSessioThreadCreateCommand("/Users/alex/.sessio/bin/sessio thread create --kind teamwork --json")).toBe(true);
    expect(isSessioThreadCreateCommand("cmd: ~/.sessio/bin/sessio thread create --kind brainstorm --json")).toBe(true);
    expect(isSessioThreadCreateCommand("sessio \\\n      thread create --kind debate --json")).toBe(true);

    expect(isSessioThreadCreateCommand("~/.sessio/bin/sessio thread show --id thread-existing --json")).toBe(false);
    expect(isSessioThreadCreateCommand("Open existing thread thread-existing on canvas")).toBe(false);
  });
});
