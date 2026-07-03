use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::models::Agent;

pub(super) struct RuntimeResourceLimiter {
    active: Mutex<HashMap<Agent, usize>>,
    waiters: Condvar,
}

impl RuntimeResourceLimiter {
    pub(super) fn new() -> Self {
        Self {
            active: Mutex::new(HashMap::new()),
            waiters: Condvar::new(),
        }
    }

    pub(super) fn acquire(
        &self,
        agent: Agent,
        queue_timeout: Duration,
        mut is_still_active: impl FnMut() -> Result<bool>,
    ) -> Result<Option<Agent>> {
        let Some(limit) = delegated_runtime_limit(agent) else {
            return Ok(None);
        };
        let queued_at = Instant::now();
        let deadline = queued_at + queue_timeout;

        loop {
            if !is_still_active()? {
                bail!(
                    "Astra run is no longer active while waiting for {} runtime capacity",
                    agent.as_str()
                );
            }

            let mut active = self
                .active
                .lock()
                .map_err(|_| anyhow::anyhow!("Astra runtime limiter lock poisoned"))?;
            let active_count = *active.get(&agent).unwrap_or(&0);
            if active_count < limit {
                active.insert(agent, active_count + 1);
                return Ok(Some(agent));
            }

            let now = Instant::now();
            if now >= deadline {
                bail!(
                    "Astra delegated runtime queue timed out after {}ms for {}",
                    queue_timeout.as_millis(),
                    agent.as_str()
                );
            }
            let remaining = deadline.saturating_duration_since(now);
            let (guard, _) = self
                .waiters
                .wait_timeout(active, remaining.min(Duration::from_millis(250)))
                .map_err(|_| anyhow::anyhow!("Astra runtime limiter lock poisoned"))?;
            drop(guard);
        }
    }

    pub(super) fn release(&self, agent: Agent) {
        if delegated_runtime_limit(agent).is_none() {
            return;
        }
        if let Ok(mut active) = self.active.lock() {
            match active.get_mut(&agent) {
                Some(count) if *count > 1 => *count -= 1,
                Some(_) => {
                    active.remove(&agent);
                }
                None => {}
            }
        }
        self.waiters.notify_all();
    }
}

fn delegated_runtime_limit(agent: Agent) -> Option<usize> {
    let _ = agent;
    None
}
