import type { Agent, ThreadWorkSnapshot, ThreadWorkSnapshotSessionRef } from "../../api";
import type { LiveRuntimeSession, LiveRuntimeState, LiveTurn } from "../../runtimeChat";

export interface WorkflowOverlayStage {
  active: boolean;
  status: "in_progress";
  activeAssistantIds: string[];
  currentAction: string | null;
  updatedAt: number;
}

export interface WorkflowOverlay {
  stages: Record<string, WorkflowOverlayStage>;
  currentAction: string | null;
  activeCount: number;
  updatedAt: number;
}

export interface WorkflowOverlayStore {
  get(blockId: string): { overlay: WorkflowOverlay; revision: number } | null;
  set(blockId: string, overlay: WorkflowOverlay): void;
  delete(blockId: string): void;
  clear(): void;
  keys(): string[];
  subscribe(blockId: string, fn: () => void): () => void;
}

export interface WorkflowOverlayCardContext {
  blockId: string;
  threadId: string;
  threadStageId: string | null;
  snapshot: ThreadWorkSnapshot;
}

export interface SessionThreadStageMap {
  bySessioRuntimeId: Map<string, {
    agent: Agent;
    childSessionId: string;
    threadId: string | null;
    stageId: string | null;
    assistantId: string | null;
  }>;
  blockIdsByThread: Map<string, Set<string>>;
  cardsByBlockId: Map<string, WorkflowOverlayCardContext>;
}

type WorkflowSessionRoute = SessionThreadStageMap["bySessioRuntimeId"] extends Map<string, infer T> ? T : never;

export function createWorkflowOverlayStore(): WorkflowOverlayStore {
  const values = new Map<string, { overlay: WorkflowOverlay; revision: number }>();
  const listeners = new Map<string, Set<() => void>>();
  const notify = (blockId: string) => {
    for (const listener of listeners.get(blockId) ?? []) {
      listener();
    }
  };
  return {
    get(blockId) {
      return values.get(blockId) ?? null;
    },
    set(blockId, overlay) {
      const current = values.get(blockId);
      if (current && workflowOverlayEquals(current.overlay, overlay)) return;
      values.set(blockId, {
        overlay,
        revision: (current?.revision ?? 0) + 1,
      });
      notify(blockId);
    },
    delete(blockId) {
      if (!values.delete(blockId)) return;
      notify(blockId);
    },
    clear() {
      if (values.size === 0) return;
      const blockIds = new Set([...values.keys(), ...listeners.keys()]);
      values.clear();
      for (const blockId of blockIds) {
        notify(blockId);
      }
    },
    keys() {
      return [...values.keys()];
    },
    subscribe(blockId, fn) {
      const next = listeners.get(blockId) ?? new Set<() => void>();
      next.add(fn);
      listeners.set(blockId, next);
      return () => {
        const current = listeners.get(blockId);
        if (!current) return;
        current.delete(fn);
        if (current.size === 0) listeners.delete(blockId);
      };
    },
  };
}

export function createWorkflowOverlayCardContext({
  blockId,
  threadId,
  threadStageId,
  workflowSnapshotJson,
}: {
  blockId: string;
  threadId: string | null | undefined;
  threadStageId: string | null | undefined;
  workflowSnapshotJson: string | null | undefined;
}): WorkflowOverlayCardContext | null {
  const snapshot = parseThreadWorkSnapshot(workflowSnapshotJson);
  if (!snapshot) return null;
  const resolvedThreadId = threadId?.trim() || snapshot.threadId.trim();
  if (!resolvedThreadId) return null;
  return {
    blockId,
    threadId: resolvedThreadId,
    threadStageId: threadStageId?.trim() || null,
    snapshot,
  };
}

