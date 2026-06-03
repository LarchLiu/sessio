import type {
  AstraModelConfig,
  AstraPlan,
  AstraTaskProposal,
  ProtocolRequest,
  StartParams,
  ThreadSnapshot,
  StageSnapshot,
} from "./protocol";

type PiBootstrapState = {
  available: boolean;
  detail: string;
  agentAvailable: boolean;
  modelConfigured: boolean;
  apiKeyConfigured: boolean;
  planningMode: "pi-agent" | "deterministic";
  provider?: string;
  modelId?: string;
};

type PiTextBlock = { type: "text"; text: string };
type PiAssistantMessage = {
  role: "assistant";
  content: Array<PiTextBlock | { type: string; [key: string]: unknown }>;
  stopReason?: string;
  errorMessage?: string;
};
type PiAgent = {
  prompt(input: string): Promise<void>;
  abort(): void;
  waitForIdle(): Promise<void>;
  state: {
    messages: Array<PiAssistantMessage | { role: string; [key: string]: unknown }>;
    errorMessage?: string;
  };
};
type PiAgentConstructor = new (options?: {
  initialState?: { systemPrompt?: string; model?: unknown; thinkingLevel?: string };
  getApiKey?: (provider: string) => string | undefined;
  streamFn?: (model: unknown, context: unknown, options?: Record<string, unknown>) => unknown;
  sessionId?: string;
}) => PiAgent;
type FauxProviderRegistration = {
  getModel(modelId?: string): unknown;
  setResponses(responses: unknown[]): void;
  unregister(): void;
};
type PiModules = {
  Agent: PiAgentConstructor;
  getModel(provider: string, modelId: string): unknown;
  parseJsonWithRepair<T>(json: string): T;
  streamSimple(model: unknown, context: unknown, options?: Record<string, unknown>): unknown;
  registerFauxProvider(options?: Record<string, unknown>): FauxProviderRegistration;
  fauxAssistantMessage(content: string, options?: Record<string, unknown>): unknown;
};
type PlannerConfig =
  | { kind: "none" }
  | { kind: "configured"; provider: string; api?: string; baseUrl?: string; modelId: string; apiKey: string; thinkingLevel: string }
  | { kind: "faux"; provider: "faux"; api: "faux"; modelId: string; planJson: string; apiKey: string; thinkingLevel: string };
type PiPlanner = {
  agent: PiAgent;
  provider: string;
  modelId: string;
  apiKeyConfigured: boolean;
  cleanup?: () => void;
};

const ASTRA_SYSTEM_PROMPT = [
  "You are Astra, Sessio's orchestration planner.",
  "Return only a JSON object with shape {\"summary\": string, \"tasks\": AstraTaskProposal[]}.",
  "Each task must include id, title, targetStageId, targetAgent, prompt, expectedOutput, and risk.",
  "Use only targetStageId values present in the supplied thread stages, or null for thread-level work.",
  "Use only targetAgent values codex, claude, or gemini, and prefer agents assigned to the target stage.",
  "Create delegatable work only. Do not claim stage mutations have already happened.",
].join("\n");

const ASTRA_PLAN_TIMEOUT_MS = 30000;

let piModulesPromise: Promise<PiModules | { error: string }> | null = null;

export async function bootstrapPi(): Promise<PiBootstrapState> {
  const modules = await loadPiModules();
  if ("error" in modules) {
    return {
      available: false,
      agentAvailable: false,
      modelConfigured: false,
      apiKeyConfigured: false,
      planningMode: "deterministic",
      detail: modules.error,
    };
  }

  return {
    available: true,
    agentAvailable: true,
    modelConfigured: false,
    apiKeyConfigured: false,
    planningMode: "deterministic",
    detail: "Pi packages loaded; Astra model configuration is supplied per request",
  };
}

export async function createPlan(params: StartParams): Promise<AstraPlan> {
  const fallback = deterministicPlan(params);
  const modules = await loadPiModules();
  if ("error" in modules) return fallback;

  const config = readPlannerConfig(params.modelConfig);
  if (config.kind === "none") return fallback;

  let planner: PiPlanner | null = null;
  try {
    planner = createPiPlanner(modules, config);
    return await createPiAgentPlan(modules, planner, params, fallback);
  } catch (error) {
    console.error(`[astra] Pi planning failed, using deterministic planner: ${errorMessage(error)}`);
    return fallback;
  } finally {
    planner?.cleanup?.();
  }
}

