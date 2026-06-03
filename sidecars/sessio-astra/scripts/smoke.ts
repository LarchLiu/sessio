function sidecarEnv(extra: Record<string, string> = {}): Record<string, string> {
  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(Bun.env)) {
    if (typeof value === "string") env[key] = value;
  }
  delete env.SESSIO_ASTRA_MODEL_PROVIDER;
  delete env.SESSIO_ASTRA_PROVIDER;
  delete env.SESSIO_ASTRA_MODEL_ID;
  delete env.SESSIO_ASTRA_MODEL;
  delete env.SESSIO_ASTRA_ALLOW_MISSING_API_KEY;
  delete env.SESSIO_ASTRA_FAUX_MODEL_ID;
  delete env.SESSIO_ASTRA_FAUX_PLAN_JSON;
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

const proc = spawnSidecar();

const request = {
  protocolVersion: 1,
  id: "1",
  method: "astra/start",
  params: {
    runId: "smoke-run",
    thread: {
      id: "thread-smoke",
      projectId: "project-smoke",
      goal: "Smoke test Astra",
      stages: [
        {
          id: "stage-plan",
          name: "Plan",
          status: "in_progress",
          order: 0,
          assistants: [{ agent: { id: "codex" } }],
        },
        {
          id: "stage-build",
          name: "Build",
          status: "not_started",
          order: 1,
          assistants: [{ agent: { id: "codex" } }],
        },
        {
          id: "stage-done",
          name: "Done",
          status: "completed",
          order: 2,
          assistants: [{ agent: { id: "codex" } }],
        },
      ],
    },
  },
};

proc.stdin.write(`${JSON.stringify(request)}\n`);
proc.stdin.end();

const output = await new Response(proc.stdout).text();
await proc.exited;

const lines = output
  .trim()
  .split("\n")
  .filter(Boolean)
  .map((line) => JSON.parse(line));

if (!lines.some((line) => line.method === "event" && line.params?.type === "plan")) {
  throw new Error(`plan event missing from smoke output: ${output}`);
}

if (!lines.some((line) => line.id === "1" && line.result?.status === "plan_ready")) {
  throw new Error(`plan response missing from smoke output: ${output}`);
}

const planResponse = lines.find((line) => line.id === "1");
if (planResponse.result?.plan?.tasks?.length !== 2) {
  throw new Error(`expected 2 pending stage tasks, got ${planResponse.result?.plan?.tasks?.length}: ${output}`);
}

console.log("sessio-astra smoke ok");

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
const piProc = spawnSidecar({ SESSIO_ASTRA_FAUX_PLAN_JSON: JSON.stringify(fauxPlan) });

piProc.stdin.write(`${JSON.stringify({ ...request, id: "pi-plan-1", params: { ...request.params, runId: "pi-run" } })}\n`);
piProc.stdin.end();

const piOutput = await new Response(piProc.stdout).text();
await piProc.exited;
const piLines = piOutput
  .trim()
  .split("\n")
  .filter(Boolean)
  .map((line) => JSON.parse(line));
const piResponse = piLines.find((line) => line.id === "pi-plan-1");
if (piResponse?.result?.plan?.summary !== fauxPlan.summary) {
  throw new Error(`Pi faux plan summary missing: ${piOutput}`);
}
if (piResponse.result.plan.tasks?.length !== 1 || piResponse.result.plan.tasks[0]?.title !== "Model plan task") {
  throw new Error(`Pi faux plan task missing: ${piOutput}`);
}
if (piResponse.result.plan.tasks[0]?.id === fauxPlan.tasks[0].id) {
  throw new Error(`Pi plan task id was not normalized: ${piOutput}`);
}

console.log("sessio-astra pi planning smoke ok");

const confirmProc = spawnSidecar();

const confirmRequest = {
  protocolVersion: 1,
  id: "confirm-1",
  method: "astra/confirm",
  params: {
    runId: "confirm-run",
    approvedTaskIds: ["task-1"],
    tasks: [
      {
        id: "task-1",
        title: "Advance stage",
        targetStageId: "stage-1",
        targetAgent: "codex",
        prompt: "do work",
        expectedOutput: "done",
        risk: "low",
      },
    ],
  },
};

confirmProc.stdin.write(`${JSON.stringify(confirmRequest)}\n`);

