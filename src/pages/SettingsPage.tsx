import { type ReactNode, useEffect, useMemo, useState } from "react";
import { DragDropProvider, type DragEndEvent } from "@dnd-kit/react";
import { isSortable, useSortable } from "@dnd-kit/react/sortable";
import {
  Anthropic,
  Aws,
  Azure,
  Bedrock,
  Cerebras,
  Claude,
  Cloudflare,
  DeepSeek,
  Fireworks,
  Gemini,
  GithubCopilot,
  Google,
  GoogleCloud,
  Groq,
  HuggingFace,
  Kimi,
  Minimax,
  Mistral,
  Moonshot,
  OpenAI,
  OpenCode,
  OpenRouter,
  Together,
  Vercel,
  VertexAI,
  WorkersAI,
  XAI,
  XiaomiMiMo,
  ZAI,
} from "@lobehub/icons";
import { ArrowLeft, Check, Circle, Download, Globe2, GripVertical, Info, Languages, LoaderCircle, Monitor, Moon, Pencil, Plus, RefreshCw, RotateCcw, Settings2, Sparkles, Sun, Trash2, Workflow } from "lucide-react";
import type { Agent, AgentAiProviderInfo, AgentInfo, AstraConfig, AssistantInfo, NetworkConfig, ProjectStageInfo, RuntimeAgentOptionMetadata, ProcessTemplateInfo } from "../api";
import {
  createProcessTemplate,
  getAstraConfig,
  getNetworkConfig,
  listAgents,
  listAssistants,
  listProcessTemplateStages,
  listProcessTemplates,
  updateAgentPreferences,
  updateAstraConfig,
  updateNetworkConfig,
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
import { AiGenerate2Icon, Robot3LineIcon } from "../components/IconifyIcon";
import { type Lang, useI18n } from "../i18n";
import type { ThemeMode } from "../theme";
import { formatVersionLabel, type UpdateState } from "../updater";
import acpMarkBlackUrl from "../../assets/acp_mark-black.svg?url";
import acpMarkWhiteUrl from "../../assets/acp_mark-white.svg?url";

type SettingsSection = "general" | "agents" | "assistants" | "processTemplates";

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
    { id: "processTemplates" as const, label: t("settings.process_templates"), icon: Workflow },
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
        <ScrollArea className="min-h-0 flex-1 bg-surface-panel" viewportClassName={"px-10 pt-6 " + (section === "processTemplates" ? "pb-6" : "pb-16")}>
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
            {section === "processTemplates" && <ProcessTemplatesSettings onError={onError} />}
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
  const [networkConfig, setNetworkConfig] = useState<NetworkConfig | null>(null);
  const [proxyEnabled, setProxyEnabled] = useState(false);
  const [proxyUrl, setProxyUrl] = useState("");
  const [noProxy, setNoProxy] = useState("");
  const [savingProxy, setSavingProxy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    getNetworkConfig()
      .then((config) => {
        if (cancelled) return;
        setNetworkConfig(config);
        setProxyEnabled(config.proxy.enabled);
        setProxyUrl(config.proxy.url ?? "");
        setNoProxy(config.proxy.noProxy ?? "");
      })
      .catch((err) => onError(String(err)));
    return () => {
      cancelled = true;
    };
  }, [onError]);

  const saveProxyConfig = async () => {
    if (savingProxy) return;
    setSavingProxy(true);
    try {
      const next = await updateNetworkConfig({
        proxy: {
          enabled: proxyEnabled,
          url: proxyUrl.trim() || null,
          noProxy: noProxy.trim() || null,
        },
      });
      setNetworkConfig(next);
      setProxyEnabled(next.proxy.enabled);
      setProxyUrl(next.proxy.url ?? "");
      setNoProxy(next.proxy.noProxy ?? "");
      onError(null);
    } catch (err) {
      onError(String(err));
    } finally {
      setSavingProxy(false);
    }
  };

  const proxyChanged = networkConfig
    ? proxyEnabled !== networkConfig.proxy.enabled
      || proxyUrl.trim() !== (networkConfig.proxy.url ?? "")
      || noProxy.trim() !== (networkConfig.proxy.noProxy ?? "")
    : false;
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
      <SettingsGroup title={t("settings.network")} flush>
        <SettingsRow icon={<Globe2 className="h-4 w-4" />} label={t("settings.proxy")} description={t("settings.proxy_description")}>
          <div className="flex items-center justify-end gap-3">
            <span className="text-caption text-ink/45">{proxyEnabled ? t("settings.proxy_enabled") : t("settings.proxy_disabled")}</span>
            <SwitchControl checked={proxyEnabled} tooltip={t("settings.proxy")} onToggle={() => setProxyEnabled((value) => !value)} />
          </div>
        </SettingsRow>
        <SettingsRow icon={<Globe2 className="h-4 w-4" />} label={t("settings.proxy_url")} description={t("settings.proxy_url_description")}>
          <input value={proxyUrl} onChange={(event) => setProxyUrl(event.target.value)} placeholder="http://127.0.0.1:7890" className={inputClassName + " w-[280px]"} />
        </SettingsRow>
        <SettingsRow icon={<Globe2 className="h-4 w-4" />} label={t("settings.no_proxy")} description={t("settings.no_proxy_description")}>
          <div className="flex items-center justify-end gap-2">
            <input value={noProxy} onChange={(event) => setNoProxy(event.target.value)} placeholder="localhost,127.0.0.1" className={inputClassName + " w-[280px]"} />
            <button type="button" disabled={savingProxy || !proxyChanged} onClick={() => void saveProxyConfig()} className="inline-flex h-9 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] disabled:opacity-35">
              {savingProxy ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Check className="h-4 w-4" />}
              {t("project.save")}
            </button>
          </div>
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
  const [selectedView, setSelectedView] = useState<"astra" | "agent">("astra");
  const [selectedAgentId, setSelectedAgentId] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [astraConfig, setAstraConfig] = useState<AstraConfig | null>(null);
  const builtinAgents = useMemo(
    () => agents.filter((agent) => agent.type === "builtin" && isSettingsAgent(agent.id)),
    [agents],
  );
  const selectedAgent =
    selectedView === "agent"
      ? builtinAgents.find((agent) => agent.id === selectedAgentId) ?? builtinAgents[0] ?? null
      : null;

  const reload = async () => {
    setLoading(true);
    try {
      const [agentsData, configData] = await Promise.all([
        listAgents(),
        getAstraConfig(),
      ]);
      setAgents(agentsData);
      setAstraConfig(configData);
      setSelectedAgentId((current) => {
        const currentExists = agentsData.some((agent) => agent.id === current && isSettingsAgent(agent.id));
        return currentExists ? current : agentsData.find((agent) => isSettingsAgent(agent.id))?.id ?? current;
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
    if (event.canceled) return;
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
            <button
              type="button"
              onClick={() => setSelectedView("astra")}
              className={
                "process-template-list-item mb-3 flex h-10 w-full min-w-0 items-center gap-2 overflow-hidden rounded-lg border border-card-border/[0.12] bg-card px-3 text-left text-body-sm transition " +
                (selectedView === "astra" ? "process-template-list-item-active" : "")
              }
            >
              <Sparkles className="h-4 w-4 shrink-0 text-card-icon/60" />
              <span className="min-w-0 flex-1 truncate font-medium text-card-fg/78">{t("astra.orchestrator")}</span>
            </button>
            <div className="overflow-hidden rounded-lg border border-card-border/[0.12] bg-card">
              <DragDropProvider onDragEnd={handleAgentDragEnd}>
                <div className="divide-y divide-card-border/10">
                  {builtinAgents.map((agent, index) => (
                    <AgentListRow
                      key={agent.id}
                      agent={agent}
                      index={index}
                      active={selectedView === "agent" && agent.id === selectedAgent?.id}
                      draggable
                      onSelect={(agentId) => {
                        setSelectedAgentId(agentId);
                        setSelectedView("agent");
                      }}
                    />
                  ))}
                  {!loading && builtinAgents.length === 0 && <div className="p-3"><EmptyState label={t("agent.empty")} /></div>}
                </div>
              </DragDropProvider>
            </div>
          </div>
        </div>
        <div className="min-w-0">
          {selectedView === "astra" && astraConfig ? (
            <SettingsGroup title={t("astra.config_title")}>
              <AstraAgentSettings
                title={t("astra.agent")}
                agentValue={astraConfig.agent ?? ""}
                modelValue={astraConfig.model ?? ""}
                effortValue={astraConfig.effort ?? ""}
                permissionValue={astraConfig.permissionMode ?? ""}
                agents={agents}
                onAgentChange={(value) => {
                  updateAstraConfig({ agent: value || null })
                    .then(setAstraConfig)
                    .catch((err) => onError(String(err)));
                }}
                onModelChange={(value) => {
                  updateAstraConfig({ model: value || null })
                    .then(setAstraConfig)
                    .catch((err) => onError(String(err)));
                }}
                onEffortChange={(value) => {
                  updateAstraConfig({ effort: value || null })
                    .then(setAstraConfig)
                    .catch((err) => onError(String(err)));
                }}
                onPermissionChange={(value) => {
                  updateAstraConfig({ permissionMode: value || null })
                    .then(setAstraConfig)
                    .catch((err) => onError(String(err)));
                }}
              />
            </SettingsGroup>
          ) : selectedAgent ? (
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

function AstraAgentSettings({
  title,
  agentValue,
  modelValue,
  effortValue,
  permissionValue,
  agents,
  onAgentChange,
  onModelChange,
  onEffortChange,
  onPermissionChange,
}: {
  title: string;
  agentValue: string;
  modelValue: string;
  effortValue: string;
  permissionValue: string;
  agents: AgentInfo[];
  onAgentChange: (value: string) => void;
  onModelChange: (value: string) => void;
  onEffortChange: (value: string) => void;
  onPermissionChange: (value: string) => void;
}) {
  const { t } = useI18n();
  const selectableAgents = useMemo(
    () => astraSelectableAgents(agents, agentValue),
    [agentValue, agents],
  );
  const agentOptions = useMemo(
    () => selectableAgents.map((agent) => ({
      value: agent.id,
      label: agent.displayName,
      icon: <SettingsAgentGlyph agentId={agent.id} className="h-4 w-4" />,
    })),
    [selectableAgents],
  );
  const effectiveAgentValue = agentOptions.some((option) => option.value === agentValue)
    ? agentValue
    : agentOptions[0]?.value ?? "";
  const selectedAgent = useMemo(
    () => selectableAgents.find((agent) => agent.id === effectiveAgentValue) ?? null,
    [effectiveAgentValue, selectableAgents],
  );
  const modelOptions = optionRows(astraAgentModelOptions(selectedAgent), modelValue);
  const effectiveModelValue = modelValue || (modelOptions[0]?.value ?? "");
  const effortOptions = optionRows(
    astraPreferenceSource(selectedAgent?.efforts ?? [], selectedAgent?.effort),
    effortValue,
  );
  const effectiveEffortValue = effortValue || (effortOptions[0]?.value ?? "");
  const permissionRows = optionRows(
    astraPreferenceSource(selectedAgent?.permissionModes ?? [], selectedAgent?.permissionMode),
    permissionValue,
  );
  const permissionOptions = selectedAgent && isRuntimeAgent(selectedAgent.id)
    ? runtimePermissionModeOptions(permissionRows, permissionValue, selectedAgent.id)
    : permissionRows;
  const effectivePermissionValue = permissionValue || (permissionOptions[0]?.value ?? "");
  const showPermissionMode = selectedAgent?.id !== "astra-pi" && permissionOptions.length > 0;

  return (
    <section className="grid gap-3">
      <div className="min-w-0">
        <AgentInlineSelect
          value={effectiveAgentValue}
          options={agentOptions}
          placeholder={title}
          onChange={onAgentChange}
        />
      </div>
      <div className="min-w-0">
        <div className="grid gap-2 rounded-lg border border-card-border/[0.12] p-3">
          <AgentPreferenceRow label={t("assistant.model")}>
            <AstraPreferenceSelect
              value={effectiveModelValue}
              options={modelOptions}
              placeholder={t("agent.no_model")}
              onChange={onModelChange}
            />
          </AgentPreferenceRow>
          <AgentPreferenceRow label={t("assistant.effort")}>
            <AstraPreferenceSelect
              value={effectiveEffortValue}
              options={effortOptions}
              placeholder={t("assistant.effort")}
              onChange={onEffortChange}
            />
          </AgentPreferenceRow>
          {showPermissionMode && (
            <AgentPreferenceRow label={t("assistant.permission_mode")}>
              <AstraPreferenceSelect
                value={effectivePermissionValue}
                options={permissionOptions}
                placeholder={t("assistant.permission_mode")}
                onChange={onPermissionChange}
              />
            </AgentPreferenceRow>
          )}
        </div>
      </div>
    </section>
  );
}

function AstraPreferenceSelect({
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
    <AgentInlineSelect
      value={value}
      options={options}
      placeholder={placeholder}
      onChange={onChange}
    />
  );
}

function astraPreferenceSource(
  options: RuntimeAgentOptionMetadata[],
  fallbackValue: string | null | undefined,
): RuntimeAgentOptionMetadata[] {
  if (options.length > 0) return options;
  if (!fallbackValue) return [];
  return [{ value: fallbackValue, label: fallbackValue, displayName: fallbackValue, enabled: true, order: 0 }];
}

function astraAgentModelOptions(agent: AgentInfo | null): RuntimeAgentOptionMetadata[] {
  if (!agent) return [];
  if (agent.id !== "astra-pi") {
    return astraPreferenceSource(agent.models, agent.model);
  }
  const provider =
    agent.aiProviders.find((item) => item.id === agent.aiProvider)
    ?? agent.aiProviders.find((item) => item.enabled)
    ?? agent.aiProviders[0];
  return astraPreferenceSource(provider?.models ?? agent.models, provider?.model ?? agent.model);
}

function astraSelectableAgents(agents: AgentInfo[], selectedAgentId: string): AgentInfo[] {
  const supported = agents.filter((agent) => isRuntimeAgent(agent.id));
  const enabled = supported.filter((agent) => agent.enabled);
  if (!selectedAgentId || enabled.some((agent) => agent.id === selectedAgentId)) {
    return enabled;
  }
  const selected = supported.find((agent) => agent.id === selectedAgentId);
  return selected ? [selected, ...enabled] : enabled;
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
        "process-template-list-item flex h-12 w-full min-w-0 items-center gap-2 px-2 text-left text-body-sm transition " +
        (active ? "process-template-list-item-active " : "") +
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
  const isAstra = agent.id === "astra-pi";
  const [aiProvider, setAiProvider] = useState(agent.aiProvider ?? "");
  const [editingAiProvider, setEditingAiProvider] = useState(agent.aiProvider ?? "");
  const [aiProviders, setAiProviders] = useState<AgentAiProviderInfo[]>(agent.aiProviders);
  const [providerDialog, setProviderDialog] = useState<{ mode: "add" | "edit"; provider: AgentAiProviderInfo } | null>(null);
  const activeAiProvider = aiProviders.find((provider) => provider.id === aiProvider) ?? aiProviders.find((provider) => provider.enabled) ?? aiProviders[0] ?? null;
  const selectedAiProvider = aiProviders.find((provider) => provider.id === editingAiProvider) ?? activeAiProvider;

  useEffect(() => {
    const selectedProvider =
      agent.id === "astra-pi"
        ? agent.aiProviders.find((provider) => provider.id === editingAiProvider)
          ?? agent.aiProviders.find((provider) => provider.id === agent.aiProvider)
          ?? agent.aiProviders.find((provider) => provider.enabled)
          ?? agent.aiProviders[0]
        : null;
    setModel(selectedProvider?.model ?? defaultModelValue(selectedProvider?.models ?? agent.models) ?? "");
    setEffort(agent.effort ?? agent.efforts[0]?.value ?? "");
    setPermissionMode(agent.permissionMode ?? agent.permissionModes[0]?.value ?? "");
    setModels(selectedProvider?.models ?? agent.models);
    setNewModelValue("");
    setNewModelDisplayName("");
    setAiProvider(agent.aiProvider ?? "");
    setEditingAiProvider((current) => {
      const defaultProviderId = agent.aiProvider ?? agent.aiProviders.find((provider) => provider.enabled)?.id ?? agent.aiProviders[0]?.id ?? "";
      return isAstra && agent.aiProviders.some((provider) => provider.id === current) ? current : defaultProviderId;
    });
    setAiProviders(agent.aiProviders);
    setProviderDialog(null);
  }, [agent]);

  const persist = async (patch: AgentPreferencePatch): Promise<AgentInfo | null> => {
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
          aiProvider: patch.aiProvider,
          aiProviders: patch.aiProviders,
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
      if (next) {
        onUpdated(next);
        return next;
      }
    } catch (err) {
      onError(String(err));
    }
    return null;
  };

  const selectModel = async (nextModel: string) => {
    setModel(nextModel);
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(aiProviders, aiProvider, nextModel, selectedAiProvider.id);
      return;
    }
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
    const nextModel = effectiveModel || value;
    setModel(nextModel ?? "");
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, nextModels), aiProvider, nextModel, selectedAiProvider.id);
      return;
    }
    await persist({ models: nextModels, model: nextModel });
  };

  const deleteModel = async (value: string) => {
    const sourceModels = activeModels;
    const nextModels = sourceModels.filter((item) => item.value !== value);
    const orderedModels = normalizeModelOrders(nextModels);
    const nextModel = effectiveModel === value ? defaultModelValue(orderedModels) : effectiveModel || defaultModelValue(orderedModels);
    setModels(orderedModels);
    setModel(nextModel ?? "");
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, orderedModels), aiProvider, nextModel, selectedAiProvider.id);
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
    const nextModel = effectiveModel === previousValue ? value : effectiveModel || defaultModelValue(nextModels);
    setModels(nextModels);
    setModel(nextModel ?? "");
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, nextModels), aiProvider, nextModel, selectedAiProvider.id);
      return;
    }
    await persist({ models: nextModels, model: nextModel });
  };

  const moveModel = async (from: number, to: number) => {
    const nextModels = normalizeModelOrders(moveOption(activeModels, from, to));
    setModels(nextModels);
    if (isAstra && selectedAiProvider) {
      const nextModel = effectiveModel || defaultModelValue(nextModels);
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, nextModels), aiProvider, nextModel, selectedAiProvider.id);
      return;
    }
    await persist({ models: nextModels });
  };

  const setDefaultModel = async (value: string) => {
    if (effectiveModel === value) return;
    setModel(value);
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(aiProviders, aiProvider, value, selectedAiProvider.id);
      return;
    }
    await persist({ model: value });
  };

  const toggleModelEnabled = async (value: string) => {
    const sourceModels = activeModels;
    const nextModels = normalizeModelOrders(sourceModels.map((item) => item.value === value ? { ...item, enabled: !item.enabled } : item));
    const nextModel = nextModels.some((item) => item.value === effectiveModel && item.enabled)
      ? effectiveModel
      : defaultModelValue(nextModels);
    setModels(nextModels);
    setModel(nextModel ?? "");
    if (isAstra && selectedAiProvider) {
      await saveAstraProviders(updateProviderModels(aiProviders, selectedAiProvider.id, nextModels), aiProvider, nextModel, selectedAiProvider.id);
      return;
    }
    await persist({ models: nextModels, model: nextModel });
  };

  const handleModelDragEnd = (event: DragEndEvent) => {
    if (event.canceled) return;
    const { source } = event.operation;
    if (!isSortable(source)) return;
    if (source.initialIndex === source.index) return;
    void moveModel(source.initialIndex, source.index);
  };

  const activeModels = isAstra && selectedAiProvider ? selectedAiProvider.models : models;
  const preferenceModels = activeModels;
  const effectiveModel = !preferenceModels.some((item) => item.value === model) ? "" : model;
  const modelOptions = optionRows(preferenceModels, effectiveModel);
  const effortOptions = optionRows(agent.efforts, effort);
  const permissionOptions = runtimeAgent
    ? runtimePermissionModeOptions(optionRows(agent.permissionModes, permissionMode), permissionMode, runtimeAgent)
    : optionRows(agent.permissionModes, permissionMode);
  const sessionCommand = agent.commands.session[0] ?? "";
  const versionCommand = agent.commands.version[0] ?? "";
  const saveAstraProviders = async (
    nextProviders: AgentAiProviderInfo[],
    nextProviderId = aiProvider,
    nextModel: string | null | undefined = undefined,
    nextEditingProviderId = editingAiProvider,
  ): Promise<AgentInfo | null> => {
    const orderedProviders = normalizeProviderOrders(nextProviders);
    const providersWithModel = nextEditingProviderId && nextModel !== undefined
      ? updateProviderInfo(orderedProviders, nextEditingProviderId, { model: nextModel ?? null })
      : orderedProviders;
    const selectedProvider = orderedProviders.find((provider) => provider.id === nextEditingProviderId)
      ?? orderedProviders.find((provider) => provider.id === nextProviderId)
      ?? orderedProviders[0]
      ?? null;
    const activeProvider = providersWithModel.find((provider) => provider.id === nextProviderId)
      ?? providersWithModel.find((provider) => provider.enabled)
      ?? providersWithModel[0]
      ?? null;
    const persistedProviderId = activeProvider?.id ?? nextProviderId;
    const selectedProviderWithModel = providersWithModel.find((provider) => provider.id === selectedProvider?.id) ?? selectedProvider;
    const activeModelsForPersist = activeProvider?.models ?? [];
    const persistedModel = activeModelsForPersist.some((item) => item.value === (activeProvider?.model ?? ""))
      ? activeProvider?.model
      : defaultModelValue(activeModelsForPersist);
    const persistedAgent = await persist({
      aiProvider: persistedProviderId,
      aiProviders: providersWithModel,
      models: activeModelsForPersist,
      model: persistedModel ?? "",
    });
    if (!persistedAgent) {
      return null;
    }
    const persistedActiveProvider =
      persistedAgent.aiProviders.find((provider) => provider.id === persistedAgent.aiProvider)
      ?? persistedAgent.aiProviders.find((provider) => provider.enabled)
      ?? persistedAgent.aiProviders[0]
      ?? null;
    const persistedSelectedProvider =
      findPersistedProvider(persistedAgent.aiProviders, selectedProviderWithModel)
      ?? persistedActiveProvider;
    const persistedSelectedModels = persistedSelectedProvider?.models ?? [];
    setAiProviders(persistedAgent.aiProviders);
    setAiProvider(persistedActiveProvider?.id ?? persistedAgent.aiProvider ?? "");
    setEditingAiProvider(persistedSelectedProvider?.id ?? "");
    setModels(persistedSelectedModels);
    setModel(persistedSelectedProvider?.model ?? defaultModelValue(persistedSelectedModels) ?? "");
    return persistedAgent;
  };
  const selectAiProviderForEditing = (nextProviderId: string) => {
    const provider = aiProviders.find((item) => item.id === nextProviderId) ?? null;
    setEditingAiProvider(nextProviderId);
    setModels(provider?.models ?? []);
    setModel(provider?.model ?? defaultModelValue(provider?.models ?? []) ?? "");
    setNewModelValue("");
    setNewModelDisplayName("");
  };
  const activateAiProvider = async (nextProviderId: string) => {
    const nextProviders = aiProviders.map((provider) => ({ ...provider, enabled: provider.id === nextProviderId }));
    const provider = nextProviders.find((item) => item.id === nextProviderId) ?? null;
    const nextModel = provider?.models.some((item) => item.value === provider.model)
      ? provider.model
      : defaultModelValue(provider?.models ?? []);
    setEditingAiProvider(nextProviderId);
    await saveAstraProviders(nextProviders, nextProviderId, nextModel, nextProviderId);
  };
  const createProviderDraft = (): AgentAiProviderInfo => {
    return {
      id: "",
      displayName: t("agent.custom_provider"),
      provider: "openai",
      api: "openai-responses",
      baseUrl: defaultBaseUrlForPiProvider("openai"),
      apiKey: null,
      model: null,
      models: [],
      enabled: aiProviders.length === 0,
      order: aiProviders.length,
    };
  };
  const openAddProviderDialog = () => {
    setProviderDialog({ mode: "add", provider: createProviderDraft() });
  };
  const openEditProviderDialog = (provider: AgentAiProviderInfo) => {
    setProviderDialog({ mode: "edit", provider: { ...provider, models: provider.models.slice() } });
  };
  const saveProviderDialog = async (provider: AgentAiProviderInfo): Promise<boolean> => {
    const nextProvider = normalizeProviderOrders([provider])[0];
    if (!nextProvider) return false;
    if (providerDialog?.mode === "edit") {
      const nextProviders = updateProviderInfo(aiProviders, provider.id, nextProvider);
      const nextProviderId = aiProvider || activeAiProvider?.id || nextProvider.id;
      const selectedProvider = nextProviders.find((item) => item.id === editingAiProvider) ?? nextProvider;
      const saved = await saveAstraProviders(nextProviders, nextProviderId, undefined, selectedProvider.id);
      if (!saved) return false;
    } else {
      const providerToAdd = { ...nextProvider, enabled: aiProviders.length === 0 };
      const nextProviders = [...aiProviders, providerToAdd];
      const nextProviderId = aiProvider || activeAiProvider?.id || providerToAdd.id;
      const saved = await saveAstraProviders(nextProviders, nextProviderId, undefined, nextProvider.id);
      if (!saved) return false;
    }
    setProviderDialog(null);
    return true;
  };
  const deleteProvider = async (providerId: string) => {
    if (aiProviders.length <= 1) return;
    const nextProviders = aiProviders.filter((provider) => provider.id !== providerId);
    const nextActiveProvider = nextProviders.find((provider) => provider.id === aiProvider)
      ?? nextProviders.find((provider) => provider.enabled)
      ?? nextProviders[0]
      ?? null;
    const nextEditingProvider = nextProviders.find((provider) => provider.id === editingAiProvider)
      ?? nextActiveProvider;
    setEditingAiProvider(nextEditingProvider?.id ?? "");
    await saveAstraProviders(nextProviders, nextActiveProvider?.id ?? "", undefined, nextEditingProvider?.id ?? "");
  };

  return (
    <>
      <SettingsGroup title={agent.displayName}>
        <div className="grid gap-5">
          <div className="flex items-start gap-3 rounded-md border border-card-border/[0.10] bg-card-chip/[0.025] p-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-card-chip/[0.08]">
              <SettingsAgentGlyph agentId={agent.id} className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 flex-wrap items-center gap-2">
                <span className="rounded bg-card-chip/8 px-1.5 py-0.5 text-meta text-card-chip-fg/55">{agent.type}</span>
                <span className={"rounded px-1.5 py-0.5 text-meta " + (agent.enabled ? "bg-ink/[0.09] text-ink/70" : "bg-card-chip/8 text-card-muted/50")}>
                  {agent.enabled ? t("agent.active") : t("agent.disabled")}
                </span>
                {agent.transport === "acp" && <AcpLogo className="h-2.5 w-auto shrink-0 opacity-75" />}
              </div>
              <div className="mt-2 flex flex-wrap gap-1.5 text-caption text-card-muted/55">
                <span className="rounded bg-card-chip/[0.06] px-1.5 py-0.5">{agent.transport}</span>
                {sessionCommand && <span className="max-w-full truncate rounded bg-card-chip/[0.06] px-1.5 py-0.5">{sessionCommand}</span>}
                {versionCommand && <span className="max-w-full truncate rounded bg-card-chip/[0.06] px-1.5 py-0.5">{versionCommand}</span>}
              </div>
            </div>
            <SwitchControl
              checked={agent.enabled}
              tooltip={agent.enabled ? t("agent.disable") : t("agent.enable")}
              onToggle={() => void persist({ enabled: !agent.enabled })}
            />
          </div>
          {isAstra && (
            <div className="grid gap-2">
              <div className="flex items-center justify-between gap-3">
                <h3 className="text-caption font-semibold text-card-fg/72">{t("agent.providers")}</h3>
                <button type="button" onClick={openAddProviderDialog} className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md px-2 text-body-sm font-medium leading-none text-card-fg/75 transition hover:bg-card-action-hover/5 hover:text-card-fg/90">
                  <Plus className="h-4 w-4" />
                  {t("agent.add_provider")}
                </button>
              </div>
              <div className="overflow-hidden rounded-md border border-card-border/[0.10] bg-card-chip/[0.025]">
                {aiProviders.map((provider) => (
                  <AgentProviderRow
                    key={provider.id}
                    provider={provider}
                    selected={provider.id === selectedAiProvider?.id}
                    canDelete={aiProviders.length > 1}
                    onSelect={selectAiProviderForEditing}
                    onActivate={activateAiProvider}
                    onEdit={openEditProviderDialog}
                    onDelete={deleteProvider}
                  />
                ))}
                {aiProviders.length === 0 && <div className="p-3"><EmptyState label={t("agent.no_providers")} /></div>}
              </div>
            </div>
          )}
          <div className="grid gap-2">
            <h3 className="text-caption font-semibold text-card-fg/72">{t("agent.preferences")}</h3>
            <div className="grid gap-2 rounded-md border border-card-border/[0.10] bg-card-chip/[0.025] p-3">
              <AgentPreferenceRow label={t("assistant.model")}>
                <AgentInlineSelect
                  value={effectiveModel}
                  options={modelOptions}
                  placeholder={t("agent.no_model")}
                  onChange={(value) => void selectModel(value)}
                />
              </AgentPreferenceRow>
              {agent.efforts.length > 0 && (
                <AgentPreferenceRow label={isAstra ? t("agent.thinking_level") : t("assistant.effort")}>
                  <AgentInlineSelect
                    value={effort}
                    options={effortOptions}
                    placeholder={isAstra ? t("agent.thinking_level") : t("assistant.effort")}
                    onChange={(value) => void selectEffort(value)}
                  />
                </AgentPreferenceRow>
              )}
              {!isAstra && (
                <AgentPreferenceRow label={t("assistant.permission_mode")}>
                  <AgentInlineSelect
                    value={permissionMode}
                    options={permissionOptions}
                    placeholder={t("assistant.permission_mode")}
                    onChange={(value) => void selectPermissionMode(value)}
                  />
                </AgentPreferenceRow>
              )}
            </div>
          </div>
          <div className="grid gap-3">
            <h3 className="text-caption font-semibold text-card-fg/72">{isAstra ? t("agent.provider_models") : t("agent.models")}</h3>
            <div className="grid gap-3 rounded-md border border-card-border/[0.10] bg-card-chip/[0.025] p-3">
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
                      defaultModel={item.value === effectiveModel}
                      canSetDefault
                      onSetDefault={setDefaultModel}
                      onSave={saveModel}
                      onToggleEnabled={toggleModelEnabled}
                      onDelete={deleteModel}
                    />
                  ))}
                  {activeModels.length === 0 && <div className="p-3"><EmptyState label={t("agent.no_models")} /></div>}
                </div>
              </DragDropProvider>
            </div>
          </div>
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
  onActivate,
  onEdit,
  onDelete,
}: {
  provider: AgentAiProviderInfo;
  selected: boolean;
  canDelete: boolean;
  onSelect: (providerId: string) => void;
  onActivate: (providerId: string) => Promise<void>;
  onEdit: (provider: AgentAiProviderInfo) => void;
  onDelete: (providerId: string) => Promise<void>;
}) {
  const { t } = useI18n();
  const detailItems = [
    provider.provider,
    provider.api,
    t("agent.model_count", { count: provider.models.length }),
  ].filter((item) => item && item.trim().length > 0);

  return (
    <div className={"grid min-h-14 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 border-b px-3 py-2.5 transition last:border-b-0 " + (selected ? "border-card-border/[0.16] bg-[rgb(var(--color-card-fg)/0.105)]" : "border-card-border/[0.08] hover:bg-card-action-hover/5")}>
      <button type="button" onClick={() => void onSelect(provider.id)} className="flex min-w-0 items-center gap-3 text-left">
        <ProviderGlyph provider={provider} />
        <span className="min-w-0">
          <span className={"block truncate text-body-sm font-medium " + (selected ? "text-card-fg/92" : "text-card-fg/82")}>{provider.displayName || provider.provider || provider.id}</span>
          <span className="mt-0.5 block truncate text-caption text-card-muted/56">{detailItems.join(" / ")}</span>
        </span>
      </button>
      <div className="flex items-center gap-1.5">
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
        <SwitchControl
          checked={provider.enabled}
          tooltip={provider.enabled ? t("agent.active_provider") : t("agent.activate_provider")}
          onToggle={() => void onActivate(provider.id)}
        />
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
  onSave: (provider: AgentAiProviderInfo) => Promise<boolean>;
}) {
  const { t } = useI18n();
  const [displayName, setDisplayName] = useState(provider.displayName);
  const [piProvider, setPiProvider] = useState(provider.provider);
  const [api, setApi] = useState(provider.api ?? "");
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl ?? "");
  const [apiKey, setApiKey] = useState(provider.apiKey ?? "");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setDisplayName(provider.displayName);
    setPiProvider(provider.provider);
    setApi(provider.api ?? "");
    setBaseUrl(provider.baseUrl ?? "");
    setApiKey(provider.apiKey ?? "");
    setSaving(false);
  }, [provider]);

  const save = async () => {
    if (saving) return;
    const nextProvider: AgentAiProviderInfo = {
      ...provider,
      displayName: displayName.trim() || piProvider.trim() || provider.id,
      provider: piProvider.trim() || provider.id,
      api: api.trim() || null,
      baseUrl: baseUrl.trim() || null,
      apiKey: apiKey.trim() || null,
    };
    setSaving(true);
    const saved = await onSave(nextProvider);
    if (!saved) setSaving(false);
  };
  const selectPiProvider = (nextPiProvider: string) => {
    const currentDefaultBaseUrl = defaultBaseUrlForPiProvider(piProvider);
    const nextDefaultBaseUrl = defaultBaseUrlForPiProvider(nextPiProvider);
    const shouldReplaceBaseUrl =
      !baseUrl.trim() || (currentDefaultBaseUrl !== null && baseUrl.trim() === currentDefaultBaseUrl);
    setPiProvider(nextPiProvider);
    if (nextDefaultBaseUrl && shouldReplaceBaseUrl) setBaseUrl(nextDefaultBaseUrl);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 px-4" onClick={() => {
      if (!saving) onClose();
    }}>
      <div className="w-full max-w-[520px] rounded-lg border border-card-border/[0.12] bg-surface-panel p-4 shadow-[0_24px_80px_rgba(0,0,0,0.22)]" onClick={(event) => event.stopPropagation()}>
        <div className="mb-3 text-body-sm font-semibold text-ink/[0.88]">{mode === "add" ? t("agent.add_provider") : t("agent.edit_provider")}</div>
        <div className="grid gap-2">
          <AgentProviderDialogField label={t("agent.provider_name")}>
            <input value={displayName} onChange={(event) => setDisplayName(event.target.value)} placeholder="OpenAI" className={inputClassName} />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.pi_provider")}>
            <AgentDialogSelect
              value={piProvider}
              options={piProviderOptions(piProvider)}
              placeholder={t("agent.pi_provider")}
              onChange={selectPiProvider}
            />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.pi_api")}>
            <AgentDialogSelect
              value={api}
              options={piApiOptions(api)}
              placeholder={t("agent.pi_api")}
              onChange={setApi}
            />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.api_base_url")}>
            <input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} placeholder="https://api.openai.com/v1" className={inputClassName} />
          </AgentProviderDialogField>
          <AgentProviderDialogField label={t("agent.api_key")}>
            <input value={apiKey} type="password" onChange={(event) => setApiKey(event.target.value)} placeholder="sk-..." className={inputClassName} />
          </AgentProviderDialogField>
          <div className="mt-1 flex justify-end gap-2">
            <button type="button" onClick={onClose} disabled={saving} className="rounded-md px-3 py-1.5 text-body-sm text-ink/45 hover:bg-ink/5 disabled:opacity-35">{t("delete.cancel")}</button>
            <button type="button" onClick={() => void save()} disabled={saving || !piProvider.trim()} className="inline-flex h-8 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] hover:text-card-fg/90 disabled:opacity-35">
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

