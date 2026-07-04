import type { CanvasBlockKind, CanvasBlockSourceType, CanvasContextRef, UpsertCanvasBlockRecordInput } from "../../canvasTypes";
import type { ThreadWorkSnapshot } from "../../api";
import type { FileCardBlockModel } from "./blocks/file-card";
import type { WorkflowCardBlockModel } from "./blocks/workflow-card";

type TextLike = {
  toString(): string;
};

type NoteChildModel = {
  flavour?: string;
  text?: TextLike | null;
  caption?: string | null;
  children?: NoteChildModel[];
};

type NoteLikeModel = {
  id: string;
  flavour?: string;
  children?: NoteChildModel[];
};

type ImageLikeModel = {
  id: string;
  flavour?: string;
  caption?: string | null;
  sourceId?: string | null;
};

type GroupLikeModel = {
  id: string;
  flavour?: string;
  title?: TextLike | string | null;
  childIds?: string[];
};

type BlockRecordMetadata = Record<string, unknown>;

type CanvasInteropBlock =
  | FileCardBlockModel
  | WorkflowCardBlockModel
  | NoteLikeModel
  | ImageLikeModel
  | GroupLikeModel;

function hasFlavour(
  model: CanvasInteropBlock,
): model is (CanvasInteropBlock & { flavour: string }) {
  return typeof (model as { flavour?: unknown }).flavour === "string";
}

export function fileCardModelToCanvasBlock(
  model: FileCardBlockModel,
): UpsertCanvasBlockRecordInput {
  return {
    blockId: model.id,
    blockKind: "file_card",
    sourceType: normalizeSourceType(model.sourceType),
    sourcePath: model.sourcePath || null,
    sourceKey: model.title || null,
    metadataJson: JSON.stringify({
      kind: "file_card",
      title: model.title,
      sourcePath: model.sourcePath,
      sourceType: model.sourceType,
      subtitle: model.subtitle,
      summary: model.summary,
      status: model.status,
      contentVersion: model.contentVersion,
      previewCollapsed: model.previewCollapsed,
    } satisfies BlockRecordMetadata),
  };
}

export function workflowCardModelToCanvasBlock(
  model: WorkflowCardBlockModel,
): UpsertCanvasBlockRecordInput {
  return {
    blockId: model.id,
    blockKind: "workflow_card",
    sourceType: normalizeSourceType(model.sourceType),
    sourcePath: null,
    sourceKey: model.threadId || model.title || null,
    metadataJson: JSON.stringify({
      kind: "workflow_card",
      title: model.title,
      sourceType: model.sourceType,
      threadId: model.threadId,
      threadStageId: model.threadStageId,
      workflowSummaryMarkdown: model.workflowSummaryMarkdown,
      executionState: model.executionState,
      lastRunId: model.lastRunId,
      workflowSnapshotJson: model.workflowSnapshotJson,
      threadGoal: model.threadGoal,
      status: model.status,
    } satisfies BlockRecordMetadata),
  };
}

export function noteModelToCanvasBlock(
  model: NoteLikeModel,
): UpsertCanvasBlockRecordInput {
  const title = extractNoteTitle(model);
  const summary = extractNoteText(model).slice(0, 320);
  return {
    blockId: model.id,
    blockKind: "note",
    sourceType: "note",
    sourcePath: null,
    sourceKey: title,
    metadataJson: JSON.stringify({
      kind: "note",
      title,
      summary,
    } satisfies BlockRecordMetadata),
  };
}

export function imageModelToCanvasBlock(
  model: ImageLikeModel,
): UpsertCanvasBlockRecordInput {
  const caption = typeof model.caption === "string" ? model.caption.trim() : "";
  return {
    blockId: model.id,
    blockKind: "image",
    sourceType: "attachment_image",
    sourcePath: null,
    sourceKey: caption || "Image",
    metadataJson: JSON.stringify({
      kind: "image",
      title: caption || "Image",
      caption: caption || null,
      sourceId: model.sourceId ?? null,
    } satisfies BlockRecordMetadata),
  };
}

export function groupModelToCanvasBlock(
  model: GroupLikeModel,
): UpsertCanvasBlockRecordInput {
  const title = toText(model.title) || "Group";
  const childIds = Array.isArray(model.childIds) ? model.childIds : [];
  return {
    blockId: model.id,
    blockKind: "group",
    sourceType: "group",
    sourcePath: null,
    sourceKey: title,
    metadataJson: JSON.stringify({
      kind: "group",
      title,
      childIds,
    } satisfies BlockRecordMetadata),
  };
}

