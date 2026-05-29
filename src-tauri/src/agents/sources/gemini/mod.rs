pub mod parser;

use anyhow::{Context, Result};
use std::path::Path;

use crate::agents::sources::registry::AgentSource;
use crate::agents::sources::shared::convert::{
    agent_kind, message_events_from_history_acp_messages, session_record_from_info,
    session_source_from_info,
};
use crate::agents::sources::types::{
    AgentKind, MessageEvent, PathEvent, PathEventKind, SessionRecord, SessionSource,
    SourceIndexTask, SourceKind, WatchPurpose, WatchRoot,
};
use crate::models::Agent;

pub struct GeminiSource;

impl AgentSource for GeminiSource {
    fn agent(&self) -> AgentKind {
        agent_kind(Agent::Gemini)
    }

    fn display_name(&self) -> &'static str {
        "Gemini"
    }

    fn roots(&self) -> Result<Vec<WatchRoot>> {
        let base = parser::base_dir()?;
        Ok(vec![WatchRoot {
            agent: self.agent(),
            path: base,
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
        let sessions = parser::parse_file(Path::new(&source.file_path))?;
        let info = sessions
            .into_iter()
            .find(|s| s.id == source.session_id)
            .with_context(|| {
                format!(
                    "gemini session {} not found in {}",
                    source.session_id, source.file_path
                )
            })?;
        Ok(session_record_from_info(&info))
    }

    fn read_messages(&self, source: &SessionSource) -> Result<Vec<MessageEvent>> {
        let events = parser::read_history_acp_messages_with_locations(
            Path::new(&source.file_path),
            &source.session_id,
        )?;
        Ok(message_events_from_history_acp_messages(source, events))
    }

    fn classify_path_event(&self, event: &PathEvent) -> Option<SourceIndexTask> {
        let file_name = event
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        if file_name.ends_with(".jsonl") && parser::is_chat_file(&event.path) {
            let source = SessionSource {
                agent: self.agent(),
                session_id: String::new(),
                scope: event.path.to_string_lossy().to_string(),
                file_path: event.path.to_string_lossy().to_string(),
                project: None,
                source_kind: SourceKind::MainSession,
                metadata: Default::default(),
            };
            if matches!(event.kind, PathEventKind::Remove) {
                return Some(SourceIndexTask::MarkSourceUnavailable(source));
            }
            return Some(SourceIndexTask::ReindexSource(source));
        }

        None
    }
}
