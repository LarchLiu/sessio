use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use tauri::{AppHandle, Emitter};

use crate::agents::sources::shared::convert::session_source_from_info;
use crate::agents::sources::types::{
    PathEvent, PathEventKind, ProjectRef, SessionSource, SourceIndexTask, SourceKind,
};
use crate::memory::build::MemoryBuildOptions;
use crate::memory::service::{MemoryBackendSyncJob, MemoryService};
use crate::memory::MemoryStore;
use crate::models::Agent;
use crate::store::SessionStore;

#[derive(Debug, Clone)]
pub enum IndexTask {
    FullRebuild,
    ReindexCodexFile(PathBuf),
    ReindexClaudeFile(PathBuf),
    ReindexClaudeProject(PathBuf),
    ReindexClaudeSubagentFile(PathBuf),
    ReindexGeminiLogs(PathBuf),
    RefreshGeminiProjectMappings,
    DeleteFile(PathBuf),
    DeleteSubagentFile(PathBuf),
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexPhase {
    Idle,
    Indexing,
    Rebuilding,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub phase: IndexPhase,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct IndexerHandle {
    tx: Sender<IndexTask>,
    state: Arc<IndexerState>,
}

struct IndexerState {
    phase: Mutex<IndexPhase>,
    last_error: Mutex<Option<String>>,
}

impl IndexerHandle {
    pub fn submit(&self, task: IndexTask) -> Result<()> {
        self.tx
            .send(task)
            .map_err(|e| anyhow::anyhow!("indexer channel closed: {e}"))
    }

    pub fn status(&self) -> IndexStatus {
        IndexStatus {
            phase: self.state.phase.lock().unwrap().clone(),
            last_error: self.state.last_error.lock().unwrap().clone(),
        }
    }
}

pub fn spawn(
    app: AppHandle,
    store: Arc<dyn SessionStore>,
    memory_store: Arc<dyn MemoryStore>,
) -> IndexerHandle {
    let (tx, rx) = unbounded::<IndexTask>();
    let (backend_sync_tx, backend_sync_rx) = unbounded::<MemoryBackendSyncJob>();
    let state = Arc::new(IndexerState {
        phase: Mutex::new(IndexPhase::Idle),
        last_error: Mutex::new(None),
    });
    let handle = IndexerHandle {
        tx: tx.clone(),
        state: state.clone(),
    };

    // Build MemoryService once at startup. Cloning the Arc lets the backend
    // sync worker, the main indexer loop, and per-source builds all share the
    // same backend / artifact sink / source registry instead of paying
    // config-load + AgentSourceRegistry construction on every task.
    let service = match MemoryService::new(
        memory_store.clone(),
        Arc::new(crate::agents::sources::builtin_agent_sources()),
    ) {
        Ok(service) => Arc::new(service),
        Err(e) => {
            log::error!("indexer: failed to initialize MemoryService: {e}");
            // Without a service the memory pipeline is dead, but the index
            // pipeline can still service sessions list. Return the handle
            // anyway so the desktop app stays usable.
            return handle;
        }
    };

    let backend_sync_store = memory_store.clone();
    let backend_sync_service = service.clone();
    thread::spawn(move || {
        run_backend_sync_loop(backend_sync_store, backend_sync_service, backend_sync_rx);
    });

    let loop_tx = tx.clone();
    thread::spawn(move || {
        let ctx = IndexLoopContext {
            app,
            store,
            memory_store,
            service,
            backend_sync_tx,
            state,
        };
        run_loop(ctx, loop_tx, rx);
    });

    handle
}

struct IndexLoopContext {
    app: AppHandle,
    store: Arc<dyn SessionStore>,
    memory_store: Arc<dyn MemoryStore>,
    service: Arc<MemoryService>,
    backend_sync_tx: Sender<MemoryBackendSyncJob>,
    state: Arc<IndexerState>,
}

fn run_loop(ctx: IndexLoopContext, tx: Sender<IndexTask>, rx: Receiver<IndexTask>) {
    while let Ok(first) = rx.recv() {
        set_phase(&ctx.state, IndexPhase::Indexing);
        {
            let mut slot = ctx.state.last_error.lock().unwrap();
            *slot = None;
        }
        let _ = ctx
            .app
            .emit("sessions_index_status", current_status(&ctx.state));
        let mut batch = vec![first];
        thread::sleep(Duration::from_millis(50));
        while let Ok(t) = rx.try_recv() {
            batch.push(t);
        }
        let coalesced = coalesce(batch);
        let had_full_rebuild = coalesced
            .iter()
            .any(|t| matches!(t, IndexTask::FullRebuild));
        if had_full_rebuild {
            set_phase(&ctx.state, IndexPhase::Rebuilding);
            let _ = ctx
                .app
                .emit("sessions_index_status", current_status(&ctx.state));
        }
        log::info!("indexer: received tasks {:?}", coalesced);
        let mut had_error = false;
        let mut last_error = None;
        let mut affected_projects = HashMap::new();
        let mut affected_sources = HashMap::new();
        for task in coalesced {
            log::info!("indexer: executing task {:?}", task);
            match execute(&task, ctx.store.as_ref()) {
                Ok(outcome) => {
                    if !outcome.affected_projects.is_empty() || !outcome.affected_sources.is_empty()
                    {
                        log::info!(
                            "indexer: task {:?} affected {} projects and {} sources",
                            task,
                            outcome.affected_projects.len(),
                            outcome.affected_sources.len()
                        );
                    }
                    for project in outcome.affected_projects {
                        affected_projects.insert(project.project_key.clone(), project);
                    }
                    for source in outcome.affected_sources {
                        affected_sources.insert(
                            (
                                source.agent.as_str().to_string(),
                                source.session_id.clone(),
                                source.file_path.clone(),
                            ),
                            source,
                        );
                    }
                }
                Err(e) => {
                    log::warn!("indexer task {:?} failed: {e}", task);
                    had_error = true;
                    last_error = Some(e.to_string());
                }
            }
        }
        let mut backend_sync_jobs: HashMap<String, MemoryBackendSyncJob> = HashMap::new();
        let mut deferred_requeues: Vec<PathBuf> = Vec::new();
        for source in affected_sources.values() {
            match build_source_memory_for_indexer(
                source,
                ctx.memory_store.as_ref(),
                ctx.service.as_ref(),
            ) {
                Ok(Some(job)) => {
                    deferred_requeues.extend(job.dependent_source_paths.iter().cloned());
                    backend_sync_jobs.insert(job.project_key.clone(), job);
                    if let Some(project) = &source.project {
                        let _ = ctx.app.emit("memory_index_updated", project);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!(
                        "memory sync for source {}:{} failed: {e}",
                        source.agent.as_str(),
                        source.session_id
                    );
                }
            }
        }
        for project in affected_projects.values() {
            let already_covered = affected_sources.values().any(|source| {
                source
                    .project
                    .as_ref()
                    .map(|p| p.project_key == project.project_key)
                    .unwrap_or(false)
            });
            if already_covered {
                continue;
            }
            match build_project_memory_for_indexer(
                project,
                ctx.memory_store.as_ref(),
                ctx.service.as_ref(),
            ) {
                Ok(Some(job)) => {
                    deferred_requeues.extend(job.dependent_source_paths.iter().cloned());
                    backend_sync_jobs.insert(job.project_key.clone(), job);
                    let _ = ctx.app.emit("memory_index_updated", project);
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!(
                        "memory sync for project {} failed: {e}",
                        project.project_key
                    );
                }
            }
        }
        for job in backend_sync_jobs.into_values() {
            if let Err(e) = ctx.backend_sync_tx.send(job) {
                log::warn!("memory backend sync queue closed: {e}");
            }
        }
        if had_full_rebuild {
            // A FullRebuild already touched every file on disk and rebuilt
            // project-level memory. Per-file reindex tasks that watcher /
            // polling queued while we were busy would just re-run the heavy
            // memory + backend sync pipeline for files we already covered. Drop them
            // and re-queue real deletes (those carry information the rebuild
            // doesn't always reconstruct, e.g. subagent removals).
            let mut dropped = 0;
            let mut requeued = 0;
            while let Ok(task) = rx.try_recv() {
                match task {
                    IndexTask::DeleteFile(_) | IndexTask::DeleteSubagentFile(_) => {
                        if tx.send(task).is_ok() {
                            requeued += 1;
                        }
                    }
                    _ => dropped += 1,
                }
            }
            if dropped > 0 || requeued > 0 {
                log::info!(
                    "indexer: post-FullRebuild drain dropped={} requeued={}",
                    dropped,
                    requeued
                );
            }
        }
        if !deferred_requeues.is_empty() {
            let registry = crate::agents::sources::builtin_agent_sources();
            // Seed with paths already in this batch's affected_sources so we
            // don't re-queue a build we're about to do anyway. Both sides
            // canonicalize to PathBuf for set equality.
            let mut seen: HashSet<PathBuf> = affected_sources
                .values()
                .map(|source| PathBuf::from(&source.file_path))
                .collect();
            let mut requeued = 0usize;
            for path in deferred_requeues {
                if !seen.insert(path.clone()) {
                    continue;
                }
                let event = PathEvent {
                    path: path.clone(),
                    kind: PathEventKind::Modify,
                };
                let mut routed = false;
                for source_task in registry.classify_path_event(&event) {
                    let Some(task) = source_task_to_index_task(source_task) else {
                        continue;
                    };
                    if let Err(e) = tx.send(task) {
                        log::warn!(
                            "indexer: failed to requeue dependent source {}: {e}",
                            path.display()
                        );
                        continue;
                    }
                    requeued += 1;
                    routed = true;
                }
                if !routed {
                    log::warn!(
                        "indexer: dependent source {} not routed by any source",
                        path.display()
                    );
                }
            }
            if requeued > 0 {
                log::info!("indexer: requeued {} dependent source tasks", requeued);
            }
        }
        {
            let mut slot = ctx.state.last_error.lock().unwrap();
            *slot = last_error;
        }
        set_phase(&ctx.state, IndexPhase::Idle);
        let _ = ctx
            .app
            .emit("sessions_index_status", current_status(&ctx.state));
        if !had_error {
            let _ = ctx.app.emit("sessions_index_updated", ());
        }
    }
}

fn current_status(state: &IndexerState) -> IndexStatus {
    IndexStatus {
        phase: state.phase.lock().unwrap().clone(),
        last_error: state.last_error.lock().unwrap().clone(),
    }
}

fn set_phase(state: &IndexerState, phase: IndexPhase) {
    *state.phase.lock().unwrap() = phase;
}

fn coalesce(tasks: Vec<IndexTask>) -> Vec<IndexTask> {
    let mut seen_codex: HashSet<PathBuf> = HashSet::new();
    let mut seen_claude_file: HashSet<PathBuf> = HashSet::new();
    let mut seen_claude: HashSet<PathBuf> = HashSet::new();
    let mut seen_claude_subagent: HashSet<PathBuf> = HashSet::new();
    let mut seen_gemini: HashSet<PathBuf> = HashSet::new();
    let mut seen_delete: HashSet<PathBuf> = HashSet::new();
    let mut seen_delete_subagent: HashSet<PathBuf> = HashSet::new();
    let mut full = false;
    let mut refresh_gemini_mappings = false;
    let mut out = Vec::new();

    for t in tasks {
        match t {
            IndexTask::FullRebuild => full = true,
            IndexTask::ReindexCodexFile(p) => {
                if seen_codex.insert(p.clone()) {
                    out.push(IndexTask::ReindexCodexFile(p));
                }
            }
            IndexTask::ReindexClaudeFile(p) => {
                if seen_claude_file.insert(p.clone()) {
                    out.push(IndexTask::ReindexClaudeFile(p));
                }
            }
            IndexTask::ReindexClaudeProject(p) => {
                if seen_claude.insert(p.clone()) {
                    out.push(IndexTask::ReindexClaudeProject(p));
                }
            }
            IndexTask::ReindexClaudeSubagentFile(p) => {
                if seen_claude_subagent.insert(p.clone()) {
                    out.push(IndexTask::ReindexClaudeSubagentFile(p));
                }
            }
            IndexTask::ReindexGeminiLogs(p) => {
                if seen_gemini.insert(p.clone()) {
                    out.push(IndexTask::ReindexGeminiLogs(p));
                }
            }
            IndexTask::RefreshGeminiProjectMappings => refresh_gemini_mappings = true,
            IndexTask::DeleteFile(p) => {
                if seen_delete.insert(p.clone()) {
                    out.push(IndexTask::DeleteFile(p));
                }
            }
            IndexTask::DeleteSubagentFile(p) => {
                if seen_delete_subagent.insert(p.clone()) {
                    out.push(IndexTask::DeleteSubagentFile(p));
                }
            }
        }
    }

    if full {
        return vec![IndexTask::FullRebuild];
    }
    // A queued project rescan covers every main jsonl in that dir, so
    // per-file main reindexes for the same project are redundant. Subagent
    // tasks live on their own track and survive the filter.
    if !seen_claude.is_empty() {
        out.retain(|task| match task {
            IndexTask::ReindexClaudeFile(p) => p
                .parent()
                .map(|parent| !seen_claude.contains(parent))
                .unwrap_or(true),
            _ => true,
        });
    }
    if refresh_gemini_mappings {
        out.push(IndexTask::RefreshGeminiProjectMappings);
    }
    out
}

#[derive(Debug, Default)]
struct TaskOutcome {
    affected_projects: Vec<ProjectRef>,
    affected_sources: Vec<SessionSource>,
}

fn execute(task: &IndexTask, store: &dyn SessionStore) -> Result<TaskOutcome> {
    match task {
        IndexTask::FullRebuild => full_rebuild(store),
        IndexTask::ReindexCodexFile(path) => reindex_codex_file(path, store),
        IndexTask::ReindexClaudeFile(path) => reindex_claude_file(path, store),
        IndexTask::ReindexClaudeProject(dir) => reindex_claude_project(dir, store),
        IndexTask::ReindexClaudeSubagentFile(path) => reindex_claude_subagent_file(path, store),
        IndexTask::ReindexGeminiLogs(path) => reindex_gemini_logs(path, store),
        IndexTask::RefreshGeminiProjectMappings => refresh_gemini_mappings(store),
        IndexTask::DeleteFile(path) => {
            let path_str = path.to_string_lossy();
            store.mark_file_path_unavailable(&path_str)?;
            store.mark_subagent_file_unavailable(&path_str)?;
            Ok(TaskOutcome::default())
        }
        IndexTask::DeleteSubagentFile(path) => {
            store.mark_subagent_file_unavailable(&path.to_string_lossy())?;
            Ok(TaskOutcome::default())
        }
    }
}

fn full_rebuild(store: &dyn SessionStore) -> Result<TaskOutcome> {
    let mut affected_projects = HashMap::new();
    let mut codex_scopes: HashSet<String> = HashSet::new();
    match crate::agents::sources::codex::parser::list_sessions() {
        Ok(sessions) => {
            for info in sessions {
                insert_session_project(&mut affected_projects, &info);
                let scope = info.file_path.clone();
                store.replace_by_scope(&scope, Agent::Codex, std::slice::from_ref(&info))?;
                for sub in &info.subagents {
                    store.upsert_subagent(Agent::Codex, &scope, &info.id, sub)?;
                }
                codex_scopes.insert(scope);
            }
        }
        Err(e) => log::warn!("codex list sessions failed: {e}"),
    }
    store.mark_missing_scopes_unavailable(Agent::Codex, &codex_scopes)?;

    let mut claude_scopes: HashSet<String> = HashSet::new();
    if let Some(root) = crate::agents::sources::claude::parser::root_dir()? {
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let dir = entry.path();
            let scope = dir.to_string_lossy().into_owned();
            match crate::agents::sources::claude::parser::scan_project_dir(&dir) {
                Ok(sessions) => {
                    for session in &sessions {
                        insert_session_project(&mut affected_projects, session);
                    }
                    store.replace_by_scope(&scope, Agent::Claude, &sessions)?;
                    // Write subagent rows on their own path, mirroring
                    // reindex_claude_project.
                    for session in &sessions {
                        for sub in &session.subagents {
                            store.upsert_subagent(Agent::Claude, &scope, &session.id, sub)?;
                        }
                    }
                    claude_scopes.insert(scope);
                }
                Err(e) => log::warn!("claude scan {} failed: {e}", dir.display()),
            }
        }
    }
    store.mark_missing_scopes_unavailable(Agent::Claude, &claude_scopes)?;

    let mut gemini_scopes: HashSet<String> = HashSet::new();
    match crate::agents::sources::gemini::parser::list_sessions() {
        Ok(sessions) => {
            for session in &sessions {
                insert_session_project(&mut affected_projects, &session);
            }
            for (scope, group) in group_by(sessions, |session| session.file_path.clone()) {
                store.replace_by_scope(&scope, Agent::Gemini, &group)?;
                gemini_scopes.insert(scope);
            }
        }
        Err(e) => log::warn!("gemini list sessions failed: {e}"),
    }
    store.mark_missing_scopes_unavailable(Agent::Gemini, &gemini_scopes)?;

    Ok(TaskOutcome {
        affected_projects: affected_projects.into_values().collect(),
        affected_sources: Vec::new(),
    })
}

fn reindex_codex_file(path: &Path, store: &dyn SessionStore) -> Result<TaskOutcome> {
    if !path.exists() {
        let path_str = path.to_string_lossy();
        store.mark_file_path_unavailable(&path_str)?;
        store.mark_subagent_file_unavailable(&path_str)?;
        return Ok(TaskOutcome::default());
    }
    let (_, archived_root) = crate::agents::sources::codex::parser::roots()?;
    let archived = path.starts_with(&archived_root);
    let mut outcome = TaskOutcome::default();
    match crate::agents::sources::codex::parser::parse_one_file_with_relation(path, archived)? {
        Some(parsed) => {
            if let Some(parent_thread_id) = parsed.parent_thread_id.clone() {
                let path_str = path.to_string_lossy();
                store.mark_file_path_unavailable(&path_str)?;
                let subagent = parsed.into_subagent();
                if let Some((parent_file, _)) =
                    crate::agents::sources::codex::parser::find_session_file_by_id(
                        &parent_thread_id,
                    )?
                {
                    let scope = parent_file.to_string_lossy().into_owned();
                    store.upsert_subagent(Agent::Codex, &scope, &parent_thread_id, &subagent)?;
                } else {
                    log::warn!(
                        "codex subagent {} parent {} not found",
                        path.display(),
                        parent_thread_id
                    );
                }
            } else {
                let info = parsed.info;
                push_session_project(&mut outcome, &info);
                push_session_source(&mut outcome, &info);
                let scope = info.file_path.clone();
                store.replace_by_scope(&scope, Agent::Codex, std::slice::from_ref(&info))?;
                for sub in &info.subagents {
                    store.upsert_subagent(Agent::Codex, &scope, &info.id, sub)?;
                }
            }
        }
        None => {
            let path_str = path.to_string_lossy();
            store.mark_file_path_unavailable(&path_str)?;
            store.mark_subagent_file_unavailable(&path_str)?;
        }
    }
    Ok(outcome)
}

fn reindex_claude_project(dir: &Path, store: &dyn SessionStore) -> Result<TaskOutcome> {
    if !dir.exists() {
        let scope = dir.to_string_lossy().into_owned();
        store.replace_by_scope(&scope, Agent::Claude, &[])?;
        return Ok(TaskOutcome::default());
    }
    let sessions = crate::agents::sources::claude::parser::scan_project_dir(dir)?;
    let mut outcome = TaskOutcome::default();
    for session in &sessions {
        push_session_project(&mut outcome, session);
    }
    let scope = dir.to_string_lossy().into_owned();
    store.replace_by_scope(&scope, Agent::Claude, &sessions)?;
    // Subagent rows are written separately so a project rescan still keeps
    // their per-file metadata fresh on a full project sweep, even though
    // replace_by_scope no longer touches the subagents table.
    for session in &sessions {
        for sub in &session.subagents {
            store.upsert_subagent(Agent::Claude, &scope, &session.id, sub)?;
        }
    }
    Ok(outcome)
}

fn reindex_claude_file(path: &Path, store: &dyn SessionStore) -> Result<TaskOutcome> {
    if !path.exists() {
        store.mark_file_path_unavailable(&path.to_string_lossy())?;
        return Ok(TaskOutcome::default());
    }
    let Some(parent) = path.parent() else {
        return Ok(TaskOutcome::default());
    };
    let mut outcome = TaskOutcome::default();
    match crate::agents::sources::claude::parser::parse_single_file(path)? {
        Some(info) => {
            push_session_project(&mut outcome, &info);
            push_session_source(&mut outcome, &info);
            // Keep scope = project dir so this row stays consistent with rows
            // produced by full project scans (PK is agent+session_id+scope).
            // Subagents on the parsed SessionInfo are ignored — a main-jsonl
            // edit doesn't entitle us to rewrite subagent rows.
            let scope = parent.to_string_lossy().into_owned();
            store.upsert_session(&scope, &info)?;
        }
        None => {
            store.mark_file_path_unavailable(&path.to_string_lossy())?;
        }
    }
    Ok(outcome)
}

fn reindex_claude_subagent_file(path: &Path, store: &dyn SessionStore) -> Result<TaskOutcome> {
    if !path.exists() {
        store.mark_subagent_file_unavailable(&path.to_string_lossy())?;
        return Ok(TaskOutcome::default());
    }
    // Path layout is `<project>/<parent_session_id>/subagents/<id>.jsonl`,
    // so the project dir is `path.parent().parent().parent()`.
    let Some(project_dir) = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
    else {
        return Ok(TaskOutcome::default());
    };
    let scope = project_dir.to_string_lossy().into_owned();
    let mut outcome = TaskOutcome::default();
    match crate::agents::sources::claude::parser::parse_single_subagent_file(path)? {
        Some((parent_session_id, info)) => {
            push_project_path(&mut outcome, Some(scope.clone()), None);
            store.upsert_subagent(Agent::Claude, &scope, &parent_session_id, &info)?;
        }
        None => {
            store.mark_subagent_file_unavailable(&path.to_string_lossy())?;
        }
    }
    Ok(outcome)
}

fn reindex_gemini_logs(path: &Path, store: &dyn SessionStore) -> Result<TaskOutcome> {
    if !path.exists() {
        let scope = path.to_string_lossy().into_owned();
        store.replace_by_scope(&scope, Agent::Gemini, &[])?;
        return Ok(TaskOutcome::default());
    }
    let sessions = crate::agents::sources::gemini::parser::parse_logs_file(path)?;
    let mut outcome = TaskOutcome::default();
    for session in &sessions {
        push_session_project(&mut outcome, session);
        push_session_source(&mut outcome, session);
    }
    let scope = path.to_string_lossy().into_owned();
    store.replace_by_scope(&scope, Agent::Gemini, &sessions)?;
    Ok(outcome)
}

fn refresh_gemini_mappings(store: &dyn SessionStore) -> Result<TaskOutcome> {
    let mut scopes: HashSet<String> = HashSet::new();
    let mut outcome = TaskOutcome::default();
    match crate::agents::sources::gemini::parser::list_sessions() {
        Ok(sessions) => {
            for session in &sessions {
                push_session_project(&mut outcome, &session);
            }
            for (scope, group) in group_by(sessions, |session| session.file_path.clone()) {
                store.replace_by_scope(&scope, Agent::Gemini, &group)?;
                scopes.insert(scope);
            }
        }
        Err(e) => log::warn!("gemini list sessions failed: {e}"),
    }
    store.mark_missing_scopes_unavailable(Agent::Gemini, &scopes)?;
    Ok(outcome)
}

fn build_project_memory_for_indexer(
    project: &ProjectRef,
    store: &dyn MemoryStore,
    service: &MemoryService,
) -> Result<Option<MemoryBackendSyncJob>> {
    let Some(project_path) = project.project_path.as_deref() else {
        return Ok(None);
    };
    let backend = service.backend_name().to_string();
    let artifacts_root = service.backend_artifacts_root();
    let summary = match service.build_project(MemoryBuildOptions {
        project_path: PathBuf::from(project_path),
        artifacts_root: artifacts_root.clone(),
    }) {
        Ok(summary) => summary,
        Err(e) => {
            store.record_memory_job(
                &project.project_key,
                &backend,
                project_path,
                "project_build",
                "retryable_failed",
                Some(&e.to_string()),
            )?;
            return Err(e);
        }
    };
    let status = if summary.errors.is_empty() {
        "succeeded"
    } else {
        "completed_with_warnings"
    };
    let error = if summary.errors.is_empty() {
        None
    } else {
        Some(summary.errors.join("\n"))
    };
    store.record_memory_job(
        &project.project_key,
        &backend,
        project_path,
        "project_build",
        status,
        error.as_deref(),
    )?;

    let project_key = summary
        .project_key
        .unwrap_or_else(|| project.project_key.clone());
    Ok(Some(MemoryBackendSyncJob {
        backend,
        project_key,
        project_path: project_path.to_string(),
        dependent_source_paths: summary.dependent_source_paths,
    }))
}

fn build_source_memory_for_indexer(
    source: &SessionSource,
    store: &dyn MemoryStore,
    service: &MemoryService,
) -> Result<Option<MemoryBackendSyncJob>> {
    let Some(project) = &source.project else {
        return Ok(None);
    };
    let Some(project_path) = project.project_path.as_deref() else {
        return Ok(None);
    };
    let backend = service.backend_name().to_string();
    let artifacts_root = service.backend_artifacts_root();
    let result = match service.build_source(source, &artifacts_root) {
        Ok(result) => result,
        Err(e) => {
            store.record_memory_job(
                &project.project_key,
                &backend,
                &source.file_path,
                "source_build",
                "retryable_failed",
                Some(&e.to_string()),
            )?;
            return Err(e);
        }
    };
    store.record_memory_job(
        &project.project_key,
        &backend,
        &source.file_path,
        "source_build",
        "succeeded",
        None,
    )?;

    let project_key = result
        .project_key
        .unwrap_or_else(|| project.project_key.clone());
    Ok(Some(MemoryBackendSyncJob {
        backend,
        project_key,
        project_path: project_path.to_string(),
        dependent_source_paths: result.dependent_source_paths,
    }))
}

fn source_task_to_index_task(task: SourceIndexTask) -> Option<IndexTask> {
    match task {
        SourceIndexTask::ReindexSource(source) => {
            let path = PathBuf::from(&source.file_path);
            let mapped = match source.agent.as_str() {
                "codex" => Some(IndexTask::ReindexCodexFile(path)),
                "claude" => match source.source_kind {
                    SourceKind::Subagent => Some(IndexTask::ReindexClaudeSubagentFile(path)),
                    _ => Some(IndexTask::ReindexClaudeFile(path)),
                },
                "gemini" => Some(IndexTask::ReindexGeminiLogs(path)),
                _ => None,
            };
            if mapped.is_none() {
                log::warn!(
                    "indexer: no IndexTask mapping for ReindexSource agent={} file={}",
                    source.agent.as_str(),
                    source.file_path
                );
            }
            mapped
        }
        SourceIndexTask::ReindexScope { agent, scope } => {
            let path = PathBuf::from(&scope);
            let mapped = match agent.as_str() {
                "claude" => Some(IndexTask::ReindexClaudeProject(path)),
                "gemini" => Some(IndexTask::ReindexGeminiLogs(path)),
                _ => None,
            };
            if mapped.is_none() {
                log::warn!(
                    "indexer: no IndexTask mapping for ReindexScope agent={} scope={}",
                    agent.as_str(),
                    scope
                );
            }
            mapped
        }
        SourceIndexTask::MarkSourceUnavailable(source) => {
            let path = PathBuf::from(&source.file_path);
            match source.source_kind {
                SourceKind::Subagent => Some(IndexTask::DeleteSubagentFile(path)),
                _ => Some(IndexTask::DeleteFile(path)),
            }
        }
        SourceIndexTask::RefreshProjectMappings { agent } => {
            let mapped = match agent.as_str() {
                "gemini" => Some(IndexTask::RefreshGeminiProjectMappings),
                _ => None,
            };
            if mapped.is_none() {
                log::warn!(
                    "indexer: no IndexTask mapping for RefreshProjectMappings agent={}",
                    agent.as_str()
                );
            }
            mapped
        }
    }
}

fn run_backend_sync_loop(
    store: Arc<dyn MemoryStore>,
    service: Arc<MemoryService>,
    rx: Receiver<MemoryBackendSyncJob>,
) {
    while let Ok(first) = rx.recv() {
        let mut pending = HashMap::new();
        pending.insert(first.project_key.clone(), first);

        loop {
            match rx.recv_timeout(Duration::from_secs(3)) {
                Ok(job) => {
                    pending.insert(job.project_key.clone(), job);
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }

        for job in pending.values() {
            sync_memory_project(job, service.as_ref(), store.as_ref());
        }
    }
}

fn sync_memory_project(
    job: &MemoryBackendSyncJob,
    service: &MemoryService,
    store: &dyn MemoryStore,
) {
    match service.sync_backend_job(job) {
        Ok(_) => {
            if let Err(e) = store.record_memory_job(
                &job.project_key,
                &job.backend,
                &job.project_path,
                "backend_sync",
                "succeeded",
                None,
            ) {
                log::warn!(
                    "failed to record memory backend success for {}: {e}",
                    job.project_key
                );
            }
        }
        Err(e) => {
            if let Err(store_error) = store.record_memory_job(
                &job.project_key,
                &job.backend,
                &job.project_path,
                "backend_sync",
                "retryable_failed",
                Some(&e.to_string()),
            ) {
                log::warn!(
                    "failed to record memory backend failure for {}: {store_error}",
                    job.project_key
                );
            }
        }
    }
}

fn push_session_project(outcome: &mut TaskOutcome, session: &crate::models::SessionInfo) {
    push_project_path(
        outcome,
        session.project_path.clone(),
        session.project_name.clone(),
    );
}

fn push_session_source(outcome: &mut TaskOutcome, session: &crate::models::SessionInfo) {
    let source = session_source_from_info(session);
    if outcome.affected_sources.iter().any(|existing| {
        existing.agent == source.agent
            && existing.session_id == source.session_id
            && existing.file_path == source.file_path
    }) {
        return;
    }
    outcome.affected_sources.push(source);
}

fn push_project_path(
    outcome: &mut TaskOutcome,
    project_path: Option<String>,
    project_name: Option<String>,
) {
    let Some(project_path) = project_path else {
        return;
    };
    let project_key = crate::agents::sources::shared::convert::project_key_for_path_or_name(
        Some(&project_path),
        project_name.as_deref(),
    );
    if outcome
        .affected_projects
        .iter()
        .any(|project| project.project_key == project_key)
    {
        return;
    }
    outcome.affected_projects.push(ProjectRef {
        project_key,
        project_path: Some(project_path),
        project_name,
    });
}

fn insert_session_project(
    projects: &mut HashMap<String, ProjectRef>,
    session: &crate::models::SessionInfo,
) {
    let Some(project_path) = session.project_path.clone() else {
        return;
    };
    let project_key = crate::agents::sources::shared::convert::project_key_for_path_or_name(
        Some(&project_path),
        session.project_name.as_deref(),
    );
    projects.entry(project_key.clone()).or_insert(ProjectRef {
        project_key,
        project_path: Some(project_path),
        project_name: session.project_name.clone(),
    });
}

#[allow(dead_code)]
fn group_by<K: std::hash::Hash + Eq, V, F: Fn(&V) -> K>(
    items: Vec<V>,
    key_fn: F,
) -> HashMap<K, Vec<V>> {
    let mut out: HashMap<K, Vec<V>> = HashMap::new();
    for item in items {
        let k = key_fn(&item);
        out.entry(k).or_default().push(item);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::sources::types::{AgentKind, SessionSource, SourceIndexTask};

    fn src(agent: &str, file_path: &str, kind: SourceKind) -> SessionSource {
        SessionSource {
            agent: AgentKind::new(agent),
            session_id: "session-1".to_string(),
            scope: file_path.to_string(),
            file_path: file_path.to_string(),
            project: None,
            source_kind: kind,
            metadata: Default::default(),
        }
    }

    #[test]
    fn watcher_reindex_source_tasks_route_to_memory_rebuild_tasks() {
        let codex = source_task_to_index_task(SourceIndexTask::ReindexSource(src(
            "codex",
            "/tmp/codex/a.jsonl",
            SourceKind::MainSession,
        )));
        assert!(matches!(codex, Some(IndexTask::ReindexCodexFile(_))));

        let claude = source_task_to_index_task(SourceIndexTask::ReindexSource(src(
            "claude",
            "/tmp/claude/project/a.jsonl",
            SourceKind::MainSession,
        )));
        assert!(matches!(claude, Some(IndexTask::ReindexClaudeFile(_))));

        let gemini = source_task_to_index_task(SourceIndexTask::ReindexSource(src(
            "gemini",
            "/tmp/gemini/project/logs.json",
            SourceKind::Logs,
        )));
        assert!(matches!(gemini, Some(IndexTask::ReindexGeminiLogs(_))));
    }

    #[test]
    fn polling_scope_tasks_route_to_project_memory_rebuild_tasks() {
        let claude = source_task_to_index_task(SourceIndexTask::ReindexScope {
            agent: AgentKind::new("claude"),
            scope: "/tmp/claude/project".to_string(),
        });
        assert!(matches!(claude, Some(IndexTask::ReindexClaudeProject(_))));

        let gemini = source_task_to_index_task(SourceIndexTask::ReindexScope {
            agent: AgentKind::new("gemini"),
            scope: "/tmp/gemini/project/logs.json".to_string(),
        });
        assert!(matches!(gemini, Some(IndexTask::ReindexGeminiLogs(_))));
    }
}
