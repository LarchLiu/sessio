pub mod cached;
mod session_rules;
pub mod sqlite;

pub(crate) use session_rules::{
    better_session_candidate, file_mtime_for, insert_best_session, is_placeholder_indexed_session,
    is_real_session_file_path, is_virtual_session_ref, now_ms,
};

use anyhow::Result;
use std::collections::{HashMap, HashSet};

use crate::agents::runtime::types::RuntimeTransportKind;
use crate::models::{
    Agent, AgentAiProviderInfo, AgentCommandsInfo, AgentInfo, AssistantAgentInfo, AssistantInfo,
    AssistantType, AstraConfig, CanvasBlockKind, CanvasBlockRecord, CanvasBlockSourceType,
    CanvasContextAnchor, CanvasDocumentInfo, CanvasDocumentState, CanvasRevisionInfo,
    ChannelSessionInfo, IssueSeverity, IssueStatus, KanbanItem, KanbanStatus, PlanRoundInfo,
    PlanRoundMode, PlanRoundSource, PlanRoundStatus, PlanTaskInfo, PlanTaskRisk,
    PlanTaskSessionInfo, PlanTaskSessionRole, PlanTaskStatus, ProcessTemplateInfo, ProjectInfo,
    ProjectStageInfo, RuntimeAgentOptionMetadata, SessionHistoryTurn, SessionInfo, StageInfo,
    StageIssueInfo, StageStatus, SubagentInfo, ThreadIndexItemInfo, ThreadInfo, ThreadKind,
    ThreadOrigin, ThreadReplayInfo, ThreadReplaySessionInfo, ThreadReplaySessionSourceInfo,
    ThreadReplaySessionSourceKind,
};

/// Optional patch fields shared by the agent-preference update methods. Every
/// field is `Some` only when that column should change; `None` leaves it as-is.
#[derive(Default)]
pub struct AgentPreferencesPatch<'a> {
    pub display_name: Option<&'a str>,
    pub enabled: Option<bool>,
    pub order: Option<i64>,
    pub ai_provider: Option<&'a str>,
    pub ai_providers: Option<&'a [AgentAiProviderInfo]>,
    pub commands: Option<&'a AgentCommandsInfo>,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub permission_mode: Option<&'a str>,
    pub models: Option<&'a [RuntimeAgentOptionMetadata]>,
    pub efforts: Option<&'a [RuntimeAgentOptionMetadata]>,
    pub permission_modes: Option<&'a [RuntimeAgentOptionMetadata]>,
}

/// Patch for updating Astra configuration. Each field is Option<Option<T>>:
/// - None: don't change this field
/// - Some(None): set to NULL
/// - Some(Some(v)): set to v
#[derive(Debug, Default)]
pub struct AstraConfigPatch<'a> {
    pub agent: Option<Option<&'a str>>,
    pub model: Option<Option<&'a str>>,
    pub effort: Option<Option<&'a str>>,
    pub permission_mode: Option<Option<&'a str>>,
}

/// The defining fields for a new assistant.
pub struct NewAssistant<'a> {
    pub name: &'a str,
    pub agent: AssistantAgentInfo,
    pub system_prompt: Option<&'a str>,
    pub color: Option<&'a str>,
    pub selected_skill_ids: Vec<String>,
    pub selected_mcp_ids: Vec<String>,
    pub assistant_type: AssistantType,
    pub process_template_id: Option<String>,
    pub project_id: Option<&'a str>,
}

/// Optional patch fields for updating a project stage. `None` leaves the column
/// unchanged; the doubly-wrapped fields distinguish "leave" from "set to null".
#[derive(Default)]
pub struct ProjectStagePatch<'a> {
    pub name: Option<&'a str>,
    pub description: Option<Option<&'a str>>,
    pub icon: Option<Option<&'a str>>,
    pub order: Option<i64>,
    pub enabled: Option<bool>,
    pub allow_empty_assistants: Option<bool>,
}

pub struct NewPlanTask<'a> {
    pub thread_stage_id: Option<&'a str>,
    pub assistant_id: Option<&'a str>,
    pub agent_participant_id: Option<&'a str>,
    pub target_agent: Agent,
    pub stage_snapshot_json: Option<&'a str>,
    pub assistant_snapshot_json: Option<&'a str>,
    pub agent_snapshot_json: &'a str,
    pub title: &'a str,
    pub prompt: &'a str,
    pub expected_output: Option<&'a str>,
    pub risk: PlanTaskRisk,
    pub sort_order: i64,
    pub status: PlanTaskStatus,
}

