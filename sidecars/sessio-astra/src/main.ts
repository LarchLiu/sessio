import { createPlan, assertStartParams, resolveModelSmoke } from "./astra";
import {
  type AstraTaskProposal,
  type AstraTaskResult,
  type StageSnapshot,
  type StartParams,
  errorResponse,
  event,
  isRequest,
  PROTOCOL_VERSION,
  response,
  type CancelParams,
  type ProtocolRequest,
  type ProtocolResponse,
  type TaskResultParams,
  type ToolCallRequest,
  type ThreadSnapshot,
} from "./protocol";

type Writer = (value: unknown) => Promise<void> | void;
type ToolCaller = (runId: string, name: string, args?: Record<string, unknown>) => Promise<unknown>;

const cancelledRuns = new Set<string>();
const taskResultsByRun = new Map<string, TaskResultParams["result"][]>();
const TERMINAL_STAGE_STATUSES = new Set(["completed", "skipped"]);
const MAX_ORCHESTRATION_ROUNDS = 25;

function redirectConsoleToStderr(): void {
  console.log = (...args: unknown[]) => console.error(...args);
  console.info = (...args: unknown[]) => console.error(...args);
  console.debug = (...args: unknown[]) => console.error(...args);
}

export async function handleRequest(
  request: ProtocolRequest,
  write: Writer,
  callTool: ToolCaller = async () => {
    throw new Error("tool bridge is unavailable");
  },
): Promise<void> {
  if (!isRequest(request)) {
    await write(errorResponse((request as { id?: string }).id ?? "unknown", "invalid_request", "invalid protocol request"));
    return;
  }

  try {
    switch (request.method) {
      case "astra/handshake":
        await write(response(request.id, { protocolVersion: PROTOCOL_VERSION, name: "sessio-astra" }));
        return;
      case "astra/start": {
        const params = assertStartParams(request);
        cancelledRuns.delete(params.runId);
        taskResultsByRun.set(params.runId, []);
        void orchestrateRun(params, callTool, write).catch(async (error) => {
          const message = errorMessage(error);
          await write(event(params.runId, "error", { status: "errored", message }));
        });
        await write(response(request.id, { status: "started" }));
        return;
      }
      case "astra/cancel": {
        const params = request.params as Partial<CancelParams> | undefined;
        if (!params || typeof params.runId !== "string") {
          throw new Error("runId is required");
        }
        cancelledRuns.add(params.runId);
        await write(event(params.runId, "cancelled", { status: "cancelled" }));
        await write(response(request.id, { status: "cancelled" }));
        return;
      }
      case "astra/task_result": {
        const params = request.params as Partial<TaskResultParams> | undefined;
        if (!params || typeof params.runId !== "string" || !isTaskResult(params.result)) {
          throw new Error("runId and result are required");
        }
        const results = taskResultsByRun.get(params.runId) ?? [];
        const existingIndex = results.findIndex(
          (result) =>
            result.taskId === params.result!.taskId &&
            result.sessioRuntimeSessionId === params.result!.sessioRuntimeSessionId,
        );
        if (existingIndex >= 0) {
          results[existingIndex] = params.result;
        } else {
          results.push(params.result);
        }
        taskResultsByRun.set(params.runId, results);
        await write(event(params.runId, "task_result", params.result));
        await write(response(request.id, { status: "received", resultCount: results.length }));
        return;
      }
      case "astra/smoke": {
        const pi = await resolveModelSmoke();
        await write(response(request.id, { protocolVersion: PROTOCOL_VERSION, pi }));
        return;
      }
      default:
        await write(errorResponse(request.id, "method_not_found", `unknown method: ${request.method}`));
    }
  } catch (error) {
    await write(errorResponse(request.id, "invalid_request", error instanceof Error ? error.message : String(error)));
  }
}

