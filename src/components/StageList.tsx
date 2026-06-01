import { useEffect, useState } from "react";
import { DragDropProvider, type DragEndEvent } from "@dnd-kit/react";
import { isSortable, useSortable } from "@dnd-kit/react/sortable";
import { Check, ChevronDown, Circle, GripVertical, Trash2 } from "lucide-react";
import type { AssistantInfo, ProjectStageInfo, StageAssistantInfo } from "../api";
import {
  deleteProjectStage,
  updateProjectStage,
  updateProjectStageAssistants,
} from "../api";
import { useI18n } from "../i18n";
import { projectStageIcon, projectStageLabel } from "../utils/stageDisplay";
import AssistantBotIcon from "./AssistantBotIcon";
import SwitchControl from "./SwitchControl";
import Tooltip from "./Tooltip";

const inputClassName = "h-9 min-w-0 rounded-md border border-input-border/[0.16] bg-input px-3 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";
const textareaClassName = "min-w-0 resize-none rounded-md border border-input-border/[0.16] bg-input px-3 py-2 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";

export default function StageList({
  stages,
  assistants,
  loading,
  dragGroup = "project-stages",
  onUpdated,
  onDeleted,
  onReload,
  onError,
}: {
  stages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  loading: boolean;
  dragGroup?: string;
  onUpdated: (stage: ProjectStageInfo) => void;
  onDeleted: (stageId: string) => void;
  onReload: () => Promise<void>;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const orderedStages = [...stages].sort((a, b) => a.order - b.order);

  const moveStage = async (stage: ProjectStageInfo, direction: -1 | 1) => {
    const index = orderedStages.findIndex((item) => item.id === stage.id);
    const next = orderedStages[index + direction];
    if (!next) return;
    try {
      onUpdated(await updateProjectStage(stage.id, { order: next.order }));
      await onReload();
    } catch (err) {
      onError(String(err));
    }
  };

  const reorderStage = async (stage: ProjectStageInfo, target: ProjectStageInfo) => {
    if (stage.id === target.id) return;
    try {
      onUpdated(await updateProjectStage(stage.id, { order: target.order }));
      await onReload();
    } catch (err) {
      onError(String(err));
    }
  };

  const handleDragEnd = (event: DragEndEvent) => {
    if (event.canceled) return;
    const { source } = event.operation;
    if (!isSortable(source)) return;
    const from = source.initialIndex;
    const to = source.index;
    if (from === to) return;
    const stage = orderedStages[from];
    const target = orderedStages[to];
    if (stage && target) void reorderStage(stage, target);
  };

  return (
    <DragDropProvider onDragEnd={handleDragEnd}>
      <div className="grid gap-3">
        {orderedStages.map((stage, index) => (
          <StageListItem
            key={stage.id}
            stage={stage}
            index={index}
            assistants={assistants}
            dragGroup={dragGroup}
            onMove={moveStage}
            onUpdated={onUpdated}
            onDeleted={onDeleted}
            onError={onError}
          />
        ))}
        {!loading && orderedStages.length === 0 && <div className="rounded-md border border-dashed border-ink/10 py-8 text-center text-body-sm text-ink/35">{t("stage.empty")}</div>}
      </div>
    </DragDropProvider>
  );
}

function StageListItem({
  stage,
  index,
  assistants,
  dragGroup,
  onMove,
  onUpdated,
  onDeleted,
  onError,
}: {
  stage: ProjectStageInfo;
  index: number;
  assistants: AssistantInfo[];
  dragGroup: string;
  onMove: (stage: ProjectStageInfo, direction: -1 | 1) => Promise<void>;
  onUpdated: (stage: ProjectStageInfo) => void;
  onDeleted: (stageId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const { handleRef, isDragSource, isDropTarget, ref } = useSortable({
    id: stage.id,
    index,
    group: dragGroup,
    transition: {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
      idle: true,
    },
  });
  const custom = stage.type === "custom";
  const [name, setName] = useState(stage.name ?? "");
  const [description, setDescription] = useState(stage.description ?? "");
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    setName(stage.name ?? "");
    setDescription(stage.description ?? "");
  }, [stage]);

  const label = projectStageLabel(stage, t);
  const selectedAssistantIds = stage.assistants.map((assistant) => assistant.assistantId);
  const assistantOptions = assistants.map((assistant) => ({
    value: assistant.id,
    label: assistant.name,
    color: assistant.color,
    agentId: assistant.agent.id,
    description: `${assistant.agent.name} · ${assistant.agent.model}`,
  }));

  const save = async () => {
    if (!custom) return;
    try {
      onUpdated(await updateProjectStage(stage.id, { name, description: description || null }));
    } catch (err) {
      onError(String(err));
    }
  };

  const toggleAssistant = async (assistantId: string) => {
    const selected = new Set(selectedAssistantIds);
    if (selected.has(assistantId)) selected.delete(assistantId);
    else selected.add(assistantId);
    try {
      onUpdated(await updateProjectStageAssistants(stage.id, Array.from(selected)));
    } catch (err) {
      onError(String(err));
    }
  };

  const toggleEnabled = async () => {
    try {
      onUpdated(await updateProjectStage(stage.id, { enabled: !stage.enabled }));
    } catch (err) {
      onError(String(err));
    }
  };

  const toggleAllowEmptyAssistants = async () => {
    try {
      onUpdated(await updateProjectStage(stage.id, { allowEmptyAssistants: !stage.allowEmptyAssistants }));
    } catch (err) {
      onError(String(err));
    }
  };

  const remove = async () => {
    if (!custom) return;
    try {
      await deleteProjectStage(stage.id);
      onDeleted(stage.id);
    } catch (err) {
      onError(String(err));
    }
  };

  return (
    <div
      ref={ref}
      data-stage-template-id={stage.id}
      className={
        "relative rounded-lg border p-3 transition duration-150 " +
        (isDragSource
          ? "z-20 cursor-grabbing border-card-border/25 bg-card shadow-[0_16px_36px_rgba(0,0,0,0.24)]"
          : isDropTarget
            ? "border-card-border/45 bg-card-active shadow-[inset_3px_0_0_rgb(var(--color-card-fg)/0.38),0_8px_24px_rgba(0,0,0,0.18)]"
            : "border-card-border/[0.12] bg-card")
      }
    >
      <div className="flex items-start gap-3">
        <button ref={handleRef} type="button" className="mt-1.5 cursor-grab touch-none rounded p-0.5 text-card-subtle/35 hover:bg-card-action-hover/5 hover:text-card-fg/60 active:cursor-grabbing">
          <GripVertical className="h-4 w-4" />
        </button>
        <button type="button" onClick={() => setExpanded((value) => !value)} className="min-w-0 flex-1 text-left">
          <div className="flex min-w-0 items-center gap-2">
            {projectStageIcon(stage, "h-4 w-4 shrink-0 text-card-icon/55")}
            <span className="truncate text-body-sm font-medium text-card-fg/85">{label}</span>
            <span className="rounded bg-card-chip/8 px-1.5 py-0.5 text-meta text-card-chip-fg/55">{stage.type}</span>
          </div>
          {stage.description && <div className="mt-1 line-clamp-2 text-caption leading-relaxed text-card-muted/60">{stage.description}</div>}
          <AssistantSummary assistants={stage.assistants} />
        </button>
        <div className="flex shrink-0 items-center gap-1">
          <button type="button" onClick={() => void onMove(stage, -1)} className="rounded p-1 text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75"><ChevronDown className="h-4 w-4 rotate-180" /></button>
          <button type="button" onClick={() => void onMove(stage, 1)} className="rounded p-1 text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75"><ChevronDown className="h-4 w-4" /></button>
          {custom && (
            <button type="button" onClick={() => void remove()} className="rounded p-1 text-card-subtle/45 hover:bg-status-error/10 hover:text-status-error"><Trash2 className="h-4 w-4" /></button>
          )}
          <StageToggle
            checked={stage.allowEmptyAssistants}
            tooltip={t("stage.allow_empty_assistants")}
            onToggle={() => void toggleAllowEmptyAssistants()}
            variant="icon"
          />
          <StageToggle
            checked={stage.enabled}
            tooltip={stage.enabled ? t("stage.enabled") : t("stage.disabled")}
            onToggle={() => void toggleEnabled()}
            variant="track"
          />
        </div>
      </div>
      {expanded && (
        <div className="mt-3 grid gap-2 border-t border-card-border/10 pt-3">
          {custom && (
            <>
              <input value={name} onChange={(event) => setName(event.target.value)} onBlur={() => void save()} className={inputClassName} />
              <textarea value={description} onChange={(event) => setDescription(event.target.value)} onBlur={() => void save()} rows={3} className={textareaClassName} />
            </>
          )}
          <div>
            <div className="mb-1.5 text-caption text-card-muted/60">{t("assistant.title")}</div>
            <div className="flex flex-wrap gap-1.5">
              {assistantOptions.map((option) => {
                const active = selectedAssistantIds.includes(option.value);
                return (
                  <button key={option.value} type="button" onClick={() => void toggleAssistant(option.value)} className={"inline-flex h-7 items-center gap-1.5 rounded-md border px-2 text-caption transition " + (active ? "border-card-border/[0.22] bg-surface text-card-fg/92" : "border-card-border/[0.10] bg-card-chip/[0.06] text-card-muted/60 hover:border-card-border/[0.16] hover:bg-card-chip/[0.08] hover:text-card-fg")}>
                    {active && <Check className="h-3 w-3 shrink-0" />}
                    <AssistantBotIcon color={option.color} className="h-3.5 w-3.5 shrink-0" />
                    {option.label}
                  </button>
                );
              })}
              {assistantOptions.length === 0 && <span className="text-caption text-card-subtle/55">{t("assistant.empty")}</span>}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function StageToggle({
  checked,
  tooltip,
  onToggle,
  variant,
}: {
  checked: boolean;
  tooltip: string;
  onToggle: () => void;
  variant: "track" | "icon";
}) {
  if (variant === "track") {
    return <SwitchControl checked={checked} tooltip={tooltip} onToggle={onToggle} />;
  }

  return (
    <Tooltip content={tooltip} placement="top">
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        onClick={onToggle}
        className={`rounded p-1 ${checked ? "bg-card-chip/[0.12] text-card-fg/75" : "text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75"}`}
      >
        {checked ? <Check className="h-4 w-4" /> : <Circle className="h-4 w-4" />}
      </button>
    </Tooltip>
  );
}

function AssistantSummary({ assistants }: { assistants: StageAssistantInfo[] }) {
  const { t } = useI18n();
  if (assistants.length === 0) {
    return <div className="mt-1 text-caption text-card-subtle/55">{t("assistant.empty")}</div>;
  }
  return (
    <div className="mt-2 flex flex-wrap gap-1.5">
      {assistants.map((assistant) => (
        <span key={assistant.assistantId} className="inline-flex h-7 items-center rounded-md border border-card-border/[0.22] bg-surface px-2 text-caption text-card-fg/92">
          <AssistantBotIcon color={assistant.color} className="mr-1.5 h-3.5 w-3.5 shrink-0" />
          {assistant.name}
        </span>
      ))}
    </div>
  );
}