function AgentDialogSelect({
  value,
  options,
  placeholder,
  onChange,
}: {
  value: string;
  options: Array<{ value: string; label: string; description?: string }>;
  placeholder: string;
  onChange: (value: string) => void;
}) {
  return (
    <InlineMenuSelect
      value={value}
      options={options}
      onChange={onChange}
      menuAlign="trigger"
      placeholder={placeholder}
      ariaLabel={placeholder}
      className="h-9 w-full !max-w-full rounded-md border border-input-border/[0.16] bg-input px-3 text-input-fg hover:text-input-fg"
      menuClassName="bg-surface-panel"
      minMenuWidth={260}
      emptyContent={placeholder}
    />
  );
}

function ProviderGlyph({ provider }: { provider: AgentAiProviderInfo }) {
  const Icon = providerIconFor(provider.provider);
  const className = "h-5 w-5 shrink-0";
  if (Icon) return <Icon className={className} />;
  const label = provider.displayName.trim() || provider.provider.trim() || provider.id;
  const initial = label.trim().charAt(0).toUpperCase() || "?";
  return (
    <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-card-border/[0.16] bg-card-chip/[0.08] text-caption font-semibold text-card-fg/70">
      {initial}
    </span>
  );
}

type ProviderIconComponent = (props: { className?: string }) => ReactNode;

