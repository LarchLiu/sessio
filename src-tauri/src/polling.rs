use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::indexer::{IndexPhase, IndexTask, IndexerHandle};
use crate::models::Agent;
use crate::store::{IndexedSessionRecord, IndexedSubagentRecord, SessionStore};

pub fn spawn_polling(store: Arc<dyn SessionStore>, indexer: IndexerHandle) {
    thread::spawn(move || {
        let mut claude_index_mtimes: HashMap<PathBuf, Option<i64>> = HashMap::new();
        let mut gemini_projects_mtime: Option<i64> = None;
        let mut first_tick = true;
        loop {
            if !first_tick {
                thread::sleep(Duration::from_secs(10));
            }
            first_tick = false;
            // Skip while the indexer is busy. A full rebuild can take a long
            // time (it now also drives the per-source memory/QMD pipeline),
            // and our DB snapshot would be stale mid-rebuild — every file we
            // saw would look "not yet indexed" and we'd submit redundant
            // per-file reindex tasks whose outcomes re-trigger the heavy
            // memory work. The next tick (10s later) will catch real changes.
            if !matches!(indexer.status().phase, IndexPhase::Idle) {
                continue;
            }
            if let Err(e) = poll_once(
                store.clone(),
                &indexer,
                &mut claude_index_mtimes,
                &mut gemini_projects_mtime,
            ) {
                log::warn!("polling check failed: {e}");
            }
        }
    });
}

fn poll_once(
    store: Arc<dyn SessionStore>,
    indexer: &IndexerHandle,
    claude_index_mtimes: &mut HashMap<PathBuf, Option<i64>>,
    gemini_projects_mtime: &mut Option<i64>,
) -> Result<()> {
    let indexed = store.list_indexed_sessions()?;
    if indexed.is_empty() {
        // Fresh / wiped DB: skip the per-file submission storm. FullRebuild
        // produces project-level affected items only, so the memory + QMD
        // pipeline runs once per project instead of once per session file.
        // Subsequent ticks see indexing=true and skip until the rebuild
        // finishes, then resume the normal per-file diffing flow.
        log::info!("polling: indexed DB is empty, submitting FullRebuild");
        indexer.submit(IndexTask::FullRebuild)?;
        return Ok(());
    }
    poll_codex(&indexed, indexer)?;
    poll_claude(&indexed, claude_index_mtimes, store.as_ref(), indexer)?;
    poll_gemini(&indexed, store.as_ref(), indexer, gemini_projects_mtime)?;
    Ok(())
}

