import { type ReactNode, useEffect, useMemo, useState } from "react";
import { DragDropProvider, type DragEndEvent } from "@dnd-kit/react";
import { isSortable, useSortable } from "@dnd-kit/react/sortable";
import AiGenerate2Icon from '@iconify-react/ri/ai-generate-2';
import Robot3LineIcon from '@iconify-react/ri/robot-3-line';
import { ArrowLeft, Check, Circle, Download, GripVertical, Info, Languages, LoaderCircle, Monitor, Moon, Pencil, Plus, RefreshCw, RotateCcw, Search, Settings2, Sun, Trash2, Workflow } from "lucide-react";
import type { Agent, AgentAiProviderInfo, AgentInfo, AssistantInfo, ProjectStageInfo, RuntimeAgentOptionMetadata, WorkflowInfo } from "../api";
import {
  createWorkflow,
  listAgents,
  listAssistants,
  listWorkflowStages,
  listWorkflows,
  updateAgentPreferences,
  updateRuntimeAgentPreferences,
} from "../api";
import CreateAssistantDialog from "../components/CreateAssistantDialog";
import CreateStageDialog from "../components/CreateStageDialog";
import AssistantCard from "../components/AssistantCard";
import { AgentGlyph } from "../components/AgentIcon";
import InlineMenuSelect from "../components/InlineMenuSelect";
import StageList from "../components/StageList";
import { runtimePermissionModeOptions } from "../components/RuntimeMenuSelect";
import ScrollArea from "../components/ScrollArea";
import SegmentedTabs from "../components/SegmentedTabs";
import SwitchControl from "../components/SwitchControl";
import Tooltip from "../components/Tooltip";
import { type Lang, useI18n } from "../i18n";
import type { ThemeMode } from "../theme";
import { formatVersionLabel, type UpdateState } from "../updater";
import acpMarkBlackUrl from "../../assets/acp_mark-black.svg?url";
import acpMarkWhiteUrl from "../../assets/acp_mark-white.svg?url";

type SettingsSection = "general" | "agents" | "assistants" | "workflows";

type AgentPreferencePatch = {
  displayName?: string | null;
  enabled?: boolean;
  order?: number;
  aiProvider?: string | null;
  aiProviders?: AgentAiProviderInfo[];
  model?: string | null;
  effort?: string | null;
  permissionMode?: string | null;
  models?: RuntimeAgentOptionMetadata[];
};

export default function SettingsPage({
  lang,
  onLangChange,
  themeMode,
  onThemeModeChange,
  rebuilding,
  indexing,
  onBack,
  onError,
  onRebuildFinished,
  appVersion,
  update,
  onOpenUpdate,
}: {
  lang: Lang;
  onLangChange: (lang: Lang) => void;
  themeMode: ThemeMode;
  onThemeModeChange: (mode: ThemeMode) => void;
  rebuilding: boolean;
  indexing: boolean;
  onBack: () => void;
  onError: (error: string | null) => void;
  onRebuildFinished: () => Promise<void> | void;
  appVersion: string;
  update: UpdateState;
  onOpenUpdate: () => void;
}) {
  const { t } = useI18n();
  const [section, setSection] = useState<SettingsSection>("general");

  const navItems = [
    { id: "general" as const, label: t("settings.general"), icon: Settings2 },
    { id: "agents" as const, label: t("agent.title"), icon: AiGenerate2Icon },
    { id: "assistants" as const, label: t("assistant.title"), icon: Robot3LineIcon },
    { id: "workflows" as const, label: t("settings.workflows"), icon: Workflow },
  ];
  const sectionTitle = navItems.find((item) => item.id === section)?.label ?? t("settings.general");

  return (
    <div className="flex h-screen text-body text-ink">
      <aside className="flex w-[300px] shrink-0 flex-col bg-surface-sidebar">
        <header data-tauri-drag-region className="h-12 shrink-0 select-none bg-surface-sidebar" />
        <div className="min-h-0 flex-1 px-2">
          <button
            type="button"
            onClick={onBack}
            data-tauri-drag-region="false"
            className="mb-3 flex h-8 w-full items-center gap-2 rounded-md px-2.5 text-left text-body-sm text-ink/55 transition hover:bg-ink/5 hover:text-ink"
          >
            <ArrowLeft className="h-4 w-4" />
            {t("settings.back_to_app")}
          </button>
          <nav data-tauri-drag-region="false" className="grid gap-1">
            {navItems.map(({ id, label, icon: Icon }) => {
              const active = section === id;
              return (
                <button
                  key={id}
                  type="button"
                  onClick={() => setSection(id)}
                  className={
                  "flex h-8 items-center gap-2 rounded-md px-2.5 text-left text-body-sm transition " +
                  (active ? "bg-ink/[0.065] text-ink/88" : "text-ink/72 hover:bg-ink/5 hover:text-ink")
                  }
                >
                  <Icon className="h-4 w-4 shrink-0" />
                  <span className="truncate">{label}</span>
                </button>
              );
            })}
          </nav>
        </div>
      </aside>
      <main className="relative flex min-w-0 flex-1 flex-col">
        <header data-tauri-drag-region className="grid h-12 shrink-0 select-none grid-cols-3 items-center border-b border-ink/[0.12] bg-surface px-5">
          <h1 data-tauri-drag-region className="col-start-2 justify-self-center truncate text-title font-semibold text-ink/85">{sectionTitle}</h1>
        </header>
        <ScrollArea className="min-h-0 flex-1 bg-surface-panel" viewportClassName={"px-10 pt-6 " + (section === "workflows" ? "pb-6" : "pb-16")}>
          <div className="mx-auto max-w-[840px]">
            {section === "general" && (
              <GeneralSettings
                lang={lang}
                themeMode={themeMode}
                rebuilding={rebuilding}
                indexing={indexing}
                onLangChange={onLangChange}
                onThemeModeChange={onThemeModeChange}
                onError={onError}
                onRebuildFinished={onRebuildFinished}
                appVersion={appVersion}
                update={update}
                onOpenUpdate={onOpenUpdate}
              />
            )}
            {section === "agents" && <AgentsSettings onError={onError} />}
            {section === "assistants" && <AssistantsSettings onError={onError} />}
            {section === "workflows" && <WorkflowsSettings onError={onError} />}
          </div>
        </ScrollArea>
      </main>
    </div>
  );
}

