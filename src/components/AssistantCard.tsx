import { useEffect, useState } from "react";
import { Pencil, Trash2 } from "lucide-react";
import type { AgentInfo, AssistantInfo } from "../api";
import { deleteAssistant, updateAssistant } from "../api";
import type { AssistantAgentInfo } from "../api";
import AssistantAgentSelector from "./AssistantAgentSelector";
import AssistantBotIcon from "./AssistantBotIcon";
import SwitchControl from "./SwitchControl";
import Tooltip from "./Tooltip";
import { useI18n } from "../i18n";

const inputClassName = "h-9 min-w-0 rounded-md border border-input-border/[0.16] bg-input px-3 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";
const textareaClassName = "min-w-0 resize-none rounded-md border border-input-border/[0.16] bg-input px-3 py-2 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";

export default function AssistantCard({
  assistant,
  agents,
  sidebarMode = false,
  onUpdated,
  onDeleted,
  onError,
}: {
  assistant: AssistantInfo;
  agents: AgentInfo[];
  sidebarMode?: boolean;
  onUpdated: (assistant: AssistantInfo) => void;
  onDeleted: (assistantId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const deletable = assistant.type !== "builtin";
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(assistant.name);
  const [systemPrompt, setSystemPrompt] = useState(assistant.systemPrompt ?? "");

  useEffect(() => {
    setName(assistant.name);
    setSystemPrompt(assistant.systemPrompt ?? "");
  }, [assistant]);

  const save = async () => {
    try {
      onUpdated(await updateAssistant(assistant.id, { name, systemPrompt }));
      setEditing(false);
    } catch (err) {
      onError(String(err));
    }
  };

  const remove = async () => {
    if (!deletable) return;
    try {
      await deleteAssistant(assistant.id);
      onDeleted(assistant.id);
    } catch (err) {
      onError(String(err));
    }
  };

  const toggleEnabled = async () => {
    try {
      onUpdated(await updateAssistant(assistant.id, { enabled: !assistant.enabled }));
    } catch (err) {
      onError(String(err));
    }
  };

  const updateAgent = async (agent: AssistantAgentInfo) => {
    try {
      onUpdated(await updateAssistant(assistant.id, { agent }));
    } catch (err) {
      onError(String(err));
    }
  };

  return (
    <div className={`rounded-lg border border-card-border/[0.12] p-3 ${sidebarMode ? "bg-ink/[0.025]" : "bg-card"} ${assistant.enabled ? "" : "opacity-45"}`}>
      {editing ? (
        <div className="grid gap-2">
          <input value={name} onChange={(event) => setName(event.target.value)} className={inputClassName} />
          <textarea value={systemPrompt} onChange={(event) => setSystemPrompt(event.target.value)} rows={4} className={textareaClassName} />
          <div className="flex justify-end gap-2">
            <button type="button" onClick={() => setEditing(false)} className="rounded-md px-3 py-1.5 text-body-sm text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
            <button type="button" onClick={() => void save()} className="rounded-md bg-ink px-3 py-1.5 text-body-sm text-[rgb(var(--color-bg-panel))]">{t("project.save")}</button>
          </div>
        </div>
      ) : (
        <div className="flex items-start gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center gap-2">
              <AssistantBotIcon color={assistant.color} className="h-4 w-4 shrink-0 text-card-icon/55" />
              <span className="truncate text-body-sm font-medium text-card-fg/75">{assistant.name}</span>
              <span className="shrink-0 rounded bg-card-chip/8 px-1.5 py-0.5 text-meta text-card-chip-fg/55">{assistant.type}</span>
            </div>
            {assistant.systemPrompt && <div className="ml-6 mt-2 line-clamp-3 whitespace-pre-wrap text-caption leading-relaxed text-card-muted/60">{assistant.systemPrompt}</div>}
            <div className="ml-6 mt-2 flex min-w-0 max-w-full">
              <AssistantAgentSelector agent={assistant.agent} agents={agents} onChange={(agent) => void updateAgent(agent)} />
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            <Tooltip content={t("assistant.edit")} placement="top">
              <button type="button" onClick={() => setEditing(true)} className="rounded p-1 text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75"><Pencil className="h-4 w-4" /></button>
            </Tooltip>
            <SwitchControl
              checked={assistant.enabled}
              tooltip={assistant.enabled ? t("assistant.disable") : t("assistant.enable")}
              onToggle={() => void toggleEnabled()}
            />
            {deletable && (
              <button type="button" onClick={() => void remove()} className="rounded p-1 text-card-subtle/45 hover:bg-status-error/10 hover:text-status-error"><Trash2 className="h-4 w-4" /></button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