function providerIconFor(provider: string): ProviderIconComponent | null {
  const key = normalizeProviderKey(provider);
  const icon = PROVIDER_ICON_BY_KEY[key];
  if (icon) return icon;
  if (key.includes("openai")) return OpenAI;
  if (key.includes("anthropic") || key.includes("claude")) return iconColor(Anthropic, Claude.Color);
  if (key.includes("google") || key.includes("gemini")) return iconColor(Google, Gemini.Color);
  if (key.includes("azure")) return iconColor(Azure, Azure);
  if (key.includes("githubcopilot")) return iconColor(GithubCopilot, GithubCopilot);
  if (key.includes("cloudflare")) return iconColor(Cloudflare, Cloudflare);
  if (key.includes("moonshot") || key.includes("kimi")) return iconColor(Moonshot, iconColor(Kimi, Moonshot));
  if (key.includes("minimax")) return iconColor(Minimax, Minimax);
  if (key.includes("xiaomi")) return iconColor(XiaomiMiMo, XiaomiMiMo);
  return null;
}

function iconColor(icon: unknown, fallback: ProviderIconComponent): ProviderIconComponent {
  return ((icon as { Color?: ProviderIconComponent }).Color ?? fallback);
}

function normalizeProviderKey(provider: string): string {
  return provider.trim().toLowerCase().replace(/[^a-z0-9]/g, "");
}