function AboutGroup({
  appVersion,
  lang,
  update,
  onOpenUpdate,
}: {
  appVersion: string;
  lang: Lang;
  update: UpdateState;
  onOpenUpdate: () => void;
}) {
  const { t } = useI18n();
  const status = update.updateReady
    ? t("settings.update_ready")
    : update.hasUpdate && update.latestVersion
      ? t("settings.update_available_version", { version: formatVersionLabel(update.latestVersion) })
      : update.checking
        ? t("settings.update_checking")
        : t("settings.update_up_to_date");
  const actionLabel = update.updateReady
    ? t("update_dialog.restart_now")
    : update.canInstall
      ? t("update_dialog.update_now")
      : t("update_dialog.download_now");
  const ActionIcon = update.updateReady ? RotateCcw : Download;

  return (
    <SettingsGroup title={t("settings.about")} flush>
      <SettingsRow icon={<Info className="h-4 w-4" />} label={t("settings.about")} description={t("settings.about_description")}>
        <div className="text-right text-caption text-ink/58">
          <div className="font-semibold text-ink/78">{formatVersionLabel(appVersion)}</div>
          <div>{status}</div>
        </div>
      </SettingsRow>
      <SettingsRow
        icon={<RefreshCw className="h-4 w-4" />}
        label={t("settings.update_status")}
        description={t("settings.last_checked_value", {
          value: update.lastCheckedAt ? formatSettingsDate(update.lastCheckedAt, lang) : t("settings.never"),
        })}
      >
        <div className="flex flex-wrap items-center justify-end gap-2">
          {(update.hasUpdate || update.updateReady) && update.latestVersion && (
            <button
              type="button"
              disabled={update.installing}
              onClick={onOpenUpdate}
              className="inline-flex h-8 items-center gap-2 rounded-md bg-blue px-3 text-body-sm font-medium text-white outline-none transition hover:bg-blue/92 focus-visible:ring-2 focus-visible:ring-blue/45 disabled:opacity-55"
            >
              {update.installing ? (
                <LoaderCircle className="h-4 w-4 animate-spin" />
              ) : (
                <ActionIcon className="h-4 w-4" />
              )}
              {actionLabel}
            </button>
          )}
          <button
            type="button"
            disabled={update.checking || update.installing}
            onClick={update.check}
            className="inline-flex h-8 items-center gap-2 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-45"
          >
            <RefreshCw className={"h-4 w-4 " + (update.checking ? "animate-spin" : "")} />
            {t("settings.check_for_updates")}
          </button>
        </div>
      </SettingsRow>
    </SettingsGroup>
  );
}

function formatSettingsDate(ts: number, lang: Lang): string {
  return new Intl.DateTimeFormat(lang === "zh" ? "zh-CN" : "en-US", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(ts));
}

function GeneralSettings({
  lang,
  themeMode,
  rebuilding,
  indexing,
  appVersion,
  update,
  onLangChange,
  onThemeModeChange,
  onError,
  onRebuildFinished,
  onOpenUpdate,
}: {
  lang: Lang;
  themeMode: ThemeMode;
  rebuilding: boolean;
  indexing: boolean;
  appVersion: string;
  update: UpdateState;
  onLangChange: (lang: Lang) => void;
  onThemeModeChange: (mode: ThemeMode) => void;
  onError: (error: string | null) => void;
  onRebuildFinished: () => Promise<void> | void;
  onOpenUpdate: () => void;
}) {
  const { t } = useI18n();
  return (
    <section className="min-w-0 max-w-full">
      <SettingsGroup title={t("settings.appearance")} flush>
        <SettingsRow icon={<Languages className="h-4 w-4" />} label={t("sidebar.language")} description={t("settings.language_description")}>
          <SegmentedControl
            value={lang}
            options={[
              { value: "en", label: "EN" },
              { value: "zh", label: "中" },
            ]}
            onChange={(value) => onLangChange(value as Lang)}
          />
        </SettingsRow>
        <SettingsRow icon={<Monitor className="h-4 w-4" />} label={t("sidebar.theme")} description={t("settings.theme_description")}>
          <ThemeSelector mode={themeMode} onChange={onThemeModeChange} />
        </SettingsRow>
      </SettingsGroup>
      <SettingsGroup title={t("settings.index")} flush>
        <SettingsRow icon={<RefreshCw className="h-4 w-4" />} label={t("sidebar.rebuild_index")} description={t("settings.rebuild_description")}>
          <button
            type="button"
            disabled={indexing}
            onClick={() => {
              import("../api")
                .then(({ rebuildSessionIndex }) => rebuildSessionIndex())
                .catch((err) => onError(String(err)))
                .finally(() => {
                  void onRebuildFinished();
                });
            }}
            className="inline-flex h-8 items-center gap-2 rounded-md border border-ink/10 bg-ink/[0.035] px-3 text-body-sm text-ink/70 transition hover:bg-ink/[0.06] hover:text-ink disabled:opacity-45"
          >
            <RefreshCw className={"h-3.5 w-3.5 " + (rebuilding ? "animate-spin" : "")} />
            {indexing ? t("sidebar.status_indexing") : t("sidebar.rebuild_index")}
          </button>
        </SettingsRow>
      </SettingsGroup>
      <AboutGroup
        appVersion={appVersion}
        lang={lang}
        update={update}
        onOpenUpdate={onOpenUpdate}
      />
    </section>
  );
}

