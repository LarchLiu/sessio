use agent_client_protocol::schema::{
    ContentBlock, ContentChunk, PermissionOption, PermissionOptionKind, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionNotification, SessionUpdate, TextContent, ToolCall, ToolCallContent, ToolCallUpdate,
    ToolCallUpdateFields,
};
use anyhow::{bail, Result};
use serde_json::Value;

use super::types::{AgentRuntimeEventPayload, RuntimeError};

pub fn fake_session_notification(update: AcpFakeSessionUpdate) -> SessionNotification {
    let update = match update {
        AcpFakeSessionUpdate::AgentMessageChunk { content } => {
            SessionUpdate::AgentMessageChunk(text_chunk(content))
        }
        AcpFakeSessionUpdate::ThoughtChunk { content } => {
            SessionUpdate::AgentThoughtChunk(text_chunk(content))
        }
        AcpFakeSessionUpdate::ToolCall { id, title, input } => {
            SessionUpdate::ToolCall(ToolCall::new(id, title).raw_input(input))
        }
        AcpFakeSessionUpdate::ToolCallOutput { id, output } => {
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                id,
                ToolCallUpdateFields::new()
                    .content(vec![ToolCallContent::from(output)])
                    .status(agent_client_protocol::schema::ToolCallStatus::Completed),
            ))
        }
        AcpFakeSessionUpdate::Error { message } => {
            SessionUpdate::AgentMessageChunk(text_chunk(format!("Runtime error: {message}")))
        }
    };
    SessionNotification::new("fake-acp-session", update)
}

pub fn fake_permission_request(
    session_id: &str,
    request_id: String,
    tool_name: String,
    input: Option<Value>,
) -> RequestPermissionRequest {
    RequestPermissionRequest::new(
        session_id.to_string(),
        ToolCallUpdate::new(
            request_id,
            ToolCallUpdateFields::new()
                .title(tool_name)
                .raw_input(input)
                .status(agent_client_protocol::schema::ToolCallStatus::Pending),
        ),
        vec![
            PermissionOption::new("allow_once", "Allow once", PermissionOptionKind::AllowOnce),
            PermissionOption::new(
                "reject_once",
                "Reject once",
                PermissionOptionKind::RejectOnce,
            ),
        ],
    )
}

pub fn permission_response_from_decision(
    request: &RequestPermissionRequest,
    approved: bool,
) -> RequestPermissionResponse {
    let option = request
        .options
        .iter()
        .find(|option| permission_option_matches(option.kind, approved));

    match option {
        Some(option) => RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new(option.option_id.clone()),
        )),
        None => RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
    }
}

pub enum AcpFakeSessionUpdate {
    AgentMessageChunk {
        content: String,
    },
    ThoughtChunk {
        content: String,
    },
    ToolCall {
        id: String,
        title: String,
        input: Option<Value>,
    },
    ToolCallOutput {
        id: String,
        output: String,
    },
    Error {
        message: String,
    },
}