function AgentModelRow({
  item,
  index,
  defaultModel,
  canSetDefault,
  onSetDefault,
  onSave,
  onToggleEnabled,
  onDelete,
}: {
  item: RuntimeAgentOptionMetadata;
  index: number;
  defaultModel: boolean;
  canSetDefault: boolean;
  onSetDefault: (value: string) => Promise<void>;
  onSave: (previousValue: string, nextValue: string, nextLabel: string) => Promise<void>;
  onToggleEnabled: (value: string) => Promise<void>;
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
        "grid min-h-12 grid-cols-[auto_minmax(0,1fr)_minmax(0,0.72fr)_auto_auto_auto_auto] items-center gap-2 border-b border-card-border/[0.08] px-3 py-2 transition last:border-b-0 " +
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
      <Tooltip content={canSetDefault ? t("agent.make_default") : t("agent.activate_provider")} placement="top">
        <button
          type="button"
          role="radio"
          aria-checked={defaultModel}
          disabled={!canSetDefault}
          onClick={() => void onSetDefault(item.value)}
          className={`rounded p-1 disabled:opacity-35 ${defaultModel ? "bg-card-chip/[0.12] text-card-fg/75" : "text-card-subtle/45 hover:bg-card-action-hover/5 hover:text-card-fg/75 disabled:hover:bg-transparent disabled:hover:text-card-subtle/45"}`}
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
      <SwitchControl
        checked={item.enabled}
        tooltip={item.enabled ? t("agent.disable_model") : t("agent.enable_model")}
        onToggle={() => void onToggleEnabled(item.value)}
      />
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

function defaultModelValue(options: RuntimeAgentOptionMetadata[]): string | null {
  return options.find((item) => item.enabled)?.value ?? options[0]?.value ?? null;
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
    id: provider.id.trim(),
    displayName: provider.displayName.trim() || provider.provider.trim() || provider.id,
    provider: provider.provider.trim() || provider.id.trim() || `provider-${index + 1}`,
    api: provider.api?.trim() || null,
    baseUrl: provider.baseUrl?.trim() || null,
    apiKey: provider.apiKey?.trim() || null,
    model: provider.model?.trim() || null,
    models: normalizeModelOrders(provider.models),
    enabled: provider.enabled ?? true,
    order: index,
  }));
}

