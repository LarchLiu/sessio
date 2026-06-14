import { useEffect, useMemo, useState } from "react";
import { Pencil, Play, Plus, Trash2 } from "lucide-react";
import {
  type Agent,
  type AgentInfo,
  type AssistantAgentInfo,
  type ProjectInfo,
  type Schedule,
  type ScheduledTask,
  type TaskTarget,
  getScheduledTasks,
  listAgents,
  listProjects,
  runScheduledTaskNow,
  saveScheduledTasks,
} from "../api";
import { useI18n } from "../i18n";
import ScrollArea from "../components/ScrollArea";
import SegmentedTabs from "../components/SegmentedTabs";
import InlineMenuSelect, { type InlineMenuSelectOption } from "../components/InlineMenuSelect";
import AssistantAgentSelector, {
  defaultAssistantAgent,
  dbAgentsAsRuntimeAgents,
} from "../components/AssistantAgentSelector";
import {
  DiscordLogoIcon,
  LarkLogoIcon,
  TelegramLogoIcon,
  WechatLogoIcon,
} from "../components/IconifyIcon";

// Platform display name + logo, mirroring the channel settings tabs.
const PLATFORM_META: {
  value: string;
  label: string;
  labelKey?: string;
  Icon: (props: { className?: string }) => React.ReactNode;
}[] = [
  { value: "telegram", label: "Telegram", Icon: TelegramLogoIcon },
  { value: "discord", label: "Discord", Icon: DiscordLogoIcon },
  { value: "feishu", label: "Feishu", labelKey: "settings.feishu_platform", Icon: LarkLogoIcon },
  { value: "wechat", label: "WeChat", Icon: WechatLogoIcon },
];

type ScheduleKind = Schedule["kind"];
type TargetKind = TaskTarget["kind"];
type LocalTaskTarget = Extract<TaskTarget, { kind: "local" }>;

function defaultSchedule(kind: ScheduleKind): Schedule {
  switch (kind) {
    case "interval":
      return { kind: "interval", everySecs: 3600 };
    case "daily":
      return { kind: "daily", hour: 9, minute: 0 };
    case "weekly":
      return { kind: "weekly", weekday: 1, hour: 9, minute: 0 };
    case "cron":
      return { kind: "cron", expr: "0 9 * * *" };
  }
}

function defaultLocalTarget(agents: AgentInfo[]): LocalTaskTarget {
  const runtimeAgent = dbAgentsAsRuntimeAgents(agents)[0] ?? null;
  const assistantAgent = defaultAssistantAgent(runtimeAgent);
  return {
    kind: "local",
    workspacePath: "",
    agent: runtimeAgent?.agent ?? "claude",
    model: assistantAgent.model.trim() || null,
    effort: assistantAgent.effort.trim() || null,
    permissionMode: assistantAgent.mode.trim() || null,
  };
}

function defaultTarget(kind: TargetKind, agents: AgentInfo[] = []): TaskTarget {
  if (kind === "local") return defaultLocalTarget(agents);
  return { kind: "im", platform: "telegram", chatId: "" };
}

function emptyTask(agents: AgentInfo[] = []): ScheduledTask {
  return {
    id: "",
    name: "",
    enabled: true,
    prompt: "",
    schedule: defaultSchedule("daily"),
    target: defaultTarget("local", agents),
    createdAtMs: 0,
    updatedAtMs: 0,
    lastRunAtMs: null,
  };
}

function scheduleErrorKey(schedule: Schedule): string | null {
  switch (schedule.kind) {
    case "interval":
      return schedule.everySecs > 0 ? null : "autoTasks.error.invalid_schedule";
    case "daily":
      return isHour(schedule.hour) && isMinute(schedule.minute) ? null : "autoTasks.error.invalid_schedule";
    case "weekly":
      return schedule.weekday >= 0 && schedule.weekday <= 6 && isHour(schedule.hour) && isMinute(schedule.minute)
        ? null
        : "autoTasks.error.invalid_schedule";
    case "cron":
      return schedule.expr.trim().split(/\s+/).length === 5 ? null : "autoTasks.error.invalid_cron";
  }
}

function isHour(value: number): boolean {
  return Number.isInteger(value) && value >= 0 && value <= 23;
}

