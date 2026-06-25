//! Background worker for scheduled tasks.
//!
//! A single thread wakes on a fixed interval and asks [`SchedulerState::tick`]
//! to fire any due tasks. Modeled on `im_bridge::idle` — the project drives all
//! its periodic background work this way rather than with an async runtime.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::SchedulerState;
use super::config::now_ms;

/// How often to check for due tasks. The finest schedule resolution is one
/// minute (cron/daily/weekly), and intervals are second-granular, so a 30s tick
/// keeps drift well under a minute without busy-looping.
const CHECK_INTERVAL: Duration = Duration::from_secs(30);

pub fn spawn(state: Arc<SchedulerState>) -> anyhow::Result<()> {
    thread::Builder::new()
        .name("scheduled-tasks".to_string())
        .spawn(move || run_loop(state))?;
    Ok(())
}

fn run_loop(state: Arc<SchedulerState>) {
    loop {
        thread::sleep(CHECK_INTERVAL);
        state.tick(now_ms());
    }
}
