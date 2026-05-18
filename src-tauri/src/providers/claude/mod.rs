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

pub struct ClaudeProvider;

impl AgentProvider for ClaudeProvider {
    fn agent(&self) -> AgentKind {
        agent_kind(Agent::Claude)
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn roots(&self) -> Result<Vec<WatchRoot>> {
        Ok(parser::root_dir()?
            .into_iter()
            .map(|path| WatchRoot {
                agent: self.agent(),
                path,
                recursive: true,
                purpose: WatchPurpose::Projects,
            })
            .collect())
    }

    fn discover(&self) -> Result<Vec<SessionSource>> {
        Ok(parser::list_sessions()?
            .iter()
            .map(session_source_from_info)
            .collect())
    }

    fn parse_source(&self, source: &SessionSource) -> Result<SessionRecord> {
        let path = PathBuf::from(&source.file_path);
        let info = parser::parse_single_file(&path)?
            .with_context(|| format!("no claude session parsed from {}", path.display()))?;
        Ok(session_record_from_info(&info))
    }

    fn read_messages(&self, source: &SessionSource) -> Result<Vec<MessageEvent>> {
        let messages = parser::read_messages_with_locations(Path::new(&source.file_path))?;
        Ok(message_events_from_messages(source, messages))
    }

    fn classify_path_event(&self, event: &PathEvent) -> Option<ProviderTask> {
        let root = parser::root_dir().ok().flatten()?;
        classify_claude_event(&root, self.agent(), event)
    }
}

fn classify_claude_event(
    root: &Path,
    agent: AgentKind,
    event: &PathEvent,
) -> Option<ProviderTask> {
    let rel = event.path.strip_prefix(root).ok()?;
    let file_name = event
        .path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    // <root>/<project>/sessions-index.json: project-level rescan covers
    // archived rows that no jsonl on disk would surface.
    if file_name == "sessions-index.json" {
        let project_dir = event.path.parent()?;
        return Some(ProviderTask::ReindexScope {
            agent,
            scope: project_dir.to_string_lossy().to_string(),
        });
    }

    if event.path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
        return None;
    }

    let mut comps = rel.components();
    let project = comps.next()?;
    let rest: Vec<_> = comps.collect();
    let is_subagent = rest.iter().any(|c| c.as_os_str() == "subagents");
    let source_kind = if is_subagent {
        SourceKind::Subagent
    } else {
        // Top-level main jsonl: rel == <project>/<id>.jsonl (2 components).
        if rest.len() != 1 {
            return None;
        }
        SourceKind::MainSession
    };

    let scope = if is_subagent {
        // Subagent path is <project>/<parent_session>/subagents/<sid>.jsonl,
        // scope = project dir so it lines up with main rows.
        root.join(project).to_string_lossy().to_string()
    } else {
        event.path.parent()?.to_string_lossy().to_string()
    };

    let source = SessionSource {
        agent,
        session_id: String::new(),
        scope,
        file_path: event.path.to_string_lossy().to_string(),
        project: None,
        source_kind,
        metadata: Default::default(),
    };

    if matches!(event.kind, PathEventKind::Remove) {
        Some(ProviderTask::MarkSourceUnavailable(source))
    } else {
        Some(ProviderTask::ReindexSource(source))
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_claude_event, AgentKind};
    use crate::providers::types::{PathEvent, PathEventKind, ProviderTask, SourceKind};
    use std::path::{Path, PathBuf};

    fn evt(path: impl Into<PathBuf>, kind: PathEventKind) -> PathEvent {
        PathEvent {
            path: path.into(),
            kind,
        }
    }

    #[test]
    fn sessions_index_json_emits_reindex_scope() {
        let root = Path::new("/tmp/claude/projects");
        let task = classify_claude_event(
            root,
            AgentKind::new("claude"),
            &evt(
                "/tmp/claude/projects/proj-A/sessions-index.json",
                PathEventKind::Modify,
            ),
        )
        .expect("emits a task");
        match task {
            ProviderTask::ReindexScope { agent, scope } => {
                assert_eq!(agent.as_str(), "claude");
                assert_eq!(scope, "/tmp/claude/projects/proj-A");
            }
            other => panic!("expected ReindexScope, got {other:?}"),
        }
    }

    #[test]
    fn main_jsonl_emits_reindex_source_main_session() {
        let root = Path::new("/tmp/claude/projects");
        let task = classify_claude_event(
            root,
            AgentKind::new("claude"),
            &evt(
                "/tmp/claude/projects/proj-A/abc-123.jsonl",
                PathEventKind::Modify,
            ),
        )
        .expect("emits a task");
        match task {
            ProviderTask::ReindexSource(source) => {
                assert_eq!(source.source_kind, SourceKind::MainSession);
                assert_eq!(source.scope, "/tmp/claude/projects/proj-A");
                assert_eq!(
                    source.file_path,
                    "/tmp/claude/projects/proj-A/abc-123.jsonl"
                );
            }
            other => panic!("expected ReindexSource, got {other:?}"),
        }
    }

    #[test]
    fn subagent_jsonl_emits_reindex_source_subagent_with_project_scope() {
        let root = Path::new("/tmp/claude/projects");
        let task = classify_claude_event(
            root,
            AgentKind::new("claude"),
            &evt(
                "/tmp/claude/projects/proj-A/parent-1/subagents/sub-9.jsonl",
                PathEventKind::Modify,
            ),
        )
        .expect("emits a task");
        match task {
            ProviderTask::ReindexSource(source) => {
                assert_eq!(source.source_kind, SourceKind::Subagent);
                assert_eq!(source.scope, "/tmp/claude/projects/proj-A");
            }
            other => panic!("expected ReindexSource, got {other:?}"),
        }
    }

    #[test]
    fn remove_event_emits_mark_source_unavailable() {
        let root = Path::new("/tmp/claude/projects");
        let main = classify_claude_event(
            root,
            AgentKind::new("claude"),
            &evt(
                "/tmp/claude/projects/proj-A/abc-123.jsonl",
                PathEventKind::Remove,
            ),
        )
        .expect("emits a task");
        match main {
            ProviderTask::MarkSourceUnavailable(source) => {
                assert_eq!(source.source_kind, SourceKind::MainSession);
            }
            other => panic!("expected MarkSourceUnavailable, got {other:?}"),
        }

        let sub = classify_claude_event(
            root,
            AgentKind::new("claude"),
            &evt(
                "/tmp/claude/projects/proj-A/parent-1/subagents/sub-9.jsonl",
                PathEventKind::Remove,
            ),
        )
        .expect("emits a task");
        match sub {
            ProviderTask::MarkSourceUnavailable(source) => {
                assert_eq!(source.source_kind, SourceKind::Subagent);
            }
            other => panic!("expected MarkSourceUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn nested_unrelated_jsonl_is_ignored() {
        let root = Path::new("/tmp/claude/projects");
        assert!(classify_claude_event(
            root,
            AgentKind::new("claude"),
            &evt(
                "/tmp/claude/projects/proj-A/abc/extra.jsonl",
                PathEventKind::Modify,
            ),
        )
        .is_none());
    }

    #[test]
    fn path_outside_root_is_ignored() {
        let root = Path::new("/tmp/claude/projects");
        assert!(classify_claude_event(
            root,
            AgentKind::new("claude"),
            &evt("/tmp/elsewhere/foo.jsonl", PathEventKind::Modify),
        )
        .is_none());
    }
}
