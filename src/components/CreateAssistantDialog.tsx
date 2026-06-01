import { useEffect, useMemo, useState } from "react";
import { Plus } from "lucide-react";
import type { AgentInfo, AssistantAgentInfo, AssistantInfo, AssistantType } from "../api";
import { createAssistant } from "../api";
import { useI18n } from "../i18n";
import AssistantAgentSelector, { dbAgentsAsRuntimeAgents, defaultAssistantAgent } from "./AssistantAgentSelector";

const inputClassName = "h-9 min-w-0 rounded-md border border-input-border/[0.16] bg-input px-3 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";
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
        type: "custom" satisfies AssistantType,
        projectId,
      });
      onCreated(assistant);
      onClose();
    } catch (err) {
      onError(String(err));
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 px-4" onClick={onClose}>
      <div className="w-full max-w-[520px] rounded-lg border border-card-border/[0.12] bg-surface-panel p-4 shadow-[0_24px_80px_rgba(0,0,0,0.22)]" onClick={(event) => event.stopPropagation()}>
        <div className="mb-3 text-body-sm font-semibold text-ink/[0.88]">{t("assistant.add")}</div>
        <div className="grid gap-2">
          <input value={name} onChange={(event) => setName(event.target.value)} placeholder={t("assistant.name")} className={inputClassName} />
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
    </div>
  );
}
