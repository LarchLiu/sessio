import { Children, useEffect, useMemo, useState, type ReactNode } from "react";
import { CalendarClock, Check, ChevronLeft, ChevronRight, Clock3, GripVertical, Pencil, Play, Plus, Trash2, Unlock, X } from "lucide-react";
import { DragDropProvider, type DragEndEvent } from "@dnd-kit/react";
import { isSortable, useSortable } from "@dnd-kit/react/sortable";
import {
  AGENT_LABEL,
  type Agent,
  type AgentInfo,
  type AssistantInfo,
  type AssistantAgentInfo,
  type ProjectInfo,
  type ProjectStageInfo,
  type RuntimeAgentMetadata,
  type Schedule,
  type ScheduledTask,
  type ScheduledTaskMode,
  type ScheduledTaskRun,
  type ScheduledTaskRunStatus,
  type TaskImPush,
  type TaskTarget,
  type ThreadAgentInfo,
  forceUnlockScheduledTask,
  getScheduledTasks,
  listAgents,
  listAssistants,
  listProjectStages,
  listProjects,
  runScheduledTaskNow,
  saveScheduledTasks,
} from "../api";
import { AgentGlyph } from "../components/AgentIcon";
import AssistantAgentSelector, {
  dbAgentsAsRuntimeAgents,
  defaultAssistantAgent,
} from "../components/AssistantAgentSelector";
import {
  DiscordLogoIcon,
  LarkLogoIcon,
  Robot3LineIcon,
  TelegramLogoIcon,
  WechatLogoIcon,
} from "../components/IconifyIcon";
import InlineMenuSelect, { type InlineMenuSelectOption } from "../components/InlineMenuSelect";
import ScrollArea from "../components/ScrollArea";
import SegmentedTabs from "../components/SegmentedTabs";
import { useI18n } from "../i18n";
import { projectStageIcon, projectStageLabel } from "../utils/stageDisplay";

const TASK_MODES: ScheduledTaskMode[] = ["chat", "process", "teamwork", "brainstorm", "debate"];

const PLATFORM_META: {
  value: string;
  label: string;
  labelKey?: string;
  Icon: (props: { className?: string }) => ReactNode;
}[] = [
  { value: "telegram", label: "Telegram", Icon: TelegramLogoIcon },
  { value: "discord", label: "Discord", Icon: DiscordLogoIcon },
  { value: "feishu", label: "Feishu", labelKey: "settings.feishu_platform", Icon: LarkLogoIcon },
  { value: "wechat", label: "WeChat", Icon: WechatLogoIcon },
];

type ScheduleKind = Schedule["kind"];

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

function defaultChatTarget(
  projects: ProjectInfo[],
  agents: AgentInfo[],
  imPush: TaskImPush | null = null,
  projectId = projects[0]?.id ?? "",
): TaskTarget {
  const runtimeAgent = dbAgentsAsRuntimeAgents(agents)[0] ?? null;
  const assistantAgent = defaultAssistantAgent(runtimeAgent);
  return {
    projectId,
    mode: "chat",
    prompt: "",
    agent: runtimeAgent?.agent ?? "codex",
    model: assistantAgent.model.trim() || null,
    effort: assistantAgent.effort.trim() || null,
    permissionMode: assistantAgent.mode.trim() || null,
    imPush,
  };
}