async function orchestrateRun(
  params: StartParams,
  callTool: ToolCaller,
  write: Writer,
): Promise<AstraTaskResult[]> {
  const runId = params.runId;
  const results: AstraTaskResult[] = [];
  const plannedTaskIds = new Set<string>();

  for (let round = 1; round <= MAX_ORCHESTRATION_ROUNDS && !cancelledRuns.has(runId); round += 1) {
    await write(event(runId, "status", { status: "planning", round }));
    const snapshot = await loadProjectSnapshot(runId, callTool);
    if (allActionableStagesTerminal(snapshot)) {
      await write(event(runId, "complete", { status: "completed", reason: "all_stages_terminal", round, results }));
      return results;
    }

    const planParams = {
      ...params,
      thread: snapshotThread(snapshot, params.thread),
      snapshot,
    };
    const plan = normalizePlanTaskIds(await createPlan(planParams), plannedTaskIds, round);
    await write(event(runId, "plan", { ...plan, round }));
    for (const candidate of plan.tasks) {
      await callTool(runId, "sessio.agent.plan_task", { ...candidate });
    }

    const tasks = dispatchableTasks(plan.tasks, snapshot);
    if (tasks.length === 0) {
      if (allActionableStagesTerminal(snapshot)) {
        await write(event(runId, "complete", { status: "completed", reason: "no_actionable_stages", round, results }));
        return results;
      }
      await write(event(runId, "error", {
        status: "errored",
        reason: "no_dispatchable_tasks",
        message: "Astra produced no dispatchable tasks for the remaining non-terminal stages.",
        round,
      }));
      return results;
    }

    await write(event(runId, "status", { status: "dispatching", round, taskIds: tasks.map((task) => task.id) }));
    for (const task of tasks) {
      if (cancelledRuns.has(runId)) break;
      await write(event(runId, "status", { status: "running", round, taskId: task.id, threadStageId: task.targetStageId ?? null }));
      const result = await dispatchTask(runId, task, callTool, write, results);
      await applyTaskResult(runId, task, result, callTool, write);
    }
  }

  if (cancelledRuns.has(runId)) {
    await write(event(runId, "complete", { status: "cancelled", results }));
    return results;
  }

  await write(event(runId, "error", {
    status: "errored",
    reason: "round_limit_reached",
    message: `Astra reached the orchestration round limit (${MAX_ORCHESTRATION_ROUNDS}).`,
    results,
  }));
  return results;
}

async function dispatchTask(
  runId: string,
  task: AstraTaskProposal,
  callTool: ToolCaller,
  write: Writer,
  results: AstraTaskResult[],
): Promise<AstraTaskResult> {
  const result = await callTool(runId, "sessio.agent.dispatch_task", { taskId: task.id }) as AstraTaskResult;
  results.push(result);
  await write(event(runId, "task_result", result));
  return result;
}

async function applyTaskResult(
  runId: string,
  task: AstraTaskProposal,
  result: AstraTaskResult,
  callTool: ToolCaller,
  write: Writer,
): Promise<void> {
  if (result.retryLimitReached) {
    const mutation = await callTool(runId, "sessio.stage.issue.add_or_update", {
      threadStageId: task.targetStageId,
      title: `Retry limit reached for ${task.title}`,
      description: result.error ?? "Sessio refused another direct dispatch for this stage.",
      severity: "high",
    });
    await write(event(runId, "stage_update_result", { taskId: task.id, result: mutation }));
    assertMutationOk(mutation, `retry-limit issue update failed for ${task.title}`);
    return;
  }
  if (result.status === "completed" && task.targetStageId) {
    const mutation = await callTool(runId, "sessio.stage.update", {
      threadStageId: task.targetStageId,
      taskId: task.id,
      status: "completed",
      summary: summarizeResult(result.output),
      outcome: result.output ?? "",
    });
    await write(event(runId, "stage_update_result", { taskId: task.id, result: mutation }));
    assertMutationOk(mutation, `stage update failed for ${task.title}`);
  } else if (task.targetStageId) {
    const mutation = await callTool(runId, "sessio.stage.issue.add_or_update", {
      threadStageId: task.targetStageId,
      title: `Astra task ${task.title} did not complete`,
      description: result.error ?? result.output ?? "Delegated task failed without output.",
      severity: result.status === "cancelled" ? "medium" : "high",
    });
    await write(event(runId, "stage_update_result", { taskId: task.id, result: mutation }));
    assertMutationOk(mutation, `issue update failed for ${task.title}`);
  }
}

async function loadProjectSnapshot(runId: string, callTool: ToolCaller): Promise<unknown> {
  return callTool(runId, "sessio.project.snapshot", {});
}