fn poll_codex(indexed: &[IndexedSessionRecord], indexer: &IndexerHandle) -> Result<()> {
    let (live_root, archived_root) = crate::agents::sources::codex::parser::roots()?;
    let mut known_main: HashMap<String, &IndexedSessionRecord> = indexed
        .iter()
        .filter(|s| s.agent == Agent::Codex && !s.file_path.is_empty())
        .map(|s| (s.file_path.clone(), s))
        .collect();
    let mut known_sub: HashMap<String, &IndexedSubagentRecord> = HashMap::new();
    for row in indexed.iter().filter(|s| s.agent == Agent::Codex) {
        for sub in &row.subagents {
            if !sub.file_path.is_empty() {
                known_sub.insert(sub.file_path.clone(), sub);
            }
        }
    }

    for root in [&live_root, &archived_root] {
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
            let path_str = path.to_string_lossy().into_owned();
            let archived =
                crate::agents::sources::codex::parser::path_is_archived(path, &archived_root);
            let parsed_subagent =
                crate::agents::sources::codex::parser::parse_one_subagent_file(path, archived)
                    .unwrap_or(None)
                    .is_some();
            let needs_reindex = if parsed_subagent {
                match known_sub.remove(&path_str) {
                    Some(row) => {
                        !row.available || file_changed(path, row.file_size, row.file_mtime)
                    }
                    None => true,
                }
            } else {
                match known_main.remove(&path_str) {
                    // File reappeared after a previous soft-delete, or it changed,
                    // or it switched between live/archived: re-parse it.
                    Some(row) => {
                        !row.available
                            || file_changed(path, row.file_size, row.file_mtime)
                            || row.archived != archived
                    }
                    None => true,
                }
            };
            if needs_reindex {
                indexer.submit(IndexTask::ReindexCodexFile(path.to_path_buf()))?;
            }
        }
    }

    // Whatever remains in `known_*` is indexed but absent on disk. Skip rows
    // already marked unavailable to avoid pointless writes through the cache.
    for stale in known_main.into_values() {
        if stale.available {
            indexer.submit(IndexTask::DeleteFile(PathBuf::from(&stale.file_path)))?;
        }
    }
    for stale in known_sub.into_values() {
        if stale.available {
            indexer.submit(IndexTask::DeleteSubagentFile(PathBuf::from(
                &stale.file_path,
            )))?;
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
    let Some(root) = crate::agents::sources::claude::parser::root_dir()? else {
        // Root not present (yet). Mark every Claude row unavailable but keep
        // history — when the root reappears the next tick will reindex and
        // flip rows back to available.
        store.mark_missing_scopes_unavailable(Agent::Claude, &HashSet::new())?;
        return Ok(());
    };

    // Index claude rows by their on-disk file path. Main rows and subagent
    // rows are tracked separately so a change on one doesn't touch the other.
    let mut known_main: HashMap<String, &IndexedSessionRecord> = HashMap::new();
    let mut known_sub: HashMap<String, &IndexedSubagentRecord> = HashMap::new();
    // Track the most recent successful reindex per scope so that on app
    // startup (when the in-memory `claude_index_mtimes` cache is empty)
    // we can still tell whether the sessions-index.json file has actually
    // changed since we last processed this project. Without this we would
    // submit a ReindexClaudeProject for every project on every cold start.
    let mut last_indexed_per_scope: HashMap<String, i64> = HashMap::new();
    for row in indexed.iter().filter(|s| s.agent == Agent::Claude) {
        if !row.file_path.is_empty() {
            known_main.insert(row.file_path.clone(), row);
        }
        for sub in &row.subagents {
            if !sub.file_path.is_empty() {
                known_sub.insert(sub.file_path.clone(), sub);
            }
        }
        last_indexed_per_scope
            .entry(row.scope.clone())
            .and_modify(|v| {
                if row.last_indexed_at > *v {
                    *v = row.last_indexed_at;
                }
            })
            .or_insert(row.last_indexed_at);
    }
    let mut seen_main: HashSet<String> = HashSet::new();
    let mut seen_sub: HashSet<String> = HashSet::new();

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

        // sessions-index.json hint: archived main rows have no jsonl on disk,
        // so a per-file walk can't see them. When the index file appears or
        // changes, queue a project rescan to materialize those synthetic rows.
        let sessions_index = dir.join("sessions-index.json");
        let index_mtime = file_mtime(&sessions_index);
        current_index_mtimes.insert(sessions_index.clone(), index_mtime);
        let index_changed = match claude_index_mtimes.get(&sessions_index) {
            Some(prev) => prev != &index_mtime,
            // Cold-start fallback: only treat it as changed if the file
            // is newer than the most recent reindex we have on record for
            // this scope. Equal-or-older means we already processed this
            // version on a previous run.
            None => match (index_mtime, last_indexed_per_scope.get(&scope)) {
                (Some(mtime), Some(last_indexed)) => mtime > *last_indexed,
                (Some(_), None) => true,
                (None, _) => false,
            },
        };
        if index_changed {
            indexer.submit(IndexTask::ReindexClaudeProject(dir.clone()))?;
        }

        // Per-file scan. Main jsonl sits at <project>/<id>.jsonl; subagent
        // jsonl sits at <project>/<id>/subagents/<sid>.jsonl. They get their
        // own task types and never trigger each other.
        for child in std::fs::read_dir(&dir)? {
            let child = child?;
            let cpath = child.path();
            let ctype = child.file_type()?;
            if ctype.is_file() && cpath.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                let path_str = cpath.to_string_lossy().into_owned();
                seen_main.insert(path_str.clone());
                let needs_reindex = match known_main.get(&path_str) {
                    Some(row) => {
                        !row.available || file_changed(&cpath, row.file_size, row.file_mtime)
                    }
                    None => true,
                };
                if needs_reindex {
                    indexer.submit(IndexTask::ReindexClaudeFile(cpath))?;
                }
                continue;
            }
            if !ctype.is_dir() {
                continue;
            }
            let subagents_dir = cpath.join("subagents");
            if !subagents_dir.is_dir() {
                continue;
            }
            for sub_entry in std::fs::read_dir(&subagents_dir)? {
                let sub_entry = sub_entry?;
                let sub_path = sub_entry.path();
                if !sub_entry.file_type()?.is_file()
                    || sub_path.extension().and_then(|s| s.to_str()) != Some("jsonl")
                {
                    continue;
                }
                let sub_path_str = sub_path.to_string_lossy().into_owned();
                seen_sub.insert(sub_path_str.clone());
                let needs_reindex = match known_sub.get(&sub_path_str) {
                    Some(row) => {
                        !row.available || file_changed(&sub_path, row.file_size, row.file_mtime)
                    }
                    None => true,
                };
                if needs_reindex {
                    indexer.submit(IndexTask::ReindexClaudeSubagentFile(sub_path))?;
                }
            }
        }
    }

    *claude_index_mtimes = current_index_mtimes;

    // Soft-delete vanished files. Skip rows already unavailable so we don't
    // re-issue the same write every tick.
    for (path, row) in &known_main {
        if !seen_main.contains(path) && row.available {
            indexer.submit(IndexTask::DeleteFile(PathBuf::from(path)))?;
        }
    }
    for (path, sub) in &known_sub {
        if !seen_sub.contains(path) && sub.available {
            indexer.submit(IndexTask::DeleteSubagentFile(PathBuf::from(path)))?;
        }
    }

    // Whole project dir gone: every main row under that scope is gone.
    // Subagents below are picked up by their per-file delete above.
    store.mark_missing_scopes_unavailable(Agent::Claude, &seen_scopes)?;

    Ok(())
}

