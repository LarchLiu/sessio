use anyhow::Result;

use crate::providers::types::{
    AgentKind, MessageEvent, PathEvent, ProviderTask, SessionRecord, SessionSource, WatchRoot,
};

pub trait AgentProvider: Send + Sync {
    fn agent(&self) -> AgentKind;
    fn display_name(&self) -> &'static str;
    fn roots(&self) -> Result<Vec<WatchRoot>>;
    fn discover(&self) -> Result<Vec<SessionSource>>;
    fn parse_source(&self, source: &SessionSource) -> Result<SessionRecord>;
    fn read_messages(&self, source: &SessionSource) -> Result<Vec<MessageEvent>>;
    fn classify_path_event(&self, event: &PathEvent) -> Option<ProviderTask>;
}

#[derive(Default)]
pub struct ProviderRegistry {
    providers: Vec<Box<dyn AgentProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<P>(&mut self, provider: P)
    where
        P: AgentProvider + 'static,
    {
        self.providers.push(Box::new(provider));
    }

    pub fn providers(&self) -> &[Box<dyn AgentProvider>] {
        &self.providers
    }

    pub fn provider_for_agent(&self, agent: &AgentKind) -> Option<&dyn AgentProvider> {
        self.providers
            .iter()
            .find(|provider| provider.agent() == *agent)
            .map(|provider| provider.as_ref())
    }

    pub fn discover_all(&self) -> Result<Vec<SessionSource>> {
        let mut out = Vec::new();
        for provider in &self.providers {
            out.extend(provider.discover()?);
        }
        Ok(out)
    }

    pub fn watch_roots(&self) -> Result<Vec<WatchRoot>> {
        let mut out = Vec::new();
        for provider in &self.providers {
            out.extend(provider.roots()?);
        }
        Ok(out)
    }

    pub fn classify_path_event(&self, event: &PathEvent) -> Vec<ProviderTask> {
        self.providers
            .iter()
            .filter_map(|provider| provider.classify_path_event(event))
            .collect()
    }
}
