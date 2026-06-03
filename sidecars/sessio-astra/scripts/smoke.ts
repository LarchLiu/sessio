const proc = Bun.spawn(["bun", "run", "src/main.ts", "--stdio"], {
  stdin: "pipe",
  stdout: "pipe",
  stderr: "pipe",
});

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

const confirmProc = Bun.spawn(["bun", "run", "src/main.ts", "--stdio"], {
  stdin: "pipe",
  stdout: "pipe",
  stderr: "pipe",
});

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
        if (message.params?.name === "sessio.agent.dispatch_task") {
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
if (!dispatchToolSeen || !stageUpdateToolSeen || !stageUpdateEventSeen) {
  throw new Error(`confirm loop did not dispatch and update stage: ${confirmOutput}`);
}

console.log("sessio-astra confirm smoke ok");

const retryProc = Bun.spawn(["bun", "run", "src/main.ts", "--stdio"], {
  stdin: "pipe",
  stdout: "pipe",
  stderr: "pipe",
});

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
        if (message.params?.name === "sessio.agent.dispatch_task") {
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

if (!retryResponseSeen || !retryIssueToolSeen || !retryStageUpdateEventSeen) {
  throw new Error(`retry-limit loop did not record issue and return: ${retryOutput}`);
}

console.log("sessio-astra retry-limit smoke ok");

const retrySuccessProc = Bun.spawn(["bun", "run", "src/main.ts", "--stdio"], {
  stdin: "pipe",
  stdout: "pipe",
  stderr: "pipe",
});

retrySuccessProc.stdin.write(`${JSON.stringify({
  ...confirmRequest,
  id: "retry-success-1",
  params: {
    ...confirmRequest.params,
    runId: "retry-success-run",
  },
})}\n`);

const retrySuccessReader = retrySuccessProc.stdout.getReader();
let retrySuccessBuffer = "";
let retrySuccessOutput = "";
let retrySuccessResponseSeen = false;
let retrySuccessDispatchCount = 0;
let retrySuccessStageUpdateSeen = false;
const retrySuccessDeadline = Date.now() + 5000;

while (!retrySuccessResponseSeen && Date.now() < retrySuccessDeadline) {
  const { value, done } = await retrySuccessReader.read();
  if (done) break;
  retrySuccessBuffer += decoder.decode(value);
  let newline = retrySuccessBuffer.indexOf("\n");
  while (newline >= 0) {
    const line = retrySuccessBuffer.slice(0, newline).trim();
    retrySuccessBuffer = retrySuccessBuffer.slice(newline + 1);
    if (line) {
      retrySuccessOutput += `${line}\n`;
      const message = JSON.parse(line);
      if (message.method === "tool/call") {
        if (message.params?.name === "sessio.agent.dispatch_task") {
          retrySuccessDispatchCount += 1;
          retrySuccessProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            result: retrySuccessDispatchCount === 1
              ? {
                  taskId: "task-1",
                  threadStageId: "stage-1",
                  sessioRuntimeSessionId: "agent-session-retry-first",
                  status: "failed",
                  output: "needs another pass",
                  error: null,
                  attemptCount: 1,
                  retryLimitReached: false,
                }
              : {
                  taskId: "task-1",
                  threadStageId: "stage-1",
                  sessioRuntimeSessionId: "agent-session-retry-second",
                  status: "completed",
                  output: "done after retry",
                  attemptCount: 2,
                  retryLimitReached: false,
                },
          })}\n`);
        } else if (message.params?.name === "sessio.stage.update") {
          retrySuccessStageUpdateSeen = true;
          retrySuccessProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            result: {
              ok: true,
              stage: { id: "stage-1", status: "completed" },
              error: null,
              appliedAt: Date.now(),
            },
          })}\n`);
        } else {
          retrySuccessProc.stdin.write(`${JSON.stringify({
            protocolVersion: 1,
            id: message.id,
            error: { code: "unexpected_tool", message: `unexpected tool ${message.params?.name}` },
          })}\n`);
        }
      }
      if (message.id === "retry-success-1" && message.result?.status === "completed") {
        retrySuccessResponseSeen = true;
      }
    }
    newline = retrySuccessBuffer.indexOf("\n");
  }
}

retrySuccessProc.stdin.end();
retrySuccessProc.kill();
await retrySuccessProc.exited;

if (!retrySuccessResponseSeen || retrySuccessDispatchCount !== 2 || !retrySuccessStageUpdateSeen) {
  throw new Error(`retry success loop did not redispatch and update: ${retrySuccessOutput}`);
}

console.log("sessio-astra retry-success smoke ok");

export {};