export function canvasInteropModelToCanvasBlock(
  model: CanvasInteropBlock,
): UpsertCanvasBlockRecordInput | null {
  if (!hasFlavour(model)) return null;
  switch (model.flavour) {
    case "sessio:file-card":
      return fileCardModelToCanvasBlock(model as FileCardBlockModel);
    case "sessio:workflow-card":
      return workflowCardModelToCanvasBlock(model as WorkflowCardBlockModel);
    case "affine:note":
      return isPlaceholderEdgelessNote(model as NoteLikeModel)
        ? null
        : noteModelToCanvasBlock(model as NoteLikeModel);
    case "affine:image":
      return imageModelToCanvasBlock(model as ImageLikeModel);
    default:
      return null;
  }
}

export function surfaceElementToCanvasBlock(
  element: { id: string; type?: string; title?: TextLike | string | null; childIds?: string[] },
): UpsertCanvasBlockRecordInput | null {
  if (element.type !== "group") return null;
  return groupModelToCanvasBlock({
    id: element.id,
    title: element.title ?? null,
    childIds: element.childIds ?? [],
  });
}

export function canvasBlockRecordToContextRef(record: {
  blockId: string;
  blockKind: CanvasBlockKind;
  sourceType: string;
  sourcePath?: string | null;
  sourceKey?: string | null;
  metadataJson?: string | null;
}): CanvasContextRef {
  const meta = tryParseJson(record.metadataJson);
  const title =
    (typeof meta?.title === "string" && meta.title.trim() ? meta.title.trim() : null)
    ?? record.sourceKey
    ?? fallbackTitleForKind(record.blockKind);
  return {
    blockId: record.blockId,
    blockKind: record.blockKind,
    sourceType: record.sourceType,
    sourcePath: record.sourcePath ?? null,
    sourceKey: record.sourceKey ?? null,
    summary: buildCanvasRefSummary(record.blockKind, title, record.sourcePath ?? null, meta),
  };
}

export function tryParseJson(value: string | null | undefined): Record<string, unknown> | null {
  if (!value || !value.trim()) return null;
  try {
    const parsed = JSON.parse(value);
    return parsed && typeof parsed === "object" ? parsed as Record<string, unknown> : null;
  } catch {
    return null;
  }
}

export function renderSelectionSummaryMarkdown(
  refs: Array<{
    title: string;
    sourcePath: string | null;
    blockKind: CanvasBlockKind;
  }>,
): string {
  const lines = [
    "# Canvas selection",
    "",
    ...refs.map((ref, index) =>
      `${index + 1}. ${ref.blockKind} - ${ref.title}${ref.sourcePath ? ` (${ref.sourcePath})` : ""}`,
    ),
    "",
    "Use the attached canvas snapshot and workflow summaries when helpful.",
  ];
  return lines.join("\n");
}

export function renderWorkflowSummaryMarkdown(meta: Record<string, unknown>, title: string): string {
  const snapshotJson =
    typeof meta.workflowSnapshotJson === "string" && meta.workflowSnapshotJson.trim()
      ? meta.workflowSnapshotJson
      : null;
  const snapshot = snapshotJson ? tryParseJson(snapshotJson) : null;
  const stages = Array.isArray(snapshot?.stages) ? snapshot.stages : [];
  const assistants = Array.isArray(snapshot?.assistants) ? snapshot.assistants : [];
  const participants = Array.isArray(snapshot?.agentParticipants) ? snapshot.agentParticipants : [];
  const rounds = Array.isArray(snapshot?.planRounds) ? snapshot.planRounds : [];
  const lines = [
    `# ${title}`,
    "",
    typeof snapshot?.goal === "string" && snapshot.goal.trim() ? snapshot.goal.trim() : "Workflow summary",
    "",
  ];
  if (typeof snapshot?.kind === "string" && snapshot.kind.trim()) {
    lines.push(`Mode: ${snapshot.kind.trim()}`, "");
  }
  if (assistants.length > 0) {
    lines.push("## Team", "");
    for (const assistant of assistants.slice(0, 8)) {
      if (!assistant || typeof assistant !== "object") continue;
      const assistantRecord = assistant as Record<string, unknown>;
      const name = typeof assistantRecord.name === "string" && assistantRecord.name.trim()
        ? assistantRecord.name.trim()
        : "Assistant";
      const agent = assistantRecord.agent && typeof assistantRecord.agent === "object"
        ? assistantRecord.agent as Record<string, unknown>
        : null;
      const agentName = typeof agent?.name === "string" && agent.name.trim()
        ? agent.name.trim()
        : typeof agent?.id === "string" && agent.id.trim()
          ? agent.id.trim()
          : "";
      lines.push(`- ${name}${agentName ? ` (${agentName})` : ""}`);
    }
    lines.push("");
  }
  if (participants.length > 0) {
    lines.push("## Participants", "");
    for (const participant of participants.slice(0, 8)) {
      if (!participant || typeof participant !== "object") continue;
      const participantRecord = participant as Record<string, unknown>;
      const agent = typeof participantRecord.agent === "string" && participantRecord.agent.trim()
        ? participantRecord.agent.trim()
        : "agent";
      const model = typeof participantRecord.model === "string" && participantRecord.model.trim()
        ? participantRecord.model.trim()
        : "";
      lines.push(`- ${agent}${model ? ` ${model}` : ""}`);
    }
    lines.push("");
  }
  if (stages.length > 0) {
    lines.push("## Stages", "");
    for (const stage of stages.slice(0, 8)) {
      if (!stage || typeof stage !== "object") continue;
      const stageRecord = stage as Record<string, unknown>;
      const name =
        typeof stageRecord.name === "string" && stageRecord.name.trim()
          ? stageRecord.name.trim()
          : "Stage";
      const status =
        typeof stageRecord.status === "string" && stageRecord.status.trim()
          ? stageRecord.status.trim()
          : "unknown";
      lines.push(`- ${name}: ${status}`);
    }
    lines.push("");
  }
  if (rounds.length > 0) {
    lines.push("## Rounds", "");
    for (const round of rounds.slice(-6)) {
      if (!round || typeof round !== "object") continue;
      const roundRecord = round as Record<string, unknown>;
      const index = typeof roundRecord.roundIndex === "number" ? roundRecord.roundIndex : "?";
      const status = typeof roundRecord.status === "string" && roundRecord.status.trim()
        ? roundRecord.status.trim()
        : "unknown";
      const taskCount = Array.isArray(roundRecord.tasks) ? roundRecord.tasks.length : 0;
      lines.push(`- Round ${index}: ${status}${taskCount > 0 ? ` (${taskCount} tasks)` : ""}`);
    }
    lines.push("");
  }
  return lines.join("\n");
}