function emptyTask(projects: ProjectInfo[], agents: AgentInfo[]): ScheduledTask {
  return {
    id: "",
    name: "",
    status: "active",
    schedule: defaultSchedule("daily"),
    target: defaultChatTarget(projects, agents),
    createdAtMs: 0,
    updatedAtMs: 0,
    lastRunAtMs: null,
    runs: [],
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

export default function AutoTasksPage({ onError }: { onError: (error: string | null) => void }) {
  const { t, lang } = useI18n();
  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [draft, setDraft] = useState<ScheduledTask | null>(null);
  const [busy, setBusy] = useState(false);
  const [calendarMonth, setCalendarMonth] = useState(() => monthStart(new Date()));
  const [selectedCalendarDate, setSelectedCalendarDate] = useState(() => startOfDay(new Date()));

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
  const calendarDays = useMemo(
    () => buildCalendarDays(calendarMonth, selectedCalendarDate, tasks),
    [calendarMonth, selectedCalendarDate, tasks],
  );
  const selectedDaySlots = useMemo(
    () => scheduledTaskSlotsForDay(selectedCalendarDate, tasks),
    [selectedCalendarDate, tasks],
  );

  const projectName = (projectId: string): string =>
    projects.find((project) => project.id === projectId)?.name || projectId || t("autoTasks.field.project");

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

  const taskSummary = (target: TaskTarget): string => {
    const mode =
      target.mode === "chat"
        ? `${t("new_chat.mode.chat")} · ${target.agent}`
        : `${t(`thread.kind.${target.mode}`)} · ${target.goal || t("autoTasks.thread.untitled")}`;
    const im = target.imPush?.enabled ? ` · ${platformLabel(target.imPush.platform)}` : "";
    return `${mode} · ${projectName(target.projectId)}${im}`;
  };

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
    if (!draft.name.trim()) {
      onError(t("autoTasks.field.name"));
      return;
    }
    const scheduleError = scheduleErrorKey(draft.schedule);
    if (scheduleError) {
      onError(t(scheduleError));
      return;
    }
    const target = draft.target;
    if (!target.projectId.trim()) {
      onError(t("autoTasks.field.project"));
      return;
    }
    if (target.mode === "chat") {
      if (!dbAgentsAsRuntimeAgents(agents).some((agent) => agent.agent === target.agent)) {
        onError(t("autoTasks.error.invalid_agent"));
        return;
      }
      if (!target.prompt.trim()) {
        onError(t("autoTasks.field.prompt"));
        return;
      }
    } else {
      if (!target.goal.trim()) {
        onError(t("autoTasks.field.thread_goal"));
        return;
      }
      if (target.mode === "process" && target.stageIds.length === 0) {
        onError(t("new_chat.thread_requires_stage"));
        return;
      }
      if (target.mode === "teamwork" && target.assistantIds.length === 0) {
        onError(t("new_chat.thread_requires_assistant"));
        return;
      }
      if (target.mode === "brainstorm" && target.agentParticipants.length < 2) {
        onError(t("new_chat.thread_requires_two_participants"));
        return;
      }
      if (target.mode === "debate" && target.agentParticipants.length !== 2) {
        onError(t("new_chat.thread_requires_exactly_two_participants"));
        return;
      }
    }
    if (target.imPush?.enabled && !target.imPush.chatId.trim()) {
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
        item.id === task.id
          ? { ...item, status: item.status === "active" ? "paused" : "active", updatedAtMs }
          : item,
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

  const handleForceUnlock = async (id: string) => {
    if (!window.confirm(t("autoTasks.force_unlock_confirm"))) return;
    setBusy(true);
    try {
      await forceUnlockScheduledTask(id);
      onError(null);
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

  const latestRunSummary = (run: ScheduledTaskRun | undefined): string | null => {
    if (!run) return null;
    const status = t(`autoTasks.run_status.${run.status ?? "completed"}`);
    const push = run.pushStatus ? ` · ${t(`autoTasks.push_status.${run.pushStatus}`)}` : "";
    const failure = run.status === "failed" && run.error ? ` · ${run.error}` : "";
    const suffix = `${push}${failure}`;
    if (run.sessionId) {
      return `${status} · ${t("autoTasks.run_output.chat", {
        agent: run.sessionAgent ? AGENT_LABEL[run.sessionAgent] : t("new_chat.mode.chat"),
        id: shortRef(run.sessionId),
      })}${suffix}`;
    }
    if (run.threadId) {
      const output = run.astraRunId
        ? t("autoTasks.run_output.thread_with_run", {
            id: shortRef(run.threadId),
            run: shortRef(run.astraRunId),
          })
        : t("autoTasks.run_output.thread", { id: shortRef(run.threadId) });
      return `${status} · ${output}${suffix}`;
    }
    return status + suffix;
  };

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col text-body text-ink">
      <ScrollArea className="min-h-0 flex-1 bg-surface-panel" viewportClassName="px-10 pt-6 pb-16">
        <div className="mx-auto max-w-[900px]">
          <div className="mb-5 flex items-center justify-between gap-4">
            <p className="text-body-sm text-ink/55">{t("autoTasks.subtitle")}</p>
            <button
              type="button"
              disabled={busy || draft !== null}
              onClick={() => setDraft(emptyTask(projects, agents))}
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
              onError={onError}
            />
          )}

          <TaskCalendar
            month={calendarMonth}
            selectedDate={selectedCalendarDate}
            days={calendarDays}
            selectedSlots={selectedDaySlots}
            weekdayNames={weekdayNames}
            onPrevMonth={() => setCalendarMonth(addMonths(calendarMonth, -1))}
            onNextMonth={() => setCalendarMonth(addMonths(calendarMonth, 1))}
            onToday={() => {
              const today = startOfDay(new Date());
              setCalendarMonth(monthStart(today));
              setSelectedCalendarDate(today);
            }}
            onSelectDate={(date) => {
              const next = startOfDay(date);
              setSelectedCalendarDate(next);
              if (!sameMonth(next, calendarMonth)) {
                setCalendarMonth(monthStart(next));
              }
            }}
            scheduleSummary={scheduleSummary}
            taskSummary={taskSummary}
          />

          {tasks.length === 0 && !draft ? (
            <p className="rounded-lg border border-dashed border-card-border/[0.14] px-4 py-10 text-center text-body-sm text-ink/45">
              {t("autoTasks.empty")}
            </p>
          ) : (
            <ul className="flex flex-col gap-2">
              {tasks.map((task) => {
                const running = taskIsRunning(task);
                const latestOutput = latestRunSummary(task.runs[0]);
                return (
                  <li
                    key={task.id}
                    className="flex items-center gap-3 rounded-lg border border-card-border/[0.10] bg-card px-4 py-3"
                  >
                    <button
                      type="button"
                      onClick={() => void handleToggle(task)}
                      title={t(`autoTasks.status.${task.status}`)}
                      className={
                        "h-4 w-4 shrink-0 rounded-full border transition " +
                        (task.status === "active"
                          ? "border-emerald bg-emerald"
                          : "border-card-border/[0.22] bg-transparent")
                      }
                    />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-body font-medium text-ink/88">
                        {task.name || task.id}
                      </div>
                      <div className="truncate text-caption text-ink/50">
                        {t(`autoTasks.status.${task.status}`)} · {scheduleSummary(task.schedule)} ·{" "}
                        {taskSummary(task.target)} · {formatLastRun(task)} ·{" "}
                        {t("autoTasks.runs_count", { count: task.runs.length })}
                        {latestOutput ? ` · ${latestOutput}` : ""}
                      </div>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      <IconButton title={t("autoTasks.run_now")} onClick={() => void handleRunNow(task.id)} disabled={busy || running}>
                        <Play className="h-4 w-4" />
                      </IconButton>
                      {running && (
                        <IconButton title={t("autoTasks.force_unlock")} onClick={() => void handleForceUnlock(task.id)} disabled={busy}>
                          <Unlock className="h-4 w-4" />
                        </IconButton>
                      )}
                      <IconButton title={running ? t("autoTasks.running_locked") : t("autoTasks.edit")} onClick={() => setDraft(task)} disabled={busy || draft !== null || running}>
                        <Pencil className="h-4 w-4" />
                      </IconButton>
                      <IconButton title={running ? t("autoTasks.running_locked") : t("autoTasks.delete")} onClick={() => void handleDelete(task.id)} disabled={busy || running}>
                        <Trash2 className="h-4 w-4" />
                      </IconButton>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </ScrollArea>
    </div>
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
  onError,
}: {
  draft: ScheduledTask;
  setDraft: (task: ScheduledTask) => void;
  onSave: () => void;
  onCancel: () => void;
  busy: boolean;
  weekdayNames: string[];
  projects: ProjectInfo[];
  agents: AgentInfo[];
  onError: (error: string | null) => void;
}) {
  const { t } = useI18n();
  const schedule = draft.schedule;
  const target = draft.target;
  const runtimeAgents = useMemo(() => dbAgentsAsRuntimeAgents(agents), [agents]);
  const [projectStages, setProjectStages] = useState<ProjectStageInfo[]>([]);
  const [assistants, setAssistants] = useState<AssistantInfo[]>([]);

  useEffect(() => {
    let cancelled = false;
    setProjectStages([]);
    setAssistants([]);
    if (!target.projectId) return;
    Promise.all([listProjectStages(target.projectId), listAssistants(target.projectId)])
      .then(([stages, assistants]) => {
        if (cancelled) return;
        setProjectStages(stages.slice().sort((a, b) => a.order - b.order));
        setAssistants(assistants.filter((assistant) => assistant.enabled));
      })
      .catch((error) => {
        if (!cancelled) onError(String(error));
      });
    return () => {
      cancelled = true;
    };
  }, [onError, target.projectId]);

  const selectableStages = useMemo(
    () => projectStages.filter((stage) => stage.enabled && (stage.allowEmptyAssistants || stage.assistants.length > 0)),
    [projectStages],
  );

  const projectOptions = useMemo<InlineMenuSelectOption[]>(
    () => projects.map((project) => ({ value: project.id, label: `${project.name} · ${project.path}` })),
    [projects],
  );

  const assistantOptions = useMemo(
    () =>
      assistants.map((assistant) => ({
        value: assistant.id,
        label: assistant.name,
        icon: assistantRobotIcon(assistant.color),
      })),
    [assistants],
  );

  const participantOptions = useMemo<InlineMenuSelectOption[]>(() => {
    const selected = new Set(
      target.mode === "brainstorm" || target.mode === "debate"
        ? target.agentParticipants.map((participant) => participantValue(participant.agent, participant.model))
        : [],
    );
    return runtimeAgents
      .flatMap((agent) => runtimeAgentModelOptions(agent))
      .filter((option) => !selected.has(option.value));
  }, [runtimeAgents, target]);

  const localAssistantAgent = useMemo<AssistantAgentInfo>(() => {
    if (target.mode !== "chat") return defaultAssistantAgent(runtimeAgents[0] ?? null);
    const selected = runtimeAgents.find((runtimeAgent) => runtimeAgent.agent === target.agent) ?? null;
    const base = defaultAssistantAgent(selected);
    return {
      id: target.agent,
      name: base.name,
      model: target.model || base.model,
      mode: target.permissionMode || base.mode,
      effort: target.effort || base.effort,
    };
  }, [runtimeAgents, target]);

  const setSchedule = (next: Schedule) => setDraft({ ...draft, schedule: next });
  const setTarget = (next: TaskTarget) => setDraft({ ...draft, target: next });

  const switchMode = (mode: ScheduledTaskMode) => {
    setTarget(targetForMode(target, mode, projects, agents, selectableStages, assistantOptions));
  };

  const setProject = (projectId: string) => {
    if (target.mode === "process") {
      setTarget({ ...target, projectId, stageIds: [] });
    } else if (target.mode === "teamwork") {
      setTarget({ ...target, projectId, assistantIds: [] });
    } else {
      setTarget({ ...target, projectId });
    }
  };

  return (
    <div className="mb-4 flex flex-col gap-4 rounded-lg border border-card-border/[0.10] bg-card p-5">
      <div className="grid gap-4 md:grid-cols-[minmax(0,1fr)_180px]">
        <Field label={t("autoTasks.field.name")}>
          <input
            className={inputClass}
            value={draft.name}
            placeholder={t("autoTasks.field.name_placeholder")}
            onChange={(e) => setDraft({ ...draft, name: e.target.value })}
          />
        </Field>
        <Field label={t("autoTasks.field.status")}>
          <SegmentedTabs
            items={(["active", "paused"] as const).map((status) => ({
              value: status,
              label: t(`autoTasks.status.${status}`),
            }))}
            value={draft.status}
            onChange={(status) => setDraft({ ...draft, status })}
            itemWidth={84}
            itemHeight={28}
            className="self-start"
          />
        </Field>
      </div>

      <Field label={t("autoTasks.field.schedule")}>
        <SegmentedTabs<ScheduleKind>
          items={(["interval", "daily", "weekly", "cron"] as ScheduleKind[]).map((kind) => ({
            value: kind,
            label: t(`autoTasks.schedule.${kind}`),
          }))}
          value={schedule.kind}
          onChange={(kind) => setSchedule(defaultSchedule(kind))}
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
                onChange={(e) => setSchedule({ kind: "interval", everySecs: Math.max(1, Number(e.target.value) || 0) })}
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
                    onChange={(value) => setSchedule({ ...schedule, weekday: Number(value) })}
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
                  onChange={(e) => setSchedule({ ...schedule, hour: clamp(Number(e.target.value), 0, 23) })}
                />
              </Labeled>
              <Labeled label={t("autoTasks.schedule.minute")}>
                <input
                  type="number"
                  min={0}
                  max={59}
                  className={inputClass + " w-24"}
                  value={schedule.minute}
                  onChange={(e) => setSchedule({ ...schedule, minute: clamp(Number(e.target.value), 0, 59) })}
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

      <Field label={t("autoTasks.field.project")}>
        <FormSelect
          width="w-full max-w-[520px]"
          ariaLabel={t("autoTasks.field.project")}
          value={target.projectId}
          options={projectOptions}
          onChange={setProject}
        />
      </Field>

      <Field label={t("autoTasks.field.task")}>
        <SegmentedTabs<ScheduledTaskMode>
          items={TASK_MODES.map((mode) => ({
            value: mode,
            label: mode === "chat" ? t("new_chat.mode.chat") : t(`thread.kind.${mode}`),
          }))}
          value={target.mode}
          onChange={switchMode}
          itemWidth={92}
          itemHeight={28}
          className="self-start"
        />
        <div className="mt-3 flex flex-col gap-3">
          {target.mode === "chat" ? (
            <>
              <Labeled label={t("autoTasks.field.prompt")}>
                <textarea
                  className={inputClass + " min-h-[36px] resize-y"}
                  value={target.prompt}
                  placeholder={t("autoTasks.field.prompt_placeholder")}
                  onChange={(e) => setTarget({ ...target, prompt: e.target.value })}
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
            <>
              <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                <Labeled label={t("autoTasks.field.thread_goal")}>
                  <input
                    className={inputClass}
                    value={target.goal}
                    placeholder={t("autoTasks.field.thread_goal_placeholder")}
                    onChange={(e) => setTarget({ ...target, goal: e.target.value })}
                  />
                </Labeled>
                <Labeled label={t("autoTasks.field.thread_description")}>
                  <textarea
                    className={inputClass + " min-h-[36px] resize-y"}
                    value={target.description ?? ""}
                    placeholder={t("autoTasks.field.thread_description_placeholder")}
                    onChange={(e) => setTarget({ ...target, description: e.target.value || null })}
                  />
                </Labeled>
              </div>
              <ThreadTemplateControls
                target={target}
                setTarget={setTarget}
                stages={selectableStages}
                assistantOptions={assistantOptions}
                participantOptions={participantOptions}
                runtimeAgents={runtimeAgents}
              />
            </>
          )}

          <ImPushControls target={target} setTarget={setTarget} />
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

interface CalendarDay {
  key: string;
  date: Date;
  inMonth: boolean;
  isToday: boolean;
  selected: boolean;
  taskCount: number;
  slots: TaskCalendarSlot[];
}

interface TaskCalendarSlot {
  key: string;
  task: ScheduledTask;
  at: Date;
  runtimeStatus: TaskCalendarRuntimeStatus;
  run: ScheduledTaskRun | null;
}

type TaskCalendarRuntimeStatus = ScheduledTaskRunStatus | "notStarted";

function TaskCalendar({
  month,
  selectedDate,
  days,
  selectedSlots,
  weekdayNames,
  onPrevMonth,
  onNextMonth,
  onToday,
  onSelectDate,
  scheduleSummary,
  taskSummary,
}: {
  month: Date;
  selectedDate: Date;
  days: CalendarDay[];
  selectedSlots: TaskCalendarSlot[];
  weekdayNames: string[];
  onPrevMonth: () => void;
  onNextMonth: () => void;
  onToday: () => void;
  onSelectDate: (date: Date) => void;
  scheduleSummary: (schedule: Schedule) => string;
  taskSummary: (target: TaskTarget) => string;
}) {
  const { t, lang } = useI18n();
  const locale = lang === "zh" ? "zh-CN" : "en-US";
  const monthLabel = month.toLocaleDateString(locale, { month: "long", year: "numeric" });
  const selectedLabel = selectedDate.toLocaleDateString(locale, {
    weekday: "long",
    month: "short",
    day: "numeric",
  });
  const totalInMonth = uniqueTaskCount(days.filter((day) => day.inMonth).flatMap((day) => day.slots));
  const selectedTaskCount = uniqueTaskCount(selectedSlots);

  return (
    <section className="mb-4 overflow-hidden rounded-xl border border-card-border/[0.10] bg-card shadow-sm">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-card-border/[0.08] px-4 py-3">
        <div className="flex min-w-0 items-center gap-2">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-brand/[0.10] text-brand">
            <CalendarClock className="h-4 w-4" />
          </span>
          <div className="min-w-0">
            <div className="text-body-sm font-semibold text-card-fg/88">{t("autoTasks.calendar.title")}</div>
            <div className="truncate text-caption text-card-muted/55">
              {t("autoTasks.calendar.month_summary", { count: totalInMonth })}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-1.5">
          <IconButton title={t("autoTasks.calendar.prev_month")} onClick={onPrevMonth}>
            <ChevronLeft className="h-4 w-4" />
          </IconButton>
          <button
            type="button"
            onClick={onToday}
            className="rounded-md border border-card-border/[0.10] bg-card-panel px-2.5 py-1.5 text-caption font-medium text-card-fg/68 transition hover:border-card-border/[0.18] hover:text-card-fg"
          >
            {t("autoTasks.calendar.today")}
          </button>
          <IconButton title={t("autoTasks.calendar.next_month")} onClick={onNextMonth}>
            <ChevronRight className="h-4 w-4" />
          </IconButton>
        </div>
      </div>

      <div className="grid gap-0 lg:grid-cols-[minmax(0,1.35fr)_minmax(280px,0.65fr)]">
        <div className="border-b border-card-border/[0.08] p-4 lg:border-b-0 lg:border-r">
          <div className="mb-3 flex items-center justify-between">
            <h2 className="text-title-sm font-semibold text-card-fg/90">{monthLabel}</h2>
            <span className="text-caption text-card-muted/50">{t("autoTasks.calendar.active_only")}</span>
          </div>
          <div className="grid grid-cols-7 gap-1.5">
            {weekdayNames.map((name) => (
              <div key={name} className="px-1 pb-1 text-center text-[10px] font-medium uppercase tracking-[0.12em] text-card-muted/45">
                {name.slice(0, lang === "zh" ? 2 : 3)}
              </div>
            ))}
            {days.map((day) => {
              const count = day.taskCount;
              const className =
                "group relative flex min-h-[72px] flex-col rounded-lg border p-2 text-left transition " +
                (day.selected
                  ? "border-brand/[0.55] bg-brand/[0.10] shadow-[inset_0_0_0_1px_rgb(var(--color-brand)/0.18)]"
                  : day.inMonth
                    ? "border-card-border/[0.08] bg-card-panel hover:border-card-border/[0.18] hover:bg-card-chip/[0.06]"
                    : "border-transparent bg-transparent opacity-45 hover:bg-card-panel/60");
              return (
                <button
                  key={day.key}
                  type="button"
                  onClick={() => onSelectDate(day.date)}
                  className={className}
                >
                  <span
                    className={
                      "flex h-5 w-5 items-center justify-center rounded-full text-caption font-medium " +
                      (day.isToday
                        ? "bg-ink text-surface"
                        : day.selected
                          ? "text-brand"
                          : "text-card-fg/70")
                    }
                  >
                    {day.date.getDate()}
                  </span>
                  <span className="mt-auto flex items-center justify-between gap-1">
                    {count > 0 ? (
                      <>
                        <span className="h-1.5 min-w-0 flex-1 rounded-full bg-brand/[0.22]">
                          <span
                            className="block h-full rounded-full bg-brand"
                            style={{ width: `${Math.min(100, 24 + count * 18)}%` }}
                          />
                        </span>
                        <span className="flex items-center gap-0.5">
                          {calendarStatusDots(day.slots).map((status) => (
                            <span key={status} className={`h-1.5 w-1.5 rounded-full ${calendarRuntimeDotClass(status)}`} />
                          ))}
                        </span>
                        <span className="rounded-full bg-card px-1.5 py-0.5 text-[10px] font-semibold text-brand shadow-sm">
                          {count}
                        </span>
                      </>
                    ) : (
                      <span className="text-[10px] text-card-muted/28">·</span>
                    )}
                  </span>
                </button>
              );
            })}
          </div>
        </div>

        <aside className="flex min-h-[300px] flex-col p-4">
          <div className="mb-3">
            <div className="text-body-sm font-semibold text-card-fg/88">{selectedLabel}</div>
            <div className="text-caption text-card-muted/50">
              {t("autoTasks.calendar.day_summary", { count: selectedTaskCount, slots: selectedSlots.length })}
            </div>
          </div>
          {selectedSlots.length === 0 ? (
            <div className="flex flex-1 items-center justify-center rounded-lg border border-dashed border-card-border/[0.12] px-4 py-10 text-center text-caption text-card-muted/45">
              {t("autoTasks.calendar.empty_day")}
            </div>
          ) : (
            <ol className="flex flex-col gap-2">
              {selectedSlots.map((slot) => (
                <li key={slot.key} className="rounded-lg border border-card-border/[0.10] bg-card-panel px-3 py-2.5">
                  <div className="mb-1 flex items-center gap-2 text-caption font-semibold text-card-fg/82">
                    <Clock3 className="h-3.5 w-3.5 text-brand" />
                    <span className="tabular-nums">{formatTime(slot.at)}</span>
                    <span className="rounded-full bg-card-chip/[0.10] px-1.5 py-0.5 text-[10px] font-medium text-card-muted/58">
                      {t(`autoTasks.schedule.${slot.task.schedule.kind}`)}
                    </span>
                    <TaskRuntimeStatusBadge status={slot.runtimeStatus} />
                  </div>
                  <div className="truncate text-body-sm font-medium text-card-fg/86">
                    {slot.task.name || slot.task.id}
                  </div>
                  <div className="mt-0.5 line-clamp-2 text-caption text-card-muted/55">
                    {taskSummary(slot.task.target)}
                  </div>
                  <div className="mt-1 text-[10px] text-card-muted/40">
                    {scheduleSummary(slot.task.schedule)}
                  </div>
                </li>
              ))}
            </ol>
          )}
        </aside>
      </div>
    </section>
  );
}

function TaskRuntimeStatusBadge({ status }: { status: TaskCalendarRuntimeStatus }) {
  const { t } = useI18n();
  const label =
    status === "notStarted"
      ? t("autoTasks.run_status.not_started")
      : t(`autoTasks.run_status.${status}`);
  return (
    <span className={`rounded-full px-1.5 py-0.5 text-[10px] font-medium ${calendarRuntimeBadgeClass(status)}`}>
      {label}
    </span>
  );
}

function ThreadTemplateControls({
  target,
  setTarget,
  stages,
  assistantOptions,
  participantOptions,
  runtimeAgents,
}: {
  target: Exclude<TaskTarget, { mode: "chat" }>;
  setTarget: (target: TaskTarget) => void;
  stages: ProjectStageInfo[];
  assistantOptions: { value: string; label: string; icon?: ReactNode }[];
  participantOptions: InlineMenuSelectOption[];
  runtimeAgents: RuntimeAgentMetadata[];
}) {
  const { t } = useI18n();
  if (target.mode === "process") {
    return (
      <ProcessStageTemplateControls
        target={target}
        setTarget={setTarget}
        stages={stages}
      />
    );
  }
  if (target.mode === "teamwork") {
    return (
      <TeamworkAssistantTemplateControls
        target={target}
        setTarget={setTarget}
        assistantOptions={assistantOptions}
      />
    );
  }
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {target.agentParticipants.map((participant, index) => (
        <span key={`${participant.agent}:${participant.model}:${index}`} className="inline-flex h-7 max-w-[260px] items-center gap-1.5 rounded-md border border-card-border/[0.10] bg-card-panel px-1.5 text-caption text-card-fg/70">
          <span className="text-card-muted/45 tabular-nums">{index + 1}</span>
          <AgentGlyph agent={participant.agent} className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 truncate">
            {AGENT_LABEL[participant.agent]}
            <span className="text-card-muted/55"> · {modelLabel(runtimeAgents, participant)}</span>
          </span>
          <button
            type="button"
            onClick={() =>
              setTarget({
                ...target,
                agentParticipants: target.agentParticipants.filter((_, i) => i !== index).map((item, order) => ({ ...item, order })),
              })
            }
            className="shrink-0 rounded p-0.5 text-card-muted/45 transition hover:bg-ink/6 hover:text-card-fg/75"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </span>
      ))}
      <FormSelect
        width="w-[210px]"
        ariaLabel={t("new_chat.add_participant")}
        value=""
        options={participantOptions}
        onChange={(value) => {
          const participant = participantFromValue(value, runtimeAgents, target.agentParticipants.length);
          if (!participant) return;
          if (target.mode === "debate" && target.agentParticipants.length >= 2) return;
          setTarget({ ...target, agentParticipants: [...target.agentParticipants, participant] });
        }}
      />
    </div>
  );
}

function TeamworkAssistantTemplateControls({
  target,
  setTarget,
  assistantOptions,
}: {
  target: Extract<TaskTarget, { mode: "teamwork" }>;
  setTarget: (target: TaskTarget) => void;
  assistantOptions: { value: string; label: string; icon?: ReactNode }[];
}) {
  const { t } = useI18n();
  const assistantById = new Map(assistantOptions.map((assistant) => [assistant.value, assistant]));
  const selectedAssistants = target.assistantIds
    .map((assistantId) => assistantById.get(assistantId))
    .filter((assistant): assistant is { value: string; label: string; icon?: ReactNode } => Boolean(assistant));
  const selectedIds = selectedAssistants.map((assistant) => assistant.value);
  const selectedSet = new Set(selectedIds);
  const availableAssistants = assistantOptions.filter((assistant) => !selectedSet.has(assistant.value));

  if (assistantOptions.length === 0) {
    return <span className="text-caption text-ink/38">{t("thread.no_assistants")}</span>;
  }

  return (
    <div className="flex flex-col gap-2">
      <Labeled label={t("autoTasks.assistants.selected")}>
        <ChipField empty={t("autoTasks.assistants.none_selected")}>
          {selectedAssistants.map((assistant) => (
            <button
              key={assistant.value}
              type="button"
              onClick={() =>
                setTarget({
                  ...target,
                  assistantIds: target.assistantIds.filter((id) => id !== assistant.value),
                })
              }
              className={chipClass(true)}
            >
              {assistant.icon}
              <span className="min-w-0 truncate">{assistant.label}</span>
              <CheckMark selected />
            </button>
          ))}
        </ChipField>
      </Labeled>
      {availableAssistants.length > 0 && (
        <Labeled label={t("autoTasks.assistants.available")}>
          <ChipField empty={t("thread.no_assistants")}>
            {availableAssistants.map((assistant) => (
              <button
                key={assistant.value}
                type="button"
                onClick={() => setTarget({ ...target, assistantIds: [...selectedIds, assistant.value] })}
                className={chipClass(false)}
              >
                {assistant.icon}
                <span className="min-w-0 truncate">{assistant.label}</span>
                <CheckMark selected={false} />
              </button>
            ))}
          </ChipField>
        </Labeled>
      )}
    </div>
  );
}

function ProcessStageTemplateControls({
  target,
  setTarget,
  stages,
}: {
  target: Extract<TaskTarget, { mode: "process" }>;
  setTarget: (target: TaskTarget) => void;
  stages: ProjectStageInfo[];
}) {
  const { t } = useI18n();
  const stagesById = useMemo(() => new Map(stages.map((stage) => [stage.id, stage])), [stages]);
  const selectedStages = target.stageIds
    .map((stageId) => stagesById.get(stageId))
    .filter((stage): stage is ProjectStageInfo => Boolean(stage));
  const selectedStageIds = selectedStages.map((stage) => stage.id);
  const selectedSet = new Set(selectedStageIds);
  const availableStages = stages.filter((stage) => !selectedSet.has(stage.id));

  const handleDragEnd = (event: DragEndEvent) => {
    if (event.canceled) return;
    const { source } = event.operation;
    if (!isSortable(source)) return;
    const from = source.initialIndex;
    const to = source.index;
    if (from === to) return;
    const nextStageIds = [...selectedStageIds];
    const [stageId] = nextStageIds.splice(from, 1);
    if (!stageId) return;
    nextStageIds.splice(to, 0, stageId);
    setTarget({ ...target, stageIds: nextStageIds });
  };

  if (stages.length === 0) {
    return <span className="text-caption text-ink/38">{t("new_chat.no_stages")}</span>;
  }

  return (
    <div className="flex flex-col gap-2">
      <Labeled label={t("autoTasks.stages.selected")}>
        <DragDropProvider onDragEnd={handleDragEnd}>
          <div className="flex flex-wrap items-center gap-1.5">
            {selectedStages.length === 0 ? (
              <span className="text-caption text-ink/38">{t("autoTasks.stages.none_selected")}</span>
            ) : (
              selectedStages.map((stage, index) => (
                <ProcessStageChip
                  key={stage.id}
                  stage={stage}
                  index={index}
                  onRemove={(stageId) =>
                    setTarget({
                      ...target,
                      stageIds: target.stageIds.filter((id) => id !== stageId),
                    })
                  }
                />
              ))
            )}
          </div>
        </DragDropProvider>
      </Labeled>
      {availableStages.length > 0 && (
        <Labeled label={t("autoTasks.stages.available")}>
          <ChipField empty={t("new_chat.no_stages")}>
            {availableStages.map((stage) => (
              <button
                key={stage.id}
                type="button"
                onClick={() => setTarget({ ...target, stageIds: [...selectedStageIds, stage.id] })}
                className={chipClass(false)}
              >
                {projectStageIcon(stage)}
                <span className="min-w-0 truncate">{projectStageLabel(stage, t)}</span>
                <CheckMark selected={false} />
              </button>
            ))}
          </ChipField>
        </Labeled>
      )}
    </div>
  );
}

function ProcessStageChip({
  stage,
  index,
  onRemove,
}: {
  stage: ProjectStageInfo;
  index: number;
  onRemove: (stageId: string) => void;
}) {
  const { t } = useI18n();
  const { handleRef, isDragSource, isDropTarget, ref } = useSortable({
    id: stage.id,
    index,
    group: "auto-task-process-stages",
    transition: {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
      idle: true,
    },
  });
  const stateClass = isDragSource
    ? "z-20 cursor-grabbing border-card-border/[0.24] bg-card-panel shadow-lg"
    : isDropTarget
      ? "border-card-border/[0.24] bg-card-chip/[0.12] shadow-[inset_2px_0_0_rgb(var(--color-card-fg)/0.20)]"
      : "border-card-border/[0.18] bg-card-chip/[0.10] text-card-fg/78";

  return (
    <span
      ref={ref}
      className={`inline-flex h-7 max-w-[220px] items-center gap-1.5 rounded-md border px-1.5 text-caption transition duration-150 ${stateClass}`}
    >
      <button
        ref={handleRef}
        type="button"
        className="cursor-grab touch-none rounded p-0.5 text-current/45 transition hover:bg-ink/5 active:cursor-grabbing"
        aria-label={t("autoTasks.stages.drag")}
        title={t("autoTasks.stages.drag")}
      >
        <GripVertical className="h-3.5 w-3.5" />
      </button>
      <button
        type="button"
        onClick={() => onRemove(stage.id)}
        className="inline-flex min-w-0 items-center gap-1.5"
      >
        {projectStageIcon(stage)}
        <span className="min-w-0 truncate">{projectStageLabel(stage, t)}</span>
        <CheckMark selected />
      </button>
    </span>
  );
}

function ImPushControls({ target, setTarget }: { target: TaskTarget; setTarget: (target: TaskTarget) => void }) {
  const { t } = useI18n();
  return (
    <>
      <label className="inline-flex w-fit items-center gap-2 text-body-sm text-ink/65">
        <input
          type="checkbox"
          checked={Boolean(target.imPush?.enabled)}
          onChange={(event) =>
            setTarget({
              ...target,
              imPush: event.target.checked
                ? { ...(target.imPush ?? { platform: "telegram", chatId: "" }), enabled: true }
                : target.imPush
                  ? { ...target.imPush, enabled: false }
                  : null,
            })
          }
        />
        {t("autoTasks.im_push.enabled")}
      </label>
      {target.imPush?.enabled && (
        <div className="flex flex-wrap gap-3">
          <Labeled label={t("autoTasks.target.platform")}>
            <FormSelect
              width="w-44"
              ariaLabel={t("autoTasks.target.platform")}
              value={target.imPush.platform}
              options={PLATFORM_META.map(({ value, label, labelKey, Icon }) => ({
                value,
                label: labelKey ? t(labelKey) : label,
                icon: <Icon className="h-3.5 w-3.5" />,
              }))}
              onChange={(value) =>
                setTarget({
                  ...target,
                  imPush: { ...(target.imPush ?? { enabled: true, chatId: "" }), platform: value },
                })
              }
            />
          </Labeled>
          <Labeled label={t("autoTasks.target.chat_id")}>
            <input
              className={inputClass + " w-64"}
              value={target.imPush.chatId}
              onChange={(e) =>
                setTarget({
                  ...target,
                  imPush: {
                    ...(target.imPush ?? { enabled: true, platform: "telegram" }),
                    chatId: e.target.value,
                  },
                })
              }
            />
          </Labeled>
        </div>
      )}
    </>
  );
}

function targetForMode(
  current: TaskTarget,
  mode: ScheduledTaskMode,
  projects: ProjectInfo[],
  agents: AgentInfo[],
  stages: ProjectStageInfo[],
  assistants: { value: string }[],
): TaskTarget {
  const projectId = current.projectId || projects[0]?.id || "";
  const imPush = current.imPush;
  const goal = current.mode === "chat" ? "" : current.goal;
  const description = current.mode === "chat" ? null : current.description;
  if (mode === "chat") {
    if (current.mode === "chat") return current;
    return defaultChatTarget(projects, agents, imPush, projectId);
  }
  if (mode === "process") {
    return {
      projectId,
      mode,
      goal,
      description,
      stageIds: current.mode === "process" ? current.stageIds : stages.map((stage) => stage.id),
      imPush,
    };
  }
  if (mode === "teamwork") {
    return {
      projectId,
      mode,
      goal,
      description,
      assistantIds: current.mode === "teamwork" ? current.assistantIds : assistants.map((assistant) => assistant.value),
      imPush,
    };
  }
  return {
    projectId,
    mode,
    goal,
    description,
    agentParticipants:
      current.mode === "brainstorm" || current.mode === "debate"
        ? current.agentParticipants.slice(0, mode === "debate" ? 2 : undefined)
        : [],
    imPush,
  };
}

function runtimeAgentModelOptions(agent: RuntimeAgentMetadata): InlineMenuSelectOption[] {
  const models =
    agent.models.length > 0
      ? agent.models.filter((model) => model.enabled && model.value.trim())
      : agent.model
        ? [{ value: agent.model, label: agent.model, displayName: agent.model, enabled: true, order: 0 }]
        : [];
  return models.map((model) => ({
    value: participantValue(agent.agent, model.value),
    label: `${AGENT_LABEL[agent.agent]} · ${model.displayName || model.label || model.value}`,
    icon: <AgentGlyph agent={agent.agent} className="h-3.5 w-3.5" />,
  }));
}

function participantFromValue(value: string, runtimeAgents: RuntimeAgentMetadata[], order: number): ThreadAgentInfo | null {
  try {
    const parsed = JSON.parse(value) as { agent?: unknown; model?: unknown };
    const model = typeof parsed.model === "string" ? parsed.model : "";
    // Validate against the live runtime agents rather than a hardcoded list, so
    // a newly added agent works without editing this function.
    const runtimeAgent = runtimeAgents.find((item) => item.agent === parsed.agent);
    if (!runtimeAgent) return null;
    return participantFromRuntimeAgent(runtimeAgent, order, model);
  } catch {
    return null;
  }
}

function participantFromRuntimeAgent(agent: RuntimeAgentMetadata, order: number, modelOverride?: string): ThreadAgentInfo | null {
  const model = modelOverride || agent.model || agent.models.find((option) => option.enabled && option.value.trim())?.value || "";
  if (!model.trim()) return null;
  return {
    participantId: "",
    agent: agent.agent,
    model,
    effort: agent.effort || agent.efforts.find((option) => option.enabled)?.value || "",
    permissionMode: agent.permissionMode || agent.permissionModes.find((option) => option.enabled)?.value || "",
    order,
    createdAt: 0,
    updatedAt: 0,
  };
}

function participantValue(agent: Agent, model: string): string {
  return JSON.stringify({ agent, model });
}

function modelLabel(runtimeAgents: RuntimeAgentMetadata[], participant: ThreadAgentInfo): string {
  const runtimeAgent = runtimeAgents.find((agent) => agent.agent === participant.agent);
  return runtimeAgent?.models.find((model) => model.value === participant.model)?.displayName
    ?? runtimeAgent?.models.find((model) => model.value === participant.model)?.label
    ?? participant.model;
}

function assistantRobotIcon(color: string | null | undefined) {
  return (
    <Robot3LineIcon
      className="h-3.5 w-3.5 shrink-0"
      style={{ color: color ?? "rgb(var(--color-brand))" }}
    />
  );
}

function ChipField({ children, empty }: { children: ReactNode; empty: string }) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {Children.count(children) > 0 ? children : <span className="text-caption text-ink/38">{empty}</span>}
    </div>
  );
}

function CheckMark({ selected }: { selected: boolean }) {
  return (
    <span className="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border border-card-border/[0.22] bg-card text-card-fg/75">
      {selected && <Check className="h-3 w-3" />}
    </span>
  );
}

function chipClass(selected: boolean): string {
  return (
    "inline-flex h-7 max-w-[190px] items-center gap-1.5 rounded-md border px-1.5 text-caption transition duration-150 " +
    (selected
      ? "border-card-border/[0.18] bg-card-chip/[0.10] text-card-fg/78"
      : "border-card-border/[0.10] bg-card-panel text-card-muted/60 hover:text-card-fg/72")
  );
}

function IconButton({
  children,
  title,
  onClick,
  disabled,
}: {
  children: ReactNode;
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

const inputClass =
  "rounded-md border border-card-border/[0.10] bg-card-panel px-2.5 py-1.5 text-body-sm text-card-fg outline-none transition focus:border-card-border/[0.24]";

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-body-sm font-medium text-ink/75">{label}</label>
      {children}
    </div>
  );
}

function Labeled({ label, children }: { label: string; children: ReactNode }) {
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
    <div className={`flex items-center rounded-md border border-card-border/[0.10] bg-card-panel ${width}`}>
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

const MAX_INTERVAL_SLOTS_PER_DAY = 96;

function buildCalendarDays(month: Date, selectedDate: Date, tasks: ScheduledTask[]): CalendarDay[] {
  const firstOfMonth = monthStart(month);
  const gridStart = addDays(firstOfMonth, -firstOfMonth.getDay());
  return Array.from({ length: 42 }, (_, index) => {
    const date = addDays(gridStart, index);
    const slots = scheduledTaskSlotsForDay(date, tasks);
    return {
      key: dateKey(date),
      date,
      inMonth: sameMonth(date, firstOfMonth),
      isToday: sameDay(date, new Date()),
      selected: sameDay(date, selectedDate),
      taskCount: uniqueTaskCount(slots),
      slots,
    };
  });
}

function scheduledTaskSlotsForDay(date: Date, tasks: ScheduledTask[]): TaskCalendarSlot[] {
  const slots = tasks
    .filter((task) => task.status === "active")
    .flatMap((task) => {
      const times = scheduledTimesForDay(task, date);
      const slots = times.map((at, index) => {
        const run = matchRunForScheduledSlot(task, at);
        const runtimeStatus: TaskCalendarRuntimeStatus = run?.status ?? "notStarted";
        return {
          key: `${task.id || task.name}:${at.getTime()}:${index}`,
          task,
          at,
          runtimeStatus,
          run,
        };
      });
      const matchedRunIds = new Set(slots.map((slot) => slot.run?.id).filter(Boolean));
      const historicalRunSlots = task.runs
        .filter(
          (run) =>
            run.trigger === "scheduled"
            && !matchedRunIds.has(run.id)
            && run.scheduledForMs != null
            && sameDay(new Date(run.scheduledForMs), date),
        )
        .map((run, index) => ({
          key: `${task.id || task.name}:run:${run.id || run.startedAtMs}:${index}`,
          task,
          at: new Date(run.scheduledForMs ?? run.startedAtMs),
          runtimeStatus: run.status,
          run,
        }));
      return [...slots, ...historicalRunSlots];
    });
  slots.sort((a, b) => a.at.getTime() - b.at.getTime() || a.task.name.localeCompare(b.task.name));
  return slots;
}

function matchRunForScheduledSlot(task: ScheduledTask, at: Date): ScheduledTaskRun | null {
  const scheduledForMs = at.getTime();
  return (
    task.runs
      .filter((run) => run.trigger === "scheduled" && run.scheduledForMs === scheduledForMs)
      .sort((a, b) => b.startedAtMs - a.startedAtMs)[0] ?? null
  );
}

function uniqueTaskCount(slots: TaskCalendarSlot[]): number {
  return new Set(slots.map((slot) => slot.task.id || slot.task.name)).size;
}

function scheduledTimesForDay(task: ScheduledTask, date: Date): Date[] {
  const start = startOfDay(date);
  const end = addDays(start, 1);
  switch (task.schedule.kind) {
    case "daily": {
      const at = atLocalTime(start, task.schedule.hour, task.schedule.minute);
      return at >= start && at < end && afterTaskCreated(task, at) ? [at] : [];
    }
    case "weekly": {
      if (start.getDay() !== task.schedule.weekday) return [];
      const at = atLocalTime(start, task.schedule.hour, task.schedule.minute);
      return at >= start && at < end && afterTaskCreated(task, at) ? [at] : [];
    }
    case "interval":
      return intervalTimesForDay(task, task.schedule.everySecs, start, end);
    case "cron":
      return cronTimesForDay(task, start, end);
  }
}

function intervalTimesForDay(task: ScheduledTask, everySecs: number, start: Date, end: Date): Date[] {
  const everyMs = everySecs * 1000;
  if (!Number.isFinite(everyMs) || everyMs <= 0) return [];
  const anchor = task.lastRunAtMs ?? task.createdAtMs;
  if (!Number.isFinite(anchor) || anchor <= 0) return [];
  let nextMs = anchor + everyMs;
  if (nextMs < start.getTime()) {
    const steps = Math.ceil((start.getTime() - nextMs) / everyMs);
    nextMs += Math.max(0, steps) * everyMs;
  }
  const times: Date[] = [];
  while (nextMs < end.getTime() && times.length < MAX_INTERVAL_SLOTS_PER_DAY) {
    times.push(new Date(nextMs));
    nextMs += everyMs;
  }
  return times;
}

function cronTimesForDay(task: ScheduledTask, start: Date, end: Date): Date[] {
  if (task.schedule.kind !== "cron") return [];
  const parsed = parseFiveFieldCron(task.schedule.expr);
  if (!parsed) return [];
  const anchorMs = Math.max(task.createdAtMs, start.getTime() - 60_000);
  const times: Date[] = [];
  for (let cursor = new Date(start); cursor < end; cursor = new Date(cursor.getTime() + 60_000)) {
    if (cursor.getTime() <= anchorMs) continue;
    if (!parsed.minutes.has(cursor.getMinutes())) continue;
    if (!parsed.hours.has(cursor.getHours())) continue;
    if (!parsed.months.has(cursor.getMonth() + 1)) continue;
    const domMatches = parsed.daysOfMonth.has(cursor.getDate());
    const dowMatches = parsed.daysOfWeek.has(cursor.getDay());
    const dayMatches =
      parsed.dayOfMonthWildcard && parsed.dayOfWeekWildcard
        ? true
        : parsed.dayOfMonthWildcard
          ? dowMatches
          : parsed.dayOfWeekWildcard
            ? domMatches
            : domMatches || dowMatches;
    if (dayMatches) {
      times.push(new Date(cursor));
    }
  }
  return times;
}

interface ParsedCron {
  minutes: Set<number>;
  hours: Set<number>;
  daysOfMonth: Set<number>;
  months: Set<number>;
  daysOfWeek: Set<number>;
  dayOfMonthWildcard: boolean;
  dayOfWeekWildcard: boolean;
}

function parseFiveFieldCron(expr: string): ParsedCron | null {
  const fields = expr.trim().split(/\s+/);
  if (fields.length !== 5) return null;
  const minutes = parseCronField(fields[0], 0, 59);
  const hours = parseCronField(fields[1], 0, 23);
  const daysOfMonth = parseCronField(fields[2], 1, 31);
  const months = parseCronField(fields[3], 1, 12);
  const daysOfWeek = parseCronField(fields[4], 0, 7);
  if (!minutes || !hours || !daysOfMonth || !months || !daysOfWeek) return null;
  if (daysOfWeek.has(7)) {
    daysOfWeek.add(0);
    daysOfWeek.delete(7);
  }
  return {
    minutes,
    hours,
    daysOfMonth,
    months,
    daysOfWeek,
    dayOfMonthWildcard: fields[2] === "*",
    dayOfWeekWildcard: fields[4] === "*",
  };
}

function parseCronField(field: string, min: number, max: number): Set<number> | null {
  const values = new Set<number>();
  for (const part of field.split(",")) {
    const trimmed = part.trim();
    if (!trimmed) return null;
    const [rangePart, stepPart] = trimmed.split("/");
    if (trimmed.split("/").length > 2) return null;
    const step = stepPart === undefined ? 1 : Number(stepPart);
    if (!Number.isInteger(step) || step <= 0) return null;
    const bounds = parseCronRange(rangePart, min, max);
    if (!bounds) return null;
    for (let value = bounds.start; value <= bounds.end; value += step) {
      values.add(value);
    }
  }
  return values.size > 0 ? values : null;
}

function parseCronRange(part: string | undefined, min: number, max: number): { start: number; end: number } | null {
  if (!part || part === "*") return { start: min, end: max };
  if (part.includes("-")) {
    const [startRaw, endRaw] = part.split("-");
    if (part.split("-").length !== 2) return null;
    const start = Number(startRaw);
    const end = Number(endRaw);
    if (!Number.isInteger(start) || !Number.isInteger(end) || start < min || end > max || start > end) return null;
    return { start, end };
  }
  const value = Number(part);
  if (!Number.isInteger(value) || value < min || value > max) return null;
  return { start: value, end: value };
}

function calendarStatusDots(slots: TaskCalendarSlot[]): TaskCalendarRuntimeStatus[] {
  const order: TaskCalendarRuntimeStatus[] = ["failed", "cancelled", "running", "completed", "notStarted"];
  const seen = new Set(slots.map((slot) => slot.runtimeStatus));
  return order.filter((status) => seen.has(status)).slice(0, 3);
}

function calendarRuntimeBadgeClass(status: TaskCalendarRuntimeStatus): string {
  switch (status) {
    case "running":
      return "bg-blue/[0.12] text-blue";
    case "completed":
      return "bg-emerald/[0.12] text-emerald";
    case "failed":
      return "bg-red-500/[0.10] text-red-500";
    case "cancelled":
      return "bg-amber-500/[0.10] text-amber-500";
    case "notStarted":
      return "bg-card-chip/[0.10] text-card-muted/58";
  }
}

function calendarRuntimeDotClass(status: TaskCalendarRuntimeStatus): string {
  switch (status) {
    case "running":
      return "bg-blue";
    case "completed":
      return "bg-emerald";
    case "failed":
      return "bg-red-500";
    case "cancelled":
      return "bg-amber-500";
    case "notStarted":
      return "bg-card-muted/35";
  }
}

function afterTaskCreated(task: ScheduledTask, at: Date): boolean {
  return !Number.isFinite(task.createdAtMs) || task.createdAtMs <= 0 || at.getTime() > task.createdAtMs;
}

function atLocalTime(day: Date, hour: number, minute: number): Date {
  return new Date(day.getFullYear(), day.getMonth(), day.getDate(), hour, minute, 0, 0);
}

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function monthStart(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), 1);
}

function addDays(date: Date, days: number): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + days);
}

function addMonths(date: Date, months: number): Date {
  return new Date(date.getFullYear(), date.getMonth() + months, 1);
}

function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

function sameMonth(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth();
}

function dateKey(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

function formatTime(date: Date): string {
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

function shortRef(value: string): string {
  return value.length > 14 ? `${value.slice(0, 6)}...${value.slice(-4)}` : value;
}

function taskIsRunning(task: ScheduledTask): boolean {
  return task.runs.some((run) => run.status === "running");
}

function clamp(value: number, min: number, max: number): number {
  if (Number.isNaN(value)) return min;
  return Math.min(max, Math.max(min, value));
}
