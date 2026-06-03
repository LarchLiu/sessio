import type {
  AstraPlan,
  AstraTaskProposal,
  ProtocolRequest,
  StartParams,
  ThreadSnapshot,
} from "./protocol";

type PiBootstrapState = {
  available: boolean;
  detail: string;
};

let piStatePromise: Promise<PiBootstrapState> | null = null;

export async function bootstrapPi(): Promise<PiBootstrapState> {
  piStatePromise ??= (async () => {
    try {
      await Promise.all([
        import("@earendil-works/pi-ai"),
        import("@earendil-works/pi-agent-core"),
      ]);
      return { available: true, detail: "Pi packages loaded" };
    } catch (error) {
      return {
        available: false,
        detail: error instanceof Error ? error.message : String(error),
      };
    }
  })();
  return piStatePromise;
}

export async function createPlan(params: StartParams): Promise<AstraPlan> {
  await bootstrapPi();
  return deterministicPlan(params);
}

export async function resolveModelSmoke(): Promise<PiBootstrapState> {
  return bootstrapPi();
}

function deterministicPlan(params: StartParams): AstraPlan {
  const stages = (params.thread.stages ?? []).slice().sort((a, b) => (a.order ?? 0) - (b.order ?? 0));
  const tasks: AstraTaskProposal[] = [];
  const pendingStages = stages.filter((stage) => stage.status !== "completed" && stage.status !== "skipped");

  for (const stage of pendingStages) {
    const blocked = stage.status === "blocked";
    const targetAgent = pickAgent(stage);
    if (!targetAgent) continue;
    tasks.push({
      id: taskId(params.thread.id, stage.id, blocked ? "unblock" : "advance", tasks.length),
      title: `${blocked ? "Unblock" : "Advance"} ${stageLabel(stage)}`,
      targetStageId: stage.id,
      targetAgent,
      prompt: buildPrompt(
        params.thread,
        stage,
        params.prompt,
        blocked
          ? "Identify the blocker, propose the smallest next action, and return concrete recovery steps."
          : "Work on this stage goal and return a concise implementation or research result with verification notes.",
      ),
      expectedOutput: blocked
        ? "A short blocker diagnosis, recommended next action, and any stage issue updates needed."
        : "Stage progress with files, decisions, or verification steps clearly summarized.",
      risk: stageRisk(stage),
    });
  }

  if (tasks.length === 0 && pendingStages.length === 0) {
    tasks.push({
      id: taskId(params.thread.id, "thread", "survey", 0),
      title: "Survey thread state",
      targetStageId: null,
      targetAgent: "codex",
      prompt: `Review the thread "${params.thread.goal}" and propose the next useful work item. ${params.prompt ?? ""}`.trim(),
      expectedOutput: "A prioritized next-step recommendation grounded in the thread state.",
      risk: "low",
    });
  }

  return {
    summary: `Astra found ${tasks.length} task${tasks.length === 1 ? "" : "s"} for "${params.thread.goal}".`,
    tasks,
  };
}

function buildPrompt(
  _thread: ThreadSnapshot,
  _stage: { id: string; name?: string | null; description?: string | null },
  userPrompt: string | null | undefined,
  instruction: string,
): string {
  const parts = [
    userPrompt ? `User orchestration instruction: ${userPrompt}` : null,
    instruction,
  ].filter(Boolean);
  return parts.join("\n\n");
}

function stageLabel(stage: { id: string; name?: string | null }): string {
  return stage.name?.trim() || stage.id;
}

function stageRisk(stage: { status?: string; issues?: unknown[] }): "low" | "medium" | "high" {
  if (stage.status === "blocked") return "high";
  if ((stage.issues ?? []).length > 0) return "medium";
  return "low";
}

function pickAgent(
  stage: { assistants?: { agent?: { id?: string } }[] },
): "codex" | "claude" | "gemini" | null {
  const id = stage.assistants?.map((assistant) => assistant.agent?.id).find(Boolean);
  return id === "claude" || id === "gemini" || id === "codex" ? id : null;
}

function taskId(threadId: string, stageId: string, kind: string, index: number): string {
  return `task-${hash(`${threadId}:${stageId}:${kind}:${index}`).slice(0, 12)}`;
}

function hash(input: string): string {
  let value = 2166136261;
  for (let i = 0; i < input.length; i += 1) {
    value ^= input.charCodeAt(i);
    value = Math.imul(value, 16777619);
  }
  return (value >>> 0).toString(16).padStart(8, "0");
}

export function assertStartParams(request: ProtocolRequest): StartParams {
  const params = request.params as Partial<StartParams> | undefined;
  if (!params || typeof params.runId !== "string" || !params.runId.trim()) {
    throw new Error("runId is required");
  }
  if (!params.thread || typeof params.thread.id !== "string") {
    throw new Error("thread is required");
  }
  return {
    runId: params.runId,
    thread: params.thread,
    snapshot: params.snapshot,
    prompt: typeof params.prompt === "string" ? params.prompt : null,
  };
}