const reader = confirmProc.stdout.getReader();
const decoder = new TextDecoder();
let confirmBuffer = "";
let confirmOutput = "";
let confirmResponseSeen = false;
let dispatchToolSeen = false;
let stageUpdateToolSeen = false;
let stageUpdateEventSeen = false;
let confirmSnapshotCount = 0;
const deadline = Date.now() + 5000;

while (!confirmResponseSeen && Date.now() < deadline) {
  const { value, done } = await reader.read();
  if (done) break;
  confirmBuffer += decoder.decode(value);
  let newline = confirmBuffer.indexOf("\n");
  while (newline >= 0) {
    const line = confirmBuffer.slice(0, newline).trim();
    confirmBuffer = confirmBuffer.slice(newline + 1);
    if (line) {
      confirmOutput += `${line}\n`;
      const message = JSON.parse(line);
      if (message.method === "tool/call") {
        if (message.params?.name === "sessio.project.snapshot") {
          confirmSnapshotCount += 1;
          confirmProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            result: {
              thread: {
                id: "thread-confirm",
                stages: [
                  { id: "stage-1", status: confirmSnapshotCount === 1 ? "in_progress" : "completed" },
                ],
              },
            },
          })}\n`);
        } else if (message.params?.name === "sessio.agent.dispatch_task") {
          dispatchToolSeen = true;
          confirmProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            result: {
              taskId: "task-1",
              threadStageId: "stage-1",
              sessioRuntimeSessionId: "agent-session-1",
              status: "completed",
              output: "done",
              attemptCount: 1,
              retryLimitReached: false,
            },
          })}\n`);
        } else if (message.params?.name === "sessio.stage.update") {
          stageUpdateToolSeen = true;
          confirmProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            result: {
              ok: true,
              stage: {
                id: "stage-1",
                status: "completed",
                summary: "done",
                outcome: "done",
              },
              error: null,
              appliedAt: Date.now(),
            },
          })}\n`);
        } else {
        confirmProc.stdin.write(`${JSON.stringify({
          protocolVersion: 1,
          id: message.id,
          error: { code: "unexpected_tool", message: `unexpected tool ${message.params?.name}` },
        })}\n`);
        }
      }
      if (message.method === "event" && message.params?.type === "stage_update_result") {
        stageUpdateEventSeen = true;
      }
      if (message.id === "confirm-1" && message.result?.status === "completed") {
        confirmResponseSeen = true;
      }
    }
    newline = confirmBuffer.indexOf("\n");
  }
}

confirmProc.stdin.end();
confirmProc.kill();
await confirmProc.exited;

if (!confirmResponseSeen) {
  throw new Error(`confirm response missing from smoke output: ${confirmOutput}`);
}
if (!dispatchToolSeen || !stageUpdateToolSeen || !stageUpdateEventSeen || confirmSnapshotCount < 2) {
  throw new Error(`confirm loop did not dispatch and update stage: ${confirmOutput}`);
}

console.log("sessio-astra confirm smoke ok");

const retryProc = spawnSidecar();

retryProc.stdin.write(`${JSON.stringify({
  ...confirmRequest,
  id: "retry-1",
  params: {
    ...confirmRequest.params,
    runId: "retry-run",
  },
})}\n`);

const retryReader = retryProc.stdout.getReader();
let retryBuffer = "";
let retryOutput = "";
let retryResponseSeen = false;
let retryIssueToolSeen = false;
let retryStageUpdateEventSeen = false;
let retrySnapshotCount = 0;
const retryDeadline = Date.now() + 5000;

while (!retryResponseSeen && Date.now() < retryDeadline) {
  const { value, done } = await retryReader.read();
  if (done) break;
  retryBuffer += decoder.decode(value);
  let newline = retryBuffer.indexOf("\n");
  while (newline >= 0) {
    const line = retryBuffer.slice(0, newline).trim();
    retryBuffer = retryBuffer.slice(newline + 1);
    if (line) {
      retryOutput += `${line}\n`;
      const message = JSON.parse(line);
      if (message.method === "tool/call") {
        if (message.params?.name === "sessio.project.snapshot") {
          retrySnapshotCount += 1;
          retryProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            result: {
              thread: {
                id: "thread-retry",
                stages: [
                  { id: "stage-1", status: retrySnapshotCount === 1 ? "in_progress" : "skipped" },
                ],
              },
            },
          })}\n`);
        } else if (message.params?.name === "sessio.agent.dispatch_task") {
          retryProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            result: {
              taskId: "task-1",
              threadStageId: "stage-1",
              sessioRuntimeSessionId: "agent-session-retry",
              status: "failed",
              output: "",
              error: "retry limit reached",
              attemptCount: 3,
              retryLimitReached: true,
            },
          })}\n`);
        } else if (message.params?.name === "sessio.stage.issue.add_or_update") {
          retryIssueToolSeen = true;
          retryProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            result: {
              ok: true,
              issue: {
                id: "issue-retry",
                threadStageId: "stage-1",
                title: "Retry limit reached for Advance stage",
              },
              error: null,
              appliedAt: Date.now(),
            },
          })}\n`);
        } else {
          retryProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            error: { code: "unexpected_tool", message: `unexpected tool ${message.params?.name}` },
          })}\n`);
        }
      }
      if (message.method === "event" && message.params?.type === "stage_update_result") {
        retryStageUpdateEventSeen = true;
      }
      if (message.id === "retry-1" && message.result?.status === "completed") {
        retryResponseSeen = true;
      }
    }
    newline = retryBuffer.indexOf("\n");
  }
}

retryProc.stdin.end();
retryProc.kill();
await retryProc.exited;

if (!retryResponseSeen || !retryIssueToolSeen || !retryStageUpdateEventSeen || retrySnapshotCount < 2) {
  throw new Error(`retry-limit loop did not record issue and return: ${retryOutput}`);
}

console.log("sessio-astra retry-limit smoke ok");

const failureIssueProc = spawnSidecar();

failureIssueProc.stdin.write(`${JSON.stringify({
  ...confirmRequest,
  id: "failure-issue-1",
  params: {
    ...confirmRequest.params,
    runId: "failure-issue-run",
  },
})}\n`);

const failureIssueReader = failureIssueProc.stdout.getReader();
let failureIssueBuffer = "";
let failureIssueOutput = "";
let failureIssueResponseSeen = false;
let failureIssueDispatchCount = 0;
let failureIssueToolSeen = false;
let failureIssueSnapshotCount = 0;
const failureIssueDeadline = Date.now() + 5000;

while (!failureIssueResponseSeen && Date.now() < failureIssueDeadline) {
  const { value, done } = await failureIssueReader.read();
  if (done) break;
  failureIssueBuffer += decoder.decode(value);
  let newline = failureIssueBuffer.indexOf("\n");
  while (newline >= 0) {
    const line = failureIssueBuffer.slice(0, newline).trim();
    failureIssueBuffer = failureIssueBuffer.slice(newline + 1);
    if (line) {
      failureIssueOutput += `${line}\n`;
      const message = JSON.parse(line);
      if (message.method === "tool/call") {
        if (message.params?.name === "sessio.project.snapshot") {
          failureIssueSnapshotCount += 1;
          failureIssueProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            result: {
              thread: {
                id: "thread-failure",
                stages: [
                  { id: "stage-1", status: failureIssueSnapshotCount === 1 ? "in_progress" : "skipped" },
                ],
              },
            },
          })}\n`);
        } else if (message.params?.name === "sessio.agent.dispatch_task") {
          failureIssueDispatchCount += 1;
          failureIssueProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            result: {
              taskId: "task-1",
              threadStageId: "stage-1",
              sessioRuntimeSessionId: "agent-session-failure",
              status: "failed",
              output: "needs a different approach",
              error: null,
              attemptCount: 1,
              retryLimitReached: false,
            },
          })}\n`);
        } else if (message.params?.name === "sessio.stage.issue.add_or_update") {
          failureIssueToolSeen = true;
          failureIssueProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            result: {
              ok: true,
              issue: { id: "issue-failure", threadStageId: "stage-1" },
              error: null,
              appliedAt: Date.now(),
            },
          })}\n`);
        } else {
          failureIssueProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            error: { code: "unexpected_tool", message: `unexpected tool ${message.params?.name}` },
          })}\n`);
        }
      }
      if (message.id === "failure-issue-1" && message.result?.status === "completed") {
        failureIssueResponseSeen = true;
      }
    }
    newline = failureIssueBuffer.indexOf("\n");
  }
}

failureIssueProc.stdin.end();
failureIssueProc.kill();
await failureIssueProc.exited;

if (!failureIssueResponseSeen || failureIssueDispatchCount !== 1 || !failureIssueToolSeen || failureIssueSnapshotCount < 2) {
  throw new Error(`failure issue loop did not record issue without redispatch: ${failureIssueOutput}`);
}

console.log("sessio-astra failure-issue smoke ok");

export {};
