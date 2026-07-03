use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::models::{
    Agent, AgentInfo, AssistantAgentInfo, AssistantInfo, AstraConfig, CanvasBlockRecord,
    CanvasContextAnchor, CanvasDocumentInfo, CanvasDocumentState, CanvasRevisionInfo,
    ChannelSessionInfo, IssueSeverity, IssueStatus, KanbanItem, KanbanStatus, PlanRoundInfo,
    PlanTaskInfo, PlanTaskSessionInfo, PlanTaskSessionRole, ProcessTemplateInfo, ProjectInfo,
    ProjectStageInfo, SessionInfo, StageInfo, StageIssueInfo, StageStatus, SubagentInfo,
    ThreadAgentInfo, ThreadIndexItemInfo, ThreadInfo, ThreadKind, ThreadOrigin,
};
use crate::store::{
    file_mtime_for, is_placeholder_indexed_session, is_real_session_file_path,
    is_virtual_session_ref, now_ms, AgentPreferencesPatch, AstraConfigPatch, AstraRunRecord,
    ChannelSessionRecord, IndexedSessionRecord, IndexedSubagentRecord, NewAssistant, NewPlanRound,
    NewPlanTaskSession, PlanTaskStatusPatch, ProjectStagePatch, RuntimeAgentCapabilityRecord,
    RuntimeAgentSelection, RuntimeAgentSessionConfigRecord, ScheduledTaskRecord,
    ScheduledTaskRunRecord, SessionHistorySnapshotRecord, SessionRef, SessionStore,
    ThreadWorkSnapshotRecord, UpsertCanvasBlockRecord,
};

// In-memory snapshot of the indexed-session view. polling reads this on every
// tick instead of hitting the underlying store; writes go through inner first
// and the snapshot is patched on success.
struct Snapshot {
    by_pk: HashMap<(Agent, String, String), IndexedSessionRecord>,
}

impl Snapshot {
    fn load_from(records: Vec<IndexedSessionRecord>) -> Self {
        let mut by_pk = HashMap::with_capacity(records.len());
        for r in records {
            by_pk.insert((r.agent, r.session_id.clone(), r.scope.clone()), r);
        }
        Self { by_pk }
    }

    fn to_vec(&self) -> Vec<IndexedSessionRecord> {
        self.by_pk.values().cloned().collect()
    }
}

pub struct CachedStore {
    inner: Arc<dyn SessionStore>,
    snapshot: RwLock<Snapshot>,
}

impl CachedStore {
    pub fn new(inner: Arc<dyn SessionStore>) -> Result<Self> {
        let records = inner.list_indexed_sessions()?;
        Ok(Self {
            inner,
            snapshot: RwLock::new(Snapshot::load_from(records)),
        })
    }

    fn refresh_from_inner(&self) -> Result<()> {
        let records = self.inner.list_indexed_sessions()?;
        *self.snapshot.write().unwrap() = Snapshot::load_from(records);
        Ok(())
    }

    fn to_indexed_session_only(scope: &str, s: &SessionInfo) -> IndexedSessionRecord {
        // Subagents live on their own lifecycle now: don't capture them here,
        // they get patched in by upsert_subagent.
        IndexedSessionRecord {
            agent: s.agent,
            session_id: s.id.clone(),
            scope: scope.to_string(),
            file_path: s.file_path.clone(),
            forked_from_agent: s.forked_from_agent,
            forked_from_id: s.forked_from_id.clone(),
            file_size: s.file_size,
            file_mtime: file_mtime_for(&s.file_path),
            last_indexed_at: now_ms(),
            available: s.available,
            archived: s.archived,
            subagents: Vec::new(),
        }
    }

    fn to_indexed_subagent(
        parent_agent: Agent,
        parent_scope: &str,
        parent_session_id: &str,
        sub: &SubagentInfo,
    ) -> IndexedSubagentRecord {
        IndexedSubagentRecord {
            parent_agent,
            parent_session_id: parent_session_id.to_string(),
            parent_scope: parent_scope.to_string(),
            subagent_id: sub.id.clone(),
            file_path: sub.file_path.clone(),
            file_size: sub.file_size,
            file_mtime: file_mtime_for(&sub.file_path),
            available: sub.available,
        }
    }

    fn upsert_session_snapshot(&self, scope: &str, session: &SessionInfo) -> Result<()> {
        let new_rec = Self::to_indexed_session_only(scope, session);
        let key = (
            new_rec.agent,
            new_rec.session_id.clone(),
            new_rec.scope.clone(),
        );
        let mut snap = self.snapshot.write().unwrap();
        if !is_real_session_file_path(&new_rec.file_path)
            && snap.by_pk.iter().any(|((agent, session_id, _), rec)| {
                *agent == new_rec.agent
                    && session_id == &new_rec.session_id
                    && is_real_session_file_path(&rec.file_path)
            })
        {
            drop(snap);
            return self.refresh_from_inner();
        }
        let placeholder_key = snap
            .by_pk
            .iter()
            .find(|((agent, session_id, scope), rec)| {
                *agent == new_rec.agent
                    && session_id == &new_rec.session_id
                    && scope != &new_rec.scope
                    && rec.file_size == 0
                    && rec.available
            })
            .map(|(key, _)| key.clone());
        let placeholder_subs = placeholder_key
            .as_ref()
            .and_then(|key| snap.by_pk.remove(key))
            .map(|rec| rec.subagents)
            .unwrap_or_default();
        // Preserve any subagents already attached to this session in the
        // snapshot; their lifecycle is independent of the main row.
        let existing_subs = snap
            .by_pk
            .get(&key)
            .map(|r| r.subagents.clone())
            .unwrap_or(placeholder_subs);
        let mut rec = new_rec;
        rec.subagents = existing_subs;
        snap.by_pk.insert(key, rec);
        Ok(())
    }
}

