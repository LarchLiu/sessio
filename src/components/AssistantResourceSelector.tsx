import { useMemo, type ReactNode } from "react";
import type { McpServerConfig, SkillMetadata } from "../api";
import { useI18n } from "../i18n";
import { useSelectableMcpServers } from "../hooks/useSelectableMcpServers";
import { useSelectableSkills } from "../hooks/useSelectableSkills";
import { getSessioPromptMarkers } from "../promptMarkers";

const sectionClassName = "rounded-lg border border-card-border/[0.12] bg-card-chip/[0.04] p-2.5";
const optionClassName = "flex min-w-0 items-start gap-2 rounded-md px-2 py-1.5 text-left transition hover:bg-card-chip/[0.08]";
const SESSIO_PROMPT_MARKERS = getSessioPromptMarkers();

export default function AssistantResourceSelector({
  selectedSkillIds,
  selectedMcpIds,
  onSelectedSkillIdsChange,
  onSelectedMcpIdsChange,
}: {
  selectedSkillIds: string[];
  selectedMcpIds: string[];
  onSelectedSkillIdsChange: (ids: string[]) => void;
  onSelectedMcpIdsChange: (ids: string[]) => void;
}) {
  const { t } = useI18n();
  const { availableSkills } = useSelectableSkills();
  const { availableMcpServers } = useSelectableMcpServers(null, {
    filterByCapabilities: false,
  });
  const systemSkills = useMemo(
    () => availableSkills.filter((skill) => skill.source === "builtin"),
    [availableSkills],
  );
  const personalSkills = useMemo(
    () => availableSkills.filter((skill) => skill.source !== "builtin"),
    [availableSkills],
  );
  const builtinMcpServers = useMemo(
    () => availableMcpServers.filter((server) => server.source === SESSIO_PROMPT_MARKERS.mcpSourceBuiltin),
    [availableMcpServers],
  );
  const customMcpServers = useMemo(
    () => availableMcpServers.filter((server) => server.source === SESSIO_PROMPT_MARKERS.mcpSourceCustom),
    [availableMcpServers],
  );

  return (
    <div className="grid gap-2">
      <ResourceSection
        title={t("assistant.default_skills")}
        empty={t("assistant.no_skills")}
        isEmpty={availableSkills.length === 0}
      >
        <SkillGroup
          label={t("new_chat.skills_system")}
          skills={systemSkills}
          selectedIds={selectedSkillIds}
          onChange={onSelectedSkillIdsChange}
        />
        <SkillGroup
          label={t("new_chat.skills_personal")}
          skills={personalSkills}
          selectedIds={selectedSkillIds}
          onChange={onSelectedSkillIdsChange}
        />
      </ResourceSection>
      <ResourceSection
        title={t("assistant.default_mcps")}
        empty={t("assistant.no_mcps")}
        isEmpty={availableMcpServers.length === 0}
      >
        <McpGroup
          label={t("new_chat.mcps_builtin")}
          servers={builtinMcpServers}
          selectedIds={selectedMcpIds}
          onChange={onSelectedMcpIdsChange}
        />
        <McpGroup
          label={t("new_chat.mcps_custom")}
          servers={customMcpServers}
          selectedIds={selectedMcpIds}
          onChange={onSelectedMcpIdsChange}
        />
      </ResourceSection>
    </div>
  );
}

function ResourceSection({
  title,
  empty,
  isEmpty,
  children,
}: {
  title: string;
  empty: string;
  isEmpty?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={sectionClassName}>
      <div className="mb-1.5 text-caption font-medium text-card-muted/60">{title}</div>
      {isEmpty ? (
        <div className="px-2 py-1.5 text-caption text-card-subtle/50">{empty}</div>
      ) : (
        <div className="grid gap-1">{children}</div>
      )}
    </div>
  );
}

function SkillGroup({
  label,
  skills,
  selectedIds,
  onChange,
}: {
  label: string;
  skills: SkillMetadata[];
  selectedIds: string[];
  onChange: (ids: string[]) => void;
}) {
  if (skills.length === 0) return null;
  return (
    <div className="grid gap-1">
      <div className="px-2 pt-1 text-caption font-medium text-card-subtle/50">{label}</div>
      {skills.map((skill) => (
        <SkillOption
          key={skill.id}
          skill={skill}
          checked={selectedIds.includes(skill.id)}
          onToggle={() => onChange(toggleId(selectedIds, skill.id))}
        />
      ))}
    </div>
  );
}

function SkillOption({
  skill,
  checked,
  onToggle,
}: {
  skill: SkillMetadata;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <label className={optionClassName}>
      <input type="checkbox" checked={checked} onChange={onToggle} className="mt-0.5 h-3.5 w-3.5 accent-ink" />
      <span className="min-w-0">
        <span className="block truncate text-body-sm text-card-fg/75">{skill.name}</span>
        {skill.description && (
          <span className="block line-clamp-2 text-caption text-card-muted/55">{skill.description}</span>
        )}
      </span>
    </label>
  );
}

function McpGroup({
  label,
  servers,
  selectedIds,
  onChange,
}: {
  label: string;
  servers: McpServerConfig[];
  selectedIds: string[];
  onChange: (ids: string[]) => void;
}) {
  if (servers.length === 0) return null;
  return (
    <div className="grid gap-1">
      <div className="px-2 pt-1 text-caption font-medium text-card-subtle/50">{label}</div>
      {servers.map((server) => (
        <McpOption
          key={server.id}
          server={server}
          checked={selectedIds.includes(server.id)}
          onToggle={() => onChange(toggleId(selectedIds, server.id))}
        />
      ))}
    </div>
  );
}

function McpOption({
  server,
  checked,
  onToggle,
}: {
  server: McpServerConfig;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <label className={optionClassName}>
      <input type="checkbox" checked={checked} onChange={onToggle} className="mt-0.5 h-3.5 w-3.5 accent-ink" />
      <span className="min-w-0">
        <span className="block truncate text-body-sm text-card-fg/75">{server.name}</span>
        {server.description && (
          <span className="block line-clamp-3 whitespace-pre-wrap text-caption text-card-muted/55">
            {server.description}
          </span>
        )}
        <span className="block text-caption text-card-muted/45">{server.transport}</span>
      </span>
    </label>
  );
}

function toggleId(ids: string[], id: string): string[] {
  return ids.includes(id)
    ? ids.filter((current) => current !== id)
    : [...ids, id];
}
