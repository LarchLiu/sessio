import { describe, expect, it } from "vitest";
import type { AstraHandle, PlanRoundInfo, ThreadWorkSnapshot } from "../src/api";
import { renderThreadOrchestrationContext, renderThreadWorkContext } from "../src/threadSnapshot";

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
    const rendered = renderThreadWorkContext(snapshot(), "opencode");

    expect(rendered).not.toContain("## Stage assistant instructions");
    expect(rendered).not.toContain("Use the builder instructions.");
  });
});

describe("renderThreadOrchestrationContext", () => {
  it("includes Astra completion and task result state", () => {
    const rendered = renderThreadOrchestrationContext({
      threadId: "thread-1",
      astraRuns: [astraRun()],
      planRounds: [planRound()],
    });

    expect(rendered).toContain('kind="orchestration_context"');
    expect(rendered).toContain("[completed] astra-1");
    expect(rendered).toContain("terminal reason: requested joke has been drafted and polished");
    expect(rendered).toContain("Round 0 [completed] parallel");
    expect(rendered).toContain("[completed] Proofread joke -> codex");
    expect(rendered).toContain("result: polished long cold joke");
  });
});

function astraRun(): AstraHandle {
  return {
    runId: "astra-1",
    threadId: "thread-1",
    projectId: "project-1",
    status: "completed",
    mode: "auto",
    plannerBackend: "pi",
    roundIndex: 1,
    roundLimit: 3,
    terminalReason: "requested joke has been drafted and polished",
    lastErrorCode: null,
    lastErrorMessage: null,
    internalPlannerSessionIds: ["planner-session"],
    runDiagnostics: [],
    error: null,
    createdAt: 1,
    updatedAt: 2,
  };
}

function planRound(): PlanRoundInfo {
  return {
    id: "round-1",
    threadId: "thread-1",
    astraRunId: "astra-1",
    roundIndex: 0,
    summary: "Proofreader delivered a polished long cold joke.",
    mode: "parallel",
    source: "astra",
    status: "completed",
    createdAt: 1,
    updatedAt: 2,
    tasks: [{
      id: "task-1",
      roundId: "round-1",
      threadStageId: null,
      assistantId: "proofreader",
      agentParticipantId: null,
      targetAgent: "codex",
      stageSnapshotJson: null,
      assistantSnapshotJson: null,
      agentSnapshotJson: "{}",
      title: "Proofread joke",
      prompt: "Polish the joke.",
      expectedOutput: "Final joke",
      risk: "low",
      sortOrder: 0,
      status: "completed",
      resultSummary: "polished long cold joke",
      error: null,
      startedAt: 1,
      completedAt: 2,
      createdAt: 1,
      updatedAt: 2,
      sessions: [{
        taskId: "task-1",
        agent: "codex",
        sessionId: "session-1",
        role: "runtime",
        attemptCount: 1,
        createdAt: 1,
        updatedAt: 2,
      }],
    }],
  };
}
