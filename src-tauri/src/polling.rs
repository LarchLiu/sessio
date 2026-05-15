use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::indexer::{IndexTask, IndexerHandle};
use crate::models::Agent;
use crate::store::SessionStore;

pub fn spawn_polling(store: Arc<dyn SessionStore>, indexer: IndexerHandle) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(10));
            if let Err(e) = poll_and_submit(store.clone(), &indexer) {
                log::warn!("polling check failed: {e}");
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
                Agent::Claude => {
                    if let Some(parent) = path.parent().map(|p| p.to_path_buf()) {
                        IndexTask::ReindexClaudeProject(parent)
                    } else {
                        continue;
                    }
                }
                Agent::Gemini => IndexTask::ReindexGeminiLogs(path),
            };
            indexer.submit(task)?;
        }
    }

    Ok(())
}
