import type { RuntimeError } from "./api";
import type {
  AcpPermissionRequest,
  AcpRenderBlock,
  AcpToolCall,
  AcpViewModel,
  LiveTurn,
} from "./runtimeChat";

export type AcpRenderItem =
  | { kind: "turnStatus"; turn: LiveTurn }
  | { kind: "workingIndicator"; turn: LiveTurn }
  | { kind: "block"; turn: LiveTurn; block: AcpRenderBlock }
  | { kind: "tool"; turn: LiveTurn; tool: AcpToolCall; history: boolean }
  | { kind: "toolGroup"; turn: LiveTurn; tools: AcpToolCall[] }
  | { kind: "permission"; turn: LiveTurn; permission: AcpPermissionRequest }
  | { kind: "error"; turn: LiveTurn; error: RuntimeError };

export interface FileEditSummary {
  source?: string;
  files?: number;
  additions?: number;
  deletions?: number;
  edits?: FileEditItem[];
}

export interface FileEditItem {
  path?: string;
  displayPath?: string;
  kind?: string;
  additions?: number;
  deletions?: number;
  detail?: string;
  details?: string[];
  patch?: string | null;
  patches?: string[];
  oldContent?: string | null;
  newContent?: string | null;
  contentDiffs?: FileEditContentDiff[];
}

export interface FileEditContentDiff {
  oldContent?: string | null;
  newContent?: string | null;
}

export function fileEditKey(edit: FileEditItem): string {
  return edit.path || edit.displayPath || "(unknown file)";
}

export function fileEditMatchesPath(edit: FileEditItem, path: string): boolean {
  if (!path) return false;
  return (
    fileEditKey(edit) === path ||
    edit.path === path ||
    edit.displayPath === path
  );
}

export function aggregateSessionFileEdits(viewModel: AcpViewModel): {
  edits: FileEditItem[];
  additions: number;
  deletions: number;
} {
  const edits: FileEditItem[] = [];
  for (const turn of viewModel.turns) {
    for (const block of turn.blocks) {
      if (block.kind !== "sessionUpdate") continue;
      if (block.updateType !== "file_edit") continue;
      const summary = parseFileEditSummary(block.data);
      if (!summary?.edits) continue;
      for (const edit of summary.edits) {
        mergeFileEditItem(edits, edit);
      }
    }
  }
  return {
    edits,
    additions: sumEditNumber(edits, "additions"),
    deletions: sumEditNumber(edits, "deletions"),
  };
}

export function acpViewModelToRenderItems(
  viewModel: AcpViewModel,
  liveTurnIds: Set<string>,
  workingIndicatorTurnId: string,
): AcpRenderItem[] {
  const items: AcpRenderItem[] = [];
  const latestLiveTurn = latestTurnWithIds(viewModel.turns, liveTurnIds);
  let lastUserIndex = -1;
  for (const turn of viewModel.turns) {
    const renderedTools = new Set<string>();
    const renderedPermissions = new Set<string>();
    const groupHistoryTools = !liveTurnIds.has(turn.turnId);
    let pendingTools: AcpToolCall[] = [];
    let pendingPermissions: AcpPermissionRequest[] = [];
    const pendingFileEditBlocks: Extract<AcpRenderBlock, { kind: "sessionUpdate" }>[] = [];
    const flushPendingTools = () => {
      if (pendingTools.length === 0) return;
      if (pendingTools.length === 1) {
        items.push({ kind: "tool", turn, tool: pendingTools[0], history: true });
      } else {
        items.push({ kind: "toolGroup", turn, tools: pendingTools });
      }
      pendingTools = [];
    };
    turn.blocks.forEach((block) => {
      if (block.kind === "tool") {
        const originalTool = turn.tools.find((item) => item.toolId === block.toolId);
        if (!originalTool || renderedTools.has(originalTool.toolId)) return;
        renderedTools.add(originalTool.toolId);
        if (groupHistoryTools) {
          pendingTools.push(originalTool);
        } else {
          items.push({ kind: "tool", turn, tool: originalTool, history: false });
        }
        return;
      }
      flushPendingTools();
      if (block.kind === "permission") {
        const permission = turn.permissions.find((item) => item.requestId === block.requestId);
        if (!permission || renderedPermissions.has(permission.requestId)) return;
        renderedPermissions.add(permission.requestId);
        if (shouldPinPendingPermission(permission)) {
          pendingPermissions.push(permission);
        } else {
          items.push({ kind: "permission", turn, permission });
        }
        return;
      }
      if (block.kind === "error") return;
      if (block.kind === "sessionUpdate" && block.updateType === "file_edit") {
        pendingFileEditBlocks.push(block);
        return;
      }
      items.push({ kind: "block", turn, block });
      if (block.kind === "user") {
        lastUserIndex = items.length - 1;
      }
    });
    flushPendingTools();
    const fileEditBlock = mergedFileEditRenderBlock(pendingFileEditBlocks);
    if (fileEditBlock) {
      items.push({ kind: "block", turn, block: fileEditBlock });
    }
    pendingPermissions.forEach((permission) => {
      items.push({ kind: "permission", turn, permission });
    });
    if (turn.error) {
      items.push({ kind: "error", turn, error: turn.error });
    }
    if (turn.turnId === workingIndicatorTurnId) {
      items.push({ kind: "workingIndicator", turn });
    }
  }
  if (latestLiveTurn) {
    const insertAt = lastUserIndex >= 0 ? lastUserIndex : 0;
    items.splice(insertAt, 0, { kind: "turnStatus", turn: latestLiveTurn });
  }
  return items;
}

