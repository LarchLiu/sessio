import { describe, expect, it } from "vitest";
import type { ThreadWorkSnapshot } from "../src/api";
import { parseWorkflowSnapshot, workflowSnapshotToView } from "../src/lib/blocksuite/blocks/workflow-card/snapshot";

function snapshot(): ThreadWorkSnapshot {
  return {
    threadId: "thread-1",
    projectId: "project-1",
    goal: "Ship live workflow cards",
    description: null,
    activeStageId: "stage-build",
    focusedStageId: "stage-build",
    stages: [
      {
        threadStageId: "stage-plan",
        projectStageId: "project-stage-plan",
        name: "Plan",
        kind: "plan",
        icon: null,
        status: "completed",
        summary: "Plan is ready.",
        outcome: null,
        assistants: [],
        issues: [],
        sessionRefs: [],
      },
      {
        threadStageId: "stage-build",
        projectStageId: "project-stage-build",
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
            color: "#2f6fed",
            agent: {
              id: "codex",
              name: "Codex",
              model: "gpt-5.3-codex",
              mode: "read-write",
              effort: "medium",
            },
            systemPrompt: null,
            order: 0,
          },
        ],
        issues: [
          {
            id: "issue-open",
            threadStageId: "stage-build",
            title: "Need verification",
            description: null,
            status: "open",
            severity: "medium",
            createdAt: 1,
            updatedAt: 1,
          },
          {
            id: "issue-resolved",
            threadStageId: "stage-build",
            title: "Resolved",
            description: null,
            status: "resolved",
            severity: "low",
            createdAt: 1,
            updatedAt: 1,
          },
        ],
        sessionRefs: [],
      },
    ],
    threadSessionRefs: [],
    detailRefs: {
      threadId: "thread-1",
      focusedStageId: "stage-build",
      stageIds: ["stage-plan", "stage-build"],
      issueIds: ["issue-open", "issue-resolved"],
      sessionRefs: [],
    },
    rollup: {
      completed: 1,
      incomplete: 1,
      blocked: 0,
      openIssues: 1,
      currentStage: "Build",
      total: 2,
    },
    capturedAt: 1,
  };
}

describe("parseWorkflowSnapshot", () => {
  it("projects stages, assistants, issues, and rollup for card rendering", () => {
    const view = parseWorkflowSnapshot(JSON.stringify(snapshot()));

    expect(view?.threadId).toBe("thread-1");
    expect(view?.goal).toBe("Ship live workflow cards");
    expect(view?.rollup).toMatchObject({
      completed: 1,
      total: 2,
      openIssues: 1,
      currentStage: "Build",
    });
    expect(view?.stages).toHaveLength(2);
    expect(view?.stages[1]).toMatchObject({
      threadStageId: "stage-build",
      status: "in_progress",
      focused: true,
      active: true,
      openIssues: 1,
    });
    expect(view?.stages[1]?.assistants[0]).toMatchObject({
      assistantId: "assistant-codex",
      name: "Builder",
      color: "#2f6fed",
      agentLabel: "Codex",
      initial: "B",
    });
  });

  it("returns null for empty or invalid snapshots", () => {
    expect(parseWorkflowSnapshot("")).toBeNull();
    expect(parseWorkflowSnapshot("{bad json")).toBeNull();
    expect(workflowSnapshotToView(null)).toBeNull();
  });
});