const PI_PROVIDER_PRESETS = [
  "openai",
  "anthropic",
  "google",
  "google-vertex",
  "amazon-bedrock",
  "azure-openai-responses",
  "openai-codex",
  "deepseek",
  "github-copilot",
  "xai",
  "groq",
  "cerebras",
  "openrouter",
  "vercel-ai-gateway",
  "zai",
  "mistral",
  "minimax",
  "minimax-cn",
  "moonshotai",
  "moonshotai-cn",
  "huggingface",
  "fireworks",
  "together",
  "opencode",
  "opencode-go",
  "kimi-coding",
  "cloudflare-workers-ai",
  "cloudflare-ai-gateway",
  "xiaomi",
  "xiaomi-token-plan-cn",
  "xiaomi-token-plan-ams",
  "xiaomi-token-plan-sgp",
];

const PROVIDER_LABEL_BY_ID: Record<string, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  google: "Google AI",
  "google-vertex": "Google Vertex AI",
  "amazon-bedrock": "Amazon Bedrock",
  "azure-openai-responses": "Azure OpenAI",
  "openai-codex": "OpenAI Codex",
  deepseek: "DeepSeek",
  "github-copilot": "GitHub Copilot",
  xai: "xAI",
  groq: "Groq",
  cerebras: "Cerebras",
  openrouter: "OpenRouter",
  "vercel-ai-gateway": "Vercel AI Gateway",
  zai: "Z.ai",
  mistral: "Mistral",
  minimax: "MiniMax",
  "minimax-cn": "MiniMax CN",
  moonshotai: "Moonshot AI",
  "moonshotai-cn": "Moonshot AI CN",
  huggingface: "Hugging Face",
  fireworks: "Fireworks",
  together: "Together AI",
  opencode: "OpenCode",
  "opencode-go": "OpenCode Go",
  "kimi-coding": "Kimi Coding",
  "cloudflare-workers-ai": "Cloudflare Workers AI",
  "cloudflare-ai-gateway": "Cloudflare AI Gateway",
  xiaomi: "Xiaomi",
  "xiaomi-token-plan-cn": "Xiaomi Token Plan CN",
  "xiaomi-token-plan-ams": "Xiaomi Token Plan AMS",
  "xiaomi-token-plan-sgp": "Xiaomi Token Plan SGP",
};