function AgentsSettings({ onError }: { onError: (error: string | null) => void }) {
  const { t } = useI18n();
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [selectedAgentId, setSelectedAgentId] = useState<string>("codex");
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const builtinAgents = useMemo(
    () => agents.filter((agent) => agent.type === "builtin" && isSettingsAgent(agent.id)),
    [agents],
  );
  const filteredAgents = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) return builtinAgents;
    return builtinAgents.filter((agent) =>
      `${agent.displayName} ${agent.name} ${agent.id} ${agent.model ?? ""}`.toLowerCase().includes(query),
    );
  }, [builtinAgents, search]);
  const selectedAgent =
    builtinAgents.find((agent) => agent.id === selectedAgentId) ?? builtinAgents[0] ?? null;

  const reload = async () => {
    setLoading(true);
    try {
      const rows = await listAgents();
      setAgents(rows);
      setSelectedAgentId((current) => {
        const currentExists = rows.some((agent) => agent.id === current && isSettingsAgent(agent.id));
        return currentExists ? current : rows.find((agent) => isSettingsAgent(agent.id))?.id ?? current;
      });
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const updateAgent = (next: AgentInfo) => {
    setAgents((prev) => prev.map((agent) => agent.id === next.id ? next : agent));
  };

  const moveAgent = async (from: number, to: number) => {
    const nextAgents = moveAgentOption(builtinAgents, from, to).map((agent, index) => ({ ...agent, order: index }));
    setAgents((prev) => prev.map((agent) => nextAgents.find((item) => item.id === agent.id) ?? agent));
    try {
      await Promise.all(nextAgents.map((agent) => updateAgentPreferences({
        agentId: agent.id,
        order: agent.order,
      })));
      const rows = await listAgents();
      setAgents(rows);
    } catch (err) {
      onError(String(err));
    }
  };

  const handleAgentDragEnd = (event: DragEndEvent) => {
    if (event.canceled || search.trim()) return;
    const { source } = event.operation;
    if (!isSortable(source)) return;
    if (source.initialIndex === source.index) return;
    void moveAgent(source.initialIndex, source.index);
  };

  return (
    <section>
      <div className="grid grid-cols-[240px_minmax(0,1fr)] gap-5">
        <div className="min-w-0">
          <div className="mb-8">
            <h2 className="mb-3 text-body-sm font-semibold text-ink/[0.88]">{t("agent.title")}</h2>
            <label className="mb-3 flex h-9 items-center gap-2 rounded-md border border-input-border/[0.12] bg-input px-2 text-input-fg">
              <Search className="h-3.5 w-3.5 shrink-0 text-input-placeholder/45" />
              <input
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder={t("agent.search")}
                className="min-w-0 flex-1 bg-transparent text-body-sm outline-none placeholder:text-input-placeholder/35"
              />
            </label>
            <div className="overflow-hidden rounded-lg border border-card-border/[0.12] bg-card">
              <DragDropProvider onDragEnd={handleAgentDragEnd}>
                <div className="divide-y divide-card-border/10">
                  {filteredAgents.map((agent, index) => (
                    <AgentListRow
                      key={agent.id}
                      agent={agent}
                      index={index}
                      active={agent.id === selectedAgent?.id}
                      draggable={!search.trim()}
                      onSelect={setSelectedAgentId}
                    />
                  ))}
                  {!loading && filteredAgents.length === 0 && <div className="p-3"><EmptyState label={t("agent.empty")} /></div>}
                </div>
              </DragDropProvider>
            </div>
          </div>
        </div>
        <div className="min-w-0">
          {selectedAgent ? (
            <AgentEditor
              key={selectedAgent.id}
              agent={selectedAgent}
              onUpdated={updateAgent}
              onError={onError}
            />
          ) : (
            !loading && <EmptyState label={t("agent.empty")} />
          )}
        </div>
      </div>
    </section>
  );
}

function AgentListRow({
  agent,
  index,
  active,
  draggable,
  onSelect,
}: {
  agent: AgentInfo;
  index: number;
  active: boolean;
  draggable: boolean;
  onSelect: (agentId: string) => void;
}) {
  const { t } = useI18n();
  const { handleRef, isDragSource, isDropTarget, ref } = useSortable({
    id: agent.id,
    index,
    group: "settings-agents",
    transition: {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
      idle: true,
    },
  });
  return (
    <div
      ref={ref}
      className={
        "workflow-list-item flex h-12 w-full min-w-0 items-center gap-2 px-2 text-left text-body-sm transition " +
        (active ? "workflow-list-item-active " : "") +
        (isDragSource ? "z-20 cursor-grabbing bg-card shadow-[0_12px_28px_rgba(0,0,0,0.22)] " : "") +
        (isDropTarget ? "bg-card-active shadow-[inset_3px_0_0_rgb(var(--color-card-fg)/0.32)] " : "")
      }
    >
      <button
        ref={handleRef}
        type="button"
        disabled={!draggable}
        className="cursor-grab touch-none rounded p-0.5 text-card-subtle/35 hover:bg-card-action-hover/5 hover:text-card-fg/60 active:cursor-grabbing disabled:cursor-default disabled:opacity-25"
      >
        <GripVertical className="h-4 w-4" />
      </button>
      <button type="button" onClick={() => onSelect(agent.id)} className="flex min-w-0 flex-1 items-center gap-2 text-left">
        <SettingsAgentGlyph agentId={agent.id} className="h-4 w-4 shrink-0" />
        <span className="min-w-0 flex-1">
          <span className="flex min-w-0 items-center gap-1.5">
            <span className="truncate font-medium text-card-fg/78">{agent.displayName}</span>
            {agent.transport === "acp" && <AcpLogo className="h-2 w-auto shrink-0 opacity-70" />}
          </span>
          <span className="mt-0.5 block truncate text-meta text-card-muted/45">{agent.model || t("agent.no_model")}</span>
        </span>
        <span className={"h-1.5 w-1.5 rounded-full " + (agent.enabled ? "bg-brand" : "bg-ink/20")} />
      </button>
    </div>
  );
}

function AcpLogo({ className }: { className?: string }) {
  const theme = useEffectiveThemeType();
  return (
    <Tooltip content="ACP" placement="top">
      <span className="inline-flex shrink-0 items-center">
        <img src={theme === "light" ? acpMarkBlackUrl : acpMarkWhiteUrl} alt="ACP" className={className} draggable={false} />
      </span>
    </Tooltip>
  );
}

function useEffectiveThemeType(): "light" | "dark" {
  const [themeType, setThemeType] = useState<"light" | "dark">(() =>
    document.documentElement.getAttribute("data-theme") === "light" ? "light" : "dark",
  );

  useEffect(() => {
    const root = document.documentElement;
    const update = () => {
      setThemeType(root.getAttribute("data-theme") === "light" ? "light" : "dark");
    };
    update();
    const observer = new MutationObserver(update);
    observer.observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    return () => observer.disconnect();
  }, []);

  return themeType;
}

function AgentEditor({
  agent,
  onUpdated,
  onError,
}: {
  agent: AgentInfo;
  onUpdated: (agent: AgentInfo) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [model, setModel] = useState(agent.model ?? agent.models.find((item) => item.enabled)?.value ?? "");
  const [effort, setEffort] = useState(agent.effort ?? agent.efforts[0]?.value ?? "");
  const [permissionMode, setPermissionMode] = useState(agent.permissionMode ?? agent.permissionModes[0]?.value ?? "");
  const [models, setModels] = useState<RuntimeAgentOptionMetadata[]>(agent.models);
  const [newModelValue, setNewModelValue] = useState("");
  const [newModelDisplayName, setNewModelDisplayName] = useState("");
  const runtimeAgent = isRuntimeAgent(agent.id) ? agent.id : null;
  const isAstra = agent.id === "astra";
  const [aiProvider, setAiProvider] = useState(agent.aiProvider ?? "");
  const [aiProviders, setAiProviders] = useState<AgentAiProviderInfo[]>(agent.aiProviders);
  const [providerDialog, setProviderDialog] = useState<{ mode: "add" | "edit"; provider: AgentAiProviderInfo } | null>(null);
  const selectedAiProvider = aiProviders.find((provider) => provider.id === aiProvider) ?? aiProviders.find((provider) => provider.enabled) ?? aiProviders[0] ?? null;

  useEffect(() => {
    setModel(agent.model ?? agent.models.find((item) => item.enabled)?.value ?? "");
    setEffort(agent.effort ?? agent.efforts[0]?.value ?? "");
    setPermissionMode(agent.permissionMode ?? agent.permissionModes[0]?.value ?? "");
    setModels(agent.models);
    setNewModelValue("");
    setNewModelDisplayName("");
    setAiProvider(agent.aiProvider ?? "");
    setAiProviders(agent.aiProviders);
    setProviderDialog(null);
  }, [agent]);

  const persist = async (patch: AgentPreferencePatch) => {
    try {
      let next: AgentInfo | undefined;
      if (runtimeAgent) {
        const runtimePatch = {
          displayName: patch.displayName,
          enabled: patch.enabled,
          order: patch.order,
          model: patch.model,
          effort: patch.effort,
          permissionMode: patch.permissionMode,
          models: patch.models,
        };
        await updateRuntimeAgentPreferences({
          agent: runtimeAgent,
          ...runtimePatch,
        });
        const rows = await listAgents();
        next = rows.find((item) => item.id === agent.id);
      } else {
        next = await updateAgentPreferences({
          agentId: agent.id,
          ...patch,
        });
      }
      if (next) onUpdated(next);
    } catch (err) {
      onError(String(err));
    }
  };

  const selectModel = async (nextModel: string) => {
    setModel(nextModel);
    await persist({ model: nextModel });
  };

  const selectEffort = async (nextEffort: string) => {
    setEffort(nextEffort);
    await persist({ effort: nextEffort });
  };

  const selectPermissionMode = async (nextMode: string) => {
    setPermissionMode(nextMode);
    await persist({ permissionMode: nextMode });
  };

  const addModel = async () => {
    const value = newModelValue.trim();
    const sourceModels = activeModels;
    if (!value || sourceModels.some((item) => item.value === value)) return;
    const displayName = newModelDisplayName.trim() || value;
    const nextModels = normalizeModelOrders([...sourceModels, { value, label: displayName, displayName, enabled: true, order: sourceModels.length }]);
    setModels(nextModels);
    setNewModelValue("");
    setNewModelDisplayName("");
    const nextModel = model || value;
    setModel(nextModel);
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, nextModels), selectedAiProvider.id, nextModel);
      return;
    }
    await persist({ models: nextModels, model: nextModel });
  };

  const deleteModel = async (value: string) => {
    const sourceModels = activeModels;
    const nextModels = sourceModels.filter((item) => item.value !== value);
    const orderedModels = normalizeModelOrders(nextModels);
    const nextModel = model === value ? orderedModels[0]?.value ?? null : model;
    setModels(orderedModels);
    setModel(nextModel ?? "");
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, orderedModels), selectedAiProvider.id, nextModel);
      return;
    }
    await persist({ models: orderedModels, model: nextModel });
  };

  const saveModel = async (previousValue: string, nextValue: string, nextDisplayName: string) => {
    const value = nextValue.trim();
    const sourceModels = activeModels;
    if (!value || sourceModels.some((item) => item.value === value && item.value !== previousValue)) return;
    const displayName = nextDisplayName.trim() || value;
    const nextModels = normalizeModelOrders(sourceModels.map((item) => item.value === previousValue ? { ...item, value, label: displayName, displayName } : item));
    const nextModel = model === previousValue ? value : model;
    setModels(nextModels);
    setModel(nextModel ?? "");
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, nextModels), selectedAiProvider.id, nextModel);
      return;
    }
    await persist({ models: nextModels, model: nextModel });
  };

  const moveModel = async (from: number, to: number) => {
    const nextModels = normalizeModelOrders(moveOption(activeModels, from, to));
    setModels(nextModels);
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, nextModels), selectedAiProvider.id, model || nextModels[0]?.value);
      return;
    }
    await persist({ models: nextModels });
  };

  const setDefaultModel = async (value: string) => {
    if (model === value) return;
    setModel(value);
    await persist({ model: value });
  };

  const handleModelDragEnd = (event: DragEndEvent) => {
    if (event.canceled) return;
    const { source } = event.operation;
    if (!isSortable(source)) return;
    if (source.initialIndex === source.index) return;
    void moveModel(source.initialIndex, source.index);
  };

  const activeModels = isAstra && selectedAiProvider ? selectedAiProvider.models : models;
  const modelOptions = optionRows(activeModels, model);
  const effortOptions = optionRows(agent.efforts, effort);
  const permissionOptions = runtimeAgent
    ? runtimePermissionModeOptions(optionRows(agent.permissionModes, permissionMode), permissionMode, runtimeAgent)
    : optionRows(agent.permissionModes, permissionMode);
  const sessionCommand = agent.commands.session[0] ?? "";
  const versionCommand = agent.commands.version[0] ?? "";
  const saveAstraProviders = async (
    nextProviders: AgentAiProviderInfo[],
    nextProviderId = aiProvider,
    nextModel: string | null | undefined = model,
  ) => {
    const orderedProviders = normalizeProviderOrders(nextProviders);
    setAiProviders(orderedProviders);
    setAiProvider(nextProviderId);
    const selectedProvider = orderedProviders.find((provider) => provider.id === nextProviderId) ?? orderedProviders[0] ?? null;
    const selectedModels = selectedProvider?.models ?? [];
    setModels(selectedModels);
    setModel(nextModel ?? "");
    await persist({
      aiProvider: nextProviderId,
      aiProviders: orderedProviders,
      models: selectedModels,
      model: nextModel,
    });
  };
  const selectAiProvider = async (nextProviderId: string) => {
    const provider = aiProviders.find((item) => item.id === nextProviderId) ?? null;
    const nextModel = provider?.models.find((item) => item.enabled)?.value ?? provider?.models[0]?.value ?? null;
    await saveAstraProviders(aiProviders, nextProviderId, nextModel);
  };
  const createProviderDraft = (): AgentAiProviderInfo => {
    const providerId = uniqueProviderId(aiProviders, "custom-provider");
    return {
      id: providerId,
      displayName: t("agent.custom_provider"),
      provider: "openai",
      api: "openai-responses",
      baseUrl: null,
      apiKey: null,
      models: [],
      enabled: true,
      order: aiProviders.length,
    };
  };
  const openAddProviderDialog = () => {
    setProviderDialog({ mode: "add", provider: createProviderDraft() });
  };
  const openEditProviderDialog = (provider: AgentAiProviderInfo) => {
    setProviderDialog({ mode: "edit", provider: { ...provider, models: provider.models.slice() } });
  };
  const saveProviderDialog = async (provider: AgentAiProviderInfo) => {
    const nextProvider = normalizeProviderOrders([provider])[0];
    if (!nextProvider) return;
    if (providerDialog?.mode === "edit") {
      const nextProviders = updateProviderInfo(aiProviders, provider.id, nextProvider);
      const nextProviderId = selectedAiProvider?.id ?? nextProvider.id;
      const selectedProvider = nextProviders.find((item) => item.id === nextProviderId) ?? nextProvider;
      const nextModel = selectedProvider.id === nextProvider.id
        ? (model || selectedProvider.models[0]?.value || null)
        : selectedProvider.models.find((item) => item.enabled)?.value ?? selectedProvider.models[0]?.value ?? null;
      await saveAstraProviders(nextProviders, nextProviderId, nextModel);
    } else {
      await saveAstraProviders([...aiProviders, nextProvider], nextProvider.id, nextProvider.models[0]?.value ?? null);
    }
    setProviderDialog(null);
  };
  const deleteProvider = async (providerId: string) => {
    if (aiProviders.length <= 1) return;
    const nextProviders = aiProviders.filter((provider) => provider.id !== providerId);
    const nextProvider = nextProviders.find((provider) => provider.id === selectedAiProvider?.id) ?? nextProviders[0] ?? null;
    const nextModel = nextProvider?.models.find((item) => item.enabled)?.value ?? nextProvider?.models[0]?.value ?? null;
    await saveAstraProviders(nextProviders, nextProvider?.id ?? "", nextModel);
  };

  return (
    <>
      <SettingsGroup title={agent.displayName}>
        <div className="flex items-start gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-card-chip/[0.08]">
            <SettingsAgentGlyph agentId={agent.id} className="h-5 w-5" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <h2 className="flex min-w-0 items-center gap-2 text-title font-semibold text-card-fg/88">
                <span className="truncate">{agent.displayName}</span>
                {agent.transport === "acp" && <AcpLogo className="h-2.5 w-auto shrink-0 opacity-75" />}
              </h2>
              <span className="rounded bg-card-chip/8 px-1.5 py-0.5 text-meta text-card-chip-fg/55">{agent.type}</span>
              <span className={"rounded px-1.5 py-0.5 text-meta " + (agent.enabled ? "bg-ink/[0.09] text-ink/70" : "bg-card-chip/8 text-card-muted/50")}>
                {agent.enabled ? t("agent.active") : t("agent.disabled")}
              </span>
            </div>
            <div className="mt-2 flex flex-wrap gap-1.5 text-caption text-card-muted/55">
              <span className="rounded bg-card-chip/[0.06] px-1.5 py-0.5">{agent.transport}</span>
              {sessionCommand && <span className="max-w-full truncate rounded bg-card-chip/[0.06] px-1.5 py-0.5">{sessionCommand}</span>}
              {versionCommand && <span className="max-w-full truncate rounded bg-card-chip/[0.06] px-1.5 py-0.5">{versionCommand}</span>}
            </div>
          </div>
          {isAstra ? (
            <SwitchControl
              checked
              tooltip={t("agent.always_enabled")}
              onToggle={() => undefined}
            />
          ) : (
            <SwitchControl
              checked={agent.enabled}
              tooltip={agent.enabled ? t("agent.disable") : t("agent.enable")}
              onToggle={() => void persist({ enabled: !agent.enabled })}
            />
          )}
        </div>
      </SettingsGroup>
      {isAstra && (
        <SettingsGroup
          title={t("agent.providers")}
          action={
            <button type="button" onClick={openAddProviderDialog} className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md px-2 text-body-sm font-medium leading-none text-card-fg/75 transition hover:bg-card-action-hover/5 hover:text-card-fg/90">
              <Plus className="h-4 w-4" />
              {t("agent.add_provider")}
            </button>
          }
          flush
        >
          <div className="divide-y divide-card-border/[0.08]">
            {aiProviders.map((provider) => (
              <AgentProviderRow
                key={provider.id}
                provider={provider}
                selected={provider.id === selectedAiProvider?.id}
                canDelete={aiProviders.length > 1}
                onSelect={selectAiProvider}
                onEdit={openEditProviderDialog}
                onDelete={deleteProvider}
              />
            ))}
            {aiProviders.length === 0 && <div className="p-3"><EmptyState label={t("agent.no_providers")} /></div>}
          </div>
        </SettingsGroup>
      )}
      <SettingsGroup title={t("agent.preferences")}>
        <div className="grid gap-2">
          <AgentPreferenceRow label={t("assistant.model")}>
            <AgentInlineSelect
              value={model}
              options={modelOptions}
              placeholder={t("agent.no_model")}
              onChange={(value) => void selectModel(value)}
            />
          </AgentPreferenceRow>
          {!isAstra && (
            <>
              <AgentPreferenceRow label={t("assistant.effort")}>
                <AgentInlineSelect
                  value={effort}
                  options={effortOptions}
                  placeholder={t("assistant.effort")}
                  onChange={(value) => void selectEffort(value)}
                />
              </AgentPreferenceRow>
              <AgentPreferenceRow label={t("assistant.permission_mode")}>
                <AgentInlineSelect
                  value={permissionMode}
                  options={permissionOptions}
                  placeholder={t("assistant.permission_mode")}
                  onChange={(value) => void selectPermissionMode(value)}
                />
              </AgentPreferenceRow>
            </>
          )}
        </div>
      </SettingsGroup>
      <SettingsGroup title={isAstra ? t("agent.provider_models") : t("agent.models")}>
        <div className="grid gap-3">
          <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,0.75fr)_auto] gap-2">
            <input value={newModelValue} onChange={(event) => setNewModelValue(event.target.value)} placeholder={t("agent.model_id")} className={inputClassName} />
            <input value={newModelDisplayName} onChange={(event) => setNewModelDisplayName(event.target.value)} placeholder={t("agent.model_name")} className={inputClassName} />
            <button type="button" onClick={() => void addModel()} disabled={!newModelValue.trim() || (isAstra && !selectedAiProvider)} className="inline-flex h-9 items-center justify-center gap-1.5 rounded-md bg-ink px-3 text-body-sm font-medium text-[rgb(var(--color-bg-panel))] disabled:opacity-35">
              <Plus className="h-4 w-4" />
              {t("agent.add_model")}
            </button>
          </div>
          <DragDropProvider onDragEnd={handleModelDragEnd}>
            <div className="overflow-hidden rounded-md border border-card-border/[0.10]">
              {activeModels.map((item, index) => (
                <AgentModelRow
                  key={item.value}
                  item={item}
                  index={index}
                  defaultModel={item.value === model}
                  onSetDefault={setDefaultModel}
                  onSave={saveModel}
                  onDelete={deleteModel}
                />
              ))}
              {activeModels.length === 0 && <div className="p-3"><EmptyState label={t("agent.no_models")} /></div>}
            </div>
          </DragDropProvider>
        </div>
      </SettingsGroup>
      {providerDialog && (
        <AgentProviderDialog
          mode={providerDialog.mode}
          provider={providerDialog.provider}
          onClose={() => setProviderDialog(null)}
          onSave={saveProviderDialog}
        />
      )}
    </>
  );
}

