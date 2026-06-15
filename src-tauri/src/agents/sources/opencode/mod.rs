pub mod parser;

use anyhow::{Context, Result};

use crate::agents::sources::registry::AgentSource;
use crate::agents::sources::shared::convert::{
    agent_kind, message_events_from_history_acp_messages, session_record_from_info,
    session_source_from_info,
};
use crate::agents::sources::types::{
    AgentKind, MessageEvent, PathEvent, SessionRecord, SessionSource, SourceIndexTask, WatchRoot,
};
use crate::models::Agent;

pub struct OpencodeSource;

impl AgentSource for OpencodeSource {
    fn agent(&self) -> AgentKind {
        agent_kind(Agent::Opencode)
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn roots(&self) -> Result<Vec<WatchRoot>> {
        // OpenCode persists everything to a SQLite database whose writes go
        // through WAL. Filesystem notifications on the .db / .db-wal pair
        // are unreliable across platforms, so the indexer relies on the
        // polling loop instead — same approach we use for AstraPi.
        Ok(Vec::new())
    }

    fn discover(&self) -> Result<Vec<SessionSource>> {
        Ok(parser::list_sessions()?
            .iter()
            .map(session_source_from_info)
            .collect())
    }

    fn parse_source(&self, source: &SessionSource) -> Result<SessionRecord> {
        let info = parser::parse_one(&source.file_path, &source.session_id)?
            .with_context(|| {
                format!(
                    "no opencode session parsed from {} (id={})",
                    source.file_path, source.session_id
                )
            })?;
        Ok(session_record_from_info(&info))
    }

    fn read_messages(&self, source: &SessionSource) -> Result<Vec<MessageEvent>> {
        let messages = parser::read_history_acp_messages_with_locations(
            &source.file_path,
            &source.session_id,
        )?;
        Ok(message_events_from_history_acp_messages(source, messages))
    }

    fn classify_path_event(&self, _event: &PathEvent) -> Option<SourceIndexTask> {
        // Polling owns OpenCode rescans; intentionally don't surface anything
        // here so the watcher doesn't need to register the OpenCode dir.
        None
    }
}
