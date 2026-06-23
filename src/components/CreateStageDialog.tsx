import { useState } from "react";
import { createPortal } from "react-dom";
import { Plus, X } from "lucide-react";
import type { ProjectStageInfo } from "../api";
import { createProjectStage } from "../api";
import { useI18n } from "../i18n";
import StageIconPicker from "./StageIconPicker";
import { projectStageIcon } from "../utils/stageDisplay";

const nameInputClassName = "h-full min-w-0 flex-1 bg-transparent text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35";
const textareaClassName = "min-w-0 resize-none rounded-md border border-input-border/[0.16] bg-input px-3 py-2 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";
const actionButtonClassName = "inline-flex h-8 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] hover:text-card-fg/90 disabled:opacity-35 disabled:hover:border-card-border/[0.12] disabled:hover:bg-card-chip/[0.08] disabled:hover:text-card-fg/75";

export default function CreateStageDialog({
  projectId = "",
  processTemplateId = null,
  onCreated,
  onClose,
  onError,
}: {
  projectId?: string;
  processTemplateId?: string | null;
  onCreated: (stage: ProjectStageInfo) => void;
  onClose: () => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [icon, setIcon] = useState<string | null>(null);

  const create = async () => {
    const nextName = name.trim();
    const nextDescription = description.trim();
    if (!nextName) return;
    try {
      onCreated(await createProjectStage(projectId, nextName, nextDescription || null, processTemplateId, icon));
      onClose();
    } catch (err) {
      onError(String(err));
    }
  };

  return createPortal(
    <div className="fixed inset-0 z-[90] flex items-center justify-center bg-black/35 px-4" onClick={onClose}>
      <div className="w-full max-w-[720px] rounded-xl border border-ink/10 bg-surface-panel p-4 shadow-2xl" onClick={(event) => event.stopPropagation()}>
        <div className="mb-4 flex items-center justify-between gap-3">
          <div className="text-body font-medium text-ink">{t("stage.add")}</div>
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
            <div className="text-caption font-medium text-card-muted/60">{t("stage.icon")}</div>
            <StageIconPicker value={icon} onChange={setIcon} />
          </div>
          <label className="flex h-9 min-w-0 items-center gap-2 rounded-md border border-input-border/[0.16] bg-input px-3 text-input-fg focus-within:border-input-focus/30">
            {icon && projectStageIcon({ kind: null, icon }, "h-4 w-4 shrink-0 text-input-fg/65")}
            <input value={name} onChange={(event) => setName(event.target.value)} placeholder={t("stage.name")} className={nameInputClassName} />
          </label>
          <textarea value={description} onChange={(event) => setDescription(event.target.value)} placeholder={t("stage.description")} rows={3} className={textareaClassName} />
          <div className="flex justify-end gap-2">
            <button type="button" onClick={onClose} className="rounded-md px-3 py-1.5 text-body-sm text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
            <button type="button" onClick={() => void create()} disabled={!name.trim()} className={actionButtonClassName}>
              <Plus className="h-4 w-4" />
              {t("stage.add")}
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}