function AgentProviderRow({
  provider,
  selected,
  canDelete,
  onSelect,
  onEdit,
  onDelete,
}: {
  provider: AgentAiProviderInfo;
  selected: boolean;
  canDelete: boolean;
  onSelect: (providerId: string) => Promise<void>;
  onEdit: (provider: AgentAiProviderInfo) => void;
  onDelete: (providerId: string) => Promise<void>;
}) {
  const { t } = useI18n();
  const detailItems = [
    provider.provider,
    provider.api,
    provider.baseUrl,
    t("agent.model_count", { count: provider.models.length }),
  ].filter((item) => item && item.trim().length > 0);

  return (
    <div className={"grid min-h-14 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-3 py-2.5 transition " + (selected ? "bg-card-active" : "hover:bg-card-action-hover/5")}>
      <button type="button" onClick={() => void onSelect(provider.id)} className="flex min-w-0 items-center gap-3 text-left">
        <span className={"flex h-5 w-5 shrink-0 items-center justify-center rounded-full border " + (selected ? "border-brand/55 bg-brand/15 text-brand" : "border-card-border/[0.14] text-transparent")}>
          {selected && <Check className="h-3.5 w-3.5" />}
        </span>
        <span className="min-w-0">
          <span className="block truncate text-body-sm font-medium text-card-fg/82">{provider.displayName || provider.provider || provider.id}</span>
          <span className="mt-0.5 block truncate text-caption text-card-muted/56">{detailItems.join(" / ")}</span>
        </span>
      </button>
      <div className="flex items-center gap-1">
        <Tooltip content={t("agent.edit_provider")} placement="top">
          <button type="button" onClick={() => onEdit(provider)} className="rounded p-1.5 text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75">
            <Pencil className="h-4 w-4" />
          </button>
        </Tooltip>
        <Tooltip content={t("agent.delete_provider")} placement="top">
          <button type="button" onClick={() => void onDelete(provider.id)} disabled={!canDelete} className="rounded p-1.5 text-card-subtle/45 hover:bg-status-error/10 hover:text-status-error disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-card-subtle/45">
            <Trash2 className="h-4 w-4" />
          </button>
        </Tooltip>
      </div>
    </div>
  );
}