pub struct NewPlanRound<'a> {
    pub thread_id: &'a str,
    pub astra_run_id: Option<&'a str>,
    pub round_index: Option<i64>,
    pub summary: Option<&'a str>,
    pub mode: PlanRoundMode,
    pub source: PlanRoundSource,
    pub status: PlanRoundStatus,
    pub tasks: Vec<NewPlanTask<'a>>,
}

pub struct PlanTaskStatusPatch<'a> {
    pub status: PlanTaskStatus,
    pub result_summary: Option<Option<&'a str>>,
    pub error: Option<Option<&'a str>>,
}

pub struct NewPlanTaskSession<'a> {
    pub task_id: &'a str,
    pub agent: Agent,
    pub session_id: &'a str,
    pub role: PlanTaskSessionRole,
    pub attempt_id: Option<&'a str>,
    pub attempt_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionRef<'a> {
    pub agent: Agent,
    pub session_id: &'a str,
}

#[derive(Debug, Clone)]
pub struct AstraRunRecord {
    pub run_id: String,
    pub thread_id: String,
    pub project_id: String,
    pub project_path: String,
    pub status: String,
    pub mode: String,
    pub planner_backend: Option<String>,
    pub round_index: Option<i64>,
    pub round_limit: i64,
    pub terminal_reason: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub internal_planner_sessions: Vec<AstraRunSessionRecord>,
    pub run_diagnostics_json: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstraRunSessionRecord {
    pub run_id: String,
    pub agent: Agent,
    pub session_id: String,
    pub role: PlanTaskSessionRole,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct IndexedSubagentRecord {
    pub parent_agent: Agent,
    pub parent_session_id: String,
    pub parent_scope: String,
    pub subagent_id: String,
    pub file_path: String,
    pub file_size: u64,
    pub file_mtime: Option<i64>,
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct IndexedSessionRecord {
    pub agent: Agent,
    pub session_id: String,
    pub scope: String,
    pub file_path: String,
    pub forked_from_agent: Option<Agent>,
    pub forked_from_id: Option<String>,
    pub file_size: u64,
    pub file_mtime: Option<i64>,
    pub last_indexed_at: i64,
    pub available: bool,
    pub archived: bool,
    pub subagents: Vec<IndexedSubagentRecord>,
}

#[derive(Debug, Clone)]
pub struct RuntimeAgentCapabilityRecord {
    pub agent: Agent,
    pub transport: RuntimeTransportKind,
    /// Detected ACP adapter version for this runtime agent, typically sourced
    /// from the adapter's configured `commands.version` probe.
    pub version: Option<String>,
    pub protocol_version: Option<String>,
    pub raw_initialize_response_json: String,
    pub raw_capabilities_json: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct RuntimeAgentSessionConfigRecord {
    pub agent: Agent,
    pub adapter_version: String,
    pub available_commands_json: String,
    pub config_options_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct RuntimeAgentSelection {
    pub agent: Agent,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct SessionHistorySnapshotRecord {
    pub child_agent: Agent,
    pub child_session_id: String,
    pub ancestor_agent: Agent,
    pub ancestor_session_id: String,
    pub ancestor_index: i64,
    pub history_cache_version: i64,
    pub created_at: i64,
    pub turns: Vec<SessionHistoryTurn>,
}

#[derive(Debug, Clone)]
pub struct ThreadWorkSnapshotRecord {
    pub child_agent: Agent,
    pub child_session_id: String,
    pub thread_id: String,
    pub stage_id: Option<String>,
    pub snapshot_json: String,
    pub version: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct UpsertCanvasBlockRecord {
    pub block_id: String,
    pub block_kind: CanvasBlockKind,
    pub source_type: CanvasBlockSourceType,
    pub source_key: Option<String>,
    pub source_path: Option<String>,
    pub metadata_json: String,
}

#[derive(Debug, Clone)]
pub struct ChannelSessionRecord {
    pub platform: String,
    pub channel_id: String,
    pub channel_type: Option<String>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub thread_id: Option<String>,
    pub display_name: Option<String>,
    pub agent: Agent,
    pub agent_session_id: String,
    pub sessio_runtime_session_id: String,
    pub workspace_path: String,
    pub metadata_json: String,
    pub last_update_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_activity_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ScheduledTaskRecord {
    pub id: String,
    pub name: String,
    pub status: String,
    pub schedule_json: String,
    pub target_json: String,
    pub project_id: String,
    pub mode: String,
    pub sort_order: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_run_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ScheduledTaskRunRecord {
    pub id: String,
    pub task_id: String,
    pub mode: String,
    pub trigger: String,
    pub status: String,
    pub started_at_ms: i64,
    pub scheduled_for_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub task_name: Option<String>,
    pub target_json: Option<String>,
    pub session_agent: Option<Agent>,
    pub session_id: Option<String>,
    /// Real ACP / jsonl session id stamped on the `sessions` row. Differs
    /// from `session_id` (which carries the runtime's internal handle).
    /// Populated only after the runtime publishes the agent session id; null
    /// for thread runs and for chat runs whose stamp never completed.
    pub agent_session_id: Option<String>,
    pub thread_id: Option<String>,
    pub astra_run_id: Option<String>,
    pub push_platform: Option<String>,
    pub push_chat_id: Option<String>,
    pub push_status: Option<String>,
    pub push_summary: Option<String>,
    pub push_error: Option<String>,
    pub push_sent_at_ms: Option<i64>,
    pub error: Option<String>,
}

pub const SCHEDULED_TASK_RUN_HISTORY_LIMIT_PER_TASK: usize = 20;

pub trait SessionStore: Send + Sync {
    fn init(&self) -> Result<()>;
    fn list_sessions(&self) -> Result<Vec<SessionInfo>>;
    fn list_all_sessions(&self) -> Result<Vec<SessionInfo>>;
    fn list_sessions_by_refs(&self, refs: &[SessionRef<'_>]) -> Result<Vec<SessionInfo>>;
    fn list_channel_sessions(&self) -> Result<Vec<ChannelSessionInfo>>;
    fn get_active_channel_session(
        &self,
        platform: &str,
        channel_id: &str,
    ) -> Result<Option<ChannelSessionRecord>>;
    fn upsert_channel_session(&self, record: &ChannelSessionRecord) -> Result<()>;
    fn update_channel_session_activity(
        &self,
        platform: &str,
        channel_id: &str,
        last_update_id: Option<i64>,
        last_activity_at: i64,
    ) -> Result<()>;
    fn mark_channel_session_ended(
        &self,
        platform: &str,
        channel_id: &str,
        agent: Agent,
        agent_session_id: &str,
        ended_at: i64,
    ) -> Result<()>;
    fn list_scheduled_tasks(&self) -> Result<Vec<ScheduledTaskRecord>>;
    /// Bounded task run history for UI/state hydration. Implementations should
    /// include runs that still need completion or push processing even if they
    /// are outside the history window.
    fn list_scheduled_task_runs(&self) -> Result<Vec<ScheduledTaskRunRecord>>;
    /// Runs that still need background completion detection or channel push.
    fn list_scheduled_task_runs_requiring_update(&self) -> Result<Vec<ScheduledTaskRunRecord>>;
    fn replace_scheduled_tasks(&self, tasks: &[ScheduledTaskRecord]) -> Result<()>;
    fn insert_scheduled_task_run(&self, run: &ScheduledTaskRunRecord) -> Result<()>;
    fn update_scheduled_task_run_status(
        &self,
        run_id: &str,
        status: &str,
        completed_at_ms: Option<i64>,
        error: Option<&str>,
    ) -> Result<()>;
    /// Stamp the real ACP / jsonl session id on a chat-mode run after the
    /// runtime publishes it. Lets historical runs (whose `session_id` is the
    /// runtime's handle, dead after restart) join back to the `sessions`
    /// table.
    fn update_scheduled_task_run_agent_session_id(
        &self,
        run_id: &str,
        agent_session_id: &str,
    ) -> Result<()>;
    fn update_scheduled_task_run_push(
        &self,
        run_id: &str,
        push_status: &str,
        push_summary: Option<&str>,
        push_error: Option<&str>,
        push_sent_at_ms: Option<i64>,
    ) -> Result<()>;
    fn update_scheduled_task_last_run(&self, task_id: &str, when_ms: i64) -> Result<()>;
    /// Mark pushes interrupted mid-summarization (by a shutdown) as failed so a
    /// restart does not re-run summarization and double-notify the channel.
    fn fail_interrupted_task_run_pushes(&self) -> Result<()>;
    fn list_indexed_sessions(&self) -> Result<Vec<IndexedSessionRecord>>;
    fn update_session_rename_title(
        &self,
        agent: Agent,
        session_id: &str,
        rename_title: Option<&str>,
    ) -> Result<()>;
    fn list_process_templates(&self) -> Result<Vec<ProcessTemplateInfo>>;
    fn create_process_template(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<ProcessTemplateInfo>;
    fn update_process_template(
        &self,
        process_template_id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
    ) -> Result<ProcessTemplateInfo>;
    fn delete_process_template(&self, process_template_id: &str) -> Result<()>;
    fn list_projects(&self) -> Result<Vec<ProjectInfo>>;
    fn add_project(
        &self,
        path: &str,
        name: Option<&str>,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo>;
    fn create_project(
        &self,
        parent_path: &str,
        name: &str,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo>;
    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        process_template_id: Option<String>,
    ) -> Result<ProjectInfo>;
    fn archive_project(&self, project_id: &str) -> Result<()>;
    fn list_agents(&self) -> Result<Vec<AgentInfo>>;
    fn get_astra_config(&self) -> Result<AstraConfig>;
    fn update_astra_config(&self, patch: AstraConfigPatch<'_>) -> Result<AstraConfig>;
    fn update_agent_preferences_by_id(
        &self,
        agent_id: &str,
        patch: AgentPreferencesPatch<'_>,
    ) -> Result<AgentInfo>;
    fn update_builtin_agent_preferences(
        &self,
        agent: Agent,
        patch: AgentPreferencesPatch<'_>,
    ) -> Result<AgentInfo>;
    fn get_last_runtime_agent_selection(&self) -> Result<Option<RuntimeAgentSelection>>;
    fn set_last_runtime_agent_selection(
        &self,
        agent: Agent,
        model: Option<&str>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
    ) -> Result<RuntimeAgentSelection>;
    fn list_assistants(&self, project_id: Option<&str>) -> Result<Vec<AssistantInfo>>;
    fn create_assistant(&self, assistant: NewAssistant<'_>) -> Result<AssistantInfo>;
    fn update_assistant(
        &self,
        assistant_id: &str,
        name: Option<&str>,
        agent: Option<AssistantAgentInfo>,
        system_prompt: Option<Option<&str>>,
        color: Option<Option<&str>>,
        selected_skill_ids: Option<Vec<String>>,
        selected_mcp_ids: Option<Vec<String>>,
        enabled: Option<bool>,
    ) -> Result<AssistantInfo>;
    fn delete_assistant(&self, assistant_id: &str) -> Result<()>;
    fn list_threads(&self, project_id: &str) -> Result<Vec<ThreadInfo>>;
    fn list_thread_index(&self, project_id: Option<&str>) -> Result<Vec<ThreadIndexItemInfo>>;
    fn get_thread_work_state(&self, thread_id: &str) -> Result<ThreadInfo>;
    fn get_thread_replay(&self, thread_id: &str) -> Result<ThreadReplayInfo> {
        let thread = self.get_thread_work_state(thread_id)?;
        let plan_rounds = self.list_plan_rounds(thread_id)?;
        let astra_runs = self.list_astra_runs(thread_id)?;
        let mut session_lookup = HashMap::<(Agent, String), SessionInfo>::new();
        let mut sessions = HashMap::<(Agent, String), ThreadReplaySessionInfo>::new();

        for session in &thread.sessions {
            insert_best_session(&mut session_lookup, session.clone());
            add_replay_session_source(
                &mut sessions,
                session.agent,
                &session.id,
                Some(session.clone()),
                ThreadReplaySessionSourceInfo {
                    kind: ThreadReplaySessionSourceKind::Thread,
                    thread_id: Some(thread.id.clone()),
                    stage_id: None,
                    plan_round_id: None,
                    plan_task_id: None,
                    astra_run_id: None,
                    role: None,
                    label: Some("thread".to_string()),
                    stage_snapshot_json: None,
                    assistant_snapshot_json: None,
                    agent_snapshot_json: None,
                    created_at: session.started_at.or(session.updated_at),
                },
            );
        }

        for stage in &thread.stages {
            for session in &stage.sessions {
                insert_best_session(&mut session_lookup, session.clone());
                add_replay_session_source(
                    &mut sessions,
                    session.agent,
                    &session.id,
                    Some(session.clone()),
                    ThreadReplaySessionSourceInfo {
                        kind: ThreadReplaySessionSourceKind::Stage,
                        thread_id: Some(thread.id.clone()),
                        stage_id: Some(stage.id.clone()),
                        plan_round_id: None,
                        plan_task_id: None,
                        astra_run_id: None,
                        role: None,
                        label: stage.name.clone().or_else(|| Some(stage.stage_id.clone())),
                        stage_snapshot_json: None,
                        assistant_snapshot_json: None,
                        agent_snapshot_json: None,
                        created_at: session.started_at.or(session.updated_at),
                    },
                );
            }
        }

        let referenced_keys =
            collect_referenced_session_keys(&plan_rounds, &astra_runs, &session_lookup);
        if !referenced_keys.is_empty() {
            let refs = referenced_keys
                .iter()
                .map(|(agent, session_id)| SessionRef {
                    agent: *agent,
                    session_id: session_id.as_str(),
                })
                .collect::<Vec<_>>();
            for session in self.list_sessions_by_refs(&refs)? {
                insert_best_session(&mut session_lookup, session);
            }
        }

        for round in &plan_rounds {
            for task in &round.tasks {
                for task_session in &task.sessions {
                    if task_session.superseded_at.is_some() {
                        continue;
                    }
                    let session = session_lookup
                        .get(&(task_session.agent, task_session.session_id.clone()))
                        .cloned();
                    add_replay_session_source(
                        &mut sessions,
                        task_session.agent,
                        &task_session.session_id,
                        session,
                        ThreadReplaySessionSourceInfo {
                            kind: ThreadReplaySessionSourceKind::PlanTask,
                            thread_id: Some(thread.id.clone()),
                            stage_id: task.thread_stage_id.clone(),
                            plan_round_id: Some(round.id.clone()),
                            plan_task_id: Some(task.id.clone()),
                            astra_run_id: round.astra_run_id.clone(),
                            role: Some(task_session.role),
                            label: Some(task.title.clone()),
                            stage_snapshot_json: task.stage_snapshot_json.clone(),
                            assistant_snapshot_json: task.assistant_snapshot_json.clone(),
                            agent_snapshot_json: Some(task.agent_snapshot_json.clone()),
                            created_at: Some(task_session.created_at),
                        },
                    );
                }
            }
        }

        for run in &astra_runs {
            for session_ref in &run.internal_planner_sessions {
                let session = session_lookup
                    .get(&(session_ref.agent, session_ref.session_id.clone()))
                    .cloned();
                add_replay_session_source(
                    &mut sessions,
                    session_ref.agent,
                    &session_ref.session_id,
                    session,
                    ThreadReplaySessionSourceInfo {
                        kind: ThreadReplaySessionSourceKind::AstraInternal,
                        thread_id: Some(thread.id.clone()),
                        stage_id: None,
                        plan_round_id: None,
                        plan_task_id: None,
                        astra_run_id: Some(run.run_id.clone()),
                        role: Some(PlanTaskSessionRole::Planner),
                        label: run
                            .planner_backend
                            .as_ref()
                            .map(|backend| format!("Astra planner: {backend}"))
                            .or_else(|| Some("Astra planner".to_string())),
                        stage_snapshot_json: None,
                        assistant_snapshot_json: None,
                        agent_snapshot_json: None,
                        created_at: Some(run.updated_at),
                    },
                );
            }
        }

        let mut sessions = sessions.into_values().collect::<Vec<_>>();
        sessions.sort_by(|a, b| {
            a.first_seen_at
                .unwrap_or(i64::MAX)
                .cmp(&b.first_seen_at.unwrap_or(i64::MAX))
                .then_with(|| a.agent.as_str().cmp(b.agent.as_str()))
                .then_with(|| a.session_id.cmp(&b.session_id))
        });

        Ok(ThreadReplayInfo {
            thread_id: thread.id,
            kind: thread.kind,
            sessions,
        })
    }
    fn create_thread(
        &self,
        project_id: &str,
        goal: &str,
        description: Option<&str>,
    ) -> Result<ThreadInfo>;
    fn create_thread_with_options(
        &self,
        project_id: &str,
        goal: &str,
        description: Option<&str>,
        kind: ThreadKind,
        assistant_ids: &[String],
        agent_participants: &[crate::models::ThreadAgentInfo],
    ) -> Result<ThreadInfo> {
        self.create_thread_with_origin(
            project_id,
            goal,
            description,
            kind,
            assistant_ids,
            agent_participants,
            ThreadOrigin::Manual,
            None,
        )
    }
    /// Full thread creation entry point — `create_thread_with_options` forwards
    /// here with `ThreadOrigin::Manual`. The scheduled-task path in
    /// `scheduled_tasks::start_thread_run` calls this directly with
    /// `ThreadOrigin::ScheduledTask` and the originating task id.
    fn create_thread_with_origin(
        &self,
        project_id: &str,
        goal: &str,
        description: Option<&str>,
        kind: ThreadKind,
        assistant_ids: &[String],
        agent_participants: &[crate::models::ThreadAgentInfo],
        origin: ThreadOrigin,
        scheduled_task_id: Option<&str>,
    ) -> Result<ThreadInfo> {
        let _ = (
            kind,
            assistant_ids,
            agent_participants,
            origin,
            scheduled_task_id,
        );
        self.create_thread(project_id, goal, description)
    }
    fn update_thread(
        &self,
        thread_id: &str,
        goal: Option<&str>,
        description: Option<Option<&str>>,
        enabled: Option<bool>,
    ) -> Result<ThreadInfo>;
    #[allow(clippy::too_many_arguments)]
    fn update_thread_with_options(
        &self,
        thread_id: &str,
        goal: Option<&str>,
        description: Option<Option<&str>>,
        enabled: Option<bool>,
        kind: Option<ThreadKind>,
        assistant_ids: Option<&[String]>,
        agent_participants: Option<&[crate::models::ThreadAgentInfo]>,
    ) -> Result<ThreadInfo> {
        let _ = (kind, assistant_ids, agent_participants);
        self.update_thread(thread_id, goal, description, enabled)
    }
    fn delete_thread(&self, thread_id: &str) -> Result<()>;
    fn create_plan_round(&self, round: NewPlanRound<'_>) -> Result<PlanRoundInfo>;
    fn get_plan_round(&self, round_id: &str) -> Result<Option<PlanRoundInfo>>;
    fn list_plan_rounds(&self, thread_id: &str) -> Result<Vec<PlanRoundInfo>>;
    fn get_plan_task_thread_id(&self, task_id: &str) -> Result<Option<String>>;
    fn update_plan_task_status(
        &self,
        task_id: &str,
        patch: PlanTaskStatusPatch<'_>,
    ) -> Result<PlanTaskInfo>;
    fn complete_plan_task_and_start_next(
        &self,
        task_id: &str,
        patch: PlanTaskStatusPatch<'_>,
    ) -> Result<PlanRoundInfo>;
    fn link_plan_task_session(
        &self,
        session: NewPlanTaskSession<'_>,
    ) -> Result<PlanTaskSessionInfo>;
    fn relink_plan_task_session(
        &self,
        from: NewPlanTaskSession<'_>,
        to_session_id: &str,
        to_role: PlanTaskSessionRole,
    ) -> Result<PlanTaskSessionInfo>;
    fn list_plan_task_sessions(&self, task_id: &str) -> Result<Vec<PlanTaskSessionInfo>>;
    fn list_project_stages(&self, project_id: &str) -> Result<Vec<ProjectStageInfo>>;
    fn list_process_template_stages(
        &self,
        process_template_id: &str,
    ) -> Result<Vec<ProjectStageInfo>>;
    fn create_project_stage(
        &self,
        project_id: &str,
        process_template_id: Option<String>,
        name: &str,
        description: Option<&str>,
        icon: Option<&str>,
    ) -> Result<ProjectStageInfo>;
    fn update_project_stage(
        &self,
        stage_id: &str,
        patch: ProjectStagePatch<'_>,
    ) -> Result<ProjectStageInfo>;
    fn update_project_stage_assistants(
        &self,
        stage_id: &str,
        assistant_ids: &[String],
    ) -> Result<ProjectStageInfo>;
    fn delete_project_stage(&self, stage_id: &str) -> Result<()>;
    fn add_thread_stage(
        &self,
        thread_id: &str,
        stage_id: &str,
        assistant_ids: &[String],
    ) -> Result<StageInfo>;
    fn update_thread_stage(
        &self,
        thread_stage_id: &str,
        assistant_ids: Option<&[String]>,
        order: Option<i64>,
        enabled: Option<bool>,
    ) -> Result<StageInfo>;
    fn update_thread_stage_state(
        &self,
        thread_stage_id: &str,
        status: Option<StageStatus>,
        summary: Option<Option<String>>,
        outcome: Option<Option<String>>,
    ) -> Result<StageInfo>;
    fn list_thread_stage_issues(&self, thread_stage_id: &str) -> Result<Vec<StageIssueInfo>>;
    fn create_thread_stage_issue(
        &self,
        thread_stage_id: &str,
        title: &str,
        description: Option<&str>,
        severity: IssueSeverity,
    ) -> Result<StageIssueInfo>;
    fn update_thread_stage_issue(
        &self,
        issue_id: &str,
        title: Option<&str>,
        description: Option<Option<&str>>,
        status: Option<IssueStatus>,
        severity: Option<IssueSeverity>,
    ) -> Result<StageIssueInfo>;
    fn delete_thread_stage_issue(&self, issue_id: &str) -> Result<()>;
    fn update_thread_stage_assistant_agent(
        &self,
        thread_stage_id: &str,
        assistant_id: &str,
        agent: AssistantAgentInfo,
    ) -> Result<StageInfo>;
    fn delete_thread_stage(&self, thread_stage_id: &str) -> Result<()>;
    fn set_thread_stage(&self, thread_id: &str, thread_stage_id: &str) -> Result<ThreadInfo>;
    fn link_thread_session(
        &self,
        thread_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<ThreadInfo>;
    fn unlink_thread_session(
        &self,
        thread_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<ThreadInfo>;
    fn link_stage_session(
        &self,
        thread_stage_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<StageInfo>;
    fn unlink_stage_session(
        &self,
        thread_stage_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<StageInfo>;
    fn list_kanban_items(&self, project_id: &str) -> Result<Vec<KanbanItem>>;
    fn create_kanban_item(
        &self,
        project_id: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<KanbanItem>;
    fn update_kanban_item(
        &self,
        item_id: &str,
        title: Option<&str>,
        description: Option<Option<&str>>,
        status: Option<KanbanStatus>,
    ) -> Result<KanbanItem>;
    fn delete_kanban_item(&self, item_id: &str) -> Result<()>;
    fn link_kanban_item_session(
        &self,
        item_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<KanbanItem>;
    fn unlink_kanban_item_session(
        &self,
        item_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<KanbanItem>;
    fn get_runtime_agent_capability(
        &self,
        agent: Agent,
    ) -> Result<Option<RuntimeAgentCapabilityRecord>>;
    fn upsert_runtime_agent_capability(&self, record: &RuntimeAgentCapabilityRecord) -> Result<()>;
    fn get_runtime_agent_session_config(
        &self,
        agent: Agent,
        adapter_version: &str,
    ) -> Result<Option<RuntimeAgentSessionConfigRecord>>;
    fn list_runtime_agent_session_configs(
        &self,
        agent: Agent,
    ) -> Result<Vec<RuntimeAgentSessionConfigRecord>>;
    fn mark_runtime_agent_session_config_needs_refresh(
        &self,
        agent: Agent,
        adapter_version: &str,
    ) -> Result<()>;
    fn upsert_runtime_agent_session_config(
        &self,
        record: &RuntimeAgentSessionConfigRecord,
    ) -> Result<()>;
    fn get_session_history_snapshots(
        &self,
        child_agent: Agent,
        child_session_id: &str,
    ) -> Result<Vec<SessionHistorySnapshotRecord>>;
    fn replace_session_history_snapshots(
        &self,
        child_agent: Agent,
        child_session_id: &str,
        snapshots: &[SessionHistorySnapshotRecord],
    ) -> Result<()>;
    fn save_thread_work_snapshot(&self, snapshot: &ThreadWorkSnapshotRecord) -> Result<()>;
    fn get_thread_work_snapshot(
        &self,
        child_agent: Agent,
        child_session_id: &str,
    ) -> Result<Option<ThreadWorkSnapshotRecord>>;
    fn replace_astra_run_sessions(
        &self,
        run_id: &str,
        sessions: &[AstraRunSessionRecord],
    ) -> Result<()>;
    fn list_astra_run_sessions(&self, run_id: &str) -> Result<Vec<AstraRunSessionRecord>>;
    fn list_astra_run_sessions_for_thread(
        &self,
        thread_id: &str,
    ) -> Result<Vec<AstraRunSessionRecord>>;
    fn upsert_astra_run(&self, run: &AstraRunRecord) -> Result<()>;
    fn get_astra_run(&self, run_id: &str) -> Result<Option<AstraRunRecord>>;
    fn get_active_astra_run(&self, thread_id: &str) -> Result<Option<AstraRunRecord>>;
    fn list_astra_runs(&self, thread_id: &str) -> Result<Vec<AstraRunRecord>>;
    /// Transition every active run to `interrupted` and return the rows that
    /// changed (with their patched status), so callers can notify listeners.
    fn interrupt_active_astra_runs(&self) -> Result<Vec<AstraRunRecord>>;
    fn cleanup_partial_astra_sessions(&self, session_ids: &[String]) -> Result<usize>;
    fn upsert_session(&self, scope: &str, session: &SessionInfo) -> Result<()>;
    /// Attach a scheduled task lineage to every existing row of an
    /// `(agent, session_id)` identity. Called from `scheduled_tasks` after
    /// `runtime.start_session` returns, so the chat-mode auto task session
    /// (and the summary push session, when `is_auxiliary = true`) carry the
    /// task linkage through subsequent reindexing. Insert paths preserve
    /// `scheduled_task_id` via `merged_scheduled_task_id`, but the runtime
    /// has already written its placeholder row by the time we get here, so
    /// we write directly.
    fn mark_session_scheduled_task(
        &self,
        agent: Agent,
        session_id: &str,
        scheduled_task_id: &str,
        is_auxiliary: bool,
    ) -> Result<()>;
    /// Set the session's origin (`thread`/`channel`) on every row of an
    /// `(agent, session_id)` identity. Called by im-bridge once the channel
    /// runtime session has been registered, since the runtime writes its
    /// placeholder before im-bridge knows it's a channel session. Same
    /// rationale as `mark_session_scheduled_task` — write directly because
    /// the row already exists. Origin is sticky so calling this with `Chat`
    /// is a no-op.
    fn mark_session_origin(
        &self,
        agent: Agent,
        session_id: &str,
        origin: crate::models::SessionOrigin,
    ) -> Result<()>;
    fn replace_by_scope(&self, scope: &str, agent: Agent, sessions: &[SessionInfo]) -> Result<()>;
    fn upsert_subagent(
        &self,
        parent_agent: Agent,
        parent_scope: &str,
        parent_session_id: &str,
        subagent: &SubagentInfo,
    ) -> Result<()>;
    fn update_message_count(
        &self,
        agent: Agent,
        session_id: Option<&str>,
        file_path: &str,
        message_count: usize,
    ) -> Result<()>;
    fn mark_file_path_unavailable(&self, file_path: &str) -> Result<()>;
    fn mark_subagent_file_unavailable(&self, file_path: &str) -> Result<()>;
    fn mark_file_path_unindexable(&self, agent: Agent, file_path: &str) -> Result<()>;
    fn mark_missing_scopes_unavailable(
        &self,
        agent: Agent,
        present: &HashSet<String>,
    ) -> Result<()>;
    fn get_or_create_canvas_document(
        &self,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<CanvasDocumentInfo>;
    fn get_canvas_document_state(&self, session_id: &str) -> Result<CanvasDocumentState>;
    fn save_canvas_draft(
        &self,
        session_id: &str,
        title: Option<&str>,
        draft_snapshot_path: &str,
        draft_snapshot_hash: &str,
    ) -> Result<CanvasDocumentInfo>;
    fn save_canvas_revision(
        &self,
        session_id: &str,
        title: Option<&str>,
        snapshot_path: &str,
        snapshot_hash: &str,
        snapshot_size_bytes: i64,
        source: &str,
    ) -> Result<(CanvasDocumentInfo, CanvasRevisionInfo)>;
    fn prune_canvas_revisions(&self, session_id: &str, keep_latest: usize) -> Result<Vec<String>>;
    fn replace_canvas_blocks(
        &self,
        session_id: &str,
        blocks: &[UpsertCanvasBlockRecord],
    ) -> Result<Vec<CanvasBlockRecord>>;
    fn create_canvas_context_anchor(
        &self,
        session_id: &str,
        anchor_block_id: Option<&str>,
        selection_block_ids_json: &str,
        selection_element_ids_json: &str,
        turn_id: &str,
        summary: Option<&str>,
    ) -> Result<CanvasContextAnchor>;
}

pub(crate) fn collect_referenced_session_keys(
    plan_rounds: &[PlanRoundInfo],
    astra_runs: &[AstraRunRecord],
    existing_sessions: &HashMap<(Agent, String), SessionInfo>,
) -> Vec<(Agent, String)> {
    let mut refs = HashSet::<(Agent, String)>::new();
    for round in plan_rounds {
        for task in &round.tasks {
            for task_session in &task.sessions {
                if task_session.superseded_at.is_some() {
                    continue;
                }
                let key = (task_session.agent, task_session.session_id.clone());
                if !existing_sessions.contains_key(&key) {
                    refs.insert(key);
                }
            }
        }
    }
    for run in astra_runs {
        for session in &run.internal_planner_sessions {
            if is_virtual_orchestrator_session_id(&session.session_id) {
                continue;
            }
            let key = (session.agent, session.session_id.clone());
            if !existing_sessions.contains_key(&key) {
                refs.insert(key);
            }
        }
    }
    refs.into_iter().collect()
}

pub(crate) fn is_virtual_orchestrator_session_id(session_id: &str) -> bool {
    session_id.trim().starts_with("deterministic-orchestrator-")
}

fn add_replay_session_source(
    sessions: &mut HashMap<(Agent, String), ThreadReplaySessionInfo>,
    agent: Agent,
    session_id: &str,
    session: Option<SessionInfo>,
    source: ThreadReplaySessionSourceInfo,
) {
    let key = (agent, session_id.to_string());
    let source_time = source.created_at;
    let entry = sessions
        .entry(key)
        .or_insert_with(|| ThreadReplaySessionInfo {
            agent,
            session_id: session_id.to_string(),
            session: None,
            sources: Vec::new(),
            first_seen_at: source_time,
            last_seen_at: source_time,
        });

    if entry.session.is_none() {
        entry.session = session;
    }
    if let Some(source_time) = source_time {
        entry.first_seen_at = Some(
            entry
                .first_seen_at
                .map(|value| value.min(source_time))
                .unwrap_or(source_time),
        );
        entry.last_seen_at = Some(
            entry
                .last_seen_at
                .map(|value| value.max(source_time))
                .unwrap_or(source_time),
        );
    }
    if !entry.sources.iter().any(|existing| {
        existing.kind == source.kind
            && existing.stage_id == source.stage_id
            && existing.plan_round_id == source.plan_round_id
            && existing.plan_task_id == source.plan_task_id
            && existing.astra_run_id == source.astra_run_id
            && existing.role == source.role
    }) {
        entry.sources.push(source);
    }
}
