import type { AssistantInfo, StageAssistantInfo, ThreadAssistantInfo } from "./api";
import { normalizeSelectedMcpIds } from "./hooks/useSelectableMcpServers";
import { getSessioPromptMarkers } from "./promptMarkers";

export const SELECTED_SKILL_IDS_OPTION = "selectedSkillIds";
export const SELECTED_MCP_IDS_OPTION = "selectedMcpIds";
const SESSIO_PROMPT_MARKERS = getSessioPromptMarkers();

export type AssistantResourceSelection =
  | Pick<AssistantInfo, "selectedSkillIds" | "selectedMcpIds">
  | Pick<StageAssistantInfo, "selectedSkillIds" | "selectedMcpIds">
  | Pick<ThreadAssistantInfo, "selectedSkillIds" | "selectedMcpIds">;

export function normalizeSelectedStringIds(ids: readonly unknown[] | null | undefined): string[] {
  const seen = new Set<string>();
  for (const id of ids ?? []) {
    if (typeof id !== "string") continue;
    const normalized = id.trim();
    if (!normalized) continue;
    seen.add(normalized);
  }
  return Array.from(seen);
}

export function assistantResourceRuntimeOptions(
  assistant: AssistantResourceSelection | null | undefined,
): Record<string, unknown> {
  if (!assistant) return {};
  return {
    [SELECTED_SKILL_IDS_OPTION]: normalizeSelectedStringIds(assistant.selectedSkillIds),
    [SELECTED_MCP_IDS_OPTION]: normalizeSelectedMcpIds(
      normalizeSelectedStringIds(assistant.selectedMcpIds),
    ),
  };
}

export function mergeRuntimeResourceOptions(
  base: Record<string, unknown>,
  extra: Record<string, unknown> | null | undefined,
): Record<string, unknown> {
  const merged = {
    ...base,
    ...(extra ?? {}),
  };
  merged[SELECTED_SKILL_IDS_OPTION] = normalizeSelectedStringIds([
    ...runtimeStringIds(base, SELECTED_SKILL_IDS_OPTION),
    ...runtimeStringIds(extra, SELECTED_SKILL_IDS_OPTION),
  ]);
  merged[SELECTED_MCP_IDS_OPTION] = normalizeSelectedMcpIds([
    ...runtimeStringIds(base, SELECTED_MCP_IDS_OPTION),
    ...runtimeStringIds(extra, SELECTED_MCP_IDS_OPTION),
  ]);
  return merged;
}

export function runtimeOptionsSelectComputerUseMcp(
  options: Record<string, unknown> | null | undefined,
): boolean {
  return runtimeStringIds(options, SELECTED_MCP_IDS_OPTION)
    .includes(builtinComputerUseMcpId());
}

export function builtinComputerUseMcpId(): string {
  return SESSIO_PROMPT_MARKERS.builtinMcpIdComputerUse;
}

function runtimeStringIds(
  options: Record<string, unknown> | null | undefined,
  key: string,
): string[] {
  const value = options?.[key];
  return Array.isArray(value) ? normalizeSelectedStringIds(value) : [];
}