function AgentProviderDialog({
  mode,
  provider,
  onClose,
  onSave,
}: {
  mode: "add" | "edit";
  provider: AgentAiProviderInfo;
  onClose: () => void;
  onSave: (provider: AgentAiProviderInfo) => Promise<void>;
}) {
  const { t } = useI18n();
  const [displayName, setDisplayName] = useState(provider.displayName);
  const [piProvider, setPiProvider] = useState(provider.provider);
  const [api, setApi] = useState(provider.api ?? "");
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl ?? "");
  const [apiKey, setApiKey] = useState(provider.apiKey ?? "");

  useEffect(() => {
    setDisplayName(provider.displayName);
    setPiProvider(provider.provider);
    setApi(provider.api ?? "");
    setBaseUrl(provider.baseUrl ?? "");
    setApiKey(provider.apiKey ?? "");
  }, [provider]);

  const save = async () => {
    const nextProvider: AgentAiProviderInfo = {
      ...provider,
      displayName: displayName.trim() || piProvider.trim() || provider.id,
      provider: piProvider.trim() || provider.id,
      api: api.trim() || null,
      baseUrl: baseUrl.trim() || null,
      apiKey: apiKey.trim() || null,
    };
    await onSave(nextProvider);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 px-4" onClick={onClose}>
      <div className="w-full max-w-[520px] rounded-lg border border-card-border/[0.12] bg-surface-panel p-4 shadow-[0_24px_80px_rgba(0,0,0,0.22)]" onClick={(event) => event.stopPropagation()}>
        <div className="mb-3 text-body-sm font-semibold text-ink/[0.88]">{mode === "add" ? t("agent.add_provider") : t("agent.edit_provider")}</div>
        <div className="grid gap-2">
          <AgentProviderDialogField label={t("agent.provider_name")}>
            <input value={displayName} onChange={(event) => setDisplayName(event.target.value)} placeholder="OpenAI" className={inputClassName} />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.pi_provider")}>
            <input value={piProvider} onChange={(event) => setPiProvider(event.target.value)} placeholder="openai" className={inputClassName} />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.pi_api")}>
            <input value={api} onChange={(event) => setApi(event.target.value)} placeholder="openai-responses" className={inputClassName} />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.api_base_url")}>
            <input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} placeholder="https://api.openai.com/v1" className={inputClassName} />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.api_key")}>
            <input value={apiKey} type="password" onChange={(event) => setApiKey(event.target.value)} placeholder="sk-..." className={inputClassName} />
          </AgentProviderDialogField>
          <div className="mt-1 flex justify-end gap-2">
            <button type="button" onClick={onClose} className="rounded-md px-3 py-1.5 text-body-sm text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
            <button type="button" onClick={() => void save()} disabled={!piProvider.trim()} className="inline-flex h-8 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] hover:text-card-fg/90 disabled:opacity-35">
              <Check className="h-4 w-4" />
              {t("project.save")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function AgentProviderDialogField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="grid gap-1.5">
      <span className="text-caption font-medium text-card-muted/60">{label}</span>
      {children}
    </label>
  );
}

