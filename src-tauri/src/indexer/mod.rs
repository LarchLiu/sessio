use crossbeam_channel::{unbounded, Receiver, Sender};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use tauri::{AppHandle, Emitter};

use crate::models::Agent;
use crate::readers;
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
) -> IndexerHandle {
    let (tx, rx) = unbounded::<IndexTask>();
    let state = Arc::new(IndexerState {
        indexing: AtomicBool::new(false),
        last_error: Mutex::new(None),
    });
    let handle = IndexerHandle {
        tx: tx.clone(),
        state: state.clone(),
    };

    thread::spawn(move || {
        run_loop(app, store, rx, state);
    });

    handle
}

fn run_loop(
    app: AppHandle,
    store: Arc<dyn SessionStore>,
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
        let mut had_error = false;
        let mut last_error = None;
        for task in coalesced {
            if let Err(e) = execute(&task, store.as_ref()) {
                log::warn!("indexer task {:?} failed: {e}", task);
                had_error = true;
                last_error = Some(e.to_string());
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

fn execute(task: &IndexTask, store: &dyn SessionStore) -> Result<()> {
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
            Ok(())
        }
        IndexTask::DeleteSubagentFile(path) => {
            store.mark_subagent_file_unavailable(&path.to_string_lossy())?;
            Ok(())
        }
    }
}

pub fn full_rebuild(store: &dyn SessionStore) -> Result<()> {
    let (codex_live, codex_archived) = readers::codex::roots()?;
    let mut codex_scopes: HashSet<String> = HashSet::new();
    for (root, archived) in [(codex_live.as_path(), false), (codex_archived.as_path(), true)] {
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            match readers::codex::parse_one_file(path, archived) {
                Ok(Some(info)) => {
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
    if let Some(root) = readers::claude::root_dir()? {
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let dir = entry.path();
            let scope = dir.to_string_lossy().into_owned();
            match readers::claude::scan_project_dir(&dir) {
                Ok(sessions) => {
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
    let (gemini_tmp, _) = readers::gemini::paths()?;
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
            match readers::gemini::parse_logs_file(&logs) {
                Ok(sessions) => {
                    store.replace_by_scope(&scope, Agent::Gemini, &sessions)?;
                    gemini_scopes.insert(scope);
                }
                Err(e) => log::warn!("gemini parse {} failed: {e}", logs.display()),
            }
        }
    }
    store.mark_missing_scopes_unavailable(Agent::Gemini, &gemini_scopes)?;

    Ok(())
}


fn reindex_codex_file(path: &Path, store: &dyn SessionStore) -> Result<()> {
    if !path.exists() {
        store.mark_file_path_unavailable(&path.to_string_lossy())?;
        return Ok(());
    }
    let (_, archived_root) = readers::codex::roots()?;
    let archived = path.starts_with(&archived_root);
    match readers::codex::parse_one_file(path, archived)? {
        Some(info) => {
            let scope = info.file_path.clone();
            store.replace_by_scope(&scope, Agent::Codex, &[info])?;
        }
        None => {
            store.mark_file_path_unavailable(&path.to_string_lossy())?;
        }
    }
    Ok(())
}

fn reindex_claude_project(dir: &Path, store: &dyn SessionStore) -> Result<()> {
    if !dir.exists() {
        let scope = dir.to_string_lossy().into_owned();
        store.replace_by_scope(&scope, Agent::Claude, &[])?;
        return Ok(());
    }
    let sessions = readers::claude::scan_project_dir(dir)?;
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
    Ok(())
}

fn reindex_claude_file(path: &Path, store: &dyn SessionStore) -> Result<()> {
    if !path.exists() {
        store.mark_file_path_unavailable(&path.to_string_lossy())?;
        return Ok(());
    }
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    match readers::claude::parse_single_file(path)? {
        Some(info) => {
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
    Ok(())
}

fn reindex_claude_subagent_file(path: &Path, store: &dyn SessionStore) -> Result<()> {
    if !path.exists() {
        store.mark_subagent_file_unavailable(&path.to_string_lossy())?;
        return Ok(());
    }
    // Path layout is `<project>/<parent_session_id>/subagents/<id>.jsonl`,
    // so the project dir is `path.parent().parent().parent()`.
    let Some(project_dir) = path.parent().and_then(|p| p.parent()).and_then(|p| p.parent())
    else {
        return Ok(());
    };
    let scope = project_dir.to_string_lossy().into_owned();
    match readers::claude::parse_single_subagent_file(path)? {
        Some((parent_session_id, info)) => {
            store.upsert_subagent(Agent::Claude, &scope, &parent_session_id, &info)?;
        }
        None => {
            store.mark_subagent_file_unavailable(&path.to_string_lossy())?;
        }
    }
    Ok(())
}

fn reindex_gemini_logs(path: &Path, store: &dyn SessionStore) -> Result<()> {
    if !path.exists() {
        let scope = path.to_string_lossy().into_owned();
        store.replace_by_scope(&scope, Agent::Gemini, &[])?;
        return Ok(());
    }
    let sessions = readers::gemini::parse_logs_file(path)?;
    let scope = path.to_string_lossy().into_owned();
    store.replace_by_scope(&scope, Agent::Gemini, &sessions)?;
    Ok(())
}

fn refresh_gemini_mappings(store: &dyn SessionStore) -> Result<()> {
    let (tmp_dir, _) = readers::gemini::paths()?;
    if !tmp_dir.exists() {
        return Ok(());
    }
    let mut scopes: HashSet<String> = HashSet::new();
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
        match readers::gemini::parse_logs_file(&logs) {
            Ok(sessions) => {
                store.replace_by_scope(&scope, Agent::Gemini, &sessions)?;
                scopes.insert(scope);
            }
            Err(e) => log::warn!("gemini parse {} failed: {e}", logs.display()),
        }
    }
    store.mark_missing_scopes_unavailable(Agent::Gemini, &scopes)?;
    Ok(())
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
