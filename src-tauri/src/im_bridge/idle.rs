//! Idle suspension for IM-owned runtime sessions.
//!
//! A channel session is not ended when idle. We only detach the in-memory
//! runtime process/transport so the next IM message can resume from the
//! persisted `channel_sessions` row.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::state::ImBridgeState;

const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(60);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

pub fn spawn(state: Arc<ImBridgeState>) -> anyhow::Result<()> {
    thread::Builder::new()
        .name("im-bridge-idle".to_string())
        .spawn(move || idle_loop(state))?;
    Ok(())
}

fn idle_loop(state: Arc<ImBridgeState>) {
    loop {
        thread::sleep(IDLE_CHECK_INTERVAL);
        let config = state.config_snapshot();
        if !config.enabled || config.idle_timeout_secs == 0 {
            continue;
        }
        let Ok(idle_timeout_ms) = i64::try_from(config.idle_timeout_secs.saturating_mul(1000))
        else {
            continue;
        };
        let idle_before_ms = now_ms().saturating_sub(idle_timeout_ms);
        for (key, session) in state.idle_suspend_candidates(idle_before_ms) {
            if state.queued_prompt_count(&key) > 0 {
                continue;
            }
            if state
                .runtime
                .active_turn_id(&session.sessio_runtime_session_id)
                .is_some()
            {
                continue;
            }
            let Some(session) = state.suspend_chat(&key) else {
                continue;
            };
            let report = state
                .runtime
                .cleanup_session_bounded(&session.sessio_runtime_session_id, CLEANUP_TIMEOUT);
            if report.dispose_error.is_none() {
                log::info!(
                    "[im-bridge:idle] suspended {} chat {} session {} after {}s idle",
                    key.platform,
                    key.chat_id,
                    session.sessio_runtime_session_id,
                    config.idle_timeout_secs
                );
            } else {
                log::warn!(
                    "[im-bridge:idle] suspended mapping but runtime cleanup reported: {:?}",
                    report.dispose_error
                );
            }
        }
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