function allActionableStagesTerminal(snapshot: unknown): boolean {
  const stages = snapshotStages(snapshot);
  return stages.length > 0 && actionableStages(stages).length === 0;
}

function dispatchableTasks(tasks: AstraTaskProposal[], snapshot: unknown): AstraTaskProposal[] {
  const openStageIds = new Set(
    actionableStages(snapshotStages(snapshot))
      .flatMap((stage) => [stage.id, stage.stageId].filter((id): id is string => typeof id === "string" && id.length > 0)),
  );
  return tasks.filter((candidate) => !candidate.targetStageId || openStageIds.has(candidate.targetStageId));
}

function actionableStages(stages: StageSnapshot[]): StageSnapshot[] {
  return stages.filter((stage) => !TERMINAL_STAGE_STATUSES.has(stage.status ?? "") && stageHasAssignableAgent(stage));
}

function stageHasAssignableAgent(stage: StageSnapshot): boolean {
  return (stage.assistants ?? []).some((assistant) => {
    const id = assistant.agent?.id;
    return id === "codex" || id === "claude" || id === "gemini";
  });
}

function snapshotStages(snapshot: unknown): StageSnapshot[] {
  const root = snapshot && typeof snapshot === "object" ? snapshot as { thread?: unknown; stages?: unknown } : {};
  const thread = root.thread && typeof root.thread === "object" ? root.thread as { stages?: unknown } : null;
  const stages = Array.isArray(thread?.stages) ? thread.stages : Array.isArray(root.stages) ? root.stages : [];
  return stages.filter(isStageSnapshot);
}

function snapshotThread(snapshot: unknown, fallback: ThreadSnapshot): ThreadSnapshot {
  const root = snapshot && typeof snapshot === "object" ? snapshot as { thread?: unknown } : {};
  if (!root.thread || typeof root.thread !== "object") return fallback;
  const thread = root.thread as Partial<ThreadSnapshot>;
  if (typeof thread.id !== "string" || typeof thread.projectId !== "string" || typeof thread.goal !== "string") {
    return fallback;
  }
  return {
    id: thread.id,
    projectId: thread.projectId,
    goal: thread.goal,
    description: typeof thread.description === "string" ? thread.description : null,
    stages: Array.isArray(thread.stages) ? thread.stages.filter(isStageSnapshot) : fallback.stages,
    sessions: Array.isArray(thread.sessions) ? thread.sessions : fallback.sessions,
  };
}

function normalizePlanTaskIds(plan: { summary: string; tasks: AstraTaskProposal[] }, seen: Set<string>, round: number): { summary: string; tasks: AstraTaskProposal[] } {
  const tasks = plan.tasks.map((task, index) => {
    let id = task.id.trim() || `astra-task-${round}-${index + 1}`;
    if (seen.has(id)) {
      const base = id;
      let suffix = 2;
      while (seen.has(`${base}-r${round}-${suffix}`)) suffix += 1;
      id = `${base}-r${round}-${suffix}`;
    }
    seen.add(id);
    return id === task.id ? task : { ...task, id };
  });
  return { summary: plan.summary, tasks };
}

function isStageSnapshot(value: unknown): value is StageSnapshot {
  if (!value || typeof value !== "object") return false;
  const stage = value as Partial<StageSnapshot>;
  return typeof stage.id === "string" && stage.id.length > 0;
}

function assertMutationOk(value: unknown, message: string): void {
  if (!value || typeof value !== "object") {
    throw new Error(message);
  }
  const result = value as { ok?: unknown; error?: unknown };
  if (result.ok !== true) {
    const detail = typeof result.error === "string" && result.error.trim() ? `: ${result.error}` : "";
    throw new Error(`${message}${detail}`);
  }
}

