type JsonRecord = Record<string, unknown>;

type StageFixture = {
  id: string;
  name: string;
  status: string;
  order: number;
  assistants: Array<{ agent: { id: "codex" | "claude" | "gemini" } }>;
};

type HarnessResult = {
  output: string;
  messages: JsonRecord[];
};

type ToolHandler = (message: JsonRecord, send: (message: JsonRecord) => void) => void;

const decoder = new TextDecoder();

function sidecarEnv(extra: Record<string, string> = {}): Record<string, string> {
  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(Bun.env)) {
    if (typeof value === "string") env[key] = value;
  }
  return { ...env, ...extra };
}

function spawnSidecar(extraEnv: Record<string, string> = {}) {
  return Bun.spawn(["bun", "run", "src/main.ts", "--stdio"], {
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
    env: sidecarEnv(extraEnv),
  });
}

function startRequest(id: string, runId: string, stages: StageFixture[], extraParams: JsonRecord = {}): JsonRecord {
  return {
    protocolVersion: 1,
    id,
    method: "astra/start",
    params: {
      runId,
      thread: {
        id: `thread-${runId}`,
        projectId: `project-${runId}`,
        goal: "Smoke test Astra",
        stages,
      },
      ...extraParams,
    },
  };
}

function stage(id: string, name: string, status = "in_progress", order = 0): StageFixture {
  return {
    id,
    name,
    status,
    order,
    assistants: [{ agent: { id: "codex" } }],
  };
}

function responseFor(id: unknown, result: unknown): JsonRecord {
  return { protocolVersion: 1, id: String(id), result };
}

function errorFor(id: unknown, message: string): JsonRecord {
  return {
    protocolVersion: 1,
    id: String(id),
    error: { code: "unexpected_tool", message },
  };
}

async function runHarness(
  request: JsonRecord,
  handleTool: ToolHandler,
  done: (messages: JsonRecord[]) => boolean,
  timeoutMs = 7000,
): Promise<HarnessResult> {
  const proc = spawnSidecar();
  const reader = proc.stdout.getReader();
  const messages: JsonRecord[] = [];
  let buffer = "";
  let output = "";
  const deadline = Date.now() + timeoutMs;
  let doneAt: number | null = null;

  const send = (message: JsonRecord) => {
    proc.stdin.write(`${JSON.stringify(message)}\n`);
  };

  send(request);

  while (Date.now() < deadline) {
    if (done(messages)) {
      doneAt ??= Date.now();
      if (Date.now() - doneAt > 150) break;
    }
    const read = await Promise.race([
      reader.read(),
      new Promise<ReadableStreamReadResult<Uint8Array>>((resolve) =>
        setTimeout(() => resolve({ done: false, value: new Uint8Array() }), 100),
      ),
    ]);
    if (read.done) break;
    if (!read.value || read.value.length === 0) continue;
    buffer += decoder.decode(read.value);
    let newline = buffer.indexOf("\n");
    while (newline >= 0) {
      const line = buffer.slice(0, newline).trim();
      buffer = buffer.slice(newline + 1);
      if (line) {
        output += `${line}\n`;
        const message = JSON.parse(line) as JsonRecord;
        messages.push(message);
        if (message.method === "tool/call") {
          handleTool(message, send);
        }
      }
      newline = buffer.indexOf("\n");
    }
  }

  proc.stdin.end();
  proc.kill();
  await proc.exited;
  return { output, messages };
}

function paramsOf(message: JsonRecord): JsonRecord {
  return (message.params && typeof message.params === "object" ? message.params : {}) as JsonRecord;
}

function argsOf(message: JsonRecord): JsonRecord {
  const params = paramsOf(message);
  return (params.args && typeof params.args === "object" ? params.args : {}) as JsonRecord;
}

function messageName(message: JsonRecord): string {
  return String(paramsOf(message).name ?? "");
}

function eventType(message: JsonRecord): string | null {
  return message.method === "event" ? String(paramsOf(message).type ?? "") : null;
}

