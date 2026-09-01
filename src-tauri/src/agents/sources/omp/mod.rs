pub mod parser;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::agents::sources::registry::AgentSource;
use crate::agents::sources::shared::convert::{
    agent_kind, session_record_from_info, session_source_from_info,
};
use crate::agents::sources::types::{
    AgentKind, MessageEvent, PathEvent, PathEventKind, SessionRecord, SessionSource,
    SourceIndexTask, SourceKind, WatchPurpose, WatchRoot,
};
use crate::models::Agent;

pub struct OmpSource;

impl AgentSource for OmpSource {
    fn agent(&self) -> AgentKind {
        agent_kind(Agent::Omp)
    }

    fn display_name(&self) -> &'static str {
        "OMP"
    }

    fn roots(&self) -> Result<Vec<WatchRoot>> {
        let Some(root) = parser::root_dir()? else {
            return Ok(Vec::new());
        };
        Ok(vec![WatchRoot {
            agent: self.agent(),
            path: root,
            recursive: true,
            purpose: WatchPurpose::Sessions,
        }])
    }

    fn discover(&self) -> Result<Vec<SessionSource>> {
        Ok(parser::list_sessions()?
            .iter()
            .map(session_source_from_info)
            .collect())
    }

    fn parse_source(&self, source: &SessionSource) -> Result<SessionRecord> {
        let path = PathBuf::from(&source.file_path);
        let info = parser::parse_session_file(&path)?
            .with_context(|| format!("no omp session parsed from {}", path.display()))?;
        Ok(session_record_from_info(&info))
    }

    fn read_messages(&self, source: &SessionSource) -> Result<Vec<MessageEvent>> {
        parser::read_message_events(Path::new(&source.file_path), source)
    }

    fn classify_path_event(&self, event: &PathEvent) -> Option<SourceIndexTask> {
        let root = parser::root_dir().ok().flatten()?;
        if event.path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            return None;
        }
        if !event.path.starts_with(&root) {
            return None;
        }
        let source = SessionSource {
            agent: self.agent(),
            session_id: String::new(),
            scope: root.to_string_lossy().to_string(),
            file_path: event.path.to_string_lossy().to_string(),
            project: None,
            source_kind: SourceKind::MainSession,
            metadata: Default::default(),
        };
        if matches!(event.kind, PathEventKind::Remove) {
            Some(SourceIndexTask::MarkSourceUnavailable(source))
        } else {
            Some(SourceIndexTask::ReindexSource(source))
        }
    }
}