function shouldPinPendingPermission(permission: AcpPermissionRequest): boolean {
  return (
    permission.options.length > 0 &&
    !permission.selectedOptionId &&
    !permission.cancelled
  );
}

export function renderItemKeys(items: AcpRenderItem[]): string[] {
  const blockCounts = new Map<string, number>();
  return items.map((item) => {
    if (item.kind !== "block") return renderItemKey(item);
    const count = blockCounts.get(item.turn.turnId) ?? 0;
    blockCounts.set(item.turn.turnId, count + 1);
    return `acp:${item.turn.turnId}:block:${count}`;
  });
}

function latestTurnWithIds(turns: LiveTurn[], ids: Set<string>): LiveTurn | null {
  for (let index = turns.length - 1; index >= 0; index -= 1) {
    const turn = turns[index];
    if (ids.has(turn.turnId)) return turn;
  }
  return null;
}

function renderItemKey(item: AcpRenderItem): string {
  if (item.kind === "turnStatus") return `acp:${item.turn.turnId}:status`;
  if (item.kind === "workingIndicator") return `acp:${item.turn.turnId}:working`;
  if (item.kind === "block") return `acp:${item.turn.turnId}:block`;
  if (item.kind === "tool") return `acp:${item.turn.turnId}:tool:${item.tool.toolId}`;
  if (item.kind === "toolGroup") {
    return `acp:${item.turn.turnId}:tool-group:${item.tools.map((tool) => tool.toolId).join(":")}`;
  }
  if (item.kind === "permission") return `acp:${item.turn.turnId}:permission:${item.permission.requestId}`;
  return `acp:${item.turn.turnId}:error`;
}

function mergedFileEditRenderBlock(
  blocks: Extract<AcpRenderBlock, { kind: "sessionUpdate" }>[],
): Extract<AcpRenderBlock, { kind: "sessionUpdate" }> | null {
  if (blocks.length === 0) return null;
  if (blocks.length === 1) return blocks[0];
  const summaries = blocks.map((block) => parseFileEditSummary(block.data));
  if (summaries.some((summary) => !summary)) {
    return blocks[blocks.length - 1];
  }
  const edits: FileEditItem[] = [];
  for (const summary of summaries) {
    for (const edit of summary?.edits ?? []) {
      mergeFileEditItem(edits, edit);
    }
  }
  const source = summaries.find((summary) => summary?.source)?.source ?? "session";
  const data: FileEditSummary = {
    source,
    files: edits.length,
    additions: sumEditNumber(edits, "additions"),
    deletions: sumEditNumber(edits, "deletions"),
    edits,
  };
  return {
    ...blocks[blocks.length - 1],
    data,
  };
}

export function parseFileEditSummary(value: unknown): FileEditSummary | null {
  let parsed = value;
  if (typeof parsed === "string") {
    try {
      parsed = JSON.parse(parsed) as unknown;
    } catch {
      return null;
    }
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return null;
  }
  const record = parsed as FileEditSummary & { text?: unknown };
  if (Array.isArray(record.edits)) return normalizeFileEditSummary(record);
  if (typeof record.text === "string") return parseFileEditSummary(record.text);
  return null;
}

function normalizeFileEditSummary(summary: FileEditSummary): FileEditSummary {
  const edits: FileEditItem[] = [];
  for (const edit of summary.edits ?? []) {
    mergeFileEditItem(edits, edit);
  }
  return {
    ...summary,
    files: edits.length,
    additions: sumEditNumber(edits, "additions"),
    deletions: sumEditNumber(edits, "deletions"),
    edits,
  };
}

