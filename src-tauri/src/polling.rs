use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::indexer::{IndexTask, IndexerHandle};
use crate::models::Agent;
use crate::readers;
use crate::store::{IndexedSessionRecord, SessionStore};

pub fn spawn_polling(store: Arc<dyn SessionStore>, indexer: IndexerHandle) {
    thread::spawn(move || {
        let mut claude_index_mtimes: HashMap<PathBuf, Option<i64>> = HashMap::new();
        let mut first_tick = true;
        loop {
            if !first_tick {
                thread::sleep(Duration::from_secs(10));
            }
            first_tick = false;
            if let Err(e) = poll_once(store.clone(), &indexer, &mut claude_index_mtimes) {
                log::warn!("polling check failed: {e}");
            }
        }
    });
}

fn poll_once(
    store: Arc<dyn SessionStore>,
    indexer: &IndexerHandle,
    claude_index_mtimes: &mut HashMap<PathBuf, Option<i64>>,
) -> Result<()> {
    let indexed = store.list_indexed_sessions()?;
    poll_codex(&indexed, indexer)?;
    poll_claude(&indexed, claude_index_mtimes, store.as_ref(), indexer)?;
    poll_gemini(&indexed, store.as_ref(), indexer)?;
    Ok(())
}

fn poll_codex(indexed: &[IndexedSessionRecord], indexer: &IndexerHandle) -> Result<()> {
    let (live_root, archived_root) = readers::codex::roots()?;
    let mut known: HashMap<String, &IndexedSessionRecord> = indexed
        .iter()
        .filter(|s| s.agent == Agent::Codex && !s.file_path.is_empty())
        .map(|s| (s.file_path.clone(), s))
        .collect();

    for root in [&live_root, &archived_root] {
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
            let path_str = path.to_string_lossy().into_owned();
            let archived = readers::codex::path_is_archived(path, &archived_root);
            let needs_reindex = match known.remove(&path_str) {
                // File reappeared after a previous soft-delete, or it changed,
                // or it switched between live/archived: re-parse it.
                Some(row) => {
                    !row.available
                        || file_changed(path, row.file_size, row.file_mtime)
                        || row.archived != archived
                }
                None => true,
            };
            if needs_reindex {
                indexer.submit(IndexTask::ReindexCodexFile(path.to_path_buf()))?;
            }
        }
    }

    // Whatever remains in `known` is indexed but absent on disk. Skip rows
    // already marked unavailable to avoid pointless writes through the cache.
    for stale in known.into_values() {
        if stale.available {
            indexer.submit(IndexTask::DeleteFile(PathBuf::from(&stale.file_path)))?;
        }
    }

    Ok(())
}

fn poll_claude(
    indexed: &[IndexedSessionRecord],
    claude_index_mtimes: &mut HashMap<PathBuf, Option<i64>>,
    store: &dyn SessionStore,
    indexer: &IndexerHandle,
) -> Result<()> {
    let Some(root) = readers::claude::root_dir()? else {
        // Root not present (yet). Mark every Claude row unavailable but keep
        // history — when the root reappears the next tick will reindex and
        // flip rows back to available.
        store.mark_missing_scopes_unavailable(Agent::Claude, &HashSet::new())?;
        return Ok(());
    };

    let mut by_scope: HashMap<String, Vec<&IndexedSessionRecord>> = HashMap::new();
    for row in indexed.iter().filter(|s| s.agent == Agent::Claude) {
        by_scope.entry(row.scope.clone()).or_default().push(row);
    }

    let mut current_index_mtimes: HashMap<PathBuf, Option<i64>> = HashMap::new();
    let mut seen_scopes: HashSet<String> = HashSet::new();

    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path();
        let scope = dir.to_string_lossy().into_owned();
        seen_scopes.insert(scope.clone());
        let sessions_index = dir.join("sessions-index.json");
        let index_mtime = file_mtime(&sessions_index);
        current_index_mtimes.insert(sessions_index.clone(), index_mtime);

        let rows = by_scope.remove(&scope).unwrap_or_default();
        let index_changed = claude_index_mtimes.get(&sessions_index) != Some(&index_mtime);
        let needs_reindex = index_changed || claude_project_needs_reindex(&dir, &rows)?;
        if needs_reindex {
            indexer.submit(IndexTask::ReindexClaudeProject(dir))?;
        }
    }

    *claude_index_mtimes = current_index_mtimes;
    store.mark_missing_scopes_unavailable(Agent::Claude, &seen_scopes)?;

    Ok(())
}