const PROVIDER_ICON_BY_KEY: Record<string, ProviderIconComponent> = {
  openai: OpenAI,
  anthropic: iconColor(Anthropic, Claude.Color),
  google: iconColor(Google, Gemini.Color),
  googlevertex: iconColor(VertexAI, iconColor(GoogleCloud, iconColor(Google, Gemini.Color))),
  amazonbedrock: iconColor(Bedrock, iconColor(Aws, Aws)),
  azureopenairesponses: iconColor(Azure, Azure),
  openaicodex: OpenAI,
  deepseek: iconColor(DeepSeek, DeepSeek),
  githubcopilot: iconColor(GithubCopilot, GithubCopilot),
  xai: iconColor(XAI, XAI),
  groq: iconColor(Groq, Groq),
  cerebras: iconColor(Cerebras, Cerebras),
  openrouter: iconColor(OpenRouter, OpenRouter),
  vercelaigateway: iconColor(Vercel, Vercel),
  zai: iconColor(ZAI, ZAI),
  mistral: iconColor(Mistral, Mistral),
  minimax: iconColor(Minimax, Minimax),
  minimaxcn: iconColor(Minimax, Minimax),
  moonshotai: iconColor(Moonshot, Moonshot),
  moonshotaicn: iconColor(Moonshot, Moonshot),
  huggingface: iconColor(HuggingFace, HuggingFace),
  fireworks: iconColor(Fireworks, Fireworks),
  together: iconColor(Together, Together),
  opencode: iconColor(OpenCode, OpenCode),
  opencodego: iconColor(OpenCode, OpenCode),
  kimicoding: iconColor(Kimi, iconColor(Moonshot, Kimi)),
  cloudflareworkersai: iconColor(WorkersAI, iconColor(Cloudflare, Cloudflare)),
  cloudflareaigateway: iconColor(Cloudflare, Cloudflare),
  xiaomi: iconColor(XiaomiMiMo, XiaomiMiMo),
  xiaomitokenplancn: iconColor(XiaomiMiMo, XiaomiMiMo),
  xiaomitokenplanams: iconColor(XiaomiMiMo, XiaomiMiMo),
  xiaomitokenplansgp: iconColor(XiaomiMiMo, XiaomiMiMo),
};