export async function resolveModelSmoke(): Promise<PiBootstrapState> {
  return bootstrapPi();
}

async function loadPiModules(): Promise<PiModules | { error: string }> {
  piModulesPromise ??= (async () => {
    try {
      const [piAi, piAgentCore] = await Promise.all([
        import("@earendil-works/pi-ai"),
        import("@earendil-works/pi-agent-core"),
      ]);
      return {
        Agent: piAgentCore.Agent as unknown as PiAgentConstructor,
        getModel: piAi.getModel as unknown as PiModules["getModel"],
        parseJsonWithRepair: piAi.parseJsonWithRepair as unknown as PiModules["parseJsonWithRepair"],
        streamSimple: piAi.streamSimple as unknown as PiModules["streamSimple"],
        registerFauxProvider: piAi.registerFauxProvider as unknown as PiModules["registerFauxProvider"],
        fauxAssistantMessage: piAi.fauxAssistantMessage as unknown as PiModules["fauxAssistantMessage"],
      };
    } catch (error) {
      return { error: errorMessage(error) };
    }
  })();
  return piModulesPromise;
}

function readPlannerConfig(rawConfig: AstraModelConfig | null | undefined): PlannerConfig {
  const provider = normalizeConfigText(rawConfig?.provider);
  const api = normalizeConfigText(rawConfig?.api);
  const modelId = normalizeConfigText(rawConfig?.modelId);
  const apiKey = normalizeConfigText(rawConfig?.apiKey);
  const baseUrl = normalizeConfigText(rawConfig?.baseUrl);
  const fauxPlanJson = normalizeConfigText(rawConfig?.fauxPlanJson);
  const thinkingLevel = normalizeThinkingLevel(rawConfig?.thinkingLevel);

  if (provider === "faux" && api === "faux" && fauxPlanJson) {
    return {
      kind: "faux",
      provider: "faux",
      api: "faux",
      modelId: modelId || "sessio-astra-faux",
      apiKey: apiKey || "faux",
      planJson: fauxPlanJson,
      thinkingLevel,
    };
  }

  if (!provider || !modelId) return { kind: "none" };
  if (!apiKey) return { kind: "none" };
  return { kind: "configured", provider, api, baseUrl, modelId, apiKey, thinkingLevel };
}

function normalizeConfigText(value: string | null | undefined): string | undefined {
  if (typeof value !== "string") return undefined;
  const text = value.trim();
  return text ? text : undefined;
}

function normalizeThinkingLevel(value: string | null | undefined): string {
  const text = normalizeConfigText(value);
  return ["off", "minimal", "low", "medium", "high", "xhigh"].includes(text ?? "") ? text! : "off";
}

function createPiPlanner(modules: PiModules, config: PlannerConfig): PiPlanner {
  if (config.kind === "none") {
    throw new Error("Astra Pi model is not configured");
  }

  if (config.kind === "faux") {
    const registration = modules.registerFauxProvider({
      provider: config.provider,
      models: [{ id: config.modelId, name: "Sessio Astra Faux Planner", reasoning: false }],
    });
    registration.setResponses([modules.fauxAssistantMessage(config.planJson)]);
    return {
      agent: createPlanningAgent(modules, registration.getModel(config.modelId), config),
      provider: config.provider,
      modelId: config.modelId,
      apiKeyConfigured: true,
      cleanup: () => registration.unregister(),
    };
  }

  const model = config.api || config.baseUrl
    ? customModel(config)
    : modules.getModel(config.provider, config.modelId);
  return {
    agent: createPlanningAgent(modules, model, config),
    provider: config.provider,
    modelId: config.modelId,
    apiKeyConfigured: Boolean(config.apiKey),
  };
}

