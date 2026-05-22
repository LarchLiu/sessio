use std::time::Duration;

use serde_json::json;

use super::manager::RuntimeManager;
use super::types::{AgentInput, AgentRuntimeEventPayload, RuntimeError};

pub fn spawn_stream(
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    turn_id: String,
    input: AgentInput,
) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = stream_fake_response(&manager, &sessio_runtime_session_id, &turn_id, input).await {
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
) -> anyhow::Result<()> {
        sleep(120);
    manager.emit(AgentRuntimeEventPayload::ReasoningDelta {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.to_string(),
        text: "Inspecting the request and preparing a streamed reply.\n".to_string(),
    })?;

    if input.text.to_ascii_lowercase().contains("tool") {
        let tool_id = format!("{turn_id}-tool-1");
        sleep(160);
        manager.emit(AgentRuntimeEventPayload::ToolStarted {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            tool_id: tool_id.clone(),
            name: "fake_lookup".to_string(),
            input: Some(json!({ "query": input.text })),
        })?;
        sleep(140);
        manager.emit(AgentRuntimeEventPayload::ToolOutputDelta {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            tool_id,
            delta: "fake lookup completed\n".to_string(),
        })?;
    }

    let response = fake_response_text(&input.text);
    for chunk in chunk_text(&response, 18) {
        sleep(45);
        manager.emit(AgentRuntimeEventPayload::TextDelta {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            text: chunk,
        })?;
    }

    sleep(80);
    manager.complete_turn(sessio_runtime_session_id, turn_id)
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
