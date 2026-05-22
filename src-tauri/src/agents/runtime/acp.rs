use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::types::{AgentRuntimeEventPayload, RuntimeError};

pub fn fake_session_update(delta: AcpFakeUpdate) -> Value {
    let update = match delta {
        AcpFakeUpdate::AgentMessageChunk { content } => {
            json!({ "sessionId": delta_session_id_placeholder(), "update": { "kind": "agent_message_chunk", "content": { "type": "text", "text": content } } })
        }
        AcpFakeUpdate::ThoughtChunk { content } => {
            json!({ "sessionId": delta_session_id_placeholder(), "update": { "kind": "thought_chunk", "content": { "type": "text", "text": content } } })
        }
        AcpFakeUpdate::ToolCall { id, title, input } => {
            json!({ "sessionId": delta_session_id_placeholder(), "update": { "kind": "tool_call", "toolCallId": id, "title": title, "rawInput": input } })
        }
        AcpFakeUpdate::ToolCallOutput { id, output } => {
            json!({ "sessionId": delta_session_id_placeholder(), "update": { "kind": "tool_call_update", "toolCallId": id, "content": { "type": "text", "text": output } } })
        }
        AcpFakeUpdate::PermissionRequest { id, tool_name, input } => {
            json!({ "sessionId": delta_session_id_placeholder(), "update": { "kind": "permission_request", "permissionRequestId": id, "toolName": tool_name, "input": input } })
        }
        AcpFakeUpdate::PermissionResponse { id, approved } => {
            json!({ "sessionId": delta_session_id_placeholder(), "update": { "kind": "permission_response", "permissionRequestId": id, "approved": approved } })
        }
        AcpFakeUpdate::Error { message } => {
            json!({ "sessionId": delta_session_id_placeholder(), "update": { "kind": "error", "message": message } })
        }
    };
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": update,
    })
}

pub enum AcpFakeUpdate {
    AgentMessageChunk {
        content: String,
    },
    ThoughtChunk {
        content: String,
    },
    ToolCall {
        id: String,
        title: String,
        input: Value,
    },
    ToolCallOutput {
        id: String,
        output: String,
    },
    PermissionRequest {
        id: String,
        tool_name: String,
        input: Value,
    },
    PermissionResponse {
        id: String,
        approved: bool,
    },
    Error {
        message: String,
    },
}

pub fn convert_session_update(
    raw: &Value,
    sessio_runtime_session_id: &str,
    turn_id: &str,
) -> Result<Option<AgentRuntimeEventPayload>> {
    let method = raw.get("method").and_then(Value::as_str);
    if method != Some("session/update") {
        return Ok(None);
    }
    let params = raw
        .get("params")
        .ok_or_else(|| anyhow::anyhow!("ACP session/update missing params"))?;
    let update = params
        .get("update")
        .ok_or_else(|| anyhow::anyhow!("ACP session/update missing update"))?;
    let kind = update
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("ACP update missing kind"))?;

    let event = match kind {
        "agent_message_chunk" => AgentRuntimeEventPayload::TextDelta {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            text: text_content(update)?,
        },
        "thought_chunk" | "thinking_chunk" => AgentRuntimeEventPayload::ReasoningDelta {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            text: text_content(update)?,
        },
        "tool_call" => AgentRuntimeEventPayload::ToolStarted {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            tool_id: update
                .get("toolCallId")
                .or_else(|| update.get("tool_call_id"))
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string(),
            name: update
                .get("title")
                .or_else(|| update.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("Tool")
                .to_string(),
            input: update.get("rawInput").or_else(|| update.get("input")).cloned(),
        },
        "tool_call_update" => AgentRuntimeEventPayload::ToolOutputDelta {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            tool_id: update
                .get("toolCallId")
                .or_else(|| update.get("tool_call_id"))
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string(),
            delta: text_content(update)?,
        },
        "permission_request" => AgentRuntimeEventPayload::PermissionRequested {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            request_id: permission_request_id(update),
            tool_name: update
                .get("toolName")
                .or_else(|| update.get("tool_name"))
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string(),
            input: update.get("input").cloned(),
        },
        "permission_response" | "permission_resolved" => {
            AgentRuntimeEventPayload::PermissionResolved {
                sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
                turn_id: turn_id.to_string(),
                request_id: permission_request_id(update),
                approved: update
                    .get("approved")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }
        }
        "error" => AgentRuntimeEventPayload::TurnError {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            error: RuntimeError::new(
                "acp_update_error",
                update
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("ACP update error"),
            ),
        },
        _ => return Ok(None),
    };
    Ok(Some(event))
}

fn text_content(update: &Value) -> Result<String> {
    if let Some(text) = update.get("text").and_then(Value::as_str) {
        return Ok(text.to_string());
    }
    let Some(content) = update.get("content") else {
        bail!("ACP text update missing content");
    };
    if let Some(text) = content.as_str() {
        return Ok(text.to_string());
    }
    if let Some(text) = content.get("text").and_then(Value::as_str) {
        return Ok(text.to_string());
    }
    if let Some(items) = content.as_array() {
        let mut out = String::new();
        for item in items {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                out.push_str(text);
            }
        }
        return Ok(out);
    }
    bail!("ACP content is not text")
}

fn permission_request_id(update: &Value) -> String {
    update
        .get("permissionRequestId")
        .or_else(|| update.get("permission_request_id"))
        .or_else(|| update.get("requestId"))
        .or_else(|| update.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or("permission")
        .to_string()
}

fn delta_session_id_placeholder() -> &'static str {
    "fake-acp-session"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_agent_message_chunk_to_text_delta() {
        let raw = fake_session_update(AcpFakeUpdate::AgentMessageChunk {
            content: "hello".to_string(),
        });
        let event = convert_session_update(&raw, "sess", "turn")
            .unwrap()
            .unwrap();
        match event {
            AgentRuntimeEventPayload::TextDelta { text, .. } => assert_eq!(text, "hello"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn converts_permission_request_and_response() {
        let raw = fake_session_update(AcpFakeUpdate::PermissionRequest {
            id: "perm-1".to_string(),
            tool_name: "fake_write".to_string(),
            input: json!({ "path": "example.txt" }),
        });
        let event = convert_session_update(&raw, "sess", "turn")
            .unwrap()
            .unwrap();
        match event {
            AgentRuntimeEventPayload::PermissionRequested {
                request_id,
                tool_name,
                input,
                ..
            } => {
                assert_eq!(request_id, "perm-1");
                assert_eq!(tool_name, "fake_write");
                assert_eq!(input, Some(json!({ "path": "example.txt" })));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let raw = fake_session_update(AcpFakeUpdate::PermissionResponse {
            id: "perm-1".to_string(),
            approved: true,
        });
        let event = convert_session_update(&raw, "sess", "turn")
            .unwrap()
            .unwrap();
        match event {
            AgentRuntimeEventPayload::PermissionResolved {
                request_id,
                approved,
                ..
            } => {
                assert_eq!(request_id, "perm-1");
                assert!(approved);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
