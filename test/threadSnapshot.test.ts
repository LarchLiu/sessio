import { describe, expect, it } from "vitest";
import type { ThreadWorkSnapshot } from "../src/api";
import { renderThreadWorkContext } from "../src/threadSnapshot";

function snapshot(): ThreadWorkSnapshot {
  return {
    threadId: "thread-1",
    projectId: "project-1",
    goal: "Ship stage prompt",
    description: null,
    activeStageId: "stage-1",
    focusedStageId: "stage-1",
    stages: [
      {
        threadStageId: "stage-1",
        projectStageId: "project-stage-1",
        name: "Build",
        kind: "build",
        icon: null,
        status: "in_progress",
        summary: null,
        outcome: null,
        assistants: [
          {
            assistantId: "assistant-codex",
            name: "Builder",
            color: null,
            agent: {
              id: "codex",
              name: "Codex",
              model: "gpt-5.3-codex",
              mode: "read-write",
              effort: "medium",
            },
            systemPrompt: "Use the builder instructions.",
            order: 0,
          },
          {
            assistantId: "assistant-claude",
            name: "Reviewer",
            color: null,
            agent: {
              id: "claude",
              name: "Claude",
              model: "claude-sonnet-4-5",
              mode: "read-only",
              effort: "medium",
            },
            systemPrompt: "Keep this review-only prompt out.",
            order: 1,
          },
        ],
        issues: [],
        sessionRefs: [],
      },
    ],
    threadSessionRefs: [],
    relatedContext: {
      sessionExcerptRefs: [],
    },
    detailRefs: {
      threadId: "thread-1",
      focusedStageId: "stage-1",
      stageIds: ["stage-1"],
      issueIds: [],
      sessionRefs: [],
    },
    rollup: {
      completed: 0,
      incomplete: 1,
      blocked: 0,
      openIssues: 0,
      currentStage: "Build",
      total: 1,
    },
    capturedAt: 1,
  };
}

describe("renderThreadWorkContext", () => {
  it("includes focused stage assistant prompt for the selected agent", () => {
    const rendered = renderThreadWorkContext(snapshot(), "codex");

    expect(rendered).toContain("## Stage assistant instructions");
    expect(rendered).toContain("### Builder");
    expect(rendered).toContain("Use the builder instructions.");
    expect(rendered).not.toContain("Keep this review-only prompt out.");
  });

  it("omits assistant prompts when no matching agent is selected", () => {
    const rendered = renderThreadWorkContext(snapshot(), "gemini");

    expect(rendered).not.toContain("## Stage assistant instructions");
    expect(rendered).not.toContain("Use the builder instructions.");
  });
});
