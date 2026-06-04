use agent_client_protocol::schema::{
    ContentBlock, ContentChunk, PermissionOption, PermissionOptionKind, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionNotification, SessionUpdate, TextContent, ToolCall, ToolCallContent, ToolCallStatus,
    ToolCallUpdate, ToolCallUpdateFields,
};
use anyhow::Result;
use serde_json::Value;

use super::types::{AcpProtocolMessage, AgentRuntimeEventPayload, RuntimeError};

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
        AcpFakeSessionUpdate::ToolCallInputUpdate { id, input } => SessionUpdate::ToolCallUpdate(
            ToolCallUpdate::new(id, ToolCallUpdateFields::new().raw_input(Some(input))),
        ),
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
    option_id: &str,
) -> RequestPermissionResponse {
    let option = request
        .options
        .iter()
        .find(|option| option.option_id.to_string() == option_id);

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
    ToolCallInputUpdate {
        id: String,
        input: Value,
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
            data: serde_json::to_value(tool_call)?,
        },
        SessionUpdate::ToolCallUpdate(tool_call_update) => {
            tool_call_update_event(tool_call_update, sessio_runtime_session_id, turn_id)?
        }
        SessionUpdate::Plan(plan) => {
            session_update_event("plan", plan, sessio_runtime_session_id, turn_id)?
        }
        SessionUpdate::AvailableCommandsUpdate(update) => session_update_event(
            "available_commands",
            update,
            sessio_runtime_session_id,
            turn_id,
        )?,
        SessionUpdate::CurrentModeUpdate(update) => {
            session_update_event("current_mode", update, sessio_runtime_session_id, turn_id)?
        }
        SessionUpdate::ConfigOptionUpdate(update) => {
            session_update_event("config_options", update, sessio_runtime_session_id, turn_id)?
        }
        SessionUpdate::SessionInfoUpdate(update) => {
            session_update_event("session_info", update, sessio_runtime_session_id, turn_id)?
        }
        _ => session_update_event(
            "unknown",
            &notification.update,
            sessio_runtime_session_id,
            turn_id,
        )?,
    };
    Ok(Some(event))
}

/// The protocol-message envelope fields for `acp_protocol_event`, grouped so
/// the builder stays under clippy's argument limit. `direction`/`message_kind`/
/// `method` are always static labels at call sites; the rest are per-message ids.
pub struct AcpProtocolEnvelope {
    pub direction: &'static str,
    pub message_kind: &'static str,
    pub method: &'static str,
    pub acp_session_id: Option<String>,
    pub turn_id: Option<String>,
    pub request_id: Option<String>,
    pub update_type: Option<String>,
}

pub fn acp_protocol_event<T: serde::Serialize>(
    sessio_runtime_session_id: &str,
    envelope: AcpProtocolEnvelope,
    data: &T,
) -> Result<AgentRuntimeEventPayload> {
    let AcpProtocolEnvelope {
        direction,
        message_kind,
        method,
        acp_session_id,
        turn_id,
        request_id,
        update_type,
    } = envelope;
    Ok(AgentRuntimeEventPayload::AcpProtocolMessage {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.clone(),
        message: AcpProtocolMessage {
            direction: direction.to_string(),
            message_kind: message_kind.to_string(),
            method: method.to_string(),
            protocol_version: Some("1".to_string()),
            acp_session_id,
            turn_id,
            request_id,
            update_type,
            data: serde_json::to_value(data)?,
        },
    })
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
        data: serde_json::to_value(request)?,
    })
}

pub fn permission_resolved_event(
    sessio_runtime_session_id: &str,
    turn_id: &str,
    request_id: &str,
    option_id: Option<String>,
) -> AgentRuntimeEventPayload {
    let approved = option_id
        .as_deref()
        .map(is_allow_permission_option_id)
        .unwrap_or(false);
    AgentRuntimeEventPayload::PermissionResolved {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.to_string(),
        request_id: request_id.to_string(),
        approved,
        option_id,
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
        ContentBlock::Image(image) => Ok(format!(
            "[image: {}{}]",
            image.mime_type,
            image
                .uri
                .as_ref()
                .map(|uri| format!(" {uri}"))
                .unwrap_or_default()
        )),
        ContentBlock::Audio(audio) => Ok(format!("[audio: {}]", audio.mime_type)),
        ContentBlock::ResourceLink(resource) => {
            Ok(format!("[resource: {} {}]", resource.name, resource.uri))
        }
        ContentBlock::Resource(resource) => {
            serde_json::to_string(resource).map_err(anyhow::Error::from)
        }
        _ => serde_json::to_string(content).map_err(anyhow::Error::from),
    }
}

