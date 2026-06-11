import type { AstraHandle, AstraRunStatus, PlanRoundInfo, PlanTaskStatus } from "./api";

export function isAstraActive(status: AstraRunStatus): boolean {
  return (
    status === "planning"
    || status === "thinking"
    || status === "awaiting_approval"
    || status === "dispatching"
    || status === "running"
  );
}

const HIDDEN_RUN_DIAGNOSTIC_KINDS = new Set(["teamwork_round_journal"]);

export function visibleAstraRunDiagnostics(diagnostics: unknown[]): unknown[] {
  return diagnostics.filter((diagnostic) => {
    if (!diagnostic || typeof diagnostic !== "object" || Array.isArray(diagnostic)) return true;
    const kind = (diagnostic as Record<string, unknown>).kind;
    return typeof kind !== "string" || !HIDDEN_RUN_DIAGNOSTIC_KINDS.has(kind);
  });
}

export function upsertAstraRun(runs: AstraHandle[], run: AstraHandle): AstraHandle[] {
  const next = runs.some((item) => item.runId === run.runId)
    ? runs.map((item) => item.runId === run.runId ? run : item)
    : [run, ...runs];
  return next.slice().sort((a, b) => b.updatedAt - a.updatedAt);
}

export function formatAstraStatus(status: string): string {
  return status.replace(/_/g, " ");
}

export function astraStatusClass(status: AstraRunStatus): string {
  switch (status) {
    case "awaiting_approval":
      return "bg-sky-500/[0.10] text-sky-500";
    case "thinking":
      return "bg-violet-500/[0.10] text-violet-500";
    case "dispatching":
    case "running":
      return "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]";
    case "errored":
      return "bg-red-500/[0.10] text-red-500";
    case "cancelled":
    case "interrupted":
      return "bg-ink/[0.08] text-ink/45";
    case "completed":
      return "bg-[rgb(var(--color-emerald)/0.12)] text-[rgb(var(--color-emerald)/0.95)]";
    case "planning":
    default:
      return "bg-amber-500/[0.10] text-amber-500";
  }
}

export function astraRiskClass(risk: "low" | "medium" | "high"): string {
  switch (risk) {
    case "high":
      return "bg-red-500/[0.10] text-red-500";
    case "medium":
      return "bg-amber-500/[0.10] text-amber-500";
    case "low":
    default:
      return "bg-ink/[0.06] text-ink/45";
  }
}

export function astraTaskStatusClass(status: PlanTaskStatus): string {
  switch (status) {
    case "running":
      return "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]";
    case "completed":
      return "bg-[rgb(var(--color-emerald)/0.12)] text-[rgb(var(--color-emerald)/0.95)]";
    case "failed":
    case "errored":
      return "bg-red-500/[0.10] text-red-500";
    case "cancelled":
      return "bg-ink/[0.08] text-ink/45";
    case "planned":
    default:
      return "bg-ink/[0.06] text-ink/45";
  }
}

export function planRoundStatusClass(status: PlanRoundInfo["status"]): string {
  switch (status) {
    case "running":
      return "bg-[rgb(var(--color-emerald)/0.10)] text-[rgb(var(--color-emerald)/0.95)]";
    case "completed":
      return "bg-[rgb(var(--color-emerald)/0.12)] text-[rgb(var(--color-emerald)/0.95)]";
    case "errored":
      return "bg-red-500/[0.10] text-red-500";
    case "cancelled":
      return "bg-ink/[0.08] text-ink/45";
    case "planned":
    default:
      return "bg-amber-500/[0.10] text-amber-500";
  }
}