function createPlanningAgent(modules: PiModules, model: unknown, config: Exclude<PlannerConfig, { kind: "none" }>): PiAgent {
  return new modules.Agent({
    initialState: {
      systemPrompt: ASTRA_SYSTEM_PROMPT,
      model,
      thinkingLevel: config.thinkingLevel,
    },
    streamFn: (nextModel, context, options = {}) =>
      modules.streamSimple(nextModel, context, {
        ...options,
        apiKey: config.apiKey,
      }),
  });
}

function customModel(config: Extract<PlannerConfig, { kind: "configured" }>): Record<string, unknown> {
  return {
    id: config.modelId,
    name: config.modelId,
    api: config.api || "openai-responses",
    provider: config.provider,
    baseUrl: config.baseUrl || "https://api.openai.com/v1",
    reasoning: config.thinkingLevel !== "off",
    input: ["text"],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: 128000,
    maxTokens: 4096,
  };
}

async function createPiAgentPlan(
  modules: PiModules,
  planner: PiPlanner,
  params: StartParams,
  fallback: AstraPlan,
): Promise<AstraPlan> {
  const timeout = setTimeout(() => planner.agent.abort(), ASTRA_PLAN_TIMEOUT_MS);
  try {
    await planner.agent.prompt(buildPlanningPrompt(params, fallback));
    await planner.agent.waitForIdle();
  } finally {
    clearTimeout(timeout);
  }

  const assistant = latestAssistantMessage(planner.agent);
  if (!assistant) {
    throw new Error("Pi Agent did not return a planning message");
  }
  if (assistant.stopReason === "error" || assistant.stopReason === "aborted") {
    throw new Error(assistant.errorMessage || planner.agent.state.errorMessage || `Pi Agent stopped with ${assistant.stopReason}`);
  }

  const text = assistant.content
    .filter((block): block is PiTextBlock => block.type === "text" && typeof (block as PiTextBlock).text === "string")
    .map((block) => block.text)
    .join("\n")
    .trim();
  if (!text) {
    throw new Error("Pi Agent returned no plan text");
  }

  const rawPlan = modules.parseJsonWithRepair<unknown>(extractJsonObject(text));
  return sanitizePiPlan(rawPlan, params, fallback);
}

function buildPlanningPrompt(params: StartParams, fallback: AstraPlan): string {
  return [
    "Create the next Astra orchestration plan for this Sessio thread.",
    "Planning input JSON:",
    stringifyForPrompt({
      runId: params.runId,
      userInstruction: params.prompt ?? null,
      thread: {
        id: params.thread.id,
        projectId: params.thread.projectId,
        goal: params.thread.goal,
        description: params.thread.description ?? null,
        stages: (params.thread.stages ?? []).map((stage) => ({
          id: stage.id,
          stageId: stage.stageId ?? null,
          name: stage.name ?? null,
          description: stage.description ?? null,
          status: stage.status ?? null,
          order: stage.order ?? null,
          agents: (stage.assistants ?? [])
            .map((assistant) => assistant.agent?.id)
            .filter((id): id is string => id === "codex" || id === "claude" || id === "gemini"),
          issueCount: (stage.issues ?? []).length,
        })),
      },
      snapshot: params.snapshot ?? null,
      deterministicDraft: fallback,
    }),
  ].join("\n\n");
}

function sanitizePiPlan(rawPlan: unknown, params: StartParams, fallback: AstraPlan): AstraPlan {
  if (!rawPlan || typeof rawPlan !== "object") {
    throw new Error("Pi plan is not an object");
  }
  const raw = rawPlan as { summary?: unknown; tasks?: unknown };
  if (!Array.isArray(raw.tasks)) {
    throw new Error("Pi plan tasks must be an array");
  }

  const stageIndex = buildStageIndex(params.thread);
  const tasks: AstraTaskProposal[] = [];
  const seenIds = new Set<string>();

  for (const item of raw.tasks.slice(0, 20)) {
    if (!item || typeof item !== "object") continue;
    const task = item as Partial<Record<keyof AstraTaskProposal, unknown>>;
    const targetStageId = normalizeTargetStageId(task.targetStageId, stageIndex);
    const targetStage = targetStageId ? stageIndex.get(targetStageId) ?? null : null;
    const targetAgent = normalizeTargetAgent(task.targetAgent, targetStage);
    const title = normalizeText(task.title, 160);
    const prompt = normalizeText(task.prompt, 8000);
    const expectedOutput = normalizeText(task.expectedOutput, 1000);
    if (!title || !targetAgent || !prompt || !expectedOutput) continue;

    const id = uniqueTaskId(
      taskId(params.thread.id, targetStageId ?? "thread", hash(`${title}:${prompt}:${targetAgent}`), tasks.length),
      seenIds,
    );
    tasks.push({
      id,
      title,
      targetStageId,
      targetAgent,
      prompt,
      expectedOutput,
      risk: normalizeRisk(task.risk, targetStage),
    });
  }

  if ((raw.tasks.length > 0 || fallback.tasks.length > 0) && tasks.length === 0) {
    throw new Error("Pi plan did not contain any valid tasks");
  }

  return {
    summary: normalizeText(raw.summary, 500) || fallback.summary,
    tasks,
  };
}

