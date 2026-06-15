use std::collections::BTreeMap;

use crate::models::Agent;

use super::types::RuntimeTransportKind;

#[derive(Debug, Clone)]
pub struct RuntimeRegistration {
    pub agent: Agent,
    pub transport: RuntimeTransportKind,
}

#[derive(Debug, Default, Clone)]
pub struct RuntimeRegistry {
    registrations: BTreeMap<AgentKey, RuntimeRegistration>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, registration: RuntimeRegistration) {
        self.registrations
            .insert(AgentKey::from(registration.agent), registration);
    }

    pub fn get(&self, agent: Agent) -> Option<&RuntimeRegistration> {
        self.registrations.get(&AgentKey::from(agent))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct AgentKey(&'static str);

impl From<Agent> for AgentKey {
    fn from(agent: Agent) -> Self {
        Self(agent.as_str())
    }
}

pub fn builtin_runtime_registry() -> RuntimeRegistry {
    let mut registry = RuntimeRegistry::new();
    for agent in Agent::ALL.iter().copied() {
        registry.register(RuntimeRegistration {
            agent,
            transport: RuntimeTransportKind::Fake,
        });
    }
    registry
}
