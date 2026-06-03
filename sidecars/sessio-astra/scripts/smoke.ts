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
        confirmProc.stdin.write(`${JSON.stringify({
          protocolVersion: 1,
          id: message.id,
          result: {
            taskId: "task-1",
            threadStageId: "stage-1",
            sessioRuntimeSessionId: "runtime-1",
            status: "completed",
            output: "done",
            attemptCount: 1,
            retryLimitReached: false,
          },
        })}\n`);
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

console.log("sessio-astra confirm smoke ok");

export {};