pub fn convert_session_notification(
    notification: &SessionNotification,
    sessio_runtime_session_id: &str,
    turn_id: &str,
) -> Result<Option<AgentRuntimeEventPayload>> {
    let event = match &notification.update {
        SessionUpdate::UserMessageChunk(_) => return Ok(None),
        SessionUpdate::AgentMessageChunk(chunk) => AgentRuntimeEventPayload::TextDelta {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            text: content_chunk_text(chunk)?,
        },
        SessionUpdate::AgentThoughtChunk(chunk) => AgentRuntimeEventPayload::ReasoningDelta {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            text: content_chunk_text(chunk)?,
        },
        SessionUpdate::ToolCall(tool_call) => AgentRuntimeEventPayload::ToolStarted {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            tool_id: tool_call.tool_call_id.to_string(),
            name: tool_call.title.clone(),
            input: tool_call.raw_input.clone(),
        },
        SessionUpdate::ToolCallUpdate(tool_call_update) => {
            AgentRuntimeEventPayload::ToolOutputDelta {
                sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
                turn_id: turn_id.to_string(),
                tool_id: tool_call_update.tool_call_id.to_string(),
                delta: tool_call_update_text(tool_call_update)?,
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(event))
}

pub fn convert_permission_request(
    request: &RequestPermissionRequest,
    sessio_runtime_session_id: &str,
    turn_id: &str,
    request_id: &str,
) -> Result<AgentRuntimeEventPayload> {
    Ok(AgentRuntimeEventPayload::PermissionRequested {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.to_string(),
        request_id: request_id.to_string(),
        tool_name: request
            .tool_call
            .fields
            .title
            .clone()
            .unwrap_or_else(|| "tool".to_string()),
        input: request.tool_call.fields.raw_input.clone(),
    })
}

pub fn permission_resolved_event(
    sessio_runtime_session_id: &str,
    turn_id: &str,
    request_id: &str,
    approved: bool,
) -> AgentRuntimeEventPayload {
    AgentRuntimeEventPayload::PermissionResolved {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.to_string(),
        request_id: request_id.to_string(),
        approved,
    }
}

pub fn turn_error_event(
    sessio_runtime_session_id: &str,
    turn_id: &str,
    code: impl Into<String>,
    message: impl Into<String>,
) -> AgentRuntimeEventPayload {
    AgentRuntimeEventPayload::TurnError {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.to_string(),
        error: RuntimeError::new(code, message),
    }
}

fn text_chunk(text: impl Into<String>) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
}

fn content_chunk_text(chunk: &ContentChunk) -> Result<String> {
    content_block_text(&chunk.content)
}

fn content_block_text(content: &ContentBlock) -> Result<String> {
    match content {
        ContentBlock::Text(text) => Ok(text.text.clone()),
        ContentBlock::Image(image) => Ok(format!("[image: {}]", image.mime_type)),
        ContentBlock::Audio(audio) => Ok(format!("[audio: {}]", audio.mime_type)),
        ContentBlock::ResourceLink(resource) => {
            serde_json::to_string(resource).map_err(anyhow::Error::from)
        }
        ContentBlock::Resource(resource) => {
            serde_json::to_string(resource).map_err(anyhow::Error::from)
        }
        _ => bail!("unsupported ACP content block"),
    }
}

fn tool_call_update_text(update: &ToolCallUpdate) -> Result<String> {
    if let Some(content) = &update.fields.content {
        return tool_call_content_text(content);
    }
    if let Some(raw_output) = &update.fields.raw_output {
        return value_text(raw_output);
    }
    if let Some(raw_input) = &update.fields.raw_input {
        return value_text(raw_input);
    }
    Ok(String::new())
}

fn tool_call_content_text(content: &[ToolCallContent]) -> Result<String> {
    let mut text = String::new();
    for item in content {
        match item {
            ToolCallContent::Content(content) => {
                text.push_str(&content_block_text(&content.content)?);
            }
            ToolCallContent::Diff(diff) => {
                text.push_str(&format!("[diff: {}]", diff.path.display()));
            }
            ToolCallContent::Terminal(terminal) => {
                text.push_str(&format!("[terminal: {}]", terminal.terminal_id));
            }
            _ => {}
        }
    }
    Ok(text)
}

fn value_text(value: &Value) -> Result<String> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    serde_json::to_string(value).map_err(anyhow::Error::from)
}

fn permission_option_matches(kind: PermissionOptionKind, approved: bool) -> bool {
    matches!(
        (approved, kind),
        (
            true,
            PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
        ) | (
            false,
            PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_agent_message_chunk_to_text_delta() {
        let notification = fake_session_notification(AcpFakeSessionUpdate::AgentMessageChunk {
            content: "hello".to_string(),
        });
        let event = convert_session_notification(&notification, "sess", "turn")
            .unwrap()
            .unwrap();
        match event {
            AgentRuntimeEventPayload::TextDelta { text, .. } => assert_eq!(text, "hello"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn converts_tool_call_and_update() {
        let notification = fake_session_notification(AcpFakeSessionUpdate::ToolCall {
            id: "tool-1".to_string(),
            title: "fake_lookup".to_string(),
            input: Some(json!({ "query": "hi" })),
        });
        let event = convert_session_notification(&notification, "sess", "turn")
            .unwrap()
            .unwrap();
        match event {
            AgentRuntimeEventPayload::ToolStarted {
                tool_id,
                name,
                input,
                ..
            } => {
                assert_eq!(tool_id, "tool-1");
                assert_eq!(name, "fake_lookup");
                assert_eq!(input, Some(json!({ "query": "hi" })));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let notification = fake_session_notification(AcpFakeSessionUpdate::ToolCallOutput {
            id: "tool-1".to_string(),
            output: "done".to_string(),
        });
        let event = convert_session_notification(&notification, "sess", "turn")
            .unwrap()
            .unwrap();
        match event {
            AgentRuntimeEventPayload::ToolOutputDelta { tool_id, delta, .. } => {
                assert_eq!(tool_id, "tool-1");
                assert_eq!(delta, "done");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn converts_permission_request_and_response() {
        let request = fake_permission_request(
            "acp-session",
            "perm-1".to_string(),
            "fake_write".to_string(),
            Some(json!({ "path": "example.txt" })),
        );
        let event = convert_permission_request(&request, "sess", "turn", "perm-1").unwrap();
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

        let response = permission_response_from_decision(&request, true);
        assert!(matches!(
            response.outcome,
            RequestPermissionOutcome::Selected(_)
        ));
    }
}