const PI_API_PRESETS = [
  "openai-responses",
  "openai-completions",
  "openai-codex-responses",
  "anthropic-messages",
  "google-generative-ai",
  "google-vertex",
  "azure-openai-responses",
  "bedrock-converse-stream",
  "mistral-conversations",
];

const DEFAULT_BASE_URL_BY_PI_PROVIDER: Record<string, string> = {
  openai: "https://api.openai.com/v1",
  anthropic: "https://api.anthropic.com/v1",
  google: "https://generativelanguage.googleapis.com/v1beta",
  "google-vertex": "https://generativelanguage.googleapis.com/v1beta",
  "azure-openai-responses": "https://{resource}.openai.azure.com/openai/v1",
  deepseek: "https://api.deepseek.com/v1",
  xai: "https://api.x.ai/v1",
  groq: "https://api.groq.com/openai/v1",
  cerebras: "https://api.cerebras.ai/v1",
  openrouter: "https://openrouter.ai/api/v1",
  "vercel-ai-gateway": "https://ai-gateway.vercel.sh/v1",
  zai: "https://api.z.ai/api/paas/v4",
  mistral: "https://api.mistral.ai/v1",
  minimax: "https://api.minimax.io/v1",
  "minimax-cn": "https://api.minimax.chat/v1",
  moonshotai: "https://api.moonshot.ai/v1",
  "moonshotai-cn": "https://api.moonshot.cn/v1",
  huggingface: "https://router.huggingface.co/v1",
  fireworks: "https://api.fireworks.ai/inference/v1",
  together: "https://api.together.xyz/v1",
  "kimi-coding": "https://api.kimi.com/v1",
  "cloudflare-ai-gateway": "https://gateway.ai.cloudflare.com/v1",
};

function presetOptions(presets: string[], selected: string, labels: Record<string, string> = {}): Array<{ value: string; label: string }> {
  const options = presets.map((value) => ({ value, label: labels[value] ?? value }));
  if (selected && !presets.includes(selected)) return [{ value: selected, label: labels[selected] ?? selected }, ...options];
  return options;
}

function piProviderOptions(selected: string): Array<{ value: string; label: string; icon?: ReactNode; menuIcon?: ReactNode }> {
  return presetOptions(PI_PROVIDER_PRESETS, selected, PROVIDER_LABEL_BY_ID).map((option) => {
    const Icon = providerIconFor(option.value);
    return {
      ...option,
      icon: Icon ? <Icon className="h-4 w-4" /> : undefined,
      menuIcon: Icon ? <Icon className="h-4 w-4" /> : undefined,
    };
  });
}

function piApiOptions(selected: string): Array<{ value: string; label: string }> {
  return presetOptions(PI_API_PRESETS, selected);
}