function isMinute(value: number): boolean {
  return Number.isInteger(value) && value >= 0 && value <= 59;
}

export default function AutoTasksPage({
  onError,
}: {
  onError: (error: string | null) => void;
}) {
  const { t, lang } = useI18n();
  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [draft, setDraft] = useState<ScheduledTask | null>(null);
  const [busy, setBusy] = useState(false);

  const load = async () => {
    try {
      const [taskList, projectList, agentList] = await Promise.all([
        getScheduledTasks(),
        listProjects(),
        listAgents(),
      ]);
      setTasks(taskList);
      setProjects(projectList);
      setAgents(agentList);
    } catch (error) {
      onError(String(error));
    }
  };

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const weekdayNames = useMemo(
    () => Array.from({ length: 7 }, (_, i) => t(`autoTasks.weekday.${i}`)),
    [t],
  );

  const scheduleSummary = (s: Schedule): string => {
    switch (s.kind) {
      case "interval":
        return `${t("autoTasks.schedule.interval")} · ${s.everySecs}s`;
      case "daily":
        return `${t("autoTasks.schedule.daily")} · ${pad(s.hour)}:${pad(s.minute)}`;
      case "weekly":
        return `${weekdayNames[s.weekday] ?? s.weekday} · ${pad(s.hour)}:${pad(s.minute)}`;
      case "cron":
        return `cron · ${s.expr}`;
    }
  };

  const platformLabel = (platform: string): string => {
    const meta = PLATFORM_META.find((item) => item.value === platform);
    if (!meta) return platform;
    return meta.labelKey ? t(meta.labelKey) : meta.label;
  };

  const targetSummary = (tg: TaskTarget): string =>
    tg.kind === "local"
      ? `${t("autoTasks.target.local")} · ${tg.agent}`
      : `${t("autoTasks.target.im")} · ${platformLabel(tg.platform)}`;

  const persist = async (next: ScheduledTask[]) => {
    setBusy(true);
    try {
      const saved = await saveScheduledTasks(next);
      setTasks(saved);
      return true;
    } catch (error) {
      onError(String(error));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const handleSave = async () => {
    if (!draft) return;
    if (!draft.name.trim() || !draft.prompt.trim()) {
      onError(t("autoTasks.field.name") + " / " + t("autoTasks.field.prompt"));
      return;
    }
    const scheduleError = scheduleErrorKey(draft.schedule);
    if (scheduleError) {
      onError(t(scheduleError));
      return;
    }
    const target = draft.target;
    if (target.kind === "local") {
      if (!target.workspacePath.trim()) {
        onError(t("autoTasks.target.workspace"));
        return;
      }
      if (!dbAgentsAsRuntimeAgents(agents).some((agent) => agent.agent === target.agent)) {
        onError(t("autoTasks.error.invalid_agent"));
        return;
      }
    }
    if (draft.target.kind === "im" && !draft.target.chatId.trim()) {
      onError(t("autoTasks.target.chat_id"));
      return;
    }
    const now = Date.now();
    const savedDraft = {
      ...draft,
      createdAtMs: draft.createdAtMs || now,
      updatedAtMs: now,
    };
    const next = savedDraft.id
      ? tasks.map((task) => (task.id === savedDraft.id ? savedDraft : task))
      : [...tasks, savedDraft];
    if (await persist(next)) setDraft(null);
  };

  const handleDelete = async (id: string) => {
    if (!window.confirm(t("autoTasks.delete_confirm"))) return;
    await persist(tasks.filter((task) => task.id !== id));
  };

  const handleToggle = async (task: ScheduledTask) => {
    const updatedAtMs = Date.now();
    await persist(
      tasks.map((item) =>
        item.id === task.id ? { ...item, enabled: !item.enabled, updatedAtMs } : item,
      ),
    );
  };

  const handleRunNow = async (id: string) => {
    setBusy(true);
    try {
      await runScheduledTaskNow(id);
      onError(t("autoTasks.ran_now"));
      await load();
    } catch (error) {
      onError(String(error));
    } finally {
      setBusy(false);
    }
  };

  const formatLastRun = (task: ScheduledTask): string =>
    task.lastRunAtMs
      ? t("autoTasks.last_run", {
          time: new Date(task.lastRunAtMs).toLocaleString(lang === "zh" ? "zh-CN" : "en-US"),
        })
      : t("autoTasks.never_run");

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col text-body text-ink">
      <ScrollArea className="min-h-0 flex-1 bg-surface-panel" viewportClassName="px-10 pt-6 pb-16">
        <div className="mx-auto max-w-[840px]">
          <div className="mb-5 flex items-center justify-between gap-4">
            <p className="text-body-sm text-ink/55">{t("autoTasks.subtitle")}</p>
            <button
              type="button"
              disabled={busy || draft !== null}
              onClick={() => setDraft(emptyTask(agents))}
              className="flex items-center gap-1.5 rounded-md bg-ink/[0.08] px-3 py-1.5 text-body-sm font-medium text-ink/85 transition hover:bg-ink/[0.12] disabled:opacity-40"
            >
              <Plus className="h-4 w-4" />
              {t("autoTasks.new")}
            </button>
          </div>

          {draft && (
            <TaskEditor
              draft={draft}
              setDraft={setDraft}
              onSave={handleSave}
              onCancel={() => setDraft(null)}
              busy={busy}
              weekdayNames={weekdayNames}
              projects={projects}
              agents={agents}
            />
          )}

          {tasks.length === 0 && !draft ? (
            <p className="rounded-lg border border-dashed border-ink/15 px-4 py-10 text-center text-body-sm text-ink/45">
              {t("autoTasks.empty")}
            </p>
          ) : (
            <ul className="flex flex-col gap-2">
              {tasks.map((task) => (
                <li
                  key={task.id}
                  className="flex items-center gap-3 rounded-lg border border-ink/10 bg-surface px-4 py-3"
                >
                  <button
                    type="button"
                    onClick={() => void handleToggle(task)}
                    title={t("autoTasks.enabled")}
                    className={
                      "h-4 w-4 shrink-0 rounded-full border transition " +
                      (task.enabled
                        ? "border-emerald bg-emerald"
                        : "border-ink/30 bg-transparent")
                    }
                  />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-body font-medium text-ink/88">
                      {task.name || task.id}
                    </div>
                    <div className="truncate text-caption text-ink/50">
                      {scheduleSummary(task.schedule)} · {targetSummary(task.target)} ·{" "}
                      {formatLastRun(task)}
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    <IconButton title={t("autoTasks.run_now")} onClick={() => void handleRunNow(task.id)} disabled={busy}>
                      <Play className="h-4 w-4" />
                    </IconButton>
                    <IconButton title={t("autoTasks.edit")} onClick={() => setDraft(task)} disabled={busy || draft !== null}>
                      <Pencil className="h-4 w-4" />
                    </IconButton>
                    <IconButton title={t("autoTasks.delete")} onClick={() => void handleDelete(task.id)} disabled={busy}>
                      <Trash2 className="h-4 w-4" />
                    </IconButton>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>
      </ScrollArea>
    </div>
  );
}

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

function IconButton({
  children,
  title,
  onClick,
  disabled,
}: {
  children: React.ReactNode;
  title: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      disabled={disabled}
      className="rounded-md p-1.5 text-ink/55 transition hover:bg-ink/5 hover:text-ink disabled:opacity-40"
    >
      {children}
    </button>
  );
}

function TaskEditor({
  draft,
  setDraft,
  onSave,
  onCancel,
  busy,
  weekdayNames,
  projects,
  agents,
}: {
  draft: ScheduledTask;
  setDraft: (task: ScheduledTask) => void;
  onSave: () => void;
  onCancel: () => void;
  busy: boolean;
  weekdayNames: string[];
  projects: ProjectInfo[];
  agents: AgentInfo[];
}) {
  const { t } = useI18n();
  const schedule = draft.schedule;
  const target = draft.target;

  const setSchedule = (next: Schedule) => setDraft({ ...draft, schedule: next });
  const setTarget = (next: TaskTarget) => setDraft({ ...draft, target: next });

  const workspaceOptions = useMemo<InlineMenuSelectOption[]>(() => {
    const options = projects.map((project) => ({
      value: project.path,
      label: `${project.name} · ${project.path}`,
    }));
    // Keep a previously-saved workspace selectable even if it is no longer a
    // registered project.
    if (
      target.kind === "local" &&
      target.workspacePath &&
      !projects.some((project) => project.path === target.workspacePath)
    ) {
      options.unshift({
        value: target.workspacePath,
        label: target.workspacePath,
      });
    }
    return options;
  }, [projects, target]);

  // Build the combined agent/model/effort/mode payload the shared selector
  // expects, layering any stored overrides over the agent's defaults.
  const localAssistantAgent = useMemo<AssistantAgentInfo>(() => {
    const agentId = target.kind === "local" ? target.agent : "claude";
    const runtimeAgents = dbAgentsAsRuntimeAgents(agents);
    const selected = runtimeAgents.find((runtimeAgent) => runtimeAgent.agent === agentId) ?? null;
    const base = defaultAssistantAgent(selected);
    if (target.kind !== "local") return base;
    return {
      id: agentId,
      name: base.name,
      model: target.model || base.model,
      mode: target.permissionMode || base.mode,
      effort: target.effort || base.effort,
    };
  }, [agents, target]);

  return (
    <div className="mb-4 flex flex-col gap-4 rounded-lg border border-ink/12 bg-surface p-5">
      <Field label={t("autoTasks.field.name")}>
        <input
          className={inputClass}
          value={draft.name}
          placeholder={t("autoTasks.field.name_placeholder")}
          onChange={(e) => setDraft({ ...draft, name: e.target.value })}
        />
      </Field>

      <Field label={t("autoTasks.field.prompt")}>
        <textarea
          className={inputClass + " min-h-[88px] resize-y"}
          value={draft.prompt}
          placeholder={t("autoTasks.field.prompt_placeholder")}
          onChange={(e) => setDraft({ ...draft, prompt: e.target.value })}
        />
      </Field>

      <Field label={t("autoTasks.field.schedule")}>
        <SegmentedTabs<ScheduleKind>
          items={(["interval", "daily", "weekly", "cron"] as ScheduleKind[]).map((k) => ({
            value: k,
            label: t(`autoTasks.schedule.${k}`),
          }))}
          value={schedule.kind}
          onChange={(k) => setSchedule(defaultSchedule(k))}
          itemWidth={72}
          itemHeight={28}
          className="self-start"
        />
        <div className="mt-3 flex flex-wrap gap-3">
          {schedule.kind === "interval" && (
            <Labeled label={t("autoTasks.schedule.every_secs")}>
              <input
                type="number"
                min={1}
                className={inputClass + " w-32"}
                value={schedule.everySecs}
                onChange={(e) =>
                  setSchedule({ kind: "interval", everySecs: Math.max(1, Number(e.target.value) || 0) })
                }
              />
            </Labeled>
          )}
          {(schedule.kind === "daily" || schedule.kind === "weekly") && (
            <>
              {schedule.kind === "weekly" && (
                <Labeled label={t("autoTasks.schedule.weekday")}>
                  <FormSelect
                    width="w-36"
                    ariaLabel={t("autoTasks.schedule.weekday")}
                    value={String(schedule.weekday)}
                    options={weekdayNames.map((name, i) => ({ value: String(i), label: name }))}
                    onChange={(v) => setSchedule({ ...schedule, weekday: Number(v) })}
                  />
                </Labeled>
              )}
              <Labeled label={t("autoTasks.schedule.hour")}>
                <input
                  type="number"
                  min={0}
                  max={23}
                  className={inputClass + " w-24"}
                  value={schedule.hour}
                  onChange={(e) =>
                    setSchedule({ ...schedule, hour: clamp(Number(e.target.value), 0, 23) })
                  }
                />
              </Labeled>
              <Labeled label={t("autoTasks.schedule.minute")}>
                <input
                  type="number"
                  min={0}
                  max={59}
                  className={inputClass + " w-24"}
                  value={schedule.minute}
                  onChange={(e) =>
                    setSchedule({ ...schedule, minute: clamp(Number(e.target.value), 0, 59) })
                  }
                />
              </Labeled>
            </>
          )}
          {schedule.kind === "cron" && (
            <Labeled label={t("autoTasks.schedule.cron_expr")}>
              <input
                className={inputClass + " w-72 font-mono"}
                value={schedule.expr}
                onChange={(e) => setSchedule({ kind: "cron", expr: e.target.value })}
              />
              <p className="mt-1 text-caption text-ink/45">{t("autoTasks.schedule.cron_hint")}</p>
            </Labeled>
          )}
        </div>
      </Field>

      <Field label={t("autoTasks.field.target")}>
        <SegmentedTabs<TargetKind>
          items={(["local", "im"] as TargetKind[]).map((k) => ({
            value: k,
            label: t(`autoTasks.target.${k}`),
          }))}
          value={target.kind}
          onChange={(k) => setTarget(defaultTarget(k, agents))}
          itemWidth={96}
          itemHeight={28}
          className="self-start"
        />
        <div className="mt-3 flex flex-col gap-3">
          {target.kind === "local" ? (
            <>
              <Labeled label={t("autoTasks.target.workspace")}>
                <FormSelect
                  width="w-full max-w-[480px]"
                  ariaLabel={t("autoTasks.target.workspace")}
                  value={target.workspacePath}
                  options={workspaceOptions}
                  onChange={(v) => setTarget({ ...target, workspacePath: v })}
                />
              </Labeled>
              <Labeled label={t("autoTasks.target.agent")}>
                <AssistantAgentSelector
                  agent={localAssistantAgent}
                  agents={agents}
                  onChange={(next) =>
                    setTarget({
                      ...target,
                      agent: (next.id || target.agent) as Agent,
                      model: next.model.trim() || null,
                      effort: next.effort.trim() || null,
                      permissionMode: next.mode.trim() || null,
                    })
                  }
                />
              </Labeled>
            </>
          ) : (
            <div className="flex flex-wrap gap-3">
              <Labeled label={t("autoTasks.target.platform")}>
                <FormSelect
                  width="w-44"
                  ariaLabel={t("autoTasks.target.platform")}
                  value={target.platform}
                  options={PLATFORM_META.map(({ value, label, labelKey, Icon }) => ({
                    value,
                    label: labelKey ? t(labelKey) : label,
                    icon: <Icon className="h-3.5 w-3.5" />,
                  }))}
                  onChange={(v) => setTarget({ ...target, platform: v })}
                />
              </Labeled>
              <Labeled label={t("autoTasks.target.chat_id")}>
                <input
                  className={inputClass + " w-64"}
                  value={target.chatId}
                  onChange={(e) => setTarget({ ...target, chatId: e.target.value })}
                />
              </Labeled>
            </div>
          )}
        </div>
      </Field>

      <div className="flex justify-end gap-2 pt-1">
        <button
          type="button"
          onClick={onCancel}
          className="rounded-md px-4 py-1.5 text-body-sm text-ink/65 transition hover:bg-ink/5"
        >
          {t("autoTasks.cancel")}
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onSave}
          className="rounded-md bg-blue px-4 py-1.5 text-body-sm font-medium text-white transition hover:bg-blue/92 disabled:opacity-40"
        >
          {t("autoTasks.save")}
        </button>
      </div>
    </div>
  );
}

const inputClass =
  "rounded-md border border-ink/15 bg-surface-panel px-2.5 py-1.5 text-body-sm text-ink outline-none transition focus:border-ink/35";

function clamp(value: number, min: number, max: number): number {
  if (Number.isNaN(value)) return min;
  return Math.min(max, Math.max(min, value));
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-body-sm font-medium text-ink/75">{label}</label>
      {children}
    </div>
  );
}

function Labeled({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-caption text-ink/50">{label}</span>
      {children}
    </div>
  );
}

function FormSelect({
  value,
  options,
  onChange,
  ariaLabel,
  width = "w-44",
}: {
  value: string;
  options: InlineMenuSelectOption[];
  onChange: (value: string) => void;
  ariaLabel: string;
  width?: string;
}) {
  return (
    <div className={`flex items-center rounded-md border border-ink/15 bg-surface-panel ${width}`}>
      <InlineMenuSelect
        value={value}
        options={options}
        onChange={onChange}
        menuAlign="trigger"
        ariaLabel={ariaLabel}
        placeholder={ariaLabel}
        className="h-8 w-full max-w-none border-r-0 px-2.5 text-ink/80 hover:text-ink"
        menuClassName="bg-surface-panel"
        minMenuWidth={180}
      />
    </div>
  );
}
