use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use serde_json::json;

use super::acp::{convert_session_notification, fake_session_notification, AcpFakeSessionUpdate};
use super::manager::RuntimeManager;
use super::types::{AgentInput, RuntimeError};

pub fn spawn_stream(
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    turn_id: String,
    input: AgentInput,
    cancel_token: Arc<AtomicBool>,
) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = stream_fake_response(
            &manager,
            &sessio_runtime_session_id,
            &turn_id,
            input,
            &cancel_token,
        )
        .await
        {
            if cancel_token.load(Ordering::Relaxed) {
                return;
            }
            let _ = manager.fail_turn(
                &sessio_runtime_session_id,
                &turn_id,
                RuntimeError::new("fake_runtime_error", error.to_string()),
            );
        }
    });
}

async fn stream_fake_response(
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    turn_id: &str,
    input: AgentInput,
    cancel_token: &AtomicBool,
) -> anyhow::Result<()> {
    sleep(2000);
    if is_cancelled(cancel_token) {
        return Ok(());
    }
    emit_fake_acp(
        manager,
        sessio_runtime_session_id,
        turn_id,
        AcpFakeSessionUpdate::ThoughtChunk {
            content: "Inspecting the request and preparing a streamed reply.\n".to_string(),
        },
    )?;

    if input.text.to_ascii_lowercase().contains("tool") {
        let tool_id = format!("{turn_id}-tool-1");
        sleep(160);
        if is_cancelled(cancel_token) {
            return Ok(());
        }
        emit_fake_acp(
            manager,
            sessio_runtime_session_id,
            turn_id,
            AcpFakeSessionUpdate::ToolCall {
                id: tool_id.clone(),
                title: "fake_lookup".to_string(),
                input: Some(json!({ "query": input.text })),
            },
        )?;
        sleep(140);
        if is_cancelled(cancel_token) {
            return Ok(());
        }
        emit_fake_acp(
            manager,
            sessio_runtime_session_id,
            turn_id,
            AcpFakeSessionUpdate::ToolCallOutput {
                id: tool_id,
                output: "fake lookup completed\n".to_string(),
            },
        )?;
    }

    if input.text.to_ascii_lowercase().contains("permission") {
        let request_id = format!("{turn_id}-permission-1");
        sleep(120);
        if is_cancelled(cancel_token) {
            return Ok(());
        }
        let approved = manager.request_permission(
            sessio_runtime_session_id,
            turn_id,
            &request_id,
            "fake_write",
            Some(json!({ "path": "example.txt", "reason": input.text })),
        )?;
        if is_cancelled(cancel_token) {
            return Ok(());
        }
        let branch = if approved {
            "Permission approved. Continuing with the allowed fake write path.\n\n"
        } else {
            "Permission rejected. Continuing without the protected fake write path.\n\n"
        };
        emit_fake_acp(
            manager,
            sessio_runtime_session_id,
            turn_id,
            AcpFakeSessionUpdate::AgentMessageChunk {
                content: branch.to_string(),
            },
        )?;
    }

    let response = fake_response_text(&input.text);
    for chunk in chunk_text(&response, 18) {
        sleep(45);
        if is_cancelled(cancel_token) {
            return Ok(());
        }
        emit_fake_acp(
            manager,
            sessio_runtime_session_id,
            turn_id,
            AcpFakeSessionUpdate::AgentMessageChunk { content: chunk },
        )?;
    }

    sleep(80);
    if is_cancelled(cancel_token) {
        return Ok(());
    }
    manager.complete_turn(sessio_runtime_session_id, turn_id)
}

fn is_cancelled(cancel_token: &AtomicBool) -> bool {
    cancel_token.load(Ordering::Relaxed)
}

fn emit_fake_acp(
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    turn_id: &str,
    update: AcpFakeSessionUpdate,
) -> anyhow::Result<()> {
    let notification = fake_session_notification(update);
    log::info!(
        "[sessio-runtime:fake-acp:session-update] {:?}",
        notification
    );
    if let Some(event) =
        convert_session_notification(&notification, sessio_runtime_session_id, turn_id)?
    {
        manager.emit(event)?;
    }
    Ok(())
}

fn fake_response_text(input: &str) -> String {
    format!(
        "Fake ACP runtime received:\n\n> {}\n\nThis is streamed through Sessio's unified runtime event model. The real ACP transport can replace this fake transport without changing the chat reducer.",
        input.trim()
    )
}

fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if current.chars().count() >= max_chars {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_text_preserves_input() {
        let text = "hello streamed world";
        let chunks = chunk_text(text, 5);
        assert_eq!(chunks.concat(), text);
        assert!(chunks.len() > 1);
    }
}