function eventData(message: JsonRecord): JsonRecord {
  const data = paramsOf(message).data;
  return (data && typeof data === "object" ? data : {}) as JsonRecord;
}

function responseStatus(message: JsonRecord): string | null {
  const result = message.result;
  return result && typeof result === "object" ? String((result as JsonRecord).status ?? "") : null;
}

function hasEvent(messages: JsonRecord[], type: string): boolean {
  return messages.some((message) => eventType(message) === type);
}

function hasTool(messages: JsonRecord[], name: string): boolean {
  return messages.some((message) => message.method === "tool/call" && messageName(message) === name);
}

function snapshotResult(runId: string, stages: StageFixture[]): JsonRecord {
  return {
    thread: {
      id: `thread-${runId}`,
      projectId: `project-${runId}`,
      goal: "Smoke test Astra",
      stages,
    },
  };
}

function taskIdFrom(message: JsonRecord): string {
  return String(argsOf(message).taskId ?? "task-missing");
}

function stageIdFrom(message: JsonRecord): string {
  return String(argsOf(message).threadStageId ?? argsOf(message).targetStageId ?? "stage-missing");
}

async function assertStartCompletesTwoStageRun(): Promise<void> {
  const runId = "smoke-run";
  const stages = [
    stage("stage-plan", "Plan", "in_progress", 0),
    stage("stage-build", "Build", "not_started", 1),
    stage("stage-done", "Done", "completed", 2),
  ];
  const taskStages = new Map<string, string>();
  let snapshotCount = 0;
  const request = startRequest("start-1", runId, stages);
  const result = await runHarness(
    request,
    (message, send) => {
      const name = messageName(message);
      if (name === "sessio.project.snapshot") {
        snapshotCount += 1;
        send(responseFor(message.id, snapshotResult(runId, stages)));
      } else if (name === "sessio.agent.plan_task") {
        taskStages.set(String(argsOf(message).id ?? "task"), String(argsOf(message).targetStageId ?? ""));
        send(responseFor(message.id, { taskIds: [String(argsOf(message).id ?? "task")] }));
      } else if (name === "sessio.agent.dispatch_task") {
        const taskId = taskIdFrom(message);
        const task = stages.find((item) => item.id === taskStages.get(taskId)) ?? stages[0];
        send(responseFor(message.id, {
          taskId,
          threadStageId: task.id,
          sessioRuntimeSessionId: `agent-session-${task.id}`,
          status: "completed",
          output: `done ${task.id}`,
          attemptCount: 1,
          retryLimitReached: false,
        }));
      } else if (name === "sessio.stage.update") {
        const id = stageIdFrom(message);
        const target = stages.find((item) => item.id === id);
        if (target) target.status = "completed";
        send(responseFor(message.id, {
          ok: true,
          stage: { id, status: "completed", summary: "done", outcome: "done" },
          error: null,
          appliedAt: Date.now(),
        }));
      } else {
        send(errorFor(message.id, `unexpected tool ${name}`));
      }
    },
    (messages) => hasEvent(messages, "complete"),
  );

  if (!result.messages.some((message) => message.id === "start-1" && responseStatus(message) === "started")) {
    throw new Error(`start response missing from smoke output: ${result.output}`);
  }
  for (const name of ["sessio.project.snapshot", "sessio.agent.plan_task", "sessio.agent.dispatch_task", "sessio.stage.update"]) {
    if (!hasTool(result.messages, name)) {
      throw new Error(`${name} missing from smoke output: ${result.output}`);
    }
  }
  if (!hasEvent(result.messages, "task_result") || !hasEvent(result.messages, "stage_update_result")) {
    throw new Error(`expected task_result/stage_update_result events: ${result.output}`);
  }
  const plannedTaskCount = result.messages.filter((message) => message.method === "tool/call" && messageName(message) === "sessio.agent.plan_task").length;
  const dispatchCount = result.messages.filter((message) => message.method === "tool/call" && messageName(message) === "sessio.agent.dispatch_task").length;
  if (snapshotCount < 2 || plannedTaskCount < 2 || dispatchCount !== 2) {
    throw new Error(`expected snapshot -> plan queue -> dispatch queue: ${result.output}`);
  }
  const complete = result.messages.find((message) => eventType(message) === "complete");
  if (eventData(complete ?? {}).reason !== "all_stages_terminal") {
    throw new Error(`expected all_stages_terminal complete event: ${result.output}`);
  }
  console.log("sessio-astra orchestration smoke ok");
}

