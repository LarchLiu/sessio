use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::models::{
    Agent, AgentAiProviderInfo, AgentInfo, AssistantAgentInfo, AssistantInfo, AssistantType,
    IssueSeverity, IssueStatus, KanbanItem, KanbanStatus, ProjectInfo, ProjectStageInfo,
    RuntimeAgentOptionMetadata, SessionInfo, StageInfo, StageIssueInfo, StageStatus, SubagentInfo,
    ThreadInfo, WorkflowInfo,
};
use crate::store::{
    AstraRunRecord, IndexedSessionRecord, IndexedSubagentRecord, RuntimeAgentCapabilityRecord,
    RuntimeAgentSelection, SessionHistoryRecord, SessionHistorySnapshotRecord, SessionStore,
    ThreadWorkSnapshotRecord,
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
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn file_mtime_for(file_path: &str) -> Option<i64> {
    if file_path.is_empty() {
        return None;
    }
    std::fs::metadata(file_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis() as i64)
        })
}

fn is_virtual_session_ref(value: &str) -> bool {
    value.trim_start().starts_with("astra://")
}

fn is_placeholder_session(record: &IndexedSessionRecord) -> bool {
    record.file_size == 0 && record.available
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

    fn list_workflows(&self) -> Result<Vec<WorkflowInfo>> {
        self.inner.list_workflows()
    }

    fn create_workflow(&self, name: &str, description: Option<&str>) -> Result<WorkflowInfo> {
        self.inner.create_workflow(name, description)
    }

    fn update_workflow(
        &self,
        workflow_id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
    ) -> Result<WorkflowInfo> {
        self.inner.update_workflow(workflow_id, name, description)
    }

    fn delete_workflow(&self, workflow_id: &str) -> Result<()> {
        self.inner.delete_workflow(workflow_id)
    }

    fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        self.inner.list_projects()
    }

    fn add_project(
        &self,
        path: &str,
        name: Option<&str>,
        workflow_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo> {
        self.inner
            .add_project(path, name, workflow_id, enabled_stage_ids)
    }

    fn create_project(
        &self,
        parent_path: &str,
        name: &str,
        workflow_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo> {
        self.inner
            .create_project(parent_path, name, workflow_id, enabled_stage_ids)
    }

    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        workflow_id: Option<String>,
    ) -> Result<ProjectInfo> {
        self.inner.update_project(project_id, name, workflow_id)
    }

    fn archive_project(&self, project_id: &str) -> Result<()> {
        self.inner.archive_project(project_id)
    }

    fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        self.inner.list_agents()
    }

    fn update_agent_preferences_by_id(
        &self,
        agent_id: &str,
        display_name: Option<&str>,
        enabled: Option<bool>,
        order: Option<i64>,
        ai_provider: Option<&str>,
        ai_providers: Option<&[AgentAiProviderInfo]>,
        model: Option<&str>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
        models: Option<&[RuntimeAgentOptionMetadata]>,
        efforts: Option<&[RuntimeAgentOptionMetadata]>,
        permission_modes: Option<&[RuntimeAgentOptionMetadata]>,
    ) -> Result<AgentInfo> {
        self.inner.update_agent_preferences_by_id(
            agent_id,
            display_name,
            enabled,
            order,
            ai_provider,
            ai_providers,
            model,
            effort,
            permission_mode,
            models,
            efforts,
            permission_modes,
        )
    }

    fn update_builtin_agent_preferences(
        &self,
        agent: Agent,
        display_name: Option<&str>,
        enabled: Option<bool>,
        order: Option<i64>,
        model: Option<&str>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
        models: Option<&[RuntimeAgentOptionMetadata]>,
        efforts: Option<&[RuntimeAgentOptionMetadata]>,
        permission_modes: Option<&[RuntimeAgentOptionMetadata]>,
    ) -> Result<AgentInfo> {
        self.inner.update_builtin_agent_preferences(
            agent,
            display_name,
            enabled,
            order,
            model,
            effort,
            permission_mode,
            models,
            efforts,
            permission_modes,
        )
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

    fn create_assistant(
        &self,
        name: &str,
        agent: AssistantAgentInfo,
        system_prompt: Option<&str>,
        color: Option<&str>,
        assistant_type: AssistantType,
        workflow_id: Option<String>,
        project_id: Option<&str>,
    ) -> Result<AssistantInfo> {
        self.inner.create_assistant(
            name,
            agent,
            system_prompt,
            color,
            assistant_type,
            workflow_id,
            project_id,
        )
    }

    fn update_assistant(
        &self,
        assistant_id: &str,
        name: Option<&str>,
        agent: Option<AssistantAgentInfo>,
        system_prompt: Option<Option<&str>>,
        color: Option<Option<&str>>,
        enabled: Option<bool>,
    ) -> Result<AssistantInfo> {
        self.inner
            .update_assistant(assistant_id, name, agent, system_prompt, color, enabled)
    }

    fn delete_assistant(&self, assistant_id: &str) -> Result<()> {
        self.inner.delete_assistant(assistant_id)
    }

    fn list_threads(&self, project_id: &str) -> Result<Vec<ThreadInfo>> {
        self.inner.list_threads(project_id)
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

    fn delete_thread(&self, thread_id: &str) -> Result<()> {
        self.inner.delete_thread(thread_id)
    }

    fn list_project_stages(&self, project_id: &str) -> Result<Vec<ProjectStageInfo>> {
        self.inner.list_project_stages(project_id)
    }

    fn list_workflow_stages(&self, workflow_id: &str) -> Result<Vec<ProjectStageInfo>> {
        self.inner.list_workflow_stages(workflow_id)
    }

    fn create_project_stage(
        &self,
        project_id: &str,
        workflow_id: Option<String>,
        name: &str,
        description: Option<&str>,
        icon: Option<&str>,
    ) -> Result<ProjectStageInfo> {
        self.inner
            .create_project_stage(project_id, workflow_id, name, description, icon)
    }

    fn update_project_stage(
        &self,
        stage_id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
        icon: Option<Option<&str>>,
        order: Option<i64>,
        enabled: Option<bool>,
        allow_empty_assistants: Option<bool>,
    ) -> Result<ProjectStageInfo> {
        self.inner.update_project_stage(
            stage_id,
            name,
            description,
            icon,
            order,
            enabled,
            allow_empty_assistants,
        )
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

    fn get_session_history(
        &self,
        agent: Agent,
        session_id: &str,
        file_path: &str,
    ) -> Result<Option<SessionHistoryRecord>> {
        self.inner.get_session_history(agent, session_id, file_path)
    }

    fn replace_session_history(&self, record: &SessionHistoryRecord) -> Result<()> {
        self.inner.replace_session_history(record)
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

    fn interrupt_active_astra_runs(&self) -> Result<()> {
        self.inner.interrupt_active_astra_runs()
    }

    fn upsert_session(&self, scope: &str, session: &SessionInfo) -> Result<()> {
        self.inner.upsert_session(scope, session)?;
        let new_rec = Self::to_indexed_session_only(scope, session);
        let key = (
            new_rec.agent,
            new_rec.session_id.clone(),
            new_rec.scope.clone(),
        );
        let mut snap = self.snapshot.write().unwrap();
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
                && !is_placeholder_session(rec)
                && !is_virtual_session_ref(&rec.scope)
                && !is_virtual_session_ref(&rec.file_path)
            {
                rec.available = false;
            }
        }
        Ok(())
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

#[allow(dead_code)]
impl CachedStore {
    // Exposed for tests / future maintenance commands that need to drop the
    // cache and rebuild it from the source of truth.
    pub fn rebuild_snapshot(&self) -> Result<()> {
        self.refresh_from_inner()
    }
}