fn tool_call_update_text(update: &ToolCallUpdate) -> Result<String> {
    if let Some(content) = &update.fields.content {
        return tool_call_content_text(content);
    }
    if let Some(raw_output) = &update.fields.raw_output {
        return value_text(raw_output);
    }
    Ok(String::new())
}

fn tool_call_update_event(
    update: &ToolCallUpdate,
    sessio_runtime_session_id: &str,
    turn_id: &str,
) -> Result<AgentRuntimeEventPayload> {
    let delta = tool_call_update_text(update)?;
    if !delta.is_empty() {
        return Ok(AgentRuntimeEventPayload::ToolOutputDelta {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            tool_id: update.tool_call_id.to_string(),
            delta,
            data: Some(serde_json::to_value(update)?),
        });
    }
    let delta = update
        .fields
        .raw_input
        .as_ref()
        .map(value_text)
        .transpose()?
        .unwrap_or_default();
    if !delta.is_empty() {
        return Ok(AgentRuntimeEventPayload::ToolInputDelta {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            tool_id: update.tool_call_id.to_string(),
            delta,
            data: Some(serde_json::to_value(update)?),
        });
    }
    if let Some(status) = update.fields.status {
        return Ok(AgentRuntimeEventPayload::ToolStatusChanged {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            tool_id: update.tool_call_id.to_string(),
            status: tool_call_status_name(status).to_string(),
            data: Some(serde_json::to_value(update)?),
        });
    }
    Ok(AgentRuntimeEventPayload::ToolInputDelta {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.to_string(),
        tool_id: update.tool_call_id.to_string(),
        delta,
        data: Some(serde_json::to_value(update)?),
    })
}

fn session_update_event<T: serde::Serialize>(
    update_type: &str,
    value: T,
    sessio_runtime_session_id: &str,
    turn_id: &str,
) -> Result<AgentRuntimeEventPayload> {
    Ok(AgentRuntimeEventPayload::SessionUpdate {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.to_string(),
        update_type: update_type.to_string(),
        data: serde_json::to_value(value)?,
    })
}

fn tool_call_status_name(status: ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Pending => "pending",
        ToolCallStatus::InProgress => "in_progress",
        ToolCallStatus::Completed => "completed",
        ToolCallStatus::Failed => "failed",
        _ => "unknown",
    }
}

fn tool_call_content_text(content: &[ToolCallContent]) -> Result<String> {
    let mut text = String::new();
    for (idx, item) in content.iter().enumerate() {
        if idx > 0 && !text.ends_with('\n') {
            text.push('\n');
        }
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

pub fn permission_option_id_from_approved(
    request: &RequestPermissionRequest,
    approved: bool,
) -> Option<String> {
    request
        .options
        .iter()
        .find(|option| permission_option_matches(option.kind, approved))
        .map(|option| option.option_id.to_string())
}

fn is_allow_permission_option_id(option_id: &str) -> bool {
    let normalized = option_id.to_ascii_lowercase();
    normalized.starts_with("allow") || normalized.contains("allow_")
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

        let notification = fake_session_notification(AcpFakeSessionUpdate::ToolCallInputUpdate {
            id: "tool-1".to_string(),
            input: json!({ "query": "latest release" }),
        });
        let event = convert_session_notification(&notification, "sess", "turn")
            .unwrap()
            .unwrap();
        match event {
            AgentRuntimeEventPayload::ToolInputDelta { tool_id, delta, .. } => {
                assert_eq!(tool_id, "tool-1");
                assert!(delta.contains("latest release"));
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

        let response = permission_response_from_decision(&request, "allow_once");
        assert!(matches!(
            response.outcome,
            RequestPermissionOutcome::Selected(_)
        ));
    }
}
