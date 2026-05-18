use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use tauri::{AppHandle, Emitter};

use crate::memory::build::{
    build_project_memory, build_source_memory, default_output_root, MemoryBuildOptions,
};
use crate::memory::qmd::{self, QmdOptions};
use crate::memory::MemoryStore;
use crate::models::Agent;
use crate::providers;
use crate::providers::types::ProjectRef;
use crate::providers::types::{AgentKind, SessionSource, SourceKind};
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
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub indexing: bool,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct IndexerHandle {
    tx: Sender<IndexTask>,
    state: Arc<IndexerState>,
}

struct IndexerState {
    indexing: AtomicBool,
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
            indexing: self.state.indexing.load(Ordering::SeqCst),
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
    let (qmd_tx, qmd_rx) = unbounded::<QmdSyncJob>();
    let state = Arc::new(IndexerState {
        indexing: AtomicBool::new(false),
        last_error: Mutex::new(None),
    });
    let handle = IndexerHandle {
        tx: tx.clone(),
        state: state.clone(),
    };

    let qmd_store = memory_store.clone();
    thread::spawn(move || {
        run_qmd_loop(qmd_store, qmd_rx);
    });

    let loop_tx = tx.clone();
    thread::spawn(move || {
        run_loop(app, store, memory_store, qmd_tx, loop_tx, rx, state);
    });

    handle
}

