import { describe, expect, it } from "vitest";
import type { AstraHandle } from "../src/api";
import {
  astraStatusClass,
  astraTaskStatusClass,
  formatAstraStatus,
  isAstraActive,
  upsertAstraRun,
  visibleAstraRunDiagnostics,
} from "../src/threadAstraView";

describe("threadAstraView", () => {
  it("classifies active Astra run statuses", () => {
    expect(isAstraActive("planning")).toBe(true);
    expect(isAstraActive("running")).toBe(true);
    expect(isAstraActive("completed")).toBe(false);
    expect(isAstraActive("cancelled")).toBe(false);
  });

  it("upserts and sorts runs by updated time", () => {
    const older = run("run-1", 1);
    const newer = run("run-2", 5);
    const updated = run("run-1", 10);

    expect(upsertAstraRun([older], newer).map((item) => item.runId)).toEqual(["run-2", "run-1"]);
    expect(upsertAstraRun([older, newer], updated).map((item) => item.runId)).toEqual(["run-1", "run-2"]);
  });

  it("formats statuses and keeps task/run classes distinct", () => {
    expect(formatAstraStatus("awaiting_approval")).toBe("awaiting approval");
    expect(astraStatusClass("errored")).toContain("red");
    expect(astraTaskStatusClass("planned")).toContain("ink");
    expect(astraTaskStatusClass("completed")).toContain("emerald");
  });

  it("hides teamwork round journal entries from visible diagnostics", () => {
    const journal = { kind: "teamwork_round_journal", roundIndex: 0, tasks: [] };
    const failure = { kind: "orchestrator_backend_failure", code: "timeout" };
    const convergence = { kind: "debate_convergence", status: "diverged" };

    const visible = visibleAstraRunDiagnostics([journal, failure, null, "raw", 7, convergence]);

    expect(visible).toEqual([failure, null, "raw", 7, convergence]);
  });
});

function run(runId: string, updatedAt: number): AstraHandle {
  return {
    runId,
    threadId: "thread-1",
    projectId: "project-1",
    continuedFromRunId: null,
    status: "running",
    mode: "auto",
    plannerBackend: null,
    roundIndex: null,
    roundLimit: 3,
    lastErrorCode: null,
    lastErrorMessage: null,
    internalPlannerSessionIds: [],
    runDiagnostics: [],
    error: null,
    terminalReason: null,
    createdAt: 1,
    updatedAt,
  };
}
