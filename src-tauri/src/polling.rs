use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::indexer::{IndexTask, IndexerHandle};
use crate::models::Agent;
use crate::readers;
use crate::store::SessionStore;

pub fn spawn_polling(store: Arc<dyn SessionStore>, indexer: IndexerHandle) {
    thread::spawn(move || {
        // Sizes of <claude_project>/sessions-index.json observed last tick.
        // sessions-index.json isn't tracked as a db row (it's project-level
        // metadata covering archived sessions), so we keep a tiny in-memory
        // snapshot just to detect changes between ticks.
        let mut claude_index_sizes: HashMap<PathBuf, u64> = HashMap::new();
        loop {
            thread::sleep(Duration::from_secs(10));
            if let Err(e) = poll_and_submit(store.clone(), &indexer) {
                log::warn!("polling check failed: {e}");
            }
            if let Err(e) = poll_claude_indexes(&mut claude_index_sizes, &indexer) {
                log::warn!("polling claude index check failed: {e}");
            }
        }
    });
}

fn poll_and_submit(store: Arc<dyn SessionStore>, indexer: &IndexerHandle) -> Result<()> {
    let sessions = store.list_sessions()?;
    let mut stale: HashMap<Agent, Vec<PathBuf>> = HashMap::new();

    for s in sessions {
        if s.file_path.is_empty() || !s.available {
            continue;
        }
        let path = PathBuf::from(&s.file_path);
        if !path.exists() {
            continue;
        }

        // 比较 DB 里的 file_size 与磁盘实际 size，不一致说明文件被修改了
        let disk_size = std::fs::metadata(&path).ok().map(|m| m.len()).unwrap_or(0);
        if s.file_size != disk_size {
            stale.entry(s.agent).or_default().push(path);
        }
    }

    for (agent, paths) in stale {
        log::info!("polling: found {} stale {} files", paths.len(), agent.as_str());
        for path in paths {
            let task = match agent {
                Agent::Codex => IndexTask::ReindexCodexFile(path),
                Agent::Claude => IndexTask::ReindexClaudeFile(path),
                Agent::Gemini => IndexTask::ReindexGeminiLogs(path),
            };
            indexer.submit(task)?;
        }
    }

    Ok(())
}

fn poll_claude_indexes(
    last_sizes: &mut HashMap<PathBuf, u64>,
    indexer: &IndexerHandle,
) -> Result<()> {
    let root = match readers::claude::root_dir()? {
        Some(r) => r,
        None => return Ok(()),
    };
    let mut current: HashMap<PathBuf, u64> = HashMap::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let project_dir = entry.path();
        let idx = project_dir.join("sessions-index.json");
        let size = match fs::metadata(&idx) {
            Ok(m) => m.len(),
            Err(_) => continue,
        };
        // Only dispatch when we've seen this file before and its size moved;
        // otherwise we'd kick off a project rescan on every startup.
        if let Some(prev) = last_sizes.get(&idx) {
            if *prev != size {
                log::info!(
                    "polling: claude sessions-index.json changed at {}",
                    idx.display()
                );
                indexer.submit(IndexTask::ReindexClaudeProject(project_dir))?;
            }
        }
        current.insert(idx, size);
    }
    *last_sizes = current;
    Ok(())
}