function AgentModelRow({
  item,
  index,
  defaultModel,
  onSetDefault,
  onSave,
  onDelete,
}: {
  item: RuntimeAgentOptionMetadata;
  index: number;
  defaultModel: boolean;
  onSetDefault: (value: string) => Promise<void>;
  onSave: (previousValue: string, nextValue: string, nextLabel: string) => Promise<void>;
  onDelete: (value: string) => Promise<void>;
}) {
  const { t } = useI18n();
  const { handleRef, isDragSource, isDropTarget, ref } = useSortable({
    id: item.value,
    index,
    group: "agent-models",
    transition: {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
      idle: true,
    },
  });
  const [editing, setEditing] = useState(false);
  const [valueDraft, setValueDraft] = useState(item.value);
  const [displayNameDraft, setDisplayNameDraft] = useState(item.displayName);

  useEffect(() => {
    setValueDraft(item.value);
    setDisplayNameDraft(item.displayName);
    setEditing(false);
  }, [item]);

  const save = async () => {
    await onSave(item.value, valueDraft, displayNameDraft);
    setEditing(false);
  };

  return (
    <div
      ref={ref}
      className={
        "grid min-h-12 grid-cols-[auto_minmax(0,1fr)_minmax(0,0.72fr)_auto_auto_auto] items-center gap-2 border-b border-card-border/[0.08] px-3 py-2 transition last:border-b-0 " +
        (isDragSource
          ? "z-20 cursor-grabbing bg-card shadow-[0_12px_28px_rgba(0,0,0,0.22)]"
          : isDropTarget
            ? "bg-card-active shadow-[inset_3px_0_0_rgb(var(--color-card-fg)/0.32)]"
            : "")
      }
    >
      <button ref={handleRef} type="button" className="cursor-grab touch-none rounded p-0.5 text-card-subtle/35 hover:bg-card-action-hover/5 hover:text-card-fg/60 active:cursor-grabbing">
        <GripVertical className="h-4 w-4" />
      </button>
      <div className="min-w-0">
        {editing ? (
          <input
            value={valueDraft}
            onChange={(event) => setValueDraft(event.target.value)}
            className="h-8 w-full min-w-0 rounded-md border border-input-border/[0.12] bg-input px-2 text-caption text-input-fg outline-none focus:border-input-focus/30"
          />
        ) : (
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-body-sm font-medium text-card-fg/80">{item.value}</span>
            {defaultModel && <span className="rounded bg-card-chip/10 px-1.5 py-0.5 text-meta text-card-chip-fg/60">{t("agent.default_model")}</span>}
          </div>
        )}
      </div>
      <div className="min-w-0">
        {editing ? (
          <input
            value={displayNameDraft}
            onChange={(event) => setDisplayNameDraft(event.target.value)}
            className="h-8 w-full min-w-0 rounded-md border border-input-border/[0.12] bg-input px-2 text-caption text-input-fg outline-none focus:border-input-focus/30"
          />
        ) : (
          <span className="block truncate text-caption text-card-muted/62">{item.displayName || item.value}</span>
        )}
      </div>
      <Tooltip content={t("agent.make_default")} placement="top">
        <button
          type="button"
          role="radio"
          aria-checked={defaultModel}
          onClick={() => void onSetDefault(item.value)}
          className={`rounded p-1 ${defaultModel ? "bg-card-chip/[0.12] text-card-fg/75" : "text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75"}`}
        >
          {defaultModel ? <Check className="h-4 w-4" /> : <Circle className="h-4 w-4" />}
        </button>
      </Tooltip>
      <Tooltip content={editing ? t("project.save") : t("agent.edit_model")} placement="top">
        <button type="button" onClick={() => editing ? void save() : setEditing(true)} className="rounded p-1 text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75">
          {editing ? <Check className="h-4 w-4" /> : <Pencil className="h-4 w-4" />}
        </button>
      </Tooltip>
      <Tooltip content={t("agent.delete_model")} placement="top">
        <button type="button" onClick={() => void onDelete(item.value)} className="rounded p-1 text-card-subtle/45 hover:bg-status-error/10 hover:text-status-error">
          <Trash2 className="h-4 w-4" />
        </button>
      </Tooltip>
    </div>
  );
}

function AgentPreferenceRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid min-h-10 grid-cols-[120px_minmax(0,1fr)] items-center gap-3 rounded-md bg-card-chip/[0.04] px-3">
      <span className="text-caption font-medium text-card-muted/60">{label}</span>
      <div className="min-w-0 justify-self-start">{children}</div>
    </div>
  );
}

function AgentInlineSelect({
  value,
  options,
  placeholder,
  onChange,
}: {
  value: string;
  options: Array<{ value: string; label: string; icon?: ReactNode }>;
  placeholder: string;
  onChange: (value: string) => void;
}) {
  return (
    <div className="flex min-w-0 max-w-[320px] items-center rounded-md text-ink/55 transition hover:bg-ink/8 hover:text-ink">
      <InlineMenuSelect
        value={value}
        options={options}
        onChange={onChange}
        menuAlign="trigger"
        placeholder={placeholder}
        ariaLabel={placeholder}
        className="h-7 max-w-[320px] border-r-0 px-1.5 py-1 text-ink/60 hover:text-ink"
        menuClassName="bg-surface-panel"
        minMenuWidth={220}
        emptyContent={placeholder}
      />
    </div>
  );
}

function optionRows(options: RuntimeAgentOptionMetadata[], selected: string): RuntimeAgentOptionMetadata[] {
  const rows = options
    .filter((option) => option.value.trim().length > 0 && option.enabled)
    .map((option) => ({ ...option, label: option.displayName || option.label || option.value }));
  if (selected && !rows.some((option) => option.value === selected)) {
    return [{ value: selected, label: selected, displayName: selected, enabled: true, order: -1 }, ...rows];
  }
  return rows;
}

