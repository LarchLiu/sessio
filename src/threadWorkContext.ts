import {
  getSessionHistory,
  type SessionHistorySnapshotGroup,
  type SessionInfo,
  type ThreadWorkSnapshot,
  type ThreadWorkSnapshotSessionRef,
} from "./api";
import { sessionDisplayTitle } from "./appUtils";

export function withThreadChatSessions(
  snapshot: ThreadWorkSnapshot,
  sessions: SessionInfo[],
): ThreadWorkSnapshot {
  const sessionRefs = sessions.map(threadSessionRef);
  const threadSessionRefs = dedupeSnapshotSessionRefs([
    ...(snapshot.threadSessionRefs ?? []),
    ...sessionRefs,
  ]);
  const existingDetailRefs = snapshot.detailRefs?.sessionRefs ?? [];
  const sourceRefs = dedupeSnapshotSessionRefs([...existingDetailRefs, ...threadSessionRefs]);
  return {
    ...snapshot,
    threadSessionRefs,
    relatedContext: {
      ...snapshot.relatedContext,
      sessionExcerptRefs: sourceRefs,
    },
    detailRefs: {
      threadId: snapshot.threadId,
      focusedStageId: snapshot.focusedStageId ?? null,
      stageIds: snapshot.detailRefs?.stageIds ?? snapshot.stages?.map((stage) => stage.threadStageId) ?? [],
      issueIds: snapshot.detailRefs?.issueIds ?? snapshot.stages?.flatMap((stage) => (stage.issues ?? []).map((issue) => issue.id)) ?? [],
      sessionRefs: sourceRefs,
    },
  };
}

export async function collectThreadHistorySnapshots(snapshot: ThreadWorkSnapshot): Promise<{
  snapshot: ThreadWorkSnapshot;
  historySnapshots: SessionHistorySnapshotGroup[];
}> {
  const sessionRefs = dedupeSnapshotSessionRefs(snapshot.detailRefs?.sessionRefs ?? []);
  const loadedRefs: ThreadWorkSnapshotSessionRef[] = [];
  const historySnapshots: SessionHistorySnapshotGroup[] = [];
  for (const ref of sessionRefs) {
    const filePath = ref.filePath ?? "";
    if (!filePath) {
      loadedRefs.push(ref);
      continue;
    }
    try {
      const result = await getSessionHistory(ref.agent, filePath, ref.sessionId);
      const ancestorIndex = historySnapshots.length;
      historySnapshots.push({
        ancestorAgent: ref.agent,
        ancestorSessionId: ref.sessionId,
        ancestorIndex,
        turns: result.turns.slice(-12),
      });
      loadedRefs.push({ ...ref, ancestorIndex });
    } catch {
      loadedRefs.push(ref);
    }
  }

  const byKey = new Map(loadedRefs.map((ref) => [`${ref.agent}:${ref.sessionId}`, ref]));
  const stages = (snapshot.stages ?? []).map((stage) => ({
    ...stage,
    sessionRefs: stage.sessionRefs.map((ref) => byKey.get(`${ref.agent}:${ref.sessionId}`) ?? ref),
  }));
  const threadSessionRefs = (snapshot.threadSessionRefs ?? []).map(
    (ref) => byKey.get(`${ref.agent}:${ref.sessionId}`) ?? ref,
  );
  const sourceRefs = dedupeSnapshotSessionRefs([...threadSessionRefs, ...stages.flatMap((stage) => stage.sessionRefs)]);
  return {
    snapshot: {
      ...snapshot,
      stages,
      threadSessionRefs,
      relatedContext: {
        sessionExcerptRefs: sourceRefs,
      },
      detailRefs: {
        threadId: snapshot.threadId,
        focusedStageId: snapshot.focusedStageId ?? null,
        stageIds: stages.map((stage) => stage.threadStageId),
        issueIds: stages.flatMap((stage) => (stage.issues ?? []).map((issue) => issue.id)),
        sessionRefs: sourceRefs,
      },
    },
    historySnapshots,
  };
}

function threadSessionRef(session: SessionInfo): ThreadWorkSnapshotSessionRef {
  return {
    agent: session.agent,
    sessionId: session.id,
    title: sessionDisplayTitle(session),
    filePath: session.filePath || null,
    sourceKind: "thread",
  };
}

function dedupeSnapshotSessionRefs(refs: ThreadWorkSnapshotSessionRef[]): ThreadWorkSnapshotSessionRef[] {
  const seen = new Set<string>();
  const result: ThreadWorkSnapshotSessionRef[] = [];
  for (const ref of refs) {
    const key = `${ref.agent}:${ref.sessionId}`;
    if (seen.has(key)) continue;
    seen.add(key);
    result.push(ref);
  }
  return result;
}
