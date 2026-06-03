import { createPlan, assertStartParams, resolveModelSmoke } from "./astra";
import {
  type AstraTaskProposal,
  type AstraTaskResult,
  errorResponse,
  event,
  isRequest,
  PROTOCOL_VERSION,
  response,
  type CancelParams,
  type ConfirmParams,
  type ProtocolRequest,
  type ProtocolResponse,
  type TaskResultParams,
  type ToolCallRequest,
} from "./protocol";

type Writer = (value: unknown) => void;
type ToolCaller = (runId: string, name: string, args?: Record<string, unknown>) => Promise<unknown>;

const cancelledRuns = new Set<string>();
const taskResultsByRun = new Map<string, TaskResultParams["result"][]>();

export async function handleRequest(
  request: ProtocolRequest,
  write: Writer,
  callTool: ToolCaller = async () => {
    throw new Error("tool bridge is unavailable");
  },
): Promise<void> {
  if (!isRequest(request)) {
    write(errorResponse((request as { id?: string }).id ?? "unknown", "invalid_request", "invalid protocol request"));
    return;
  }

  try {
    switch (request.method) {
      case "astra/handshake":
        write(response(request.id, { protocolVersion: PROTOCOL_VERSION, name: "sessio-astra" }));
        return;
      case "astra/start": {
        const params = assertStartParams(request);
        cancelledRuns.delete(params.runId);
        write(event(params.runId, "status", { status: "planning" }));
        const plan = await createPlan(params);
        if (cancelledRuns.has(params.runId)) {
          write(event(params.runId, "cancelled", { status: "cancelled" }));
          write(response(request.id, { status: "cancelled" }));
          return;
        }
        write(event(params.runId, "plan", plan));
        write(response(request.id, { status: "plan_ready", plan }));
        return;
      }
      case "astra/confirm": {
        const params = request.params as Partial<ConfirmParams> | undefined;
        if (!params || typeof params.runId !== "string" || !Array.isArray(params.approvedTaskIds)) {
          throw new Error("runId and approvedTaskIds are required");
        }
        write(event(params.runId, "status", { status: "dispatching", approvedTaskIds: params.approvedTaskIds }));
        const tasks = (Array.isArray(params.tasks) ? params.tasks : []).filter(isTaskProposal);
        const results = await runConfirmedPlan(params.runId, params.approvedTaskIds, tasks, callTool, write);
        write(response(request.id, { status: "completed", results }));
        return;
      }
      case "astra/cancel": {
        const params = request.params as Partial<CancelParams> | undefined;
        if (!params || typeof params.runId !== "string") {
          throw new Error("runId is required");
        }
        cancelledRuns.add(params.runId);
        write(event(params.runId, "cancelled", { status: "cancelled" }));
        write(response(request.id, { status: "cancelled" }));
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
        write(event(params.runId, "task_result", params.result));
        write(response(request.id, { status: "received", resultCount: results.length }));
        return;
      }
      case "astra/smoke": {
        const pi = await resolveModelSmoke();
        write(response(request.id, { protocolVersion: PROTOCOL_VERSION, pi }));
        return;
      }
      default:
        write(errorResponse(request.id, "method_not_found", `unknown method: ${request.method}`));
    }
  } catch (error) {
    write(errorResponse(request.id, "invalid_request", error instanceof Error ? error.message : String(error)));
  }
}