fn run_loop(
    app: AppHandle,
    store: Arc<dyn SessionStore>,
    memory_store: Arc<dyn MemoryStore>,
    qmd_tx: Sender<QmdSyncJob>,
    tx: Sender<IndexTask>,
    rx: Receiver<IndexTask>,
    state: Arc<IndexerState>,
) {
    while let Ok(first) = rx.recv() {
        state.indexing.store(true, Ordering::SeqCst);
        {
            let mut slot = state.last_error.lock().unwrap();
            *slot = None;
        }
        let _ = app.emit("sessions_index_status", current_status(&state));
        let mut batch = vec![first];
        thread::sleep(Duration::from_millis(50));
        while let Ok(t) = rx.try_recv() {
            batch.push(t);
        }
        let coalesced = coalesce(batch);
        let had_full_rebuild = coalesced
            .iter()
            .any(|t| matches!(t, IndexTask::FullRebuild));
        log::info!("indexer: received tasks {:?}", coalesced);
        let mut had_error = false;
        let mut last_error = None;
        let mut affected_projects = HashMap::new();
        let mut affected_sources = HashMap::new();
        for task in coalesced {
            log::info!("indexer: executing task {:?}", task);
            match execute(&task, store.as_ref()) {
                Ok(outcome) => {
                    if !outcome.affected_projects.is_empty() || !outcome.affected_sources.is_empty() {
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
        let mut qmd_jobs = HashMap::new();
        for source in affected_sources.values() {
            match build_source_memory_for_indexer(source, memory_store.as_ref()) {
                Ok(Some(job)) => {
                    qmd_jobs.insert(job.project_key.clone(), job);
                    if let Some(project) = &source.project {
                        let _ = app.emit("memory_index_updated", project);
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
            match build_project_memory_for_indexer(project, memory_store.as_ref()) {
                Ok(Some(job)) => {
                    qmd_jobs.insert(job.project_key.clone(), job);
                    let _ = app.emit("memory_index_updated", project);
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
        for job in qmd_jobs.into_values() {
            if let Err(e) = qmd_tx.send(job) {
                log::warn!("qmd sync queue closed: {e}");
            }
        }
        if had_full_rebuild {
            // A FullRebuild already touched every file on disk and rebuilt
            // project-level memory. Per-file reindex tasks that watcher /
            // polling queued while we were busy would just re-run the heavy
            // memory + QMD pipeline for files we already covered. Drop them
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
        {
            let mut slot = state.last_error.lock().unwrap();
            *slot = last_error;
        }
        state.indexing.store(false, Ordering::SeqCst);
        let _ = app.emit("sessions_index_status", current_status(&state));
        if !had_error {
            let _ = app.emit("sessions_index_updated", ());
        }
    }
}

fn current_status(state: &IndexerState) -> IndexStatus {
    IndexStatus {
        indexing: state.indexing.load(Ordering::SeqCst),
        last_error: state.last_error.lock().unwrap().clone(),
    }
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

#[derive(Debug, Clone)]
struct QmdSyncJob {
    project_key: String,
    project_path: String,
    cards_root: PathBuf,
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
            store.mark_file_path_unavailable(&path.to_string_lossy())?;
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
    let (codex_live, codex_archived) = providers::codex::parser::roots()?;
    let mut codex_scopes: HashSet<String> = HashSet::new();
    for (root, archived) in [
        (codex_live.as_path(), false),
        (codex_archived.as_path(), true),
    ] {
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            match providers::codex::parser::parse_one_file(path, archived) {
                Ok(Some(info)) => {
                    insert_session_project(&mut affected_projects, &info);
                    let scope = info.file_path.clone();
                    store.replace_by_scope(&scope, Agent::Codex, &[info])?;
                    codex_scopes.insert(scope);
                }
                Ok(None) => {}
                Err(e) => log::warn!("codex parse {} failed: {e}", path.display()),
            }
        }
    }
    store.mark_missing_scopes_unavailable(Agent::Codex, &codex_scopes)?;

    let mut claude_scopes: HashSet<String> = HashSet::new();
    if let Some(root) = providers::claude::parser::root_dir()? {
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let dir = entry.path();
            let scope = dir.to_string_lossy().into_owned();
            match providers::claude::parser::scan_project_dir(&dir) {
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
    let (gemini_tmp, _) = providers::gemini::parser::paths()?;
    if gemini_tmp.exists() {
        for entry in std::fs::read_dir(&gemini_tmp)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let logs = entry.path().join("logs.json");
            if !logs.exists() {
                continue;
            }
            let scope = logs.to_string_lossy().into_owned();
            match providers::gemini::parser::parse_logs_file(&logs) {
                Ok(sessions) => {
                    for session in &sessions {
                        insert_session_project(&mut affected_projects, session);
                    }
                    store.replace_by_scope(&scope, Agent::Gemini, &sessions)?;
                    gemini_scopes.insert(scope);
                }
                Err(e) => log::warn!("gemini parse {} failed: {e}", logs.display()),
            }
        }
    }
    store.mark_missing_scopes_unavailable(Agent::Gemini, &gemini_scopes)?;

    Ok(TaskOutcome {
        affected_projects: affected_projects.into_values().collect(),
        affected_sources: Vec::new(),
    })
}

fn reindex_codex_file(path: &Path, store: &dyn SessionStore) -> Result<TaskOutcome> {
    if !path.exists() {
        store.mark_file_path_unavailable(&path.to_string_lossy())?;
        return Ok(TaskOutcome::default());
    }
    let (_, archived_root) = providers::codex::parser::roots()?;
    let archived = path.starts_with(&archived_root);
    let mut outcome = TaskOutcome::default();
    match providers::codex::parser::parse_one_file(path, archived)? {
        Some(info) => {
            push_session_project(&mut outcome, &info);
            push_session_source(&mut outcome, &info);
            let scope = info.file_path.clone();
            store.replace_by_scope(&scope, Agent::Codex, &[info])?;
        }
        None => {
            store.mark_file_path_unavailable(&path.to_string_lossy())?;
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
    let sessions = providers::claude::parser::scan_project_dir(dir)?;
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
    match providers::claude::parser::parse_single_file(path)? {
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
    match providers::claude::parser::parse_single_subagent_file(path)? {
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
    let sessions = providers::gemini::parser::parse_logs_file(path)?;
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
    let (tmp_dir, _) = providers::gemini::parser::paths()?;
    if !tmp_dir.exists() {
        return Ok(TaskOutcome::default());
    }
    let mut scopes: HashSet<String> = HashSet::new();
    let mut outcome = TaskOutcome::default();
    for entry in std::fs::read_dir(&tmp_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let logs = entry.path().join("logs.json");
        if !logs.exists() {
            continue;
        }
        let scope = logs.to_string_lossy().into_owned();
        match providers::gemini::parser::parse_logs_file(&logs) {
            Ok(sessions) => {
                for session in &sessions {
                    push_session_project(&mut outcome, session);
                }
                store.replace_by_scope(&scope, Agent::Gemini, &sessions)?;
                scopes.insert(scope);
            }
            Err(e) => log::warn!("gemini parse {} failed: {e}", logs.display()),
        }
    }
    store.mark_missing_scopes_unavailable(Agent::Gemini, &scopes)?;
    Ok(outcome)
}

fn build_project_memory_for_indexer(
    project: &ProjectRef,
    store: &dyn MemoryStore,
) -> Result<Option<QmdSyncJob>> {
    let Some(project_path) = project.project_path.as_deref() else {
        return Ok(None);
    };
    let output_root = default_output_root()?;
    let registry = providers::builtin_providers();
    let summary = match build_project_memory(
        &registry,
        store,
        &MemoryBuildOptions {
            project_path: PathBuf::from(project_path),
            output_root: output_root.clone(),
        },
    ) {
        Ok(summary) => summary,
        Err(e) => {
            store.record_memory_job(
                &project.project_key,
                project_path,
                "memory_build",
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
        project_path,
        "memory_build",
        status,
        error.as_deref(),
    )?;

    let project_key = summary
        .project_key
        .unwrap_or_else(|| project.project_key.clone());
    let cards_root = output_root.join(&project_key);
    Ok(Some(QmdSyncJob {
        project_key,
        project_path: project_path.to_string(),
        cards_root,
    }))
}

fn build_source_memory_for_indexer(
    source: &SessionSource,
    store: &dyn MemoryStore,
) -> Result<Option<QmdSyncJob>> {
    let Some(project) = &source.project else {
        return Ok(None);
    };
    let Some(project_path) = project.project_path.as_deref() else {
        return Ok(None);
    };
    let output_root = default_output_root()?;
    let registry = providers::builtin_providers();
    let result = match build_source_memory(&registry, store, &output_root, source) {
        Ok(result) => result,
        Err(e) => {
            store.record_memory_job(
                &project.project_key,
                &source.file_path,
                "memory_build",
                "retryable_failed",
                Some(&e.to_string()),
            )?;
            return Err(e);
        }
    };
    store.record_memory_job(
        &project.project_key,
        &source.file_path,
        "memory_build",
        "succeeded",
        None,
    )?;

    let project_key = result
        .project_key
        .unwrap_or_else(|| project.project_key.clone());
    let cards_root = output_root.join(&project_key);
    Ok(Some(QmdSyncJob {
        project_key,
        project_path: project_path.to_string(),
        cards_root,
    }))
}

fn run_qmd_loop(store: Arc<dyn MemoryStore>, rx: Receiver<QmdSyncJob>) {
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
            sync_qmd_project(job, store.as_ref());
        }
        if auto_embed_enabled() {
            sync_qmd_embed(pending.values(), store.as_ref());
        }
    }
}

fn sync_qmd_project(job: &QmdSyncJob, store: &dyn MemoryStore) {
    let qmd_options = QmdOptions::default();
    match qmd::ensure_project_collection(&qmd_options, &job.project_key, &job.cards_root)
        .and_then(|_| qmd::update_index(&qmd_options))
    {
        Ok(_) => {
            if let Err(e) = store.record_memory_job(
                &job.project_key,
                &job.project_path,
                "qmd_update",
                "succeeded",
                None,
            ) {
                log::warn!("failed to record qmd success for {}: {e}", job.project_key);
            }
        }
        Err(e) => {
            if let Err(store_error) = store.record_memory_job(
                &job.project_key,
                &job.project_path,
                "qmd_update",
                "retryable_failed",
                Some(&e.to_string()),
            ) {
                log::warn!(
                    "failed to record qmd failure for {}: {store_error}",
                    job.project_key
                );
            }
        }
    }
}

fn sync_qmd_embed<'a>(jobs: impl Iterator<Item = &'a QmdSyncJob> + Clone, store: &dyn MemoryStore) {
    let qmd_options = QmdOptions::default();
    match qmd::embed_index(&qmd_options) {
        Ok(_) => {
            for job in jobs {
                if let Err(e) = store.record_memory_job(
                    &job.project_key,
                    &job.project_path,
                    "qmd_embed",
                    "succeeded",
                    None,
                ) {
                    log::warn!(
                        "failed to record qmd embed success for {}: {e}",
                        job.project_key
                    );
                }
            }
        }
        Err(e) => {
            let error = e.to_string();
            for job in jobs {
                if let Err(store_error) = store.record_memory_job(
                    &job.project_key,
                    &job.project_path,
                    "qmd_embed",
                    "retryable_failed",
                    Some(&error),
                ) {
                    log::warn!(
                        "failed to record qmd embed failure for {}: {store_error}",
                        job.project_key
                    );
                }
            }
        }
    }
}

fn auto_embed_enabled() -> bool {
    matches!(
        env::var("SESSIO_QMD_AUTO_EMBED").ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

fn push_session_project(outcome: &mut TaskOutcome, session: &crate::models::SessionInfo) {
    push_project_path(
        outcome,
        session.project_path.clone(),
        session.project_name.clone(),
    );
}

fn push_session_source(outcome: &mut TaskOutcome, session: &crate::models::SessionInfo) {
    let source = SessionSource {
        agent: AgentKind::new(session.agent.as_str()),
        session_id: session.id.clone(),
        scope: session.file_path.clone(),
        file_path: session.file_path.clone(),
        project: session
            .project_path
            .as_ref()
            .map(|project_path| ProjectRef {
                project_key: providers::shared::convert::project_key_for_path_or_name(
                    Some(project_path),
                    session.project_name.as_deref(),
                ),
                project_path: Some(project_path.clone()),
                project_name: session.project_name.clone(),
            }),
        source_kind: if session.archived {
            SourceKind::Archive
        } else {
            SourceKind::MainSession
        },
        metadata: Default::default(),
    };
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
    let project_key = providers::shared::convert::project_key_for_path_or_name(
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
    let project_key = providers::shared::convert::project_key_for_path_or_name(
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