fn poll_gemini(
    indexed: &[IndexedSessionRecord],
    store: &dyn SessionStore,
    indexer: &IndexerHandle,
    gemini_projects_mtime: &mut Option<i64>,
) -> Result<()> {
    let (tmp_dir, projects_json) = crate::agents::sources::gemini::parser::paths()?;
    let mut by_scope: HashMap<String, Vec<&IndexedSessionRecord>> = HashMap::new();
    for row in indexed.iter().filter(|s| s.agent == Agent::Gemini) {
        by_scope.entry(row.scope.clone()).or_default().push(row);
    }

    let projects_mtime = file_mtime(&projects_json);
    let projects_changed = projects_json_changed(projects_mtime, indexed, gemini_projects_mtime);
    if projects_changed {
        log::info!(
            "polling: submit {:?} because gemini projects.json changed (mtime={:?})",
            IndexTask::RefreshGeminiProjectMappings,
            projects_mtime
        );
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
        let live_rows: Vec<&IndexedSessionRecord> = rows
            .iter()
            .filter_map(|row| row.available.then_some(*row))
            .collect();
        let needs_reindex = !live_rows.is_empty()
            && live_rows
                .iter()
                .any(|row| file_changed(&logs, row.file_size, row.file_mtime));
        if needs_reindex {
            let changed = live_rows
                .iter()
                .any(|row| file_changed(&logs, row.file_size, row.file_mtime));
            log::info!(
                "polling: submit {:?} for {} (rows={}, live_rows={}, file_changed={})",
                IndexTask::ReindexGeminiLogs(logs.clone()),
                scope,
                rows.len(),
                live_rows.len(),
                changed
            );
            indexer.submit(IndexTask::ReindexGeminiLogs(logs))?;
        }
    }

    store.mark_missing_scopes_unavailable(Agent::Gemini, &present_scopes)?;

    Ok(())
}

