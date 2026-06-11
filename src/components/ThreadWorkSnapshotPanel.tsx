import { useEffect, useState } from "react";
import type {
  Agent,
  ThreadWorkSnapshotResult,
  ThreadWorkSnapshotSourceRef,
  ThreadWorkSnapshotSourcesResult,
} from "../api";
import { getThreadWorkSnapshot, getThreadWorkSnapshotSources } from "../api";
import { useI18n } from "../i18n";

export function useThreadWorkSnapshot(agent: Agent, sessionId: string): {
  snapshot: ThreadWorkSnapshotResult | null;
  sources: ThreadWorkSnapshotSourcesResult | null;
} {
  const [snapshot, setSnapshot] = useState<ThreadWorkSnapshotResult | null>(null);
  const [sources, setSources] = useState<ThreadWorkSnapshotSourcesResult | null>(null);

  useEffect(() => {
    let cancelled = false;
    setSnapshot(null);
    setSources(null);
    getThreadWorkSnapshot(agent, sessionId)
      .then((nextSnapshot) => {
        if (cancelled) return;
        setSnapshot(nextSnapshot);
        if (!nextSnapshot) return;
        getThreadWorkSnapshotSources(agent, sessionId)
          .then((nextSources) => {
            if (!cancelled) setSources(nextSources);
          })
          .catch((err) => console.warn("load thread work snapshot sources failed", err));
      })
      .catch((err) => console.warn("load thread work snapshot failed", err));
    return () => {
      cancelled = true;
    };
  }, [agent, sessionId]);

  return { snapshot, sources };
}