export function buildSessionThreadStageMap(
  cards: WorkflowOverlayCardContext[],
  runtimeSessionAliases: Record<string, string>,
): SessionThreadStageMap {
  const bySessioRuntimeId = new Map<string, WorkflowSessionRoute>();
  const blockIdsByThread = new Map<string, Set<string>>();
  const cardsByBlockId = new Map<string, WorkflowOverlayCardContext>();

  for (const card of cards) {
    cardsByBlockId.set(card.blockId, card);
    const blockIds = blockIdsByThread.get(card.threadId) ?? new Set<string>();
    blockIds.add(card.blockId);
    blockIdsByThread.set(card.threadId, blockIds);

    for (const route of snapshotSessionRoutes(card.snapshot)) {
      const identity = sessionIdentity(route.agent, route.childSessionId);
      const runtimeId = runtimeSessionAliases[identity] ?? route.childSessionId;
      if (!runtimeId) continue;
      if (bySessioRuntimeId.has(runtimeId)) continue;
      bySessioRuntimeId.set(runtimeId, route);
    }
  }

  return {
    bySessioRuntimeId,
    blockIdsByThread,
    cardsByBlockId,
  };
}

export function projectWorkflowLiveOverlays({
  cards,
  runtimeSessionAliases,
  liveState,
}: {
  cards: WorkflowOverlayCardContext[];
  runtimeSessionAliases: Record<string, string>;
  liveState: LiveRuntimeState;
}): Map<string, WorkflowOverlay> {
  const sessionMap = buildSessionThreadStageMap(cards, runtimeSessionAliases);
  const overlays = new Map<string, WorkflowOverlay>();
  for (const liveSession of Object.values(liveState.sessions)) {
    const route = liveSessionWorkflowRoute(liveSession, sessionMap);
    if (!route?.threadId) continue;
    const activeTurns = liveSession.turns.filter(isActiveTurn);
    if (activeTurns.length === 0) continue;
    const latestTurn = activeTurns.reduce((latest, turn) =>
      turn.updatedAt >= latest.updatedAt ? turn : latest,
    );
    const action = liveTurnAction(latestTurn);
    const blockIds = sessionMap.blockIdsByThread.get(route.threadId);
    if (!blockIds) continue;

    for (const blockId of blockIds) {
      const card = sessionMap.cardsByBlockId.get(blockId);
      if (!card) continue;
      if (card.threadStageId && route.stageId !== card.threadStageId) continue;
      const overlay = overlays.get(blockId) ?? emptyOverlay();
      overlay.activeCount += 1;
      overlay.updatedAt = Math.max(overlay.updatedAt, latestTurn.updatedAt);
      overlay.currentAction = latestAction(overlay.currentAction, overlay.updatedAt, action, latestTurn.updatedAt);
      if (route.stageId) {
        const currentStage = overlay.stages[route.stageId];
        overlay.stages[route.stageId] = mergeStageOverlay(currentStage, {
          active: true,
          status: "in_progress",
          activeAssistantIds: route.assistantId ? [route.assistantId] : [],
          currentAction: action,
          updatedAt: latestTurn.updatedAt,
        });
      }
      overlays.set(blockId, overlay);
    }
  }
  return overlays;
}

function liveSessionWorkflowRoute(
  liveSession: LiveRuntimeSession,
  sessionMap: SessionThreadStageMap,
): WorkflowSessionRoute | null {
  const explicitRoute = sessionMap.bySessioRuntimeId.get(liveSession.sessioRuntimeSessionId)
    ?? sessionMap.bySessioRuntimeId.get(liveSession.agentRuntimeSessionId);
  if (explicitRoute) return explicitRoute;

  const metadata = liveSession.metadata ?? {};
  const threadId = stringMeta(metadata, "astraThreadId") ?? stringMeta(metadata, "threadId");
  if (!threadId || !sessionMap.blockIdsByThread.has(threadId)) return null;
  if (stringMeta(metadata, "astraPurpose") !== "orchestration" && !(metadata.astraInternal && stringMeta(metadata, "astraRunId"))) {
    return null;
  }
  return {
    agent: liveSession.agent,
    childSessionId: liveSession.agentRuntimeSessionId || liveSession.sessioRuntimeSessionId,
    threadId,
    stageId: null,
    assistantId: null,
  };
}

function parseThreadWorkSnapshot(workflowSnapshotJson: string | null | undefined): ThreadWorkSnapshot | null {
  const trimmed = workflowSnapshotJson?.trim();
  if (!trimmed) return null;
  try {
    const value = JSON.parse(trimmed);
    if (!value || typeof value !== "object" || Array.isArray(value)) return null;
    const snapshot = value as Partial<ThreadWorkSnapshot>;
    return typeof snapshot.threadId === "string" ? snapshot as ThreadWorkSnapshot : null;
  } catch {
    return null;
  }
}