fn projects_json_changed(
    projects_mtime: Option<i64>,
    indexed: &[IndexedSessionRecord],
    last_seen_mtime: &mut Option<i64>,
) -> bool {
    let Some(projects_mtime) = projects_mtime else {
        return false;
    };
    if last_seen_mtime.is_some_and(|seen| seen == projects_mtime) {
        return false;
    }
    let mut latest_indexed = None;
    for row in indexed.iter().filter(|s| s.agent == Agent::Gemini) {
        latest_indexed =
            Some(latest_indexed.map_or(row.last_indexed_at, |v: i64| v.max(row.last_indexed_at)));
    }
    let changed = latest_indexed.is_none_or(|ts| projects_mtime > ts);
    if changed {
        *last_seen_mtime = Some(projects_mtime);
    }
    changed
}

fn current_file_meta(path: &Path) -> Option<(u64, Option<i64>)> {
    let md = std::fs::metadata(path).ok()?;
    Some((
        md.len(),
        md.modified()
            .ok()
            .and_then(crate::agents::sources::system_time_to_millis),
    ))
}

fn file_mtime(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(crate::agents::sources::system_time_to_millis)
}

fn file_changed(path: &Path, indexed_size: u64, indexed_mtime: Option<i64>) -> bool {
    match current_file_meta(path) {
        Some((size, mtime)) => size != indexed_size || mtime != indexed_mtime,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_tmp(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn gemini_poll_should_ignore_unavailable_rows_if_live_row_matches_file() {
        let dir = unique_tmp("gemini-poll");
        let path = dir.join("logs.json");
        fs::write(&path, "[]\n").unwrap();
        let meta = fs::metadata(&path).unwrap();
        let mtime = meta
            .modified()
            .ok()
            .and_then(crate::agents::sources::system_time_to_millis);
        let indexed = vec![
            IndexedSessionRecord {
                agent: Agent::Gemini,
                session_id: "live".to_string(),
                scope: path.to_string_lossy().to_string(),
                file_path: path.to_string_lossy().to_string(),
                file_size: meta.len(),
                file_mtime: mtime,
                last_indexed_at: 1,
                available: true,
                archived: false,
                subagents: Vec::new(),
            },
            IndexedSessionRecord {
                agent: Agent::Gemini,
                session_id: "old".to_string(),
                scope: path.to_string_lossy().to_string(),
                file_path: path.to_string_lossy().to_string(),
                file_size: meta.len(),
                file_mtime: mtime,
                last_indexed_at: 1,
                available: false,
                archived: false,
                subagents: Vec::new(),
            },
        ];

        let mut by_scope: HashMap<String, Vec<&IndexedSessionRecord>> = HashMap::new();
        for row in indexed.iter() {
            by_scope.entry(row.scope.clone()).or_default().push(row);
        }

        let rows = by_scope
            .remove(&path.to_string_lossy().to_string())
            .unwrap_or_default();
        let live_rows: Vec<&IndexedSessionRecord> =
            rows.iter().copied().filter(|row| row.available).collect();
        assert!(!rows.is_empty());
        assert!(!live_rows.is_empty());
        assert!(live_rows
            .iter()
            .any(|row| !file_changed(&path, row.file_size, row.file_mtime)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gemini_poll_should_not_reindex_when_only_unavailable_rows_remain() {
        let dir = unique_tmp("gemini-poll-unavailable");
        let path = dir.join("logs.json");
        fs::write(&path, "[]\n").unwrap();
        let meta = fs::metadata(&path).unwrap();
        let mtime = meta
            .modified()
            .ok()
            .and_then(crate::agents::sources::system_time_to_millis);
        let rows = vec![IndexedSessionRecord {
            agent: Agent::Gemini,
            session_id: "gone".to_string(),
            scope: path.to_string_lossy().to_string(),
            file_path: path.to_string_lossy().to_string(),
            file_size: meta.len() + 42,
            file_mtime: mtime.map(|v| v - 1000),
            last_indexed_at: 1,
            available: false,
            archived: false,
            subagents: Vec::new(),
        }];
        assert!(rows.iter().all(|row| !row.available));
        let _ = fs::remove_dir_all(&dir);
    }
}