async function assertModelSmoke(): Promise<void> {
  const smokeProc = spawnSidecar();
  const smokeRequest = {
    protocolVersion: 1,
    id: "smoke-model-1",
    method: "astra/smoke",
  };

  smokeProc.stdin.write(`${JSON.stringify(smokeRequest)}\n`);
  smokeProc.stdin.end();

  const smokeOutput = await new Response(smokeProc.stdout).text();
  await smokeProc.exited;
  const smokeLines = smokeOutput
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
  const smokeResponse = smokeLines.find((line) => line.id === "smoke-model-1");
  if (smokeResponse?.result?.pi?.available !== true || smokeResponse.result.pi.planningMode !== "deterministic") {
    throw new Error(`expected deterministic Pi smoke without model config: ${smokeOutput}`);
  }
  if (smokeResponse.result.pi.agentAvailable !== true || smokeResponse.result.pi.modelConfigured !== false) {
    throw new Error(`expected Pi agent bootstrap with no configured model: ${smokeOutput}`);
  }
  console.log("sessio-astra model smoke ok");
}

async function assertFauxPlanRunsThroughStart(): Promise<void> {
  const runId = "pi-run";
  const stages = [stage("stage-plan", "Plan", "in_progress", 0)];
  const fauxPlan = {
    summary: "Faux Pi planned one task.",
    tasks: [
      {
        id: "model-picked-id-is-replaced",
        title: "Model plan task",
        targetStageId: "stage-plan",
        targetAgent: "codex",
        prompt: "Use the model-supplied plan to advance the stage.",
        expectedOutput: "A short model-planned result.",
        risk: "medium",
      },
    ],
  };
  const result = await runHarness(
    startRequest("pi-plan-1", runId, stages, {
      modelConfig: {
        provider: "faux",
        api: "faux",
        modelId: "sessio-astra-faux",
        apiKey: "faux",
        fauxPlanJson: JSON.stringify(fauxPlan),
      },
    }),
    (message, send) => {
      const name = messageName(message);
      if (name === "sessio.project.snapshot") {
        send(responseFor(message.id, snapshotResult(runId, stages)));
      } else if (name === "sessio.agent.plan_task") {
        send(responseFor(message.id, { taskIds: [String(argsOf(message).id ?? "task")] }));
      } else if (name === "sessio.agent.dispatch_task") {
        send(responseFor(message.id, {
          taskId: taskIdFrom(message),
          threadStageId: "stage-plan",
          sessioRuntimeSessionId: "agent-session-model",
          status: "completed",
          output: "done",
          attemptCount: 1,
          retryLimitReached: false,
        }));
      } else if (name === "sessio.stage.update") {
        stages[0].status = "completed";
        send(responseFor(message.id, {
          ok: true,
          stage: { id: "stage-plan", status: "completed", summary: "done", outcome: "done" },
          error: null,
          appliedAt: Date.now(),
        }));
      } else {
        send(errorFor(message.id, `unexpected tool ${name}`));
      }
    },
    (messages) => hasEvent(messages, "complete"),
  );

  const plan = result.messages.find((message) => eventType(message) === "plan");
  const tasks = (eventData(plan ?? {}).tasks ?? []) as JsonRecord[];
  if (eventData(plan ?? {}).summary !== fauxPlan.summary) {
    throw new Error(`Pi faux plan summary missing: ${result.output}`);
  }
  if (tasks.length !== 1 || tasks[0]?.title !== "Model plan task") {
    throw new Error(`Pi faux plan task missing: ${result.output}`);
  }
  if (tasks[0]?.id === fauxPlan.tasks[0].id) {
    throw new Error(`Pi plan task id was not normalized: ${result.output}`);
  }
  console.log("sessio-astra pi orchestration smoke ok");
}