function defaultBaseUrlForPiProvider(provider: string): string | null {
  return DEFAULT_BASE_URL_BY_PI_PROVIDER[provider.trim()] ?? null;
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

function findPersistedProvider(
  providers: AgentAiProviderInfo[],
  draft: AgentAiProviderInfo | null,
): AgentAiProviderInfo | null {
  if (!draft) return null;
  if (draft.id) {
    const byId = providers.find((provider) => provider.id === draft.id);
    if (byId) return byId;
  }
  return providers.find((provider) => provider.order === draft.order) ?? null;
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
  return id === "astra-pi" || id === "codex" || id === "claude" || id === "gemini";
}

function isSettingsAgent(id: string): boolean {
  return isRuntimeAgent(id);
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

  const sharedAssistants = assistants.filter((assistant) => assistant.projectId === null && assistant.processTemplateId === null);
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

function ProcessTemplatesSettings({ onError }: { onError: (error: string | null) => void }) {
  const { t } = useI18n();
  const [processTemplates, setProcessTemplates] = useState<ProcessTemplateInfo[]>([]);
  const [selectedProcessTemplateId, setSelectedProcessTemplateId] = useState("code");
  const [stages, setStages] = useState<ProjectStageInfo[]>([]);
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);
  const [newProcessTemplateName, setNewProcessTemplateName] = useState("");
  const [newProcessTemplateDescription, setNewProcessTemplateDescription] = useState("");
  const [showCreateProcessTemplate, setShowCreateProcessTemplate] = useState(false);
  const [loading, setLoading] = useState(true);

  const selectedProcessTemplate = processTemplates.find((process) => process.id === selectedProcessTemplateId) ?? processTemplates[0] ?? null;
  const availableAssistants = assistants.filter((assistant) => assistant.enabled && assistant.projectId === null && (
    assistant.processTemplateId === selectedProcessTemplateId ||
    (assistant.processTemplateId === null && assistant.type === "custom")
  ));

  const reloadAll = async () => {
    setLoading(true);
    try {
      const [processRows, assistantRows] = await Promise.all([listProcessTemplates(), listAssistants(null)]);
      setProcessTemplates(processRows);
      setAssistants(assistantRows);
      setSelectedProcessTemplateId((current) => processRows.some((process) => process.id === current) ? current : processRows[0]?.id ?? "code");
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  };

  const reloadStages = async (processTemplateId: string) => {
    try {
      setStages(await listProcessTemplateStages(processTemplateId));
    } catch (err) {
      onError(String(err));
    }
  };

  useEffect(() => {
    void reloadAll();
  }, []);

  useEffect(() => {
    if (selectedProcessTemplateId) void reloadStages(selectedProcessTemplateId);
  }, [selectedProcessTemplateId]);

  const createNewProcessTemplate = async () => {
    const name = newProcessTemplateName.trim();
    if (!name) return;
    try {
      const process = await createProcessTemplate(name, newProcessTemplateDescription);
      setProcessTemplates((prev) => [...prev, process]);
      setSelectedProcessTemplateId(process.id);
      setNewProcessTemplateName("");
      setNewProcessTemplateDescription("");
      setShowCreateProcessTemplate(false);
    } catch (err) {
      onError(String(err));
    }
  };

  const refreshStages = async () => {
    if (selectedProcessTemplateId) await reloadStages(selectedProcessTemplateId);
  };

  const processDescription = (process: ProcessTemplateInfo) => {
    if (!process.description) return t("settings.process_template_no_description");
    return process.type === "builtin" ? t(process.description) : process.description;
  };

  return (
    <section>
      <div className="grid grid-cols-[240px_minmax(0,1fr)] gap-5">
        <div className="min-w-0">
          <SettingsGroup
            title={t("settings.process_templates")}
            flush
            action={
              <button type="button" onClick={() => setShowCreateProcessTemplate(true)} className="inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 text-body-sm font-medium leading-none text-card-fg/75 transition hover:text-card-fg/90">
                <Plus className="h-3.5 w-3.5" />
                {t("settings.add_process_template")}
              </button>
            }
          >
            <div className="divide-y divide-card-border/10">
              {processTemplates.map((process) => (
                <Tooltip key={process.id} content={processDescription(process)} placement="right">
                  <button
                    type="button"
                    onClick={() => setSelectedProcessTemplateId(process.id)}
                    className={"process-template-list-item flex h-10 w-full min-w-0 items-center justify-between px-3 text-left text-body-sm transition " + (process.id === selectedProcessTemplateId ? "process-template-list-item-active" : "")}
                  >
                    <span className="truncate">{process.name}</span>
                    <span className="rounded bg-card-chip/8 px-1.5 py-0.5 text-meta text-card-chip-fg/55">{process.type}</span>
                  </button>
                </Tooltip>
              ))}
            </div>
          </SettingsGroup>
        </div>
        <div className="min-w-0">
          {selectedProcessTemplate && (
            <ProcessTemplateEditor
              stages={stages}
              assistants={availableAssistants}
              loading={loading}
              processTemplateId={selectedProcessTemplateId}
              onStageCreated={(stage) => setStages((prev) => [...prev, stage].sort((a, b) => a.order - b.order))}
              onStageUpdated={(stage) => setStages((prev) => prev.map((item) => item.id === stage.id ? stage : item).sort((a, b) => a.order - b.order))}
              onStagesReload={refreshStages}
              onStageDeleted={(id) => setStages((prev) => prev.filter((stage) => stage.id !== id))}
              onError={onError}
            />
          )}
        </div>
      </div>
      {showCreateProcessTemplate && (
        <CreateProcessTemplateDialog
          name={newProcessTemplateName}
          description={newProcessTemplateDescription}
          onNameChange={setNewProcessTemplateName}
          onDescriptionChange={setNewProcessTemplateDescription}
          onCreate={() => void createNewProcessTemplate()}
          onClose={() => setShowCreateProcessTemplate(false)}
        />
      )}
    </section>
  );
}

function CreateProcessTemplateDialog({
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
        <div className="mb-3 text-body-sm font-semibold text-ink/[0.88]">{t("settings.add_process_template")}</div>
        <div className="grid gap-2">
          <input value={name} onChange={(event) => onNameChange(event.target.value)} placeholder={t("settings.process_template_name")} className={inputClassName} />
          <textarea value={description} onChange={(event) => onDescriptionChange(event.target.value)} placeholder={t("settings.process_template_description")} rows={3} className={textareaClassName} />
          <div className="flex justify-end gap-2">
            <button type="button" onClick={onClose} className="rounded-md px-3 py-1.5 text-body-sm text-ink/45 hover:bg-ink/5">{t("delete.cancel")}</button>
            <button type="button" onClick={onCreate} disabled={!name.trim()} className="inline-flex h-8 items-center gap-1.5 rounded-md border border-card-border/[0.12] bg-card-chip/[0.08] px-3 text-body-sm font-medium text-card-fg/75 transition hover:border-card-border/[0.18] hover:bg-card-chip/[0.12] hover:text-card-fg/90 disabled:opacity-35">
              <Plus className="h-4 w-4" />
              {t("settings.add_process_template")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function ProcessTemplateEditor({
  stages,
  assistants,
  loading,
  processTemplateId,
  onStageCreated,
  onStageUpdated,
  onStagesReload,
  onStageDeleted,
  onError,
}: {
  stages: ProjectStageInfo[];
  assistants: AssistantInfo[];
  loading: boolean;
  processTemplateId: string;
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
          dragGroup="process-stages"
          onUpdated={onStageUpdated}
          onDeleted={onStageDeleted}
          onReload={onStagesReload}
          onError={onError}
        />
      </SettingsGroup>
      {showCreateStage && (
        <CreateStageDialog
          processTemplateId={processTemplateId}
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