export default function ThreadWorkSnapshotPanel({
  snapshot,
  sources,
}: {
  snapshot: ThreadWorkSnapshotResult;
  sources: ThreadWorkSnapshotSourceRef[];
}) {
  const { t } = useI18n();
  const work = snapshot.snapshot;
  const workRecord = asRecord(work);
  const rollup = snapshotRollup(workRecord.rollup);
  if (!rollup) {
    return (
      <AstraTaskSnapshotPanel
        snapshot={snapshot}
        sources={sources}
        workRecord={workRecord}
      />
    );
  }

  const stages = Array.isArray(work.stages) ? work.stages : [];
  const hasStages = stages.length > 0;
  const openIssues = rollup.openIssues ?? stages.reduce(
    (total, stage) => total + (stage.issues ?? []).filter((issue) => issue.status === "open").length,
    0,
  );
  const taskChips = astraTaskSnapshotChips(workRecord, t);
  return (
    <section className="rounded-lg border border-card-border/[0.12] bg-card px-3 py-2.5 text-body-sm">
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <div className="text-caption uppercase text-ink/35">{t("thread.snapshot")}</div>
          <div className="truncate font-medium text-ink/80">{work.goal ?? snapshot.threadId}</div>
        </div>
        {hasStages && (
          <div className="flex flex-wrap items-center gap-1.5 text-caption text-ink/45">
            <span className="rounded bg-ink/[0.06] px-1.5 py-0.5">
              {t("thread.snapshot_complete", {
                completed: rollup.completed,
                total: rollup.total,
              })}
            </span>
            <span className="rounded bg-ink/[0.06] px-1.5 py-0.5">
              {t("thread.snapshot_blocked", { count: rollup.blocked })}
            </span>
            <span className="rounded bg-ink/[0.06] px-1.5 py-0.5">
              {t("thread.snapshot_open_issues", { count: openIssues })}
            </span>
          </div>
        )}
      </div>
      {taskChips.length > 0 && (
        <SnapshotChips chips={taskChips} />
      )}
      {hasStages && (
        <div className="mt-2 grid gap-1.5">
          {stages.map((stage) => (
            <div
              key={stage.threadStageId}
              className={
                "grid min-w-0 grid-cols-[minmax(0,1fr)_auto] gap-2 rounded-md border px-2 py-1.5 " +
                (stage.threadStageId === work.focusedStageId
                  ? "border-[rgb(var(--color-emerald)/0.35)] bg-[rgb(var(--color-emerald)/0.06)]"
                  : "border-card-border/[0.10] bg-card-panel")
              }
            >
              <div className="min-w-0">
                <div className="truncate font-medium text-ink/70">{stage.name}</div>
                {(stage.summary || stage.outcome) && (
                  <div className="truncate text-caption text-ink/40">
                    {stage.summary ?? stage.outcome}
                  </div>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-1.5 text-caption text-ink/45">
                <span>{t(`stage.status.${stage.status}`)}</span>
                {(stage.issues ?? []).filter((issue) => issue.status === "open").length > 0 && (
                  <span className="rounded bg-ink/[0.06] px-1 py-0.5">
                    {t("thread.snapshot_stage_issues", {
                      count: (stage.issues ?? []).filter((issue) => issue.status === "open").length,
                    })}
                  </span>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
      <ThreadWorkSnapshotSources sources={sources} />
    </section>
  );
}

function AstraTaskSnapshotPanel({
  snapshot,
  sources,
  workRecord,
}: {
  snapshot: ThreadWorkSnapshotResult;
  sources: ThreadWorkSnapshotSourceRef[];
  workRecord: Record<string, unknown>;
}) {
  const { t } = useI18n();
  const chips = astraTaskSnapshotChips(workRecord, t);

  return (
    <section className="rounded-lg border border-card-border/[0.12] bg-card px-3 py-2.5 text-body-sm">
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <div className="text-caption uppercase text-ink/35">{t("thread.snapshot")}</div>
          <div className="truncate font-medium text-ink/80">{pickString(workRecord.goal) ?? snapshot.threadId}</div>
        </div>
      </div>
      {chips.length > 0 && (
        <SnapshotChips chips={chips} />
      )}
      <ThreadWorkSnapshotSources sources={sources} />
    </section>
  );
}

function astraTaskSnapshotChips(
  workRecord: Record<string, unknown>,
  t: (key: string, vars?: Record<string, string | number>) => string,
): string[] {
  const task = asRecord(workRecord.task);
  const contextPolicy = asRecord(workRecord.contextPolicy);
  const assistantSnapshot = asRecord(workRecord.assistantSnapshot);
  const agentSnapshot = asRecord(workRecord.agentSnapshot);
  const agentInfo = asRecord(agentSnapshot.agentInfo);
  const hasTaskContext =
    Object.keys(task).length > 0 ||
    Object.keys(contextPolicy).length > 0 ||
    Object.keys(assistantSnapshot).length > 0 ||
    Object.keys(agentSnapshot).length > 0 ||
    pickString(workRecord.focusedAssistantId) != null;
  if (!hasTaskContext) return [];
  const kind = pickString(workRecord.kind);
  const taskTitle = pickString(task.title) ?? pickString(task.id);
  const assistantLabel = pickString(assistantSnapshot.name)
    ?? pickString(assistantSnapshot.assistantId)
    ?? pickString(task.assistantId)
    ?? pickString(workRecord.focusedAssistantId);
  const agentLabel = pickString(agentInfo.displayName)
    ?? pickString(agentInfo.name)
    ?? pickString(agentSnapshot.agent)
    ?? pickString(task.targetAgent);
  const policyMode = pickString(contextPolicy.mode);
  const laneId = pickString(contextPolicy.laneId);
  return [
    kind ? threadSnapshotKindLabel(kind, t) : null,
    taskTitle ? t("thread.snapshot_task", { value: taskTitle }) : null,
    assistantLabel ? t("thread.snapshot_assistant", { value: assistantLabel }) : null,
    agentLabel ? t("thread.snapshot_agent", { value: agentLabel }) : null,
    policyMode ? t("thread.snapshot_policy", { value: policyMode.replace(/_/g, " ") }) : null,
    laneId ? t("thread.snapshot_lane", { value: laneId }) : null,
  ].filter((chip): chip is string => Boolean(chip));
}

function SnapshotChips({ chips }: { chips: string[] }) {
  return (
    <div className="mt-2 flex flex-wrap gap-1.5 text-caption text-ink/45">
      {chips.map((chip) => (
        <span key={chip} className="max-w-full truncate rounded bg-ink/[0.06] px-1.5 py-0.5">
          {chip}
        </span>
      ))}
    </div>
  );
}

function ThreadWorkSnapshotSources({
  sources,
}: {
  sources: ThreadWorkSnapshotSourceRef[];
}) {
  if (sources.length === 0) return null;
  return (
    <div className="mt-2 flex flex-wrap gap-1.5">
      {sources.slice(0, 10).map((source) => (
        <span
          key={`${source.kind}:${source.id}`}
          title={source.filePath ?? source.sessionId ?? source.id}
          className="max-w-[260px] truncate rounded border border-card-border/[0.10] bg-card-panel px-1.5 py-0.5 text-caption text-ink/45"
        >
          {source.kind}: {source.label}
          {source.ancestorIndex != null ? ` #${source.ancestorIndex}` : ""}
        </span>
      ))}
      {sources.length > 10 && (
        <span className="rounded border border-card-border/[0.10] bg-card-panel px-1.5 py-0.5 text-caption text-ink/35">
          +{sources.length - 10}
        </span>
      )}
    </div>
  );
}

type SnapshotRollup = {
  completed: number;
  total: number;
  blocked: number;
  openIssues: number | null;
};

function snapshotRollup(value: unknown): SnapshotRollup | null {
  const record = asRecord(value);
  const completed = pickNumber(record.completed);
  const total = pickNumber(record.total);
  const blocked = pickNumber(record.blocked);
  if (completed == null || total == null || blocked == null) return null;
  return {
    completed,
    total,
    blocked,
    openIssues: pickNumber(record.openIssues),
  };
}

function threadSnapshotKindLabel(
  kind: string,
  t: (key: string, vars?: Record<string, string | number>) => string,
): string {
  switch (kind) {
    case "process":
    case "teamwork":
    case "brainstorm":
    case "debate":
      return t(`thread.kind.${kind}`);
    default:
      return kind;
  }
}

function pickString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function pickNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}