export function workflowSnapshotToMarkdown(snapshot: ThreadWorkSnapshot | null): string {
  if (!snapshot) return "";
  return renderWorkflowSummaryMarkdown(
    {
      workflowSnapshotJson: JSON.stringify(snapshot),
    },
    snapshot.goal || "Workflow",
  );
}

export function buildCanvasRefSummary(
  kind: CanvasBlockKind,
  title: string,
  sourcePath: string | null,
  meta: Record<string, unknown> | null,
): string {
  if (kind === "workflow_card") {
    return `${title}${sourcePath ? ` (${sourcePath})` : ""}`;
  }
  if (kind === "image") {
    return `${title}${sourcePath ? ` from ${sourcePath}` : ""}`;
  }
  if (kind === "file_card") {
    return `${title}${sourcePath ? ` at ${sourcePath}` : ""}`;
  }
  if (kind === "note") {
    return typeof meta?.summary === "string" && meta.summary.trim()
      ? meta.summary.trim()
      : title;
  }
  return title;
}

export function fallbackTitleForKind(kind: CanvasBlockKind): string {
  switch (kind) {
    case "file_card":
      return "File";
    case "workflow_card":
      return "Workflow";
    case "image":
      return "Image";
    case "group":
      return "Group";
    case "note":
    default:
      return "Canvas note";
  }
}

function normalizeSourceType(value: string): CanvasBlockSourceType {
  switch (value) {
    case "edited_file":
    case "workspace_file":
    case "attachment_image":
    case "workflow_definition":
    case "inline_markdown":
    case "note":
    case "group":
      return value;
    default:
      return "workspace_file";
  }
}

function extractNoteTitle(model: NoteLikeModel): string {
  const text = extractNoteText(model);
  const title = text.split(/\r?\n/).map(line => line.trim()).find(Boolean) ?? "";
  return title.slice(0, 120) || "New note";
}

function isPlaceholderEdgelessNote(model: NoteLikeModel): boolean {
  return extractNoteText(model).trim().length === 0;
}

function extractNoteText(model: NoteLikeModel): string {
  const chunks: string[] = [];
  walkChildren(model.children ?? [], chunks);
  return chunks.join("\n").replace(/\n{3,}/g, "\n\n").trim();
}

function walkChildren(children: NoteChildModel[], chunks: string[]) {
  for (const child of children) {
    const direct = child.text?.toString().trim();
    if (direct) {
      chunks.push(direct);
    } else if (typeof child.caption === "string" && child.caption.trim()) {
      chunks.push(child.caption.trim());
    }
    if (Array.isArray(child.children) && child.children.length > 0) {
      walkChildren(child.children, chunks);
    }
  }
}

function toText(value: TextLike | string | null | undefined): string {
  if (typeof value === "string") return value.trim();
  if (value && typeof value.toString === "function") return value.toString().trim();
  return "";
}
