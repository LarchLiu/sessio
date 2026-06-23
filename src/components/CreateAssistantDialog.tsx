import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { Plus, X } from "lucide-react";
import type { AgentInfo, AssistantAgentInfo, AssistantInfo, AssistantType } from "../api";
import { createAssistant } from "../api";
import { useI18n } from "../i18n";
import AssistantAgentSelector, { dbAgentsAsRuntimeAgents, defaultAssistantAgent } from "./AssistantAgentSelector";
import AssistantColorPicker from "./AssistantColorPicker";
import AssistantBotIcon from "./AssistantBotIcon";

const nameInputClassName = "h-full min-w-0 flex-1 bg-transparent text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35";
const textareaClassName = "min-w-0 resize-none rounded-md border border-input-border/[0.16] bg-input px-3 py-2 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";
const actionButtonClassName = "inline-flex h-8 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] hover:text-card-fg/90 disabled:opacity-35 disabled:hover:border-card-border/[0.12] disabled:hover:bg-card-chip/[0.08] disabled:hover:text-card-fg/75";

export default function CreateAssistantDialog({
  agents,
  projectId = null,
  onCreated,
  onClose,
  onError,
}: {
  agents: AgentInfo[];
  projectId?: string | null;
  onCreated: (assistant: AssistantInfo) => void;
  onClose: () => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const runtimeAgents = useMemo(() => dbAgentsAsRuntimeAgents(agents), [agents]);
  const [name, setName] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [color, setColor] = useState<string | null>(null);
  const [agentDraft, setAgentDraft] = useState<AssistantAgentInfo>(() => defaultAssistantAgent(runtimeAgents[0] ?? null));

  useEffect(() => {
    if (runtimeAgents.some((agent) => agent.agent === agentDraft.id)) return;
    if (runtimeAgents[0]) setAgentDraft(defaultAssistantAgent(runtimeAgents[0]));
  }, [agentDraft.id, runtimeAgents]);

  const create = async () => {
    const nextName = name.trim();
    if (!nextName || !agentDraft.id) return;
    try {
      const assistant = await createAssistant({
        name: nextName,
        agent: agentDraft,
        systemPrompt,
        color,
        type: "custom" satisfies AssistantType,
        projectId,
      });
      onCreated(assistant);
      onClose();
    } catch (err) {
      onError(String(err));
    }
  };

  return createPortal(
    <div className="fixed inset-0 z-[90] flex items-center justify-center bg-black/35 px-4" onClick={onClose}>
      <div className="w-full max-w-[720px] rounded-xl border border-ink/10 bg-surface-panel p-4 shadow-2xl" onClick={(event) => event.stopPropagation()}>
        <div className="mb-4 flex items-center justify-between gap-3">
          <div className="text-body font-medium text-ink">{t("assistant.add")}</div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-md p-1 text-ink/45 hover:bg-ink/5 hover:text-ink"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <div className="text-caption font-medium text-card-muted/60">{t("assistant.color")}</div>
            <AssistantColorPicker value={color} onChange={setColor} />
          </div>
          <label className="flex h-9 min-w-0 items-center gap-2 rounded-md border border-input-border/[0.16] bg-input px-3 text-input-fg focus-within:border-input-focus/30">
            {color && <AssistantBotIcon color={color} className="h-4 w-4 shrink-0" />}
            <input value={name} onChange={(event) => setName(event.target.value)} placeholder={t("assistant.name")} className={nameInputClassName} />
          </label>
          <textarea value={systemPrompt} onChange={(event) => setSystemPrompt(event.target.value)} placeholder={t("assistant.system_prompt")} rows={4} className={textareaClassName} />
          <AssistantAgentSelector agent={agentDraft} agents={agents} onChange={setAgentDraft} />
          <div className="flex justify-end gap-2">
            <button type="button" onClick={onClose} className="rounded-md px-3 py-1.5 text-body-sm text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
            <button type="button" onClick={() => void create()} disabled={!name.trim() || !agentDraft.id} className={actionButtonClassName}>
              <Plus className="h-4 w-4" />
              {t("assistant.add")}
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}