function summarizeResult(output: string | undefined): string {
  const text = (output ?? "").trim();
  if (!text) return "Astra delegated task completed.";
  return text.length > 500 ? `${text.slice(0, 500)}...` : text;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isTaskResult(value: unknown): value is TaskResultParams["result"] {
  if (!value || typeof value !== "object") return false;
  const result = value as Partial<TaskResultParams["result"]>;
  return (
    typeof result.taskId === "string" &&
    result.taskId.length > 0 &&
    typeof result.sessioRuntimeSessionId === "string" &&
    result.sessioRuntimeSessionId.length > 0 &&
    (result.status === "completed" ||
      result.status === "failed" ||
      result.status === "errored" ||
      result.status === "cancelled")
  );
}

async function runStdio(): Promise<void> {
  let buffer = "";
  let requestSeq = 1;
  let closing = false;
  let outputClosed = false;
  let writeChain: Promise<void> = Promise.resolve();
  const handlers = new Set<Promise<void>>();
  const pending = new Map<string, {
    resolve: (value: unknown) => void;
    reject: (error: Error) => void;
    timeout: ReturnType<typeof setTimeout>;
  }>();
  const write = (message: unknown): Promise<void> => {
    const line = `${JSON.stringify(message)}\n`;
    writeChain = writeChain.catch(() => undefined).then(async () => {
      if (outputClosed) return;
      await Bun.stdout.write(line);
    });
    return writeChain;
  };
  const callTool: ToolCaller = (runId, name, args) => {
    if (closing) {
      return Promise.reject(new Error("stdio closed"));
    }
    const id = `sidecar-tool-${requestSeq++}`;
    const message: ToolCallRequest = {
      protocolVersion: PROTOCOL_VERSION,
      id,
      method: "tool/call",
      params: { runId, name, args },
    };
    return new Promise((resolve, reject) => {
      const timeoutMs = name === "sessio.agent.dispatch_task" ? 65 * 60_000 : 60_000;
      const timeout = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`tool call timed out: ${name}`));
      }, timeoutMs);
      pending.set(id, { resolve, reject, timeout });
      void write(message).catch((error) => {
        clearTimeout(timeout);
        pending.delete(id);
        reject(error instanceof Error ? error : new Error(String(error)));
      });
    });
  };
  const rejectPending = (reason: string) => {
    const error = new Error(reason);
    for (const [id, waiter] of pending) {
      clearTimeout(waiter.timeout);
      pending.delete(id);
      waiter.reject(error);
    }
  };
  const beginShutdown = (reason: string) => {
    if (closing) return;
    closing = true;
    rejectPending(reason);
  };
  process.on("SIGTERM", () => {
    beginShutdown("received SIGTERM");
    process.exit(0);
  });
  process.on("SIGINT", () => {
    beginShutdown("received SIGINT");
    process.exit(0);
  });

  try {
    for await (const chunk of Bun.stdin.stream()) {
      buffer += new TextDecoder().decode(chunk);
      let newline = buffer.indexOf("\n");
      while (newline >= 0) {
        const line = buffer.slice(0, newline).trim();
        buffer = buffer.slice(newline + 1);
        if (line) {
          try {
            const message = JSON.parse(line);
            if (isResponse(message) && pending.has(message.id)) {
              const waiter = pending.get(message.id)!;
              pending.delete(message.id);
              clearTimeout(waiter.timeout);
              if (message.error) {
                waiter.reject(new Error(message.error.message));
              } else {
                waiter.resolve(message.result);
              }
            } else {
              const handler = handleRequest(message, write, callTool).catch((error) => {
                return write(errorResponse(
                  typeof message?.id === "string" ? message.id : "unknown",
                  "internal_error",
                  error instanceof Error ? error.message : String(error),
                ));
              }).finally(() => {
                handlers.delete(handler);
              });
              handlers.add(handler);
            }
          } catch (error) {
            void write(errorResponse("unknown", "parse_error", error instanceof Error ? error.message : String(error)));
          }
        }
        newline = buffer.indexOf("\n");
      }
    }
  } finally {
    beginShutdown("stdin closed");
    await Promise.allSettled([...handlers]);
    await writeChain;
    outputClosed = true;
  }
}

function isResponse(value: unknown): value is ProtocolResponse {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<ProtocolResponse>;
  return candidate.protocolVersion === PROTOCOL_VERSION && typeof candidate.id === "string";
}

if (import.meta.main) {
  if (Bun.argv.includes("--stdio")) {
    redirectConsoleToStderr();
    await runStdio();
  } else if (Bun.argv.includes("--smoke")) {
    const pi = await resolveModelSmoke();
    console.log(JSON.stringify({ protocolVersion: PROTOCOL_VERSION, pi }));
  } else {
    console.error("sessio-astra requires --stdio");
    process.exit(2);
  }
}