async function assertRetryLimitRecordsIssue(): Promise<void> {
  const runId = "retry-run";
  const stages = [stage("stage-retry", "Retry", "in_progress", 0)];
  const result = await runHarness(
    startRequest("retry-1", runId, stages),
    (message, send) => {
      const name = messageName(message);
      if (name === "sessio.project.snapshot") {
        send(responseFor(message.id, snapshotResult(runId, stages)));
      } else if (name === "sessio.agent.plan_task") {
        send(responseFor(message.id, { taskIds: [String(argsOf(message).id ?? "task")] }));
      } else if (name === "sessio.agent.dispatch_task") {
        send(responseFor(message.id, {
          taskId: taskIdFrom(message),
          threadStageId: "stage-retry",
          sessioRuntimeSessionId: "agent-session-retry",
          status: "failed",
          output: "",
          error: "retry limit reached",
          attemptCount: 3,
          retryLimitReached: true,
        }));
      } else if (name === "sessio.stage.issue.add_or_update") {
        stages[0].status = "skipped";
        send(responseFor(message.id, {
          ok: true,
          issue: { id: "issue-retry", threadStageId: "stage-retry" },
          error: null,
          appliedAt: Date.now(),
        }));
      } else {
        send(errorFor(message.id, `unexpected tool ${name}`));
      }
    },
    (messages) => hasEvent(messages, "complete"),
  );

  if (!hasTool(result.messages, "sessio.stage.issue.add_or_update") || !hasEvent(result.messages, "stage_update_result")) {
    throw new Error(`retry-limit loop did not record issue: ${result.output}`);
  }
  console.log("sessio-astra retry-limit smoke ok");
}

async function assertFailedTaskRecordsIssueWithoutRedispatch(): Promise<void> {
  const runId = "failure-issue-run";
  const stages = [stage("stage-failure", "Failure", "in_progress", 0)];
  let dispatchCount = 0;
  const result = await runHarness(
    startRequest("failure-issue-1", runId, stages),
    (message, send) => {
      const name = messageName(message);
      if (name === "sessio.project.snapshot") {
        send(responseFor(message.id, snapshotResult(runId, stages)));
      } else if (name === "sessio.agent.plan_task") {
        send(responseFor(message.id, { taskIds: [String(argsOf(message).id ?? "task")] }));
      } else if (name === "sessio.agent.dispatch_task") {
        dispatchCount += 1;
        send(responseFor(message.id, {
          taskId: taskIdFrom(message),
          threadStageId: "stage-failure",
          sessioRuntimeSessionId: "agent-session-failure",
          status: "failed",
          output: "needs a different approach",
          error: null,
          attemptCount: 1,
          retryLimitReached: false,
        }));
      } else if (name === "sessio.stage.issue.add_or_update") {
        stages[0].status = "skipped";
        send(responseFor(message.id, {
          ok: true,
          issue: { id: "issue-failure", threadStageId: "stage-failure" },
          error: null,
          appliedAt: Date.now(),
        }));
      } else {
        send(errorFor(message.id, `unexpected tool ${name}`));
      }
    },
    (messages) => hasEvent(messages, "complete"),
  );

  if (dispatchCount !== 1 || !hasTool(result.messages, "sessio.stage.issue.add_or_update")) {
    throw new Error(`failure issue loop did not record one issue without redispatch: ${result.output}`);
  }
  console.log("sessio-astra failure-issue smoke ok");
}

await assertStartCompletesTwoStageRun();
await assertModelSmoke();
await assertFauxPlanRunsThroughStart();
await assertRetryLimitRecordsIssue();
await assertFailedTaskRecordsIssueWithoutRedispatch();

export {};
