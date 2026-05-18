pub mod parser;

use anyhow::{Context, Result};
use std::path::Path;

use crate::models::Agent;
use crate::providers::registry::AgentProvider;
use crate::providers::shared::convert::{
    agent_kind, message_events_from_messages, session_record_from_info, session_source_from_info,
};
use crate::providers::types::{
    AgentKind, MessageEvent, PathEvent, ProviderTask, SessionRecord, SessionSource, WatchPurpose,
    WatchRoot,
};

pub struct GeminiProvider;

impl AgentProvider for GeminiProvider {
    fn agent(&self) -> AgentKind {
        agent_kind(Agent::Gemini)
    }

    fn display_name(&self) -> &'static str {
        "Gemini"
    }

    fn roots(&self) -> Result<Vec<WatchRoot>> {
        let (tmp, projects_json) = parser::paths()?;
        Ok(vec![
            WatchRoot {
                agent: self.agent(),
                path: tmp,
                recursive: true,
                purpose: WatchPurpose::Logs,
            },
            WatchRoot {
                agent: self.agent(),
                path: projects_json,
                recursive: false,
                purpose: WatchPurpose::ProjectMappings,
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
        let sessions = parser::parse_logs_file(Path::new(&source.file_path))?;
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
        let messages = parser::read_messages(Path::new(&source.file_path), &source.session_id)?;
        Ok(message_events_from_messages(source, messages))
    }

    fn classify_path_event(&self, event: &PathEvent) -> Option<ProviderTask> {
        let (tmp_dir, projects_json) = parser::paths().ok()?;
        let file_name = event
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        // projects.json is the global Gemini project-mapping file. We don't
        // care whether it lives in tmp_dir or its parent; matching by name
        // and exact path keeps this stable across Gemini versions that move
        // the file around.
        if event.path == projects_json || file_name == "projects.json" {
            return Some(ProviderTask::RefreshProjectMappings {
                agent: self.agent(),
            });
        }

        if !event.path.starts_with(&tmp_dir) || file_name != "logs.json" {
            return None;
        }

        Some(ProviderTask::ReindexScope {
            agent: self.agent(),
            scope: event.path.to_string_lossy().to_string(),
        })
    }
}
