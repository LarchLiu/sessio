use std::collections::HashSet;

use serde_json::{json, Value};

use crate::agents::runtime::RuntimeCleanupReport;

use super::{now_ms, ASTRA_RUNTIME_CLEANUP_TIMEOUT_MS};

pub(super) const MAX_ASTRA_RUN_DIAGNOSTICS: usize = 100;

pub(super) fn trim_astra_run_diagnostics(values: &mut Vec<Value>) {
    if values.len() > MAX_ASTRA_RUN_DIAGNOSTICS {
        values.drain(0..values.len() - MAX_ASTRA_RUN_DIAGNOSTICS);
    }
}

pub(super) fn delegated_lifecycle_diagnostic(
    code: &str,
    task_id: &str,
    live_runtime_session_id: &str,
    session_id: Option<&str>,
    attempt_count: u32,
    message: &str,
) -> Value {
    json!({
        "kind": "delegated_task_lifecycle",
        "code": code,
        "taskId": task_id,
        "liveRuntimeSessionId": live_runtime_session_id,
        "sessionId": session_id,
        "attemptCount": attempt_count,
        "message": message,
        "timestamp": now_ms(),
    })
}

pub(super) fn delegated_dispatch_diagnostic(
    code: &str,
    task_id: &str,
    attempt_count: u32,
    message: &str,
) -> Value {
    json!({
        "kind": "delegated_task_lifecycle",
        "code": code,
        "taskId": task_id,
        "liveRuntimeSessionId": Value::Null,
        "sessionId": Value::Null,
        "attemptCount": attempt_count,
        "message": message,
        "timestamp": now_ms(),
    })
}

pub(super) fn delegated_dispatch_error_code(message: &str) -> &'static str {
    if message.contains("runtime queue timed out") {
        "queue_timeout"
    } else if message.contains("no longer active") {
        "dispatch_cancelled"
    } else {
        "dispatch_failed"
    }
}

pub(super) fn delegated_runtime_cleanup_diagnostics(
    report: &RuntimeCleanupReport,
    task_id: &str,
    live_runtime_session_id: &str,
    session_id: Option<&str>,
    attempt_count: u32,
) -> Vec<Value> {
    let mut diagnostics = Vec::new();
    if let Some(message) = report.cancel_error.as_deref() {
        diagnostics.push(delegated_lifecycle_diagnostic(
            "runtime_cancel_failed",
            task_id,
            live_runtime_session_id,
            session_id,
            attempt_count,
            message,
        ));
    }
    if let Some(message) = report.dispose_error.as_deref() {
        diagnostics.push(delegated_lifecycle_diagnostic(
            "runtime_dispose_failed",
            task_id,
            live_runtime_session_id,
            session_id,
            attempt_count,
            message,
        ));
    }
    if report.timed_out {
        diagnostics.push(delegated_lifecycle_diagnostic(
            "runtime_cleanup_timed_out",
            task_id,
            live_runtime_session_id,
            session_id,
            attempt_count,
            &format!(
                "runtime cleanup exceeded {}ms",
                ASTRA_RUNTIME_CLEANUP_TIMEOUT_MS
            ),
        ));
    }
    if report.force_detached {
        diagnostics.push(delegated_lifecycle_diagnostic(
            "runtime_force_detached",
            task_id,
            live_runtime_session_id,
            session_id,
            attempt_count,
            "runtime session was detached from Astra coordination after cancellation; ACP worker termination is best-effort",
        ));
    }
    diagnostics
}

pub(super) fn dedupe_session_ref_values(values: Vec<Value>) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for value in values {
        let Some(agent) = value.get("agent").and_then(Value::as_str) else {
            continue;
        };
        let Some(session_id) = value.get("sessionId").and_then(Value::as_str) else {
            continue;
        };
        if seen.insert(format!("{agent}:{session_id}")) {
            out.push(value);
        }
    }
    out
}
