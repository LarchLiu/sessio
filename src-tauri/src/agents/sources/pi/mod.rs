pub mod parser;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::agents::sources::registry::AgentSource;
use crate::agents::sources::shared::convert::{
    agent_kind, message_events_from_history_acp_messages, session_record_from_info,
    session_source_from_info,
};
use crate::agents::sources::types::{
    AgentKind, MessageEvent, PathEvent, PathEventKind, SessionRecord, SessionSource,
    SourceIndexTask, SourceKind, WatchRoot,
};
use crate::models::Agent;

pub struct PiSource;

impl AgentSource for PiSource {
    fn agent(&self) -> AgentKind {
        agent_kind(Agent::AstraPi)
    }

    fn display_name(&self) -> &'static str {
        "Astra Pi"
    }

    fn roots(&self) -> Result<Vec<WatchRoot>> {
        // Pi/AstraPi session files are discovered by polling after final
        // transcript persistence; live streaming must not trigger watcher IO.
        Ok(Vec::new())
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
            .with_context(|| format!("no pi session parsed from {}", path.display()))?;
        Ok(session_record_from_info(&info))
    }

    fn read_messages(&self, source: &SessionSource) -> Result<Vec<MessageEvent>> {
        let messages = parser::read_history_acp_messages_with_locations(
            Path::new(&source.file_path),
            &source.session_id,
        )?;
        Ok(message_events_from_history_acp_messages(source, messages))
    }

    fn classify_path_event(&self, event: &PathEvent) -> Option<SourceIndexTask> {
        let root = parser::root_dir().ok().flatten()?;
        classify_pi_event(&root, self.agent(), event)
    }
}

fn classify_pi_event(root: &Path, agent: AgentKind, event: &PathEvent) -> Option<SourceIndexTask> {
    if event.path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
        return None;
    }
    if !event.path.starts_with(root) {
        return None;
    }
    let source = SessionSource {
        agent,
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