function mergeFileEditItem(edits: FileEditItem[], next: FileEditItem) {
  const key = fileEditKey(next);
  const existing = edits.find((edit) => fileEditKey(edit) === key);
  if (!existing) {
    edits.push(normalizeFileEditItem(next));
    return;
  }
  mergeFileEditValues(existing, next);
}

function normalizeFileEditItem(edit: FileEditItem): FileEditItem {
  return {
    ...edit,
    patches: mergeTextVariants(edit.patches, edit.patch),
    details: mergeTextVariants(edit.details, edit.detail),
    contentDiffs: mergeContentDiffs(edit.contentDiffs, contentDiffFromEdit(edit)),
  };
}

function mergeFileEditValues(existing: FileEditItem, next: FileEditItem) {
  if (existing.kind && next.kind && existing.kind !== next.kind) {
    existing.kind = "mixed";
  }
  existing.additions = (existing.additions ?? 0) + (next.additions ?? 0);
  existing.deletions = (existing.deletions ?? 0) + (next.deletions ?? 0);
  existing.patch = mergeOptionalText(existing.patch, next.patch);
  existing.patches = mergeTextVariants(
    mergeTextVariants(existing.patches, existing.patch),
    mergeTextVariants(next.patches, next.patch),
  );
  existing.detail = mergeOptionalText(existing.detail, next.detail);
  existing.details = mergeTextVariants(
    mergeTextVariants(existing.details, existing.detail),
    mergeTextVariants(next.details, next.detail),
  );
  existing.contentDiffs = mergeContentDiffs(
    mergeContentDiffs(
      existing.contentDiffs,
      contentDiffFromEdit(existing),
    ),
    mergeContentDiffs(next.contentDiffs, contentDiffFromEdit(next)),
  );
  existing.oldContent = mergeOptionalContent(existing.oldContent, next.oldContent);
  existing.newContent = mergeOptionalContent(existing.newContent, next.newContent);
  existing.displayPath ??= next.displayPath;
  existing.path ??= next.path;
}

function mergeOptionalText(current: string | null | undefined, next: string | null | undefined): string | undefined {
  if (typeof current === "string" && current.trim()) return current;
  if (typeof next === "string" && next.trim()) return next;
  return undefined;
}

function mergeOptionalContent(current: string | null | undefined, next: string | null | undefined): string | undefined {
  if (typeof current === "string") return current;
  if (typeof next === "string") return next;
  return undefined;
}

function mergeTextVariants(
  left?: string[] | string | null,
  right?: string[] | string | null,
): string[] | undefined {
  const values = [...normalizeTextVariants(left), ...normalizeTextVariants(right)];
  if (values.length === 0) return undefined;
  return Array.from(new Set(values));
}

function mergeContentDiffs(
  left?: FileEditContentDiff[] | FileEditContentDiff | null,
  right?: FileEditContentDiff[] | FileEditContentDiff | null,
): FileEditContentDiff[] | undefined {
  const diffs = [...normalizeContentDiffs(left), ...normalizeContentDiffs(right)];
  if (diffs.length === 0) return undefined;
  return dedupeContentDiffs(diffs);
}

function normalizeTextVariants(value?: string[] | string | null): string[] {
  const values = Array.isArray(value) ? value : value == null ? [] : [value];
  return values.filter((item): item is string => typeof item === "string" && item.trim().length > 0);
}

function contentDiffFromEdit(edit: FileEditItem): FileEditContentDiff | undefined {
  if (
    typeof edit.oldContent !== "string" &&
    typeof edit.newContent !== "string"
  ) {
    return undefined;
  }
  return {
    oldContent: edit.oldContent,
    newContent: edit.newContent,
  };
}

function normalizeContentDiffs(
  value?: FileEditContentDiff[] | FileEditContentDiff | null,
): FileEditContentDiff[] {
  const values = Array.isArray(value) ? value : value == null ? [] : [value];
  return values.filter(
    (item): item is FileEditContentDiff =>
      Boolean(item) &&
      (typeof item.oldContent === "string" ||
        typeof item.newContent === "string"),
  );
}

function dedupeContentDiffs(diffs: FileEditContentDiff[]): FileEditContentDiff[] {
  const seen = new Set<string>();
  const result: FileEditContentDiff[] = [];
  for (const diff of diffs) {
    const key = `${diff.oldContent ?? ""}\u0001${diff.newContent ?? ""}`;
    if (seen.has(key)) continue;
    seen.add(key);
    result.push(diff);
  }
  return result;
}

function sumEditNumber(edits: FileEditItem[], key: "additions" | "deletions"): number {
  return edits.reduce((sum, edit) => {
    const value = edit[key];
    return sum + (typeof value === "number" ? value : 0);
  }, 0);
}