fn poll_gemini(
    indexed: &[IndexedSessionRecord],
    store: &dyn SessionStore,
    indexer: &IndexerHandle,
) -> Result<()> {
    let (tmp_dir, projects_json) = readers::gemini::paths()?;
    let mut by_scope: HashMap<String, Vec<&IndexedSessionRecord>> = HashMap::new();
    for row in indexed.iter().filter(|s| s.agent == Agent::Gemini) {
        by_scope.entry(row.scope.clone()).or_default().push(row);
    }

    let projects_mtime = file_mtime(&projects_json);
    let projects_changed = projects_json_changed(projects_mtime, indexed);
    if projects_changed {
        indexer.submit(IndexTask::RefreshGeminiProjectMappings)?;
    }

    if !tmp_dir.exists() {
        store.mark_missing_scopes_unavailable(Agent::Gemini, &HashSet::new())?;
        return Ok(());
    }

    let mut present_scopes: HashSet<String> = HashSet::new();

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
        present_scopes.insert(scope.clone());
        let rows = by_scope.remove(&scope).unwrap_or_default();
        let needs_reindex = rows.is_empty()
            || rows.iter().any(|row| {
                !row.available || file_changed(&logs, row.file_size, row.file_mtime)
            });
        if needs_reindex {
            indexer.submit(IndexTask::ReindexGeminiLogs(logs))?;
        }
    }

    store.mark_missing_scopes_unavailable(Agent::Gemini, &present_scopes)?;

    Ok(())
}

fn claude_project_needs_reindex(
    project_dir: &Path,
    rows: &[&IndexedSessionRecord],
) -> Result<bool> {
    if rows.is_empty() {
        return Ok(true);
    }
    let mut indexed_files: HashMap<String, (u64, Option<i64>)> = HashMap::new();
    for row in rows {
        if !row.file_path.is_empty() {
            indexed_files.insert(row.file_path.clone(), (row.file_size, row.file_mtime));
        }
        for sub in &row.subagents {
            indexed_files.insert(sub.file_path.clone(), (sub.file_size, sub.file_mtime));
        }
    }

    let mut disk_files: HashMap<String, (u64, Option<i64>)> = HashMap::new();
    for entry in std::fs::read_dir(project_dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            // Can't stat? Treat as changed and let the indexer surface any
            // real error on its own (parse, etc).
            let Some(meta) = current_file_meta(&path) else {
                return Ok(true);
            };
            disk_files.insert(path.to_string_lossy().into_owned(), meta);
            continue;
        }
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let subagents_dir = path.join("subagents");
        if !subagents_dir.is_dir() {
            continue;
        }
        for sub in std::fs::read_dir(subagents_dir)? {
            let sub = sub?;
            let sub_path = sub.path();
            if sub.file_type()?.is_file() && sub_path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                let Some(meta) = current_file_meta(&sub_path) else {
                    return Ok(true);
                };
                disk_files.insert(sub_path.to_string_lossy().into_owned(), meta);
            }
        }
    }

    Ok(disk_files != indexed_files)
}

fn projects_json_changed(
    projects_mtime: Option<i64>,
    indexed: &[IndexedSessionRecord],
) -> bool {
    let Some(projects_mtime) = projects_mtime else {
        return false;
    };
    let mut latest_indexed = None;
    for row in indexed.iter().filter(|s| s.agent == Agent::Gemini) {
        latest_indexed = Some(latest_indexed.map_or(row.last_indexed_at, |v: i64| v.max(row.last_indexed_at)));
    }
    latest_indexed.is_none_or(|ts| projects_mtime > ts)
}

fn current_file_meta(path: &Path) -> Option<(u64, Option<i64>)> {
    let md = std::fs::metadata(path).ok()?;
    Some((md.len(), md.modified().ok().and_then(readers::system_time_to_millis)))
}

fn file_mtime(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(readers::system_time_to_millis)
}

fn file_changed(path: &Path, indexed_size: u64, indexed_mtime: Option<i64>) -> bool {
    match current_file_meta(path) {
        Some((size, mtime)) => size != indexed_size || mtime != indexed_mtime,
        None => true,
    }
}