function buildStageIndex(thread: ThreadSnapshot): Map<string, StageSnapshot> {
  const index = new Map<string, StageSnapshot>();
  for (const stage of thread.stages ?? []) {
    index.set(stage.id, stage);
    if (stage.stageId) index.set(stage.stageId, stage);
  }
  return index;
}

function normalizeTargetStageId(
  value: unknown,
  stageIndex: Map<string, StageSnapshot>,
): string | null {
  if (value === null || value === undefined || value === "") return null;
  if (typeof value !== "string") return null;
  const stage = stageIndex.get(value);
  return stage?.id ?? null;
}

function normalizeTargetAgent(
  value: unknown,
  stage: StageSnapshot | null,
): "codex" | "claude" | "gemini" | null {
  const requested = value === "codex" || value === "claude" || value === "gemini" ? value : null;
  if (!stage) return requested ?? "codex";
  const allowed = stage.assistants
    ?.map((assistant) => assistant.agent?.id)
    .filter((id): id is "codex" | "claude" | "gemini" => id === "codex" || id === "claude" || id === "gemini") ?? [];
  if (requested && allowed.includes(requested)) return requested;
  return pickAgent(stage);
}

function normalizeRisk(value: unknown, stage: StageSnapshot | null): "low" | "medium" | "high" {
  if (value === "low" || value === "medium" || value === "high") return value;
  return stage ? stageRisk(stage) : "medium";
}

function normalizeText(value: unknown, maxLength: number): string | null {
  if (typeof value !== "string") return null;
  const text = value.trim();
  if (!text) return null;
  return text.length > maxLength ? `${text.slice(0, maxLength - 3)}...` : text;
}

function uniqueTaskId(id: string, seenIds: Set<string>): string {
  let candidate = id;
  let index = 1;
  while (seenIds.has(candidate)) {
    candidate = `${id}-${index}`;
    index += 1;
  }
  seenIds.add(candidate);
  return candidate;
}

function latestAssistantMessage(agent: PiAgent): PiAssistantMessage | null {
  for (let index = agent.state.messages.length - 1; index >= 0; index -= 1) {
    const message = agent.state.messages[index];
    if (message.role === "assistant" && Array.isArray((message as PiAssistantMessage).content)) {
      return message as PiAssistantMessage;
    }
  }
  return null;
}

function extractJsonObject(text: string): string {
  const fenced = text.match(/```(?:json)?\s*([\s\S]*?)```/i);
  const candidate = fenced?.[1]?.trim() ?? text.trim();
  const start = candidate.indexOf("{");
  const end = candidate.lastIndexOf("}");
  if (start < 0 || end < start) {
    throw new Error("Pi plan did not include a JSON object");
  }
  return candidate.slice(start, end + 1);
}

function stringifyForPrompt(value: unknown, maxLength = 24000): string {
  const text = JSON.stringify(value, null, 2);
  if (text.length <= maxLength) return text;
  return `${text.slice(0, maxLength)}\n...truncated`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
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
    modelConfig: isModelConfig(params.modelConfig) ? params.modelConfig : null,
  };
}

function isModelConfig(value: unknown): value is AstraModelConfig {
  return !value || typeof value === "object";
}