function moveOption(options: RuntimeAgentOptionMetadata[], from: number, to: number): RuntimeAgentOptionMetadata[] {
  if (from < 0 || to < 0 || from >= options.length || to >= options.length || from === to) return options;
  const next = [...options];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

function normalizeModelOrders(options: RuntimeAgentOptionMetadata[]): RuntimeAgentOptionMetadata[] {
  return options.map((option, index) => ({
    ...option,
    label: option.displayName || option.label || option.value,
    displayName: option.displayName || option.label || option.value,
    enabled: option.enabled ?? true,
    order: index,
  }));
}

function normalizeProviderOrders(options: AgentAiProviderInfo[]): AgentAiProviderInfo[] {
  return options.map((provider, index) => ({
    ...provider,
    id: provider.id.trim() || `provider-${index + 1}`,
    displayName: provider.displayName.trim() || provider.provider.trim() || provider.id,
    provider: provider.provider.trim() || provider.id.trim() || `provider-${index + 1}`,
    api: provider.api?.trim() || null,
    baseUrl: provider.baseUrl?.trim() || null,
    apiKey: provider.apiKey?.trim() || null,
    models: normalizeModelOrders(provider.models),
    enabled: provider.enabled ?? true,
    order: index,
  }));
}

function updateProviderInfo(
  providers: AgentAiProviderInfo[],
  providerId: string,
  patch: Partial<AgentAiProviderInfo>,
): AgentAiProviderInfo[] {
  return providers.map((provider) => provider.id === providerId ? { ...provider, ...patch } : provider);
}

function updateProviderModels(
  providers: AgentAiProviderInfo[],
  providerId: string,
  models: RuntimeAgentOptionMetadata[],
): AgentAiProviderInfo[] {
  return updateProviderInfo(providers, providerId, { models });
}

function uniqueProviderId(providers: AgentAiProviderInfo[], baseId: string): string {
  const existing = new Set(providers.map((provider) => provider.id));
  let index = providers.length + 1;
  let candidate = baseId;
  while (existing.has(candidate)) {
    candidate = `${baseId}-${index}`;
    index += 1;
  }
  return candidate;
}

function moveAgentOption(options: AgentInfo[], from: number, to: number): AgentInfo[] {
  if (from < 0 || to < 0 || from >= options.length || to >= options.length || from === to) return options;
  const next = [...options];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

function SettingsAgentGlyph({
  agentId,
  className,
}: {
  agentId: string;
  className?: string;
}) {
  if (isRuntimeAgent(agentId)) {
    return <AgentGlyph agent={agentId} className={className} />;
  }
  return <AiGenerate2Icon className={className} />;
}

function isRuntimeAgent(id: string): id is Agent {
  return id === "codex" || id === "claude" || id === "gemini";
}

function isSettingsAgent(id: string): boolean {
  return isRuntimeAgent(id) || id === "astra";
}

function AssistantsSettings({ onError }: { onError: (error: string | null) => void }) {
  const { t } = useI18n();
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [tab, setTab] = useState<"builtin" | "custom">("builtin");
  const [showCreate, setShowCreate] = useState(false);

  const reload = async () => {
    setLoading(true);
    try {
      const [assistantRows, agentRows] = await Promise.all([
        listAssistants(null),
        listAgents(),
      ]);
      setAssistants(assistantRows);
      setAgents(agentRows);
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const sharedAssistants = assistants.filter((assistant) => assistant.projectId === null && assistant.workflowId === null);
  const builtin = sharedAssistants.filter((assistant) => assistant.type === "builtin");
  const custom = sharedAssistants.filter((assistant) => assistant.type === "custom");
  const visible = tab === "builtin" ? builtin : custom;

  return (
    <section>
      <div className="mb-3 flex items-center justify-between gap-4">
        <SegmentedTabs
          items={[
            { value: "builtin", label: t("assistant.builtin") },
            { value: "custom", label: t("assistant.custom") },
          ]}
          value={tab}
          onChange={setTab}
          itemWidth={96}
          itemHeight={28}
          padding={3}
        />
        <button type="button" onClick={() => setShowCreate(true)} className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] hover:text-card-fg/90">
          <Plus className="h-4 w-4" />
          {t("assistant.add")}
        </button>
      </div>
      <div className="rounded-lg border border-card-border/[0.12] bg-card p-5">
        <div className="grid gap-2">
          {visible.map((assistant) => (
            <AssistantCard
              key={assistant.id}
              assistant={assistant}
              agents={agents}
              onUpdated={(next) => setAssistants((prev) => prev.map((item) => item.id === next.id ? next : item))}
              onDeleted={(id) => setAssistants((prev) => prev.filter((item) => item.id !== id))}
              onError={onError}
            />
          ))}
          {!loading && visible.length === 0 && <EmptyState label={t("assistant.empty")} />}
        </div>
      </div>
      {showCreate && (
        <CreateAssistantDialog
          agents={agents}
          onCreated={(assistant) => setAssistants((prev) => [assistant, ...prev])}
          onClose={() => setShowCreate(false)}
          onError={onError}
        />
      )}
    </section>
  );
}

function WorkflowsSettings({ onError }: { onError: (error: string | null) => void }) {
  const { t } = useI18n();
  const [workflows, setWorkflows] = useState<WorkflowInfo[]>([]);
  const [selectedWorkflowId, setSelectedWorkflowId] = useState("code");
  const [stages, setStages] = useState<ProjectStageInfo[]>([]);
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);
  const [newWorkflowName, setNewWorkflowName] = useState("");
  const [newWorkflowDescription, setNewWorkflowDescription] = useState("");
  const [showCreateWorkflow, setShowCreateWorkflow] = useState(false);
  const [loading, setLoading] = useState(true);

  const selectedWorkflow = workflows.find((workflow) => workflow.id === selectedWorkflowId) ?? workflows[0] ?? null;
  const availableAssistants = assistants.filter((assistant) => assistant.enabled && assistant.projectId === null && (
    assistant.workflowId === selectedWorkflowId ||
    (assistant.workflowId === null && assistant.type === "custom")
  ));

  const reloadAll = async () => {
    setLoading(true);
    try {
      const [workflowRows, assistantRows] = await Promise.all([listWorkflows(), listAssistants(null)]);
      setWorkflows(workflowRows);
      setAssistants(assistantRows);
      setSelectedWorkflowId((current) => workflowRows.some((workflow) => workflow.id === current) ? current : workflowRows[0]?.id ?? "code");
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  };

  const reloadStages = async (workflowId: string) => {
    try {
      setStages(await listWorkflowStages(workflowId));
    } catch (err) {
      onError(String(err));
    }
  };

  useEffect(() => {
    void reloadAll();
  }, []);

  useEffect(() => {
    if (selectedWorkflowId) void reloadStages(selectedWorkflowId);
  }, [selectedWorkflowId]);

  const createNewWorkflow = async () => {
    const name = newWorkflowName.trim();
    if (!name) return;
    try {
      const workflow = await createWorkflow(name, newWorkflowDescription);
      setWorkflows((prev) => [...prev, workflow]);
      setSelectedWorkflowId(workflow.id);
      setNewWorkflowName("");
      setNewWorkflowDescription("");
      setShowCreateWorkflow(false);
    } catch (err) {
      onError(String(err));
    }
  };

  const refreshStages = async () => {
    if (selectedWorkflowId) await reloadStages(selectedWorkflowId);
  };

  const workflowDescription = (workflow: WorkflowInfo) => {
    if (!workflow.description) return t("settings.workflow_no_description");
    return workflow.type === "builtin" ? t(workflow.description) : workflow.description;
  };

  return (
    <section>
      <div className="grid grid-cols-[240px_minmax(0,1fr)] gap-5">
        <div className="min-w-0">
          <SettingsGroup
            title={t("settings.workflows")}
            flush
            action={
              <button type="button" onClick={() => setShowCreateWorkflow(true)} className="inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 text-body-sm font-medium leading-none text-card-fg/75 transition hover:text-card-fg/90">
                <Plus className="h-3.5 w-3.5" />
                {t("settings.add_workflow")}
              </button>
            }
          >
            <div className="divide-y divide-card-border/10">
              {workflows.map((workflow) => (
                <Tooltip key={workflow.id} content={workflowDescription(workflow)} placement="right">
                  <button
                    type="button"
                    onClick={() => setSelectedWorkflowId(workflow.id)}
                    className={"workflow-list-item flex h-10 w-full min-w-0 items-center justify-between px-3 text-left text-body-sm transition " + (workflow.id === selectedWorkflowId ? "workflow-list-item-active" : "")}
                  >
                    <span className="truncate">{workflow.name}</span>
                    <span className="rounded bg-card-chip/8 px-1.5 py-0.5 text-meta text-card-chip-fg/55">{workflow.type}</span>
                  </button>
                </Tooltip>
              ))}
            </div>
          </SettingsGroup>
        </div>
        <div className="min-w-0">
          {selectedWorkflow && (
            <WorkflowEditor
              stages={stages}
              assistants={availableAssistants}
              loading={loading}
              workflowId={selectedWorkflowId}
              onStageCreated={(stage) => setStages((prev) => [...prev, stage].sort((a, b) => a.order - b.order))}
              onStageUpdated={(stage) => setStages((prev) => prev.map((item) => item.id === stage.id ? stage : item).sort((a, b) => a.order - b.order))}
              onStagesReload={refreshStages}
              onStageDeleted={(id) => setStages((prev) => prev.filter((stage) => stage.id !== id))}
              onError={onError}
            />
          )}
        </div>
      </div>
      {showCreateWorkflow && (
        <CreateWorkflowDialog
          name={newWorkflowName}
          description={newWorkflowDescription}
          onNameChange={setNewWorkflowName}
          onDescriptionChange={setNewWorkflowDescription}
          onCreate={() => void createNewWorkflow()}
          onClose={() => setShowCreateWorkflow(false)}
        />
      )}
    </section>
  );
}

function CreateWorkflowDialog({
  name,
  description,
  onNameChange,
  onDescriptionChange,
  onCreate,
  onClose,
}: {
  name: string;
  description: string;
  onNameChange: (value: string) => void;
  onDescriptionChange: (value: string) => void;
  onCreate: () => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 px-4" onClick={onClose}>
      <div className="w-full max-w-[520px] rounded-lg border border-card-border/[0.12] bg-surface-panel p-4 shadow-[0_24px_80px_rgba(0,0,0,0.22)]" onClick={(event) => event.stopPropagation()}>
        <div className="mb-3 text-body-sm font-semibold text-ink/[0.88]">{t("settings.add_workflow")}</div>
        <div className="grid gap-2">
          <input value={name} onChange={(event) => onNameChange(event.target.value)} placeholder={t("settings.workflow_name")} className={inputClassName} />
          <textarea value={description} onChange={(event) => onDescriptionChange(event.target.value)} placeholder={t("settings.workflow_description")} rows={3} className={textareaClassName} />
          <div className="flex justify-end gap-2">
            <button type="button" onClick={onClose} className="rounded-md px-3 py-1.5 text-body-sm text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
            <button type="button" onClick={onCreate} disabled={!name.trim()} className="inline-flex h-8 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] hover:text-card-fg/90 disabled:opacity-35">
              <Plus className="h-4 w-4" />
              {t("settings.add_workflow")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function WorkflowEditor({
  stages,
  assistants,
  loading,
  workflowId,
  onStageCreated,
  onStageUpdated,
  onStagesReload,
  onStageDeleted,
  onError,
}: {
  stages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  loading: boolean;
  workflowId: string;
  onStageCreated: (stage: ProjectStageInfo) => void;
  onStageUpdated: (stage: ProjectStageInfo) => void;
  onStagesReload: () => Promise<void>;
  onStageDeleted: (stageId: string) => void;
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const [showCreateStage, setShowCreateStage] = useState(false);

  return (
    <>
      <SettingsGroup
        title={t("stage.project_stages")}
        action={
          <button type="button" onClick={() => setShowCreateStage(true)} className="inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 text-body-sm font-medium leading-none text-card-fg/75 transition hover:text-card-fg/90">
            <Plus className="h-3.5 w-3.5" />
            {t("stage.add")}
          </button>
        }
      >
        <StageList
          stages={stages}
          assistants={assistants}
          loading={loading}
          dragGroup="workflow-stages"
          onUpdated={onStageUpdated}
          onDeleted={onStageDeleted}
          onReload={onStagesReload}
          onError={onError}
        />
      </SettingsGroup>
      {showCreateStage && (
        <CreateStageDialog
          workflowId={workflowId}
          onCreated={onStageCreated}
          onClose={() => setShowCreateStage(false)}
          onError={onError}
        />
      )}
    </>
  );
}

function SettingsGroup({ title, children, flush = false, action = null }: { title: string; children: ReactNode; flush?: boolean; action?: ReactNode }) {
  return (
    <div className="mb-8 last:mb-0">
      <div className="mb-3 flex items-center justify-between gap-3">
        <h2 className="text-body-sm font-semibold text-ink/[0.88]">{title}</h2>
        {action}
      </div>
      <div className={"overflow-hidden rounded-lg border border-card-border/[0.12] bg-card " + (flush ? "" : "p-3")}>{children}</div>
    </div>
  );
}

function SettingsRow({
  icon,
  label,
  description,
  children,
}: {
  icon: ReactNode;
  label: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <div className="grid min-h-[72px] grid-cols-[minmax(0,1fr)_auto] items-center gap-4 border-b border-ink/[0.12] px-3 py-3 last:border-b-0">
      <div className="flex min-w-0 gap-3">
        <span className="mt-0.5 text-ink/55">{icon}</span>
        <span className="min-w-0">
          <span className="block text-body-sm font-medium text-ink/75">{label}</span>
          <span className="mt-1 block text-caption leading-relaxed text-ink/60">{description}</span>
        </span>
      </div>
      {children}
    </div>
  );
}

function SegmentedControl({
  value,
  options,
  onChange,
}: {
  value: string;
  options: Array<{ value: string; label: string }>;
  onChange: (value: string) => void;
}) {
  return (
    <div className="inline-flex rounded-md bg-ink/[0.045] p-0.5">
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          onClick={() => onChange(option.value)}
          className={"h-7 min-w-9 rounded px-2 text-body-sm transition " + (value === option.value ? "bg-ink/[0.06] text-ink/85" : "text-ink/45 hover:text-ink/75")}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function ThemeSelector({ mode, onChange }: { mode: ThemeMode; onChange: (mode: ThemeMode) => void }) {
  const { t } = useI18n();
  const items = [
    { value: "light" as const, icon: Sun, label: t("theme.light") },
    { value: "dark" as const, icon: Moon, label: t("theme.dark") },
    { value: "system" as const, icon: Monitor, label: t("theme.system") },
  ];
  return (
    <div className="inline-flex rounded-md bg-ink/[0.045] p-0.5">
      {items.map(({ value, icon: Icon, label }) => (
        <Tooltip key={value} content={label} placement="top">
          <button
            type="button"
            onClick={() => onChange(value)}
            className={"flex h-7 w-8 items-center justify-center rounded transition " + (mode === value ? "bg-ink/[0.06] text-ink/85" : "text-ink/45 hover:text-ink/75")}
          >
            <Icon className="h-4 w-4" />
          </button>
        </Tooltip>
      ))}
    </div>
  );
}

function EmptyState({ label }: { label: string }) {
  return <div className="rounded-md border border-dashed border-ink/10 py-8 text-center text-body-sm text-ink/35">{label}</div>;
}

const inputClassName = "h-9 min-w-0 rounded-md border border-input-border/[0.16] bg-input px-3 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";
const textareaClassName = "min-w-0 resize-none rounded-md border border-input-border/[0.16] bg-input px-3 py-2 text-body-sm text-input-fg outline-none placeholder:text-input-placeholder/35 focus:border-input-focus/30";
