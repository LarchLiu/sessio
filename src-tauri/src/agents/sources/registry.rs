use anyhow::Result;

use crate::agents::sources::types::{
    AgentKind, MessageEvent, PathEvent, SessionRecord, SessionSource, SourceIndexTask, WatchRoot,
};

pub trait AgentSource: Send + Sync {
    fn agent(&self) -> AgentKind;
    fn display_name(&self) -> &'static str;
    fn roots(&self) -> Result<Vec<WatchRoot>>;
    fn discover(&self) -> Result<Vec<SessionSource>>;
    fn parse_source(&self, source: &SessionSource) -> Result<SessionRecord>;
    fn read_messages(&self, source: &SessionSource) -> Result<Vec<MessageEvent>>;
    fn classify_path_event(&self, event: &PathEvent) -> Option<SourceIndexTask>;
}

#[derive(Default)]
pub struct AgentSourceRegistry {
    sources: Vec<Box<dyn AgentSource>>,
}

impl AgentSourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<S>(&mut self, source: S)
    where
        S: AgentSource + 'static,
    {
        self.sources.push(Box::new(source));
    }

    pub fn sources(&self) -> &[Box<dyn AgentSource>] {
        &self.sources
    }

    pub fn source_for_agent(&self, agent: &AgentKind) -> Option<&dyn AgentSource> {
        self.sources
            .iter()
            .find(|source| source.agent() == *agent)
            .map(|source| source.as_ref())
    }

    pub fn discover_all(&self) -> Result<Vec<SessionSource>> {
        let mut out = Vec::new();
        for source in &self.sources {
            out.extend(source.discover()?);
        }
        Ok(out)
    }

    pub fn watch_roots(&self) -> Result<Vec<WatchRoot>> {
        let mut out = Vec::new();
        for source in &self.sources {
            out.extend(source.roots()?);
        }
        Ok(out)
    }

    pub fn classify_path_event(&self, event: &PathEvent) -> Vec<SourceIndexTask> {
        self.sources
            .iter()
            .filter_map(|source| source.classify_path_event(event))
            .collect()
    }
}