function snapshotSessionRoutes(snapshot: ThreadWorkSnapshot): WorkflowSessionRoute[] {
  const byIdentity = new Map<string, WorkflowSessionRoute>();
  const addRoute = (
    ref: ThreadWorkSnapshotSessionRef,
    stageId: string | null,
    assistantId: string | null,
  ) => {
    const identity = sessionIdentity(ref.agent, ref.sessionId);
    if (byIdentity.has(identity)) return;
    byIdentity.set(identity, {
      agent: ref.agent,
      childSessionId: ref.sessionId,
      threadId: snapshot.threadId,
      stageId,
      assistantId,
    });
  };

  for (const stage of snapshot.stages ?? []) {
    for (const ref of stage.sessionRefs ?? []) {
      addRoute(ref, stage.threadStageId, assistantIdForStageAgent(snapshot, stage.threadStageId, ref.agent));
    }
  }
  for (const ref of snapshot.threadSessionRefs ?? []) {
    addRoute(ref, null, null);
  }
  for (const ref of snapshot.detailRefs?.sessionRefs ?? []) {
    addRoute(ref, null, null);
  }
  return [...byIdentity.values()];
}

function assistantIdForStageAgent(
  snapshot: ThreadWorkSnapshot,
  stageId: string,
  agent: Agent,
): string | null {
  const stage = snapshot.stages?.find((item) => item.threadStageId === stageId);
  const matches = (stage?.assistants ?? []).filter((assistant) => assistant.agent.id === agent);
  return matches.length === 1 ? matches[0].assistantId : null;
}

function emptyOverlay(): WorkflowOverlay {
  return {
    stages: {},
    currentAction: null,
    activeCount: 0,
    updatedAt: 0,
  };
}

function mergeStageOverlay(
  current: WorkflowOverlayStage | undefined,
  next: WorkflowOverlayStage,
): WorkflowOverlayStage {
  if (!current || next.updatedAt >= current.updatedAt) {
    return {
      ...next,
      activeAssistantIds: dedupeStrings([
        ...(current?.activeAssistantIds ?? []),
        ...next.activeAssistantIds,
      ]),
    };
  }
  return {
    ...current,
    activeAssistantIds: dedupeStrings([
      ...current.activeAssistantIds,
      ...next.activeAssistantIds,
    ]),
  };
}

function latestAction(
  currentAction: string | null,
  currentUpdatedAt: number,
  nextAction: string | null,
  nextUpdatedAt: number,
) {
  if (!nextAction) return currentAction;
  return nextUpdatedAt >= currentUpdatedAt ? nextAction : currentAction;
}

function isActiveTurn(turn: LiveTurn): boolean {
  return turn.status === "pending" || turn.status === "streaming" || turn.status === "cancelling";
}

function liveTurnAction(turn: LiveTurn): string {
  const permission = turn.permissions.find((item) =>
    item.options.length > 0 && !item.selectedOptionId && !item.cancelled
  );
  if (permission) return `Waiting for ${permission.toolName}`;
  const tool = [...turn.tools].reverse().find((item) => item.title.trim());
  if (tool) return tool.title.trim();
  const text = latestTurnText(turn);
  if (text) return text;
  return turn.status === "cancelling" ? "Cancelling" : "Running";
}

function latestTurnText(turn: LiveTurn): string | null {
  for (const block of [...turn.blocks].reverse()) {
    if (block.kind !== "assistant" && block.kind !== "thought") continue;
    for (const content of [...block.blocks].reverse()) {
      if (content.type !== "text") continue;
      const text = content.text.trim().replace(/\s+/g, " ");
      if (text) return text.length > 80 ? `${text.slice(0, 77)}...` : text;
    }
  }
  return null;
}

function dedupeStrings(values: string[]): string[] {
  return [...new Set(values.filter(Boolean))];
}

function workflowOverlayEquals(a: WorkflowOverlay, b: WorkflowOverlay): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

function stringMeta(metadata: Record<string, unknown>, key: string): string | null {
  const value = metadata[key];
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function sessionIdentity(agent: Agent, sessionId: string): string {
  return `${agent}:${sessionId}`;
}