impl SessionStore for CachedStore {
    fn init(&self) -> Result<()> {
        self.inner.init()
    }

    fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        self.inner.list_sessions()
    }

    fn list_all_sessions(&self) -> Result<Vec<SessionInfo>> {
        self.inner.list_all_sessions()
    }

    fn list_sessions_by_refs(&self, refs: &[SessionRef<'_>]) -> Result<Vec<SessionInfo>> {
        self.inner.list_sessions_by_refs(refs)
    }

    fn list_channel_sessions(&self) -> Result<Vec<ChannelSessionInfo>> {
        self.inner.list_channel_sessions()
    }

    fn get_active_channel_session(
        &self,
        platform: &str,
        channel_id: &str,
    ) -> Result<Option<ChannelSessionRecord>> {
        self.inner.get_active_channel_session(platform, channel_id)
    }

    fn upsert_channel_session(&self, record: &ChannelSessionRecord) -> Result<()> {
        self.inner.upsert_channel_session(record)
    }

    fn update_channel_session_activity(
        &self,
        platform: &str,
        channel_id: &str,
        last_update_id: Option<i64>,
        last_activity_at: i64,
    ) -> Result<()> {
        self.inner.update_channel_session_activity(
            platform,
            channel_id,
            last_update_id,
            last_activity_at,
        )
    }

    fn mark_channel_session_ended(
        &self,
        platform: &str,
        channel_id: &str,
        agent: Agent,
        agent_session_id: &str,
        ended_at: i64,
    ) -> Result<()> {
        self.inner.mark_channel_session_ended(
            platform,
            channel_id,
            agent,
            agent_session_id,
            ended_at,
        )
    }

    fn list_scheduled_tasks(&self) -> Result<Vec<ScheduledTaskRecord>> {
        self.inner.list_scheduled_tasks()
    }

    fn list_scheduled_task_runs(&self) -> Result<Vec<ScheduledTaskRunRecord>> {
        self.inner.list_scheduled_task_runs()
    }

    fn list_scheduled_task_runs_requiring_update(&self) -> Result<Vec<ScheduledTaskRunRecord>> {
        self.inner.list_scheduled_task_runs_requiring_update()
    }

    fn replace_scheduled_tasks(&self, tasks: &[ScheduledTaskRecord]) -> Result<()> {
        self.inner.replace_scheduled_tasks(tasks)
    }

    fn insert_scheduled_task_run(&self, run: &ScheduledTaskRunRecord) -> Result<()> {
        self.inner.insert_scheduled_task_run(run)
    }

    fn update_scheduled_task_run_status(
        &self,
        run_id: &str,
        status: &str,
        completed_at_ms: Option<i64>,
        error: Option<&str>,
    ) -> Result<()> {
        self.inner
            .update_scheduled_task_run_status(run_id, status, completed_at_ms, error)
    }

    fn update_scheduled_task_run_agent_session_id(
        &self,
        run_id: &str,
        agent_session_id: &str,
    ) -> Result<()> {
        self.inner
            .update_scheduled_task_run_agent_session_id(run_id, agent_session_id)
    }

    fn update_scheduled_task_run_push(
        &self,
        run_id: &str,
        push_status: &str,
        push_summary: Option<&str>,
        push_error: Option<&str>,
        push_sent_at_ms: Option<i64>,
    ) -> Result<()> {
        self.inner.update_scheduled_task_run_push(
            run_id,
            push_status,
            push_summary,
            push_error,
            push_sent_at_ms,
        )
    }

    fn update_scheduled_task_last_run(&self, task_id: &str, when_ms: i64) -> Result<()> {
        self.inner.update_scheduled_task_last_run(task_id, when_ms)
    }

    fn fail_interrupted_task_run_pushes(&self) -> Result<()> {
        self.inner.fail_interrupted_task_run_pushes()
    }

    fn list_indexed_sessions(&self) -> Result<Vec<IndexedSessionRecord>> {
        Ok(self.snapshot.read().unwrap().to_vec())
    }

    fn update_session_rename_title(
        &self,
        agent: Agent,
        session_id: &str,
        rename_title: Option<&str>,
    ) -> Result<()> {
        self.inner
            .update_session_rename_title(agent, session_id, rename_title)?;
        self.refresh_from_inner()
    }

    fn list_process_templates(&self) -> Result<Vec<ProcessTemplateInfo>> {
        self.inner.list_process_templates()
    }

    fn create_process_template(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<ProcessTemplateInfo> {
        self.inner.create_process_template(name, description)
    }

    fn update_process_template(
        &self,
        process_template_id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
    ) -> Result<ProcessTemplateInfo> {
        self.inner
            .update_process_template(process_template_id, name, description)
    }

    fn delete_process_template(&self, process_template_id: &str) -> Result<()> {
        self.inner.delete_process_template(process_template_id)
    }

    fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        self.inner.list_projects()
    }

    fn add_project(
        &self,
        path: &str,
        name: Option<&str>,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo> {
        self.inner
            .add_project(path, name, process_template_id, enabled_stage_ids)
    }

    fn create_project(
        &self,
        parent_path: &str,
        name: &str,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo> {
        self.inner
            .create_project(parent_path, name, process_template_id, enabled_stage_ids)
    }

    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        process_template_id: Option<String>,
    ) -> Result<ProjectInfo> {
        self.inner
            .update_project(project_id, name, process_template_id)
    }

    fn archive_project(&self, project_id: &str) -> Result<()> {
        self.inner.archive_project(project_id)
    }

    fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        self.inner.list_agents()
    }

    fn get_astra_config(&self) -> Result<AstraConfig> {
        self.inner.get_astra_config()
    }

    fn update_astra_config(&self, patch: AstraConfigPatch<'_>) -> Result<AstraConfig> {
        self.inner.update_astra_config(patch)
    }

    fn update_agent_preferences_by_id(
        &self,
        agent_id: &str,
        patch: AgentPreferencesPatch<'_>,
    ) -> Result<AgentInfo> {
        self.inner.update_agent_preferences_by_id(agent_id, patch)
    }

    fn update_builtin_agent_preferences(
        &self,
        agent: Agent,
        patch: AgentPreferencesPatch<'_>,
    ) -> Result<AgentInfo> {
        self.inner.update_builtin_agent_preferences(agent, patch)
    }

    fn get_last_runtime_agent_selection(&self) -> Result<Option<RuntimeAgentSelection>> {
        self.inner.get_last_runtime_agent_selection()
    }

    fn set_last_runtime_agent_selection(
        &self,
        agent: Agent,
        model: Option<&str>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
    ) -> Result<RuntimeAgentSelection> {
        self.inner
            .set_last_runtime_agent_selection(agent, model, effort, permission_mode)
    }

    fn list_assistants(&self, project_id: Option<&str>) -> Result<Vec<AssistantInfo>> {
        self.inner.list_assistants(project_id)
    }

    fn create_assistant(&self, assistant: NewAssistant<'_>) -> Result<AssistantInfo> {
        self.inner.create_assistant(assistant)
    }

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
    ) -> Result<AssistantInfo> {
        self.inner.update_assistant(
            assistant_id,
            name,
            agent,
            system_prompt,
            color,
            selected_skill_ids,
            selected_mcp_ids,
            enabled,
        )
    }

    fn delete_assistant(&self, assistant_id: &str) -> Result<()> {
        self.inner.delete_assistant(assistant_id)
    }

    fn list_threads(&self, project_id: &str) -> Result<Vec<ThreadInfo>> {
        self.inner.list_threads(project_id)
    }

    fn list_thread_index(&self, project_id: Option<&str>) -> Result<Vec<ThreadIndexItemInfo>> {
        self.inner.list_thread_index(project_id)
    }

    fn get_thread_work_state(&self, thread_id: &str) -> Result<ThreadInfo> {
        self.inner.get_thread_work_state(thread_id)
    }

    fn create_thread(
        &self,
        project_id: &str,
        goal: &str,
        description: Option<&str>,
    ) -> Result<ThreadInfo> {
        self.inner.create_thread(project_id, goal, description)
    }

    fn create_thread_with_options(
        &self,
        project_id: &str,
        goal: &str,
        description: Option<&str>,
        kind: ThreadKind,
        assistant_ids: &[String],
        agent_participants: &[ThreadAgentInfo],
    ) -> Result<ThreadInfo> {
        self.inner.create_thread_with_options(
            project_id,
            goal,
            description,
            kind,
            assistant_ids,
            agent_participants,
        )
    }

    fn create_thread_with_origin(
        &self,
        project_id: &str,
        goal: &str,
        description: Option<&str>,
        kind: ThreadKind,
        assistant_ids: &[String],
        agent_participants: &[ThreadAgentInfo],
        origin: ThreadOrigin,
        scheduled_task_id: Option<&str>,
    ) -> Result<ThreadInfo> {
        self.inner.create_thread_with_origin(
            project_id,
            goal,
            description,
            kind,
            assistant_ids,
            agent_participants,
            origin,
            scheduled_task_id,
        )
    }

    fn update_thread(
        &self,
        thread_id: &str,
        goal: Option<&str>,
        description: Option<Option<&str>>,
        enabled: Option<bool>,
    ) -> Result<ThreadInfo> {
        self.inner
            .update_thread(thread_id, goal, description, enabled)
    }

    fn update_thread_with_options(
        &self,
        thread_id: &str,
        goal: Option<&str>,
        description: Option<Option<&str>>,
        enabled: Option<bool>,
        kind: Option<ThreadKind>,
        assistant_ids: Option<&[String]>,
        agent_participants: Option<&[ThreadAgentInfo]>,
    ) -> Result<ThreadInfo> {
        self.inner.update_thread_with_options(
            thread_id,
            goal,
            description,
            enabled,
            kind,
            assistant_ids,
            agent_participants,
        )
    }

    fn delete_thread(&self, thread_id: &str) -> Result<()> {
        self.inner.delete_thread(thread_id)
    }

    fn create_plan_round(&self, round: NewPlanRound<'_>) -> Result<PlanRoundInfo> {
        self.inner.create_plan_round(round)
    }

    fn get_plan_round(&self, round_id: &str) -> Result<Option<PlanRoundInfo>> {
        self.inner.get_plan_round(round_id)
    }

    fn list_plan_rounds(&self, thread_id: &str) -> Result<Vec<PlanRoundInfo>> {
        self.inner.list_plan_rounds(thread_id)
    }

    fn get_plan_task_thread_id(&self, task_id: &str) -> Result<Option<String>> {
        self.inner.get_plan_task_thread_id(task_id)
    }

    fn update_plan_task_status(
        &self,
        task_id: &str,
        patch: PlanTaskStatusPatch<'_>,
    ) -> Result<PlanTaskInfo> {
        self.inner.update_plan_task_status(task_id, patch)
    }

    fn complete_plan_task_and_start_next(
        &self,
        task_id: &str,
        patch: PlanTaskStatusPatch<'_>,
    ) -> Result<PlanRoundInfo> {
        self.inner.complete_plan_task_and_start_next(task_id, patch)
    }

    fn link_plan_task_session(
        &self,
        session: NewPlanTaskSession<'_>,
    ) -> Result<PlanTaskSessionInfo> {
        self.inner.link_plan_task_session(session)
    }

    fn relink_plan_task_session(
        &self,
        from: NewPlanTaskSession<'_>,
        to_session_id: &str,
        to_role: PlanTaskSessionRole,
    ) -> Result<PlanTaskSessionInfo> {
        self.inner
            .relink_plan_task_session(from, to_session_id, to_role)
    }

    fn list_plan_task_sessions(&self, task_id: &str) -> Result<Vec<PlanTaskSessionInfo>> {
        self.inner.list_plan_task_sessions(task_id)
    }

    fn list_project_stages(&self, project_id: &str) -> Result<Vec<ProjectStageInfo>> {
        self.inner.list_project_stages(project_id)
    }

    fn list_process_template_stages(
        &self,
        process_template_id: &str,
    ) -> Result<Vec<ProjectStageInfo>> {
        self.inner.list_process_template_stages(process_template_id)
    }

    fn create_project_stage(
        &self,
        project_id: &str,
        process_template_id: Option<String>,
        name: &str,
        description: Option<&str>,
        icon: Option<&str>,
    ) -> Result<ProjectStageInfo> {
        self.inner
            .create_project_stage(project_id, process_template_id, name, description, icon)
    }

    fn update_project_stage(
        &self,
        stage_id: &str,
        patch: ProjectStagePatch<'_>,
    ) -> Result<ProjectStageInfo> {
        self.inner.update_project_stage(stage_id, patch)
    }

    fn update_project_stage_assistants(
        &self,
        stage_id: &str,
        assistant_ids: &[String],
    ) -> Result<ProjectStageInfo> {
        self.inner
            .update_project_stage_assistants(stage_id, assistant_ids)
    }

    fn delete_project_stage(&self, stage_id: &str) -> Result<()> {
        self.inner.delete_project_stage(stage_id)
    }

    fn add_thread_stage(
        &self,
        thread_id: &str,
        stage_id: &str,
        assistant_ids: &[String],
    ) -> Result<StageInfo> {
        self.inner
            .add_thread_stage(thread_id, stage_id, assistant_ids)
    }

    fn update_thread_stage(
        &self,
        thread_stage_id: &str,
        assistant_ids: Option<&[String]>,
        order: Option<i64>,
        enabled: Option<bool>,
    ) -> Result<StageInfo> {
        self.inner
            .update_thread_stage(thread_stage_id, assistant_ids, order, enabled)
    }

    fn update_thread_stage_state(
        &self,
        thread_stage_id: &str,
        status: Option<StageStatus>,
        summary: Option<Option<String>>,
        outcome: Option<Option<String>>,
    ) -> Result<StageInfo> {
        self.inner
            .update_thread_stage_state(thread_stage_id, status, summary, outcome)
    }

    fn list_thread_stage_issues(&self, thread_stage_id: &str) -> Result<Vec<StageIssueInfo>> {
        self.inner.list_thread_stage_issues(thread_stage_id)
    }

    fn create_thread_stage_issue(
        &self,
        thread_stage_id: &str,
        title: &str,
        description: Option<&str>,
        severity: IssueSeverity,
    ) -> Result<StageIssueInfo> {
        self.inner
            .create_thread_stage_issue(thread_stage_id, title, description, severity)
    }

    fn update_thread_stage_issue(
        &self,
        issue_id: &str,
        title: Option<&str>,
        description: Option<Option<&str>>,
        status: Option<IssueStatus>,
        severity: Option<IssueSeverity>,
    ) -> Result<StageIssueInfo> {
        self.inner
            .update_thread_stage_issue(issue_id, title, description, status, severity)
    }

    fn delete_thread_stage_issue(&self, issue_id: &str) -> Result<()> {
        self.inner.delete_thread_stage_issue(issue_id)
    }

    fn update_thread_stage_assistant_agent(
        &self,
        thread_stage_id: &str,
        assistant_id: &str,
        agent: AssistantAgentInfo,
    ) -> Result<StageInfo> {
        self.inner
            .update_thread_stage_assistant_agent(thread_stage_id, assistant_id, agent)
    }

    fn delete_thread_stage(&self, thread_stage_id: &str) -> Result<()> {
        self.inner.delete_thread_stage(thread_stage_id)
    }

    fn set_thread_stage(&self, thread_id: &str, thread_stage_id: &str) -> Result<ThreadInfo> {
        self.inner.set_thread_stage(thread_id, thread_stage_id)
    }

    fn link_thread_session(
        &self,
        thread_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<ThreadInfo> {
        self.inner.link_thread_session(thread_id, agent, session_id)
    }

    fn unlink_thread_session(
        &self,
        thread_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<ThreadInfo> {
        self.inner
            .unlink_thread_session(thread_id, agent, session_id)
    }

    fn link_stage_session(
        &self,
        thread_stage_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<StageInfo> {
        self.inner
            .link_stage_session(thread_stage_id, agent, session_id)
    }

    fn unlink_stage_session(
        &self,
        thread_stage_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<StageInfo> {
        self.inner
            .unlink_stage_session(thread_stage_id, agent, session_id)
    }

    fn list_kanban_items(&self, project_id: &str) -> Result<Vec<KanbanItem>> {
        self.inner.list_kanban_items(project_id)
    }

    fn create_kanban_item(
        &self,
        project_id: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<KanbanItem> {
        self.inner
            .create_kanban_item(project_id, title, description)
    }

    fn update_kanban_item(
        &self,
        item_id: &str,
        title: Option<&str>,
        description: Option<Option<&str>>,
        status: Option<KanbanStatus>,
    ) -> Result<KanbanItem> {
        self.inner
            .update_kanban_item(item_id, title, description, status)
    }

    fn delete_kanban_item(&self, item_id: &str) -> Result<()> {
        self.inner.delete_kanban_item(item_id)
    }

    fn link_kanban_item_session(
        &self,
        item_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<KanbanItem> {
        self.inner
            .link_kanban_item_session(item_id, agent, session_id)
    }

    fn unlink_kanban_item_session(
        &self,
        item_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<KanbanItem> {
        self.inner
            .unlink_kanban_item_session(item_id, agent, session_id)
    }

    fn get_runtime_agent_capability(
        &self,
        agent: Agent,
    ) -> Result<Option<RuntimeAgentCapabilityRecord>> {
        self.inner.get_runtime_agent_capability(agent)
    }

    fn upsert_runtime_agent_capability(&self, record: &RuntimeAgentCapabilityRecord) -> Result<()> {
        self.inner.upsert_runtime_agent_capability(record)
    }

    fn get_runtime_agent_session_config(
        &self,
        agent: Agent,
        adapter_version: &str,
    ) -> Result<Option<RuntimeAgentSessionConfigRecord>> {
        self.inner
            .get_runtime_agent_session_config(agent, adapter_version)
    }

    fn list_runtime_agent_session_configs(
        &self,
        agent: Agent,
    ) -> Result<Vec<RuntimeAgentSessionConfigRecord>> {
        self.inner.list_runtime_agent_session_configs(agent)
    }

    fn mark_runtime_agent_session_config_needs_refresh(
        &self,
        agent: Agent,
        adapter_version: &str,
    ) -> Result<()> {
        self.inner
            .mark_runtime_agent_session_config_needs_refresh(agent, adapter_version)
    }

    fn upsert_runtime_agent_session_config(
        &self,
        record: &RuntimeAgentSessionConfigRecord,
    ) -> Result<()> {
        self.inner.upsert_runtime_agent_session_config(record)
    }

    fn get_session_history_snapshots(
        &self,
        child_agent: Agent,
        child_session_id: &str,
    ) -> Result<Vec<SessionHistorySnapshotRecord>> {
        self.inner
            .get_session_history_snapshots(child_agent, child_session_id)
    }

    fn replace_session_history_snapshots(
        &self,
        child_agent: Agent,
        child_session_id: &str,
        snapshots: &[SessionHistorySnapshotRecord],
    ) -> Result<()> {
        self.inner
            .replace_session_history_snapshots(child_agent, child_session_id, snapshots)
    }

    fn save_thread_work_snapshot(&self, snapshot: &ThreadWorkSnapshotRecord) -> Result<()> {
        self.inner.save_thread_work_snapshot(snapshot)
    }

    fn get_thread_work_snapshot(
        &self,
        child_agent: Agent,
        child_session_id: &str,
    ) -> Result<Option<ThreadWorkSnapshotRecord>> {
        self.inner
            .get_thread_work_snapshot(child_agent, child_session_id)
    }

    fn replace_astra_run_sessions(
        &self,
        run_id: &str,
        sessions: &[crate::store::AstraRunSessionRecord],
    ) -> Result<()> {
        self.inner.replace_astra_run_sessions(run_id, sessions)
    }

    fn list_astra_run_sessions(
        &self,
        run_id: &str,
    ) -> Result<Vec<crate::store::AstraRunSessionRecord>> {
        self.inner.list_astra_run_sessions(run_id)
    }

    fn list_astra_run_sessions_for_thread(
        &self,
        thread_id: &str,
    ) -> Result<Vec<crate::store::AstraRunSessionRecord>> {
        self.inner.list_astra_run_sessions_for_thread(thread_id)
    }

    fn upsert_astra_run(&self, run: &AstraRunRecord) -> Result<()> {
        self.inner.upsert_astra_run(run)
    }

    fn get_astra_run(&self, run_id: &str) -> Result<Option<AstraRunRecord>> {
        self.inner.get_astra_run(run_id)
    }

    fn get_active_astra_run(&self, thread_id: &str) -> Result<Option<AstraRunRecord>> {
        self.inner.get_active_astra_run(thread_id)
    }

    fn list_astra_runs(&self, thread_id: &str) -> Result<Vec<AstraRunRecord>> {
        self.inner.list_astra_runs(thread_id)
    }

    fn interrupt_active_astra_runs(&self) -> Result<Vec<AstraRunRecord>> {
        self.inner.interrupt_active_astra_runs()
    }

    fn cleanup_partial_astra_sessions(&self, session_ids: &[String]) -> Result<usize> {
        let changed = self.inner.cleanup_partial_astra_sessions(session_ids)?;
        if changed > 0 {
            self.refresh_from_inner()?;
        }
        Ok(changed)
    }

    fn upsert_session(&self, scope: &str, session: &SessionInfo) -> Result<()> {
        self.inner.upsert_session(scope, session)?;
        self.upsert_session_snapshot(scope, session)
    }

    fn mark_session_scheduled_task(
        &self,
        agent: Agent,
        session_id: &str,
        scheduled_task_id: &str,
        is_auxiliary: bool,
    ) -> Result<()> {
        // CachedStore::list_sessions delegates straight to inner, so the
        // sqlite UPDATE is enough; no snapshot rewrite needed.
        self.inner
            .mark_session_scheduled_task(agent, session_id, scheduled_task_id, is_auxiliary)
    }

    fn mark_session_origin(
        &self,
        agent: Agent,
        session_id: &str,
        origin: crate::models::SessionOrigin,
    ) -> Result<()> {
        self.inner.mark_session_origin(agent, session_id, origin)
    }

    fn replace_by_scope(&self, scope: &str, agent: Agent, sessions: &[SessionInfo]) -> Result<()> {
        self.inner.replace_by_scope(scope, agent, sessions)?;
        let new_ids: HashSet<String> = sessions.iter().map(|s| s.id.clone()).collect();
        let mut snap = self.snapshot.write().unwrap();
        // Mirror inner's semantics: rows whose session_id isn't in the new set
        // get marked unavailable; rows that match are replaced wholesale.
        for ((rec_agent, sid, rec_scope), rec) in snap.by_pk.iter_mut() {
            if *rec_agent == agent
                && rec_scope == scope
                && !new_ids.contains(sid)
                && !is_virtual_session_ref(&rec.scope)
                && !is_virtual_session_ref(&rec.file_path)
            {
                rec.available = false;
            }
        }
        for s in sessions {
            let key = (agent, s.id.clone(), scope.to_string());
            let existing_subs = snap
                .by_pk
                .get(&key)
                .map(|r| r.subagents.clone())
                .unwrap_or_default();
            let mut rec = Self::to_indexed_session_only(scope, s);
            rec.subagents = existing_subs;
            snap.by_pk.insert(key, rec);
        }
        Ok(())
    }

    fn upsert_subagent(
        &self,
        parent_agent: Agent,
        parent_scope: &str,
        parent_session_id: &str,
        subagent: &SubagentInfo,
    ) -> Result<()> {
        self.inner
            .upsert_subagent(parent_agent, parent_scope, parent_session_id, subagent)?;
        let rec =
            Self::to_indexed_subagent(parent_agent, parent_scope, parent_session_id, subagent);
        let key = (
            parent_agent,
            parent_session_id.to_string(),
            parent_scope.to_string(),
        );
        let mut snap = self.snapshot.write().unwrap();
        if let Some(session) = snap.by_pk.get_mut(&key) {
            if let Some(existing) = session
                .subagents
                .iter_mut()
                .find(|s| s.subagent_id == rec.subagent_id)
            {
                *existing = rec;
            } else {
                session.subagents.push(rec);
            }
        }
        // If the parent isn't in the snapshot yet, the row is in the inner
        // store but invisible until the next list_indexed_sessions reload. We
        // accept that gap: the next ReindexClaudeProject (sessions-index.json
        // hint) will materialize the synthetic parent and rebuild the link.
        Ok(())
    }

    fn update_message_count(
        &self,
        agent: Agent,
        session_id: Option<&str>,
        file_path: &str,
        message_count: usize,
    ) -> Result<()> {
        self.inner
            .update_message_count(agent, session_id, file_path, message_count)
    }

    fn mark_file_path_unavailable(&self, file_path: &str) -> Result<()> {
        self.inner.mark_file_path_unavailable(file_path)?;
        if is_virtual_session_ref(file_path) {
            return Ok(());
        }
        let mut snap = self.snapshot.write().unwrap();
        for rec in snap.by_pk.values_mut() {
            if rec.file_path == file_path {
                rec.available = false;
            }
        }
        Ok(())
    }

    fn mark_subagent_file_unavailable(&self, file_path: &str) -> Result<()> {
        self.inner.mark_subagent_file_unavailable(file_path)?;
        let mut snap = self.snapshot.write().unwrap();
        for session in snap.by_pk.values_mut() {
            for sub in session.subagents.iter_mut() {
                if sub.file_path == file_path {
                    sub.available = false;
                }
            }
        }
        Ok(())
    }

    fn mark_file_path_unindexable(&self, agent: Agent, file_path: &str) -> Result<()> {
        self.inner.mark_file_path_unindexable(agent, file_path)?;
        self.refresh_from_inner()?;
        Ok(())
    }

    fn mark_missing_scopes_unavailable(
        &self,
        agent: Agent,
        present: &HashSet<String>,
    ) -> Result<()> {
        self.inner.mark_missing_scopes_unavailable(agent, present)?;
        let mut snap = self.snapshot.write().unwrap();
        for rec in snap.by_pk.values_mut() {
            if rec.agent == agent
                && !present.contains(&rec.scope)
                && !is_placeholder_indexed_session(rec)
                && !is_virtual_session_ref(&rec.scope)
                && !is_virtual_session_ref(&rec.file_path)
            {
                rec.available = false;
            }
        }
        Ok(())
    }

    fn get_or_create_canvas_document(
        &self,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<CanvasDocumentInfo> {
        self.inner.get_or_create_canvas_document(session_id, title)
    }

    fn get_canvas_document_state(&self, session_id: &str) -> Result<CanvasDocumentState> {
        self.inner.get_canvas_document_state(session_id)
    }

    fn save_canvas_draft(
        &self,
        session_id: &str,
        title: Option<&str>,
        draft_snapshot_path: &str,
        draft_snapshot_hash: &str,
    ) -> Result<CanvasDocumentInfo> {
        self.inner
            .save_canvas_draft(session_id, title, draft_snapshot_path, draft_snapshot_hash)
    }

    fn save_canvas_revision(
        &self,
        session_id: &str,
        title: Option<&str>,
        snapshot_path: &str,
        snapshot_hash: &str,
        snapshot_size_bytes: i64,
        source: &str,
    ) -> Result<(CanvasDocumentInfo, CanvasRevisionInfo)> {
        self.inner.save_canvas_revision(
            session_id,
            title,
            snapshot_path,
            snapshot_hash,
            snapshot_size_bytes,
            source,
        )
    }

    fn prune_canvas_revisions(&self, session_id: &str, keep_latest: usize) -> Result<Vec<String>> {
        self.inner.prune_canvas_revisions(session_id, keep_latest)
    }

    fn replace_canvas_blocks(
        &self,
        session_id: &str,
        blocks: &[UpsertCanvasBlockRecord],
    ) -> Result<Vec<CanvasBlockRecord>> {
        self.inner.replace_canvas_blocks(session_id, blocks)
    }

    fn create_canvas_context_anchor(
        &self,
        session_id: &str,
        anchor_block_id: Option<&str>,
        selection_block_ids_json: &str,
        selection_element_ids_json: &str,
        turn_id: &str,
        summary: Option<&str>,
    ) -> Result<CanvasContextAnchor> {
        self.inner.create_canvas_context_anchor(
            session_id,
            anchor_block_id,
            selection_block_ids_json,
            selection_element_ids_json,
            turn_id,
            summary,
        )
    }
}

#[allow(dead_code)]
impl CachedStore {
    // Exposed for tests / future maintenance commands that need to drop the
    // cache and rebuild it from the source of truth.
    pub fn rebuild_snapshot(&self) -> Result<()> {
        self.refresh_from_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::sqlite::SqliteStore;
    use std::path::PathBuf;

    fn unique_db(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}-{}.db", std::process::id(), now_ms()))
    }

    fn test_session(id: &str, file_path: &str, file_size: u64) -> SessionInfo {
        SessionInfo {
            id: id.to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 1,
            rename_title: Some(format!("Session {id}")),
            title: None,
            first_user_message: Some("hello".to_string()),
            file_path: file_path.to_string(),
            file_size,
            partial: file_size == 0,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        }
    }

    fn test_subagent(id: &str, file_path: &str) -> SubagentInfo {
        SubagentInfo {
            id: id.to_string(),
            agent_type: Some("research".to_string()),
            description: Some("Research helper".to_string()),
            started_at: Some(11),
            updated_at: Some(21),
            message_count: 2,
            first_user_message: Some("subtask".to_string()),
            file_path: file_path.to_string(),
            file_size: 17,
            partial: false,
            available: true,
        }
    }

    #[test]
    fn cached_store_replaces_placeholder_with_real_row() {
        let path = unique_db("sessio-cached-placeholder-real-row");
        let sqlite = Arc::new(SqliteStore::open(&path).unwrap());
        sqlite.init().unwrap();
        let store = CachedStore::new(sqlite).unwrap();

        let placeholder = test_session("session-1", "", 0);
        let real_path = "/tmp/project/session-1.jsonl";
        let mut real = test_session("session-1", real_path, 42);
        real.partial = false;

        store.upsert_session("", &placeholder).unwrap();
        store.upsert_session(real_path, &real).unwrap();

        let rows: Vec<_> = store
            .list_indexed_sessions()
            .unwrap()
            .into_iter()
            .filter(|row| row.agent == Agent::Codex && row.session_id == "session-1")
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].scope, real_path);
        assert_eq!(rows[0].file_path, real_path);
        assert_eq!(rows[0].file_size, 42);
        assert!(rows[0].available);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cached_store_replace_by_scope_preserves_subagents() {
        let path = unique_db("sessio-cached-replace-subagents");
        let sqlite = Arc::new(SqliteStore::open(&path).unwrap());
        sqlite.init().unwrap();
        let store = CachedStore::new(sqlite).unwrap();

        let scope = "/tmp/project/session-parent.jsonl";
        let session = test_session("parent", scope, 20);
        let subagent = test_subagent("subagent-1", "/tmp/project/subagent-1.jsonl");

        store.upsert_session(scope, &session).unwrap();
        store
            .upsert_subagent(Agent::Codex, scope, "parent", &subagent)
            .unwrap();

        let replacement = SessionInfo {
            file_size: 99,
            updated_at: Some(30),
            ..session.clone()
        };
        store
            .replace_by_scope(scope, Agent::Codex, &[replacement])
            .unwrap();

        let row = store
            .list_indexed_sessions()
            .unwrap()
            .into_iter()
            .find(|row| row.agent == Agent::Codex && row.session_id == "parent")
            .unwrap();
        assert_eq!(row.file_size, 99);
        assert_eq!(row.subagents.len(), 1);
        assert_eq!(row.subagents[0].subagent_id, "subagent-1");
        assert_eq!(row.subagents[0].file_path, "/tmp/project/subagent-1.jsonl");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cached_store_cleanup_partial_astra_sessions_refreshes_snapshot() {
        let path = unique_db("sessio-cached-astra-cleanup-refresh");
        let sqlite = Arc::new(SqliteStore::open(&path).unwrap());
        sqlite.init().unwrap();
        let store = CachedStore::new(sqlite).unwrap();

        let placeholder = test_session("astra-placeholder", "", 0);
        let real = test_session("astra-real", "/tmp/project/astra-real.jsonl", 42);
        store.upsert_session("", &placeholder).unwrap();
        store
            .upsert_session("/tmp/project/astra-real.jsonl", &real)
            .unwrap();

        let changed = store
            .cleanup_partial_astra_sessions(&[
                "astra-placeholder".to_string(),
                "astra-real".to_string(),
            ])
            .unwrap();
        assert_eq!(changed, 1);

        let rows = store.list_indexed_sessions().unwrap();
        let placeholder = rows
            .iter()
            .find(|row| row.session_id == "astra-placeholder")
            .unwrap();
        let real = rows
            .iter()
            .find(|row| row.session_id == "astra-real")
            .unwrap();
        assert!(!placeholder.available);
        assert!(placeholder.archived);
        assert!(real.available);
        assert!(!real.archived);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cached_store_preserves_virtual_sessions_when_scopes_disappear() {
        let path = unique_db("sessio-cached-astra-virtual-scope");
        let sqlite = Arc::new(SqliteStore::open(&path).unwrap());
        sqlite.init().unwrap();
        let store = CachedStore::new(sqlite).unwrap();
        let virtual_path = "astra://run-1/session/astra-child";
        let session = SessionInfo {
            id: "astra-child".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 1,
            rename_title: Some("Astra delegated task".to_string()),
            title: None,
            first_user_message: Some("# Sessio stage task".to_string()),
            file_path: virtual_path.to_string(),
            file_size: 0,
            partial: true,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };

        store.upsert_session(virtual_path, &session).unwrap();
        store
            .mark_missing_scopes_unavailable(Agent::Codex, &HashSet::new())
            .unwrap();
        store
            .replace_by_scope(virtual_path, Agent::Codex, &[])
            .unwrap();
        store.mark_file_path_unavailable(virtual_path).unwrap();

        let row = store
            .list_indexed_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.agent == Agent::Codex && session.session_id == "astra-child")
            .unwrap();
        assert!(row.available);
        assert_eq!(row.scope, virtual_path);
        assert_eq!(row.file_path, virtual_path);

        let _ = std::fs::remove_file(path);
    }
}