async function runConfirmedPlan(
  runId: string,
  approvedTaskIds: string[],
  tasks: AstraTaskProposal[],
  callTool: ToolCaller,
  write: Writer,
): Promise<AstraTaskResult[]> {
  const approved = tasks.filter((task) => approvedTaskIds.includes(task.id));
  const results: AstraTaskResult[] = [];
  for (const task of approved) {
    if (cancelledRuns.has(runId)) break;
    write(event(runId, "status", { status: "running", taskId: task.id, threadStageId: task.targetStageId ?? null }));
    const result = await dispatchTaskWithRetry(runId, task, callTool, write, results);
    if (result.retryLimitReached) {
      const mutation = await callTool(runId, "sessio.stage.issue.add_or_update", {
        threadStageId: task.targetStageId,
        title: `Retry limit reached for ${task.title}`,
        description: result.error ?? "Sessio refused another direct dispatch for this stage.",
        severity: "high",
      });
      write(event(runId, "stage_update_result", { taskId: task.id, result: mutation }));
      assertMutationOk(mutation, `retry-limit issue update failed for ${task.title}`);
      continue;
    }
    if (result.status === "completed" && task.targetStageId) {
      const mutation = await callTool(runId, "sessio.stage.update", {
        threadStageId: task.targetStageId,
        taskId: task.id,
        status: "completed",
        summary: summarizeResult(result.output),
        outcome: result.output ?? "",
      });
      write(event(runId, "stage_update_result", { taskId: task.id, result: mutation }));
      assertMutationOk(mutation, `stage update failed for ${task.title}`);
    } else if (task.targetStageId) {
      const mutation = await callTool(runId, "sessio.stage.issue.add_or_update", {
        threadStageId: task.targetStageId,
        title: `Astra task ${task.title} did not complete`,
        description: result.error ?? result.output ?? "Delegated task failed without output.",
        severity: result.status === "cancelled" ? "medium" : "high",
      });
      write(event(runId, "stage_update_result", { taskId: task.id, result: mutation }));
      assertMutationOk(mutation, `issue update failed for ${task.title}`);
    }
  }
  write(event(runId, "complete", { status: cancelledRuns.has(runId) ? "cancelled" : "completed", results }));
  return results;
}

async function dispatchTaskWithRetry(
  runId: string,
  task: AstraTaskProposal,
  callTool: ToolCaller,
  write: Writer,
  results: AstraTaskResult[],
): Promise<AstraTaskResult> {
  const first = await callTool(runId, "sessio.agent.dispatch_task", { taskId: task.id }) as AstraTaskResult;
  results.push(first);
  write(event(runId, "task_result", first));
  if (first.status === "completed" || first.retryLimitReached || cancelledRuns.has(runId)) {
    return first;
  }

  write(event(runId, "status", {
    status: "running",
    taskId: task.id,
    threadStageId: task.targetStageId ?? null,
    retrying: true,
  }));
  const retry = await callTool(runId, "sessio.agent.dispatch_task", { taskId: task.id }) as AstraTaskResult;
  results.push(retry);
  write(event(runId, "task_result", retry));
  return retry;
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

function isTaskProposal(value: unknown): value is AstraTaskProposal {
  if (!value || typeof value !== "object") return false;
  const task = value as Partial<AstraTaskProposal>;
  return typeof task.id === "string" && typeof task.prompt === "string" && typeof task.title === "string";
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
  const pending = new Map<string, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();
  const write = (message: unknown) => {
    Bun.stdout.write(`${JSON.stringify(message)}\n`);
  };
  const callTool: ToolCaller = (runId, name, args) => {
    const id = `sidecar-tool-${requestSeq++}`;
    const message: ToolCallRequest = {
      protocolVersion: PROTOCOL_VERSION,
      id,
      method: "tool/call",
      params: { runId, name, args },
    };
    write(message);
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
    });
  };
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
            if (message.error) {
              waiter.reject(new Error(message.error.message));
            } else {
              waiter.resolve(message.result);
            }
          } else {
            void handleRequest(message, write, callTool).catch((error) => {
              write(errorResponse(
                typeof message?.id === "string" ? message.id : "unknown",
                "internal_error",
                error instanceof Error ? error.message : String(error),
              ));
            });
          }
        } catch (error) {
          write(errorResponse("unknown", "parse_error", error instanceof Error ? error.message : String(error)));
        }
      }
      newline = buffer.indexOf("\n");
    }
  }
}

function isResponse(value: unknown): value is ProtocolResponse {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<ProtocolResponse>;
  return candidate.protocolVersion === PROTOCOL_VERSION && typeof candidate.id === "string";
}

if (import.meta.main) {
  if (Bun.argv.includes("--stdio")) {
    await runStdio();
  } else if (Bun.argv.includes("--smoke")) {
    const pi = await resolveModelSmoke();
    console.log(JSON.stringify({ protocolVersion: PROTOCOL_VERSION, pi }));
  } else {
    console.error("sessio-astra requires --stdio");
    process.exit(2);
  }
}
