pub mod parser;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::models::Agent;
use crate::providers::registry::AgentProvider;
use crate::providers::shared::convert::{
    agent_kind, message_events_from_messages, session_record_from_info, session_source_from_info,
};
use crate::providers::types::{
    AgentKind, MessageEvent, PathEvent, PathEventKind, ProviderTask, SessionRecord, SessionSource,
    SourceKind, WatchPurpose, WatchRoot,
};

pub struct CodexProvider;

impl AgentProvider for CodexProvider {
    fn agent(&self) -> AgentKind {
        agent_kind(Agent::Codex)
    }

    fn display_name(&self) -> &'static str {
        "Codex"
    }

    fn roots(&self) -> Result<Vec<WatchRoot>> {
        let (live, archived) = parser::roots()?;
        Ok(vec![
            WatchRoot {
                agent: self.agent(),
                path: live,
                recursive: true,
                purpose: WatchPurpose::Sessions,
            },
            WatchRoot {
                agent: self.agent(),
                path: archived,
                recursive: true,
                purpose: WatchPurpose::Sessions,
            },
        ])
    }

    fn discover(&self) -> Result<Vec<SessionSource>> {
        Ok(parser::list_sessions()?
            .iter()
            .map(session_source_from_info)
            .collect())
    }

    fn parse_source(&self, source: &SessionSource) -> Result<SessionRecord> {
        let (_, archived_root) = parser::roots()?;
        let path = PathBuf::from(&source.file_path);
        let archived = parser::path_is_archived(&path, &archived_root);
        let info = parser::parse_one_file(&path, archived)?
            .with_context(|| format!("no codex session parsed from {}", path.display()))?;
        Ok(session_record_from_info(&info))
    }

    fn read_messages(&self, source: &SessionSource) -> Result<Vec<MessageEvent>> {
        let messages = parser::read_messages(Path::new(&source.file_path))?;
        Ok(message_events_from_messages(source, messages))
    }

    fn classify_path_event(&self, event: &PathEvent) -> Option<ProviderTask> {
        if event.path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            return None;
        }
        let (live, archived) = parser::roots().ok()?;
        let is_archived = event.path.starts_with(&archived);
        if !event.path.starts_with(&live) && !is_archived {
            return None;
        }
        let source = SessionSource {
            agent: self.agent(),
            session_id: String::new(),
            scope: event.path.to_string_lossy().to_string(),
            file_path: event.path.to_string_lossy().to_string(),
            project: None,
            source_kind: if is_archived {
                SourceKind::Archive
            } else {
                SourceKind::MainSession
            },
            metadata: Default::default(),
        };
        if matches!(event.kind, PathEventKind::Remove) {
            Some(ProviderTask::MarkSourceUnavailable(source))
        } else {
            Some(ProviderTask::ReindexSource(source))
        }
    }
}
