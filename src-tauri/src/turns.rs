use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::agents::runtime::types::{
    AcpProtocolMessage, AgentAttachment, AgentAttachmentKind, AgentRuntimeEventPayload,
    RuntimeCapabilitySet, RuntimeError, RuntimeTransportKind, RuntimeTurnStatus,
};
use crate::agents::sources::types::HistoryAcpMessage;
use crate::models::{
    is_system_noise, sessio_attachment_marker_name, strip_injected_context, text_content_blocks,
    Agent, SessionContentBlock, SessionHistoryBlock, SessionHistoryPermissionOption,
    SessionHistoryPermissionRequest, SessionHistoryToolCall, SessionHistoryTurn,
};

const MAX_PROTOCOL_MESSAGES: usize = 240;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct AcpCanonicalSessionState {
    pub plan: Option<AcpPlan>,
    pub available_commands: Vec<AcpAvailableCommand>,
    pub current_mode_id: Option<String>,
    pub config_options: Vec<AcpSessionConfigOption>,
    pub session_info: Option<AcpSessionInfo>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpPlan {
    pub entries: Vec<AcpPlanEntry>,
    pub meta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpPlanEntry {
    pub content: String,
    pub priority: String,
    pub status: String,
    pub meta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpAvailableCommand {
    pub name: String,
    pub description: String,
    pub input: Value,
    pub meta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpSessionConfigOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    #[serde(rename = "type")]
    pub option_type: Option<String>,
    pub current_value: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Value>,
    pub meta: Value,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpSessionInfo {
    pub title: Option<String>,
    pub updated_at: Option<String>,
    pub meta: Value,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRuntimeSessionSnapshot {
    pub sessio_runtime_session_id: String,
    pub agent: Agent,
    pub agent_runtime_session_id: String,
    pub transport: RuntimeTransportKind,
    pub workspace_path: String,
    pub capabilities: RuntimeCapabilitySet,
    pub turns: Vec<SessionHistoryTurn>,
    pub session_state: AcpCanonicalSessionState,
    pub protocol_messages: Vec<AcpProtocolMessage>,
    pub ended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRuntimeTurnSnapshotEvent {
    pub sequence: u64,
    pub timestamp: i64,
    pub session: LiveRuntimeSessionSnapshot,
}

#[derive(Debug, Clone)]
pub struct RuntimeTurnState {
    pub sessio_runtime_session_id: String,
    pub agent: Agent,
    pub agent_runtime_session_id: String,
    pub transport: RuntimeTransportKind,
    pub workspace_path: String,
    pub capabilities: RuntimeCapabilitySet,
    pub turns: Vec<SessionHistoryTurn>,
    pub session_state: AcpCanonicalSessionState,
    pub protocol_messages: Vec<AcpProtocolMessage>,
    pub ended: bool,
}

impl RuntimeTurnState {
    pub fn new(
        sessio_runtime_session_id: impl Into<String>,
        agent: Agent,
        agent_runtime_session_id: impl Into<String>,
        transport: RuntimeTransportKind,
        workspace_path: impl Into<String>,
        capabilities: RuntimeCapabilitySet,
    ) -> Self {
        Self {
            sessio_runtime_session_id: sessio_runtime_session_id.into(),
            agent,
            agent_runtime_session_id: agent_runtime_session_id.into(),
            transport,
            workspace_path: workspace_path.into(),
            capabilities,
            turns: Vec::new(),
            session_state: AcpCanonicalSessionState::default(),
            protocol_messages: Vec::new(),
            ended: false,
        }
    }

    pub fn snapshot(&self) -> LiveRuntimeSessionSnapshot {
        LiveRuntimeSessionSnapshot {
            sessio_runtime_session_id: self.sessio_runtime_session_id.clone(),
            agent: self.agent,
            agent_runtime_session_id: self.agent_runtime_session_id.clone(),
            transport: self.transport,
            workspace_path: self.workspace_path.clone(),
            capabilities: self.capabilities.clone(),
            turns: self.turns.clone(),
            session_state: self.session_state.clone(),
            protocol_messages: self.protocol_messages.clone(),
            ended: self.ended,
        }
    }
}

pub fn history_user_message(text: impl Into<String>, timestamp: Option<i64>) -> AcpProtocolMessage {
    history_prompt_message(history_text_content(text.into()), timestamp)
}

pub fn history_prompt_message(content: Vec<Value>, timestamp: Option<i64>) -> AcpProtocolMessage {
    history_protocol_message(
        "client_to_agent",
        "request",
        "session/prompt",
        None,
        None,
        json!({
            "prompt": content,
            "meta": history_message_meta(timestamp),
        }),
    )
}

pub fn history_assistant_message(
    text: impl Into<String>,
    timestamp: Option<i64>,
) -> AcpProtocolMessage {
    history_content_update(
        "agent_message_chunk",
        history_text_content(text.into()),
        timestamp,
    )
}

pub fn history_thought_message(
    text: impl Into<String>,
    timestamp: Option<i64>,
) -> AcpProtocolMessage {
    history_content_update(
        "agent_thought_chunk",
        history_text_content(text.into()),
        timestamp,
    )
}

pub fn history_content_update(
    update_type: &str,
    content: Vec<Value>,
    timestamp: Option<i64>,
) -> AcpProtocolMessage {
    history_session_update_message(
        update_type,
        json!({
            "sessionUpdate": update_type,
            "content": content,
            "meta": history_message_meta(timestamp),
        }),
        timestamp,
    )
}

pub fn history_tool_call_message(
    tool_call_id: Option<String>,
    title: impl Into<String>,
    raw_input: Value,
    timestamp: Option<i64>,
) -> AcpProtocolMessage {
    let tool_call_id =
        tool_call_id.unwrap_or_else(|| history_synthetic_id("history-tool", timestamp));
    let title = title.into();
    history_session_update_message(
        "tool_call",
        json!({
            "sessionUpdate": "tool_call",
            "toolCallId": tool_call_id,
            "title": title,
            "kind": if title == "TodoWrite" { "task_list" } else { "tool_call" },
            "status": if title == "TodoWrite" { "completed" } else { "pending" },
            "rawInput": raw_input,
            "meta": history_message_meta(timestamp),
        }),
        timestamp,
    )
}

pub fn history_todo_message(
    todos: Value,
    timestamp: Option<i64>,
    tool_call_id: Option<String>,
) -> AcpProtocolMessage {
    history_tool_call_message(
        tool_call_id,
        "TodoWrite",
        normalize_task_list_update_value(todos, "todos"),
        timestamp,
    )
}

pub fn history_tool_result_message(
    tool_call_id: Option<String>,
    output: Value,
    timestamp: Option<i64>,
) -> AcpProtocolMessage {
    let tool_call_id =
        tool_call_id.unwrap_or_else(|| history_synthetic_id("history-tool-result", timestamp));
    history_session_update_message(
        "tool_call_update",
        json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": tool_call_id,
            "status": "completed",
            "rawOutput": output,
            "meta": history_message_meta(timestamp),
        }),
        timestamp,
    )
}

pub fn history_session_update_message(
    update_type: impl Into<String>,
    mut update: Value,
    timestamp: Option<i64>,
) -> AcpProtocolMessage {
    let update_type = update_type.into();
    if let Some(object) = update.as_object_mut() {
        object
            .entry("sessionUpdate".to_string())
            .or_insert_with(|| Value::String(update_type.clone()));
        object
            .entry("meta".to_string())
            .or_insert_with(|| history_message_meta(timestamp));
    }
    history_protocol_message(
        "agent_to_client",
        "notification",
        "session/update",
        None,
        Some(update_type),
        json!({ "update": update }),
    )
}

pub fn history_permission_request_message(
    request_id: Option<String>,
    tool_name: impl Into<String>,
    input: Value,
    options: Vec<Value>,
    selected_option_id: Option<String>,
    cancelled: Option<bool>,
    tool_call: Option<Value>,
    raw: Value,
    timestamp: Option<i64>,
) -> Vec<AcpProtocolMessage> {
    let request_id =
        request_id.unwrap_or_else(|| history_synthetic_id("history-permission", timestamp));
    let tool_name = tool_name.into();
    let tool_call = tool_call.unwrap_or_else(|| {
        json!({
            "toolCallId": request_id,
            "fields": {
                "title": tool_name,
                "rawInput": input,
                "status": "pending",
            }
        })
    });
    let mut messages = vec![history_protocol_message(
        "agent_to_client",
        "request",
        "session/request_permission",
        Some(request_id.clone()),
        None,
        json!({
            "toolCall": tool_call,
            "options": options,
            "raw": raw,
            "meta": history_message_meta(timestamp),
        }),
    )];
    if selected_option_id.is_some() || cancelled.unwrap_or(false) {
        let outcome = selected_option_id
            .map(|option_id| json!({ "outcome": "selected", "optionId": option_id }))
            .unwrap_or_else(|| json!({ "outcome": "cancelled" }));
        messages.push(history_protocol_message(
            "client_to_agent",
            "response",
            "session/request_permission",
            Some(request_id),
            None,
            json!({
                "outcome": outcome,
                "meta": history_message_meta(timestamp),
            }),
        ));
    }
    messages
}

fn history_protocol_message(
    direction: impl Into<String>,
    message_kind: impl Into<String>,
    method: impl Into<String>,
    request_id: Option<String>,
    update_type: Option<String>,
    data: Value,
) -> AcpProtocolMessage {
    AcpProtocolMessage {
        direction: direction.into(),
        message_kind: message_kind.into(),
        method: method.into(),
        protocol_version: Some("1".to_string()),
        acp_session_id: None,
        turn_id: None,
        request_id,
        update_type,
        data,
    }
}

fn history_text_content(text: String) -> Vec<Value> {
    text_content_blocks(&text)
        .into_iter()
        .map(history_content_block_value)
        .collect()
}

fn history_content_block_value(block: SessionContentBlock) -> Value {
    match block.kind.as_str() {
        "text" => json!({ "type": "text", "text": block.text.unwrap_or_default() }),
        "image" => json!({
            "type": "image",
            "uri": block.uri,
            "mimeType": block.mime_type,
            "meta": block.meta,
        }),
        "audio" => json!({
            "type": "audio",
            "uri": block.uri,
            "mimeType": block.mime_type,
            "meta": block.meta,
        }),
        "resource_link" => json!({
            "type": "resource_link",
            "uri": block.uri.unwrap_or_default(),
            "name": block.name,
            "title": block.title,
            "description": block.description,
            "mimeType": block.mime_type,
            "size": block.size,
            "meta": block.meta,
        }),
        "resource" => json!({
            "type": "resource",
            "uri": block.uri,
            "name": block.name,
            "mimeType": block.mime_type,
            "text": block.text,
            "blob": block.blob,
            "resource": block.resource,
            "meta": block.meta,
        }),
        other => json!({
            "type": other,
            "uri": block.uri,
            "name": block.name,
            "mimeType": block.mime_type,
            "text": block.text,
            "blob": block.blob,
            "resource": block.resource,
            "meta": block.meta,
        }),
    }
}

fn history_message_meta(timestamp: Option<i64>) -> Value {
    json!({
        "source": "history",
        "synthetic": true,
        "timestamp": timestamp,
    })
}

fn history_synthetic_id(prefix: &str, timestamp: Option<i64>) -> String {
    timestamp
        .map(|value| format!("{prefix}-{value}"))
        .unwrap_or_else(|| prefix.to_string())
}

pub fn session_history_turns_from_acp_messages(
    rows: &[HistoryAcpMessage],
) -> Vec<SessionHistoryTurn> {
    let mut turns = Vec::new();
    let mut current: Option<TurnBuilder> = None;

    for (index, row) in rows.iter().enumerate() {
        let timestamp = row.timestamp.unwrap_or(index as i64);
        let message = &row.message;
        let starts_turn = is_history_turn_start(message);
        if starts_turn || current.is_none() {
            if let Some(turn) = current.take() {
                turns.push(turn.finish_history());
            }
            current = Some(TurnBuilder::new(
                format!("history-turn-{index}"),
                timestamp,
                RuntimeTurnStatus::Completed,
            ));
        }
        let turn = current.get_or_insert_with(|| {
            TurnBuilder::new(
                format!("history-turn-{index}"),
                timestamp,
                RuntimeTurnStatus::Completed,
            )
        });
        turn.touch(timestamp);
        turn.turn.protocol_messages =
            append_turn_protocol_message(&turn.turn.protocol_messages, message);
        apply_acp_message_to_turn(&mut turn.turn, message, timestamp);
    }

    if let Some(turn) = current {
        turns.push(turn.finish_history());
    }
    turns
}

fn is_history_turn_start(message: &AcpProtocolMessage) -> bool {
    message.method == "session/prompt"
        && message.direction == "client_to_agent"
        && message.message_kind == "request"
}

pub fn apply_runtime_event_to_state(
    state: &mut RuntimeTurnState,
    payload: &AgentRuntimeEventPayload,
    timestamp: i64,
) {
    match payload {
        AgentRuntimeEventPayload::SessionStarted {
            agent,
            agent_runtime_session_id,
            transport,
            workspace_path,
            capabilities,
            ..
        } => {
            state.agent = *agent;
            state.agent_runtime_session_id = agent_runtime_session_id.clone();
            state.transport = *transport;
            state.workspace_path = workspace_path.clone();
            state.capabilities = capabilities.clone();
            state.ended = false;
        }
        AgentRuntimeEventPayload::TurnStarted { turn_id, .. } => {
            state
                .upsert_turn(turn_id, timestamp)
                .set_status(RuntimeTurnStatus::Streaming, timestamp);
        }
        AgentRuntimeEventPayload::TurnCompleted { turn_id, .. } => {
            let turn = state.upsert_turn(turn_id, timestamp);
            turn.set_status(RuntimeTurnStatus::Completed, timestamp);
            postprocess_turn(turn);
        }
        AgentRuntimeEventPayload::TurnCancelled { turn_id, .. } => {
            let turn = state.upsert_turn(turn_id, timestamp);
            turn.set_status(RuntimeTurnStatus::Cancelled, timestamp);
            postprocess_turn(turn);
        }
        AgentRuntimeEventPayload::TurnError { turn_id, error, .. } => {
            let turn = state.upsert_turn(turn_id, timestamp);
            turn.set_error(error.clone(), timestamp);
            postprocess_turn(turn);
        }
        AgentRuntimeEventPayload::PermissionResolved {
            turn_id,
            request_id,
            approved,
            option_id,
            ..
        } => {
            state.upsert_turn(turn_id, timestamp).resolve_permission(
                request_id,
                *approved,
                option_id.clone(),
                timestamp,
            );
        }
        AgentRuntimeEventPayload::PermissionRequested {
            turn_id,
            request_id,
            tool_name,
            input,
            data,
            ..
        } => {
            let permission =
                permission_from_runtime_event(request_id, tool_name, input.clone(), data.clone());
            upsert_permission(state.upsert_turn(turn_id, timestamp), permission, timestamp);
        }
        AgentRuntimeEventPayload::AcpProtocolMessage {
            turn_id, message, ..
        } => {
            state.protocol_messages = append_protocol_message(&state.protocol_messages, message);
            state.session_state = apply_session_level_message(&state.session_state, message);
            if let Some(turn_id) = turn_id.as_deref() {
                let turn = state.upsert_turn(turn_id, timestamp);
                turn.protocol_messages =
                    append_turn_protocol_message(&turn.protocol_messages, message);
                turn.touch(timestamp);
                apply_acp_message_to_turn(turn, message, timestamp);
            }
        }
        AgentRuntimeEventPayload::SessionEnded { .. } => {
            state.ended = true;
        }
        _ => {}
    }
}

pub fn apply_optimistic_user_message(
    state: &mut RuntimeTurnState,
    turn_id: &str,
    text: &str,
    attachments: &[AgentAttachment],
    timestamp: i64,
) {
    let turn = state.upsert_turn(turn_id, timestamp);
    turn.set_status(RuntimeTurnStatus::Streaming, timestamp);
    if turn.blocks.iter().any(|block| block.kind == "user") {
        return;
    }
    let blocks = optimistic_user_content_blocks(text, attachments);
    if blocks.is_empty() {
        return;
    }
    turn.blocks.push(SessionHistoryBlock {
        kind: "user".to_string(),
        blocks,
        raw: Some(json!({ "optimistic": true, "text": text })),
        tool_id: None,
        request_id: None,
        update_type: None,
        data: None,
        error: None,
        timestamp: Some(timestamp),
    });
}

impl RuntimeTurnState {
    fn upsert_turn(&mut self, turn_id: &str, timestamp: i64) -> &mut SessionHistoryTurn {
        if let Some(index) = self.turns.iter().position(|turn| turn.turn_id == turn_id) {
            return &mut self.turns[index];
        }
        self.turns.push(new_turn(
            turn_id.to_string(),
            timestamp,
            RuntimeTurnStatus::Pending,
        ));
        self.turns.last_mut().expect("turn just pushed")
    }
}

trait SessionHistoryTurnExt {
    fn touch(&mut self, timestamp: i64);
    fn set_status(&mut self, status: RuntimeTurnStatus, timestamp: i64);
    fn set_error(&mut self, error: RuntimeError, timestamp: i64);
    fn resolve_permission(
        &mut self,
        request_id: &str,
        approved: bool,
        option_id: Option<String>,
        timestamp: i64,
    );
}

impl SessionHistoryTurnExt for SessionHistoryTurn {
    fn touch(&mut self, timestamp: i64) {
        self.started_at = self.started_at.min(timestamp);
        self.updated_at = self.updated_at.max(timestamp);
    }

    fn set_status(&mut self, status: RuntimeTurnStatus, timestamp: i64) {
        self.status = runtime_turn_status(status).to_string();
        self.touch(timestamp);
    }

    fn set_error(&mut self, error: RuntimeError, timestamp: i64) {
        self.status = runtime_turn_status(RuntimeTurnStatus::Failed).to_string();
        let error_value = serde_json::to_value(&error).unwrap_or(Value::Null);
        self.error = Some(error_value.clone());
        self.updated_at = timestamp;
        self.blocks.push(SessionHistoryBlock {
            kind: "error".to_string(),
            blocks: Vec::new(),
            raw: None,
            tool_id: None,
            request_id: None,
            update_type: None,
            data: None,
            error: Some(error_value),
            timestamp: Some(timestamp),
        });
    }

    fn resolve_permission(
        &mut self,
        request_id: &str,
        approved: bool,
        option_id: Option<String>,
        timestamp: i64,
    ) {
        if let Some(permission) = self
            .permissions
            .iter_mut()
            .find(|permission| permission.request_id == request_id)
        {
            permission.cancelled = false;
            permission.selected_option_id = option_id.or_else(|| {
                permission
                    .options
                    .iter()
                    .find(|option| {
                        if approved {
                            option.kind.starts_with("allow")
                        } else {
                            option.kind.starts_with("reject")
                        }
                    })
                    .map(|option| option.option_id.clone())
            });
        }
        self.touch(timestamp);
    }
}

struct TurnBuilder {
    turn: SessionHistoryTurn,
}

impl TurnBuilder {
    fn new(turn_id: String, timestamp: i64, status: RuntimeTurnStatus) -> Self {
        Self {
            turn: new_turn(turn_id, timestamp, status),
        }
    }

    fn touch(&mut self, timestamp: i64) {
        self.turn.touch(timestamp);
    }

    fn finish_history(mut self) -> SessionHistoryTurn {
        postprocess_turn(&mut self.turn);
        self.turn.status = runtime_turn_status(RuntimeTurnStatus::Completed).to_string();
        self.turn
    }
}

fn new_turn(turn_id: String, timestamp: i64, status: RuntimeTurnStatus) -> SessionHistoryTurn {
    SessionHistoryTurn {
        turn_id,
        status: runtime_turn_status(status).to_string(),
        blocks: Vec::new(),
        tools: Vec::new(),
        permissions: Vec::new(),
        protocol_messages: Vec::new(),
        stop_reason: None,
        error: None,
        started_at: timestamp,
        updated_at: timestamp,
    }
}

fn apply_acp_message_to_turn(
    turn: &mut SessionHistoryTurn,
    message: &AcpProtocolMessage,
    timestamp: i64,
) {
    if message.method == "session/prompt" && message.direction == "client_to_agent" {
        let prompt = as_object(&message.data)
            .get("prompt")
            .cloned()
            .unwrap_or(Value::Null);
        turn.set_status(RuntimeTurnStatus::Streaming, timestamp);
        let blocks = normalize_user_content_blocks(&prompt);
        if !blocks.is_empty() {
            replace_or_append_user_block(turn, blocks, Some(message.data.clone()), timestamp);
        }
        return;
    }

    if message.method == "session/prompt" && message.direction == "agent_to_client" {
        let stop_reason = string_field(&message.data, "stopReason");
        turn.stop_reason = stop_reason.clone();
        turn.set_status(
            if stop_reason.as_deref() == Some("cancelled") {
                RuntimeTurnStatus::Cancelled
            } else {
                RuntimeTurnStatus::Completed
            },
            timestamp,
        );
        return;
    }

    if message.method == "session/request_permission" {
        if message.message_kind == "request" {
            if let Some(permission) = permission_from_message(message) {
                upsert_permission(turn, permission, timestamp);
            }
            return;
        }
        if message.message_kind == "response" {
            let option_id = selected_permission_option_id(&message.data);
            if let Some(latest) = turn.permissions.last_mut() {
                latest.selected_option_id = option_id.clone();
                latest.cancelled = option_id.is_none();
            }
            return;
        }
    }

    if message.method != "session/update" {
        return;
    }
    let update = as_object(&message.data)
        .get("update")
        .cloned()
        .unwrap_or(Value::Null);
    let update_type = session_update_type(&update, message.update_type.as_deref())
        .unwrap_or_else(|| "unknown".to_string());

    match update_type.as_str() {
        "user_message_chunk" => {
            let blocks = normalize_user_content_blocks(&update);
            if !blocks.is_empty() {
                append_content_block(turn, "user", blocks, update, timestamp);
            }
        }
        "agent_message_chunk" => {
            append_content_block(
                turn,
                "assistant",
                normalize_content_blocks(&update, Some("assistant")),
                update,
                timestamp,
            );
            if turn.status == "pending" {
                turn.set_status(RuntimeTurnStatus::Streaming, timestamp);
            }
        }
        "agent_thought_chunk" => {
            append_content_block(
                turn,
                "thought",
                normalize_content_blocks(&update, Some("thought")),
                update,
                timestamp,
            );
        }
        "tool_call" | "tool_call_update" => {
            let tool = tool_from_value(&update, timestamp);
            let tool_id = tool.tool_id.clone();
            upsert_tool(turn, tool);
            ensure_tool_block(turn, tool_id, timestamp);
            postprocess_turn(turn);
        }
        "available_commands" | "current_mode" | "config_options" | "session_info" => {}
        "plan" => turn
            .blocks
            .push(session_update_block(&update_type, update, timestamp)),
        _ => {
            turn.blocks
                .push(session_update_block(&update_type, update, timestamp));
            postprocess_turn(turn);
        }
    }
}

fn apply_session_level_message(
    state: &AcpCanonicalSessionState,
    message: &AcpProtocolMessage,
) -> AcpCanonicalSessionState {
    let mut next = state.clone();
    if let Some(patch) = session_response_state(message) {
        if let Some(config_options) = patch.config_options {
            next.config_options = config_options;
        }
        if let Some(current_mode_id) = patch.current_mode_id {
            next.current_mode_id = current_mode_id;
        }
        if let Some(session_info) = patch.session_info {
            next.session_info = session_info;
        }
        return next;
    }

    if message.method != "session/update" {
        return next;
    }
    let update = as_object(&message.data)
        .get("update")
        .cloned()
        .unwrap_or(Value::Null);
    let Some(update_type) = session_update_type(&update, message.update_type.as_deref()) else {
        return next;
    };
    match update_type.as_str() {
        "plan" => next.plan = Some(normalize_plan(&update)),
        "available_commands" => {
            next.available_commands = array_field(&update, "availableCommands")
                .iter()
                .map(normalize_available_command)
                .collect();
        }
        "current_mode" => next.current_mode_id = string_field(&update, "currentModeId"),
        "config_options" => {
            let options = array_field(&update, "configOptions")
                .iter()
                .map(normalize_session_config_option)
                .collect();
            next.config_options = dedupe_session_config_options(options);
        }
        "session_info" => next.session_info = Some(normalize_session_info(&update)),
        _ => {}
    }
    next
}

#[derive(Default)]
struct SessionStatePatch {
    config_options: Option<Vec<AcpSessionConfigOption>>,
    current_mode_id: Option<Option<String>>,
    session_info: Option<Option<AcpSessionInfo>>,
}

fn session_response_state(message: &AcpProtocolMessage) -> Option<SessionStatePatch> {
    if message.direction != "agent_to_client" || message.message_kind != "response" {
        return None;
    }
    if !matches!(
        message.method.as_str(),
        "session/new" | "session/load" | "session/resume" | "session/fork"
    ) {
        return None;
    }
    let data = &message.data;
    let modes = normalize_mode_config_option(value_field(data, "modes"));
    let models = normalize_model_config_option(value_field(data, "models"));
    let mut options: Vec<AcpSessionConfigOption> = Vec::new();
    if let Some(mode) = modes.clone() {
        options.push(mode);
    }
    if let Some(model) = models {
        options.push(model);
    }
    options.extend(
        array_field(data, "configOptions")
            .iter()
            .map(normalize_session_config_option),
    );

    let mut patch = SessionStatePatch::default();
    if !options.is_empty() {
        patch.config_options = Some(dedupe_session_config_options(options));
    }
    if let Some(mode) = modes {
        patch.current_mode_id = Some(
            mode.current_value
                .as_str()
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        );
    }
    if value_field(data, "sessionId").is_some() || value_field(data, "session_id").is_some() {
        patch.session_info = Some(Some(normalize_session_info(&json!({
            "meta": value_field(data, "meta")
                .or_else(|| value_field(data, "_meta"))
                .unwrap_or(Value::Null)
        }))));
    }
    if patch.config_options.is_some()
        || patch.current_mode_id.is_some()
        || patch.session_info.is_some()
    {
        Some(patch)
    } else {
        None
    }
}

fn append_content_block(
    turn: &mut SessionHistoryTurn,
    kind: &str,
    blocks: Vec<SessionContentBlock>,
    raw: Value,
    timestamp: i64,
) {
    if blocks.is_empty() {
        return;
    }
    if let Some(last) = turn.blocks.last_mut() {
        if last.kind == kind {
            last.blocks = merge_adjacent_text_blocks(
                last.blocks
                    .iter()
                    .cloned()
                    .chain(blocks)
                    .collect::<Vec<_>>(),
            );
            if last.timestamp.is_none() {
                last.timestamp = Some(timestamp);
            }
            turn.touch(timestamp);
            return;
        }
    }
    turn.blocks.push(SessionHistoryBlock {
        kind: kind.to_string(),
        blocks: merge_adjacent_text_blocks(blocks),
        raw: Some(raw),
        tool_id: None,
        request_id: None,
        update_type: None,
        data: None,
        error: None,
        timestamp: Some(timestamp),
    });
    turn.touch(timestamp);
}

fn replace_or_append_user_block(
    turn: &mut SessionHistoryTurn,
    blocks: Vec<SessionContentBlock>,
    raw: Option<Value>,
    timestamp: i64,
) {
    let block = SessionHistoryBlock {
        kind: "user".to_string(),
        blocks,
        raw,
        tool_id: None,
        request_id: None,
        update_type: None,
        data: None,
        error: None,
        timestamp: Some(timestamp),
    };
    if let Some(index) = turn.blocks.iter().position(|item| item.kind == "user") {
        turn.blocks[index] = block;
    } else {
        turn.blocks.insert(0, block);
    }
}

fn session_update_block(update_type: &str, update: Value, timestamp: i64) -> SessionHistoryBlock {
    SessionHistoryBlock {
        kind: "sessionUpdate".to_string(),
        blocks: Vec::new(),
        raw: None,
        tool_id: None,
        request_id: None,
        update_type: Some(update_type.to_string()),
        data: Some(update),
        error: None,
        timestamp: Some(timestamp),
    }
}

fn ensure_tool_block(turn: &mut SessionHistoryTurn, tool_id: String, timestamp: i64) {
    if turn
        .blocks
        .iter()
        .any(|block| block.kind == "tool" && block.tool_id.as_deref() == Some(&tool_id))
    {
        return;
    }
    turn.blocks.push(SessionHistoryBlock {
        kind: "tool".to_string(),
        blocks: Vec::new(),
        raw: None,
        tool_id: Some(tool_id),
        request_id: None,
        update_type: None,
        data: None,
        error: None,
        timestamp: Some(timestamp),
    });
}

fn ensure_permission_block(turn: &mut SessionHistoryTurn, request_id: String, timestamp: i64) {
    if turn
        .blocks
        .iter()
        .any(|block| block.kind == "permission" && block.request_id.as_deref() == Some(&request_id))
    {
        return;
    }
    turn.blocks.push(SessionHistoryBlock {
        kind: "permission".to_string(),
        blocks: Vec::new(),
        raw: None,
        tool_id: None,
        request_id: Some(request_id),
        update_type: None,
        data: None,
        error: None,
        timestamp: Some(timestamp),
    });
}

fn postprocess_turn(turn: &mut SessionHistoryTurn) {
    merge_terminal_polling_tools(turn);
    append_tool_diff_file_edit_block(turn);
    merge_file_edit_blocks(turn);
}

fn merge_terminal_polling_tools(turn: &mut SessionHistoryTurn) {
    let mut remove_tool_ids = HashSet::new();
    let mut running_sessions: Vec<(String, String)> = Vec::new();
    let mut merges: Vec<(String, String, i64)> = Vec::new();

    for index in 0..turn.tools.len() {
        let tool_id = turn.tools[index].tool_id.clone();
        if let Some(polling_session_id) = write_stdin_polling_tool_session_id(&turn.tools[index]) {
            if let Some((_, target_tool_id)) = running_sessions
                .iter()
                .rev()
                .find(|(session_id, _)| session_id == &polling_session_id)
                .cloned()
            {
                let output = tool_output_text(&turn.tools[index]);
                if output.trim().is_empty() {
                    continue;
                }
                merges.push((target_tool_id, output, turn.tools[index].updated_at));
                remove_tool_ids.insert(tool_id);
                continue;
            }
        }
        if let Some(running_session_id) = tool_running_session_id(&turn.tools[index]) {
            running_sessions.push((running_session_id, tool_id));
        }
    }

    for (target_tool_id, output, updated_at) in merges {
        if let Some(target) = turn
            .tools
            .iter_mut()
            .find(|candidate| candidate.tool_id == target_tool_id)
        {
            append_tool_output(target, &output, updated_at);
        }
    }

    if remove_tool_ids.is_empty() {
        return;
    }
    turn.tools
        .retain(|tool| !remove_tool_ids.contains(&tool.tool_id));
    turn.blocks.retain(|block| {
        block.kind != "tool"
            || block
                .tool_id
                .as_ref()
                .map(|tool_id| !remove_tool_ids.contains(tool_id))
                .unwrap_or(true)
    });
}

fn write_stdin_polling_tool_session_id(tool: &SessionHistoryToolCall) -> Option<String> {
    if tool.title != "write_stdin" && tool.title != "Write Stdin" {
        return None;
    }
    let input = object_like(&tool.raw_input)?;
    let chars = input.get("chars").and_then(Value::as_str).unwrap_or("");
    if !chars.is_empty() {
        return None;
    }
    session_id_from_object(input)
}

fn tool_running_session_id(tool: &SessionHistoryToolCall) -> Option<String> {
    tool_output_running_session_id(&tool_output_text(tool))
        .or_else(|| object_like(&tool.raw_input).and_then(session_id_from_object))
}

fn object_like(value: &Value) -> Option<Map<String, Value>> {
    if let Some(object) = value.as_object() {
        return Some(object.clone());
    }
    let text = value.as_str()?;
    serde_json::from_str::<Value>(text)
        .ok()?
        .as_object()
        .cloned()
}

fn session_id_from_object(object: Map<String, Value>) -> Option<String> {
    object
        .get("session_id")
        .or_else(|| object.get("sessionId"))
        .and_then(|value| {
            value
                .as_i64()
                .map(|number| number.to_string())
                .or_else(|| value.as_str().map(ToString::to_string))
        })
}

fn tool_output_running_session_id(text: &str) -> Option<String> {
    let marker = "Process running with session ID";
    let start = text.find(marker)? + marker.len();
    text[start..]
        .split_whitespace()
        .next()
        .map(ToString::to_string)
}

fn append_tool_output(tool: &mut SessionHistoryToolCall, output: &str, updated_at: i64) {
    let left = tool_output_text(tool).trim_end().to_string();
    let right = output.trim_start();
    tool.raw_output = if left.is_empty() {
        Value::String(right.to_string())
    } else if right.is_empty() {
        Value::String(left)
    } else {
        Value::String(format!("{left}\n\n{right}"))
    };
    tool.updated_at = tool.updated_at.max(updated_at);
}

fn tool_output_text(tool: &SessionHistoryToolCall) -> String {
    tool.raw_output
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if tool.raw_output.is_null() {
                String::new()
            } else {
                serde_json::to_string_pretty(&tool.raw_output).unwrap_or_default()
            }
        })
}

fn merge_file_edit_blocks(turn: &mut SessionHistoryTurn) {
    let file_edit_indexes: Vec<usize> = turn
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            if block.kind == "sessionUpdate" && block.update_type.as_deref() == Some("file_edit") {
                Some(index)
            } else {
                None
            }
        })
        .collect();
    if file_edit_indexes.len() <= 1 {
        return;
    }

    let mut summaries = Vec::new();
    for index in &file_edit_indexes {
        if let Some(summary) = file_edit_summary_from_block(&turn.blocks[*index]) {
            summaries.push(summary);
        }
    }
    if summaries.is_empty() {
        return;
    }
    let first_timestamp = file_edit_indexes
        .first()
        .and_then(|index| turn.blocks[*index].timestamp);
    let last_timestamp = file_edit_indexes
        .last()
        .and_then(|index| turn.blocks[*index].timestamp)
        .or(first_timestamp);
    let merged = merged_file_edit_update(&summaries);
    turn.blocks.retain(|block| {
        !(block.kind == "sessionUpdate" && block.update_type.as_deref() == Some("file_edit"))
    });
    turn.blocks.push(session_update_block(
        "file_edit",
        merged,
        last_timestamp.unwrap_or(turn.updated_at),
    ));
}

fn append_tool_diff_file_edit_block(turn: &mut SessionHistoryTurn) {
    if !is_turn_finished(turn) || has_file_edit_block(turn) {
        return;
    }
    let edits = merge_file_edit_items(tool_diff_file_edits(turn));
    if edits.is_empty() {
        return;
    }
    let update = json!({
        "source": "acp",
        "files": edits.len(),
        "additions": sum_edit_number(&edits, "additions"),
        "deletions": sum_edit_number(&edits, "deletions"),
        "edits": edits,
    });
    turn.blocks
        .push(session_update_block("file_edit", update, turn.updated_at));
}

fn is_turn_finished(turn: &SessionHistoryTurn) -> bool {
    !matches!(turn.status.as_str(), "pending" | "streaming" | "cancelling")
}

fn has_file_edit_block(turn: &SessionHistoryTurn) -> bool {
    turn.blocks.iter().any(|block| {
        block.kind == "sessionUpdate" && block.update_type.as_deref() == Some("file_edit")
    })
}

fn tool_diff_file_edits(turn: &SessionHistoryTurn) -> Vec<Value> {
    turn.tools
        .iter()
        .filter(|tool| tool_diff_file_edit_succeeded(tool))
        .flat_map(|tool| tool.content.iter().filter_map(tool_content_to_file_edit))
        .collect()
}

fn tool_diff_file_edit_succeeded(tool: &SessionHistoryToolCall) -> bool {
    let status = tool.status.to_ascii_lowercase();
    let has_success_status = matches!(status.as_str(), "completed" | "success" | "succeeded");
    has_success_status && !tool_output_indicates_failed_edit(&tool.raw_output)
}

fn tool_output_indicates_failed_edit(output: &Value) -> bool {
    fn text_indicates_failure(text: &str) -> bool {
        let normalized = text.trim_start().to_ascii_lowercase();
        normalized.starts_with("error")
            || normalized.starts_with("failed")
            || normalized.starts_with("failure")
            || normalized.starts_with("cancelled")
            || normalized.starts_with("canceled")
            || normalized.starts_with("aborted")
            || normalized.contains("permission denied")
            || normalized.contains("denied by user")
            || normalized.contains("rejected by user")
            || normalized.contains("not approved")
    }

    match output {
        Value::String(text) => text_indicates_failure(text),
        Value::Array(items) => items.iter().any(tool_output_indicates_failed_edit),
        Value::Object(object) => {
            if object
                .get("isError")
                .or_else(|| object.get("is_error"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                return true;
            }
            if string_field(output, "status")
                .map(|status| {
                    matches!(
                        status.to_ascii_lowercase().as_str(),
                        "error" | "failed" | "failure" | "cancelled" | "canceled" | "rejected"
                    )
                })
                .unwrap_or(false)
            {
                return true;
            }
            ["error", "message", "text", "content", "output", "stderr"]
                .iter()
                .filter_map(|key| object.get(*key))
                .any(tool_output_indicates_failed_edit)
        }
        _ => false,
    }
}

fn tool_content_to_file_edit(content: &Value) -> Option<Value> {
    if string_field(content, "type").as_deref() != Some("diff") {
        return None;
    }
    let path = string_field(content, "path").or_else(|| string_field(content, "filePath"));
    let old_content =
        string_field(content, "oldText").or_else(|| string_field(content, "old_text"));
    let new_content =
        string_field(content, "newText").or_else(|| string_field(content, "new_text"));
    if path.is_none() && old_content.is_none() && new_content.is_none() {
        return None;
    }
    Some(json!({
        "path": path.clone().unwrap_or_else(|| "file".to_string()),
        "displayPath": path.clone().unwrap_or_else(|| "file".to_string()),
        "kind": if old_content.is_none() { "create" } else { "modify" },
        "additions": count_non_empty_lines(new_content.as_deref()),
        "deletions": count_non_empty_lines(old_content.as_deref()),
        "oldContent": old_content,
        "newContent": new_content,
        "detail": if path.is_none() {
            serde_json::to_string_pretty(content).ok()
        } else {
            None
        },
    }))
}

fn count_non_empty_lines(value: Option<&str>) -> i64 {
    value
        .map(|text| text.lines().filter(|line| !line.is_empty()).count() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
struct FileEditSummary {
    source: Option<String>,
    edits: Vec<Value>,
}

fn file_edit_summary_from_block(block: &SessionHistoryBlock) -> Option<FileEditSummary> {
    let data = block.data.as_ref()?;
    file_edit_summary_from_value(data).or_else(|| {
        data.get("text")
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .and_then(|value| file_edit_summary_from_value(&value))
    })
}

fn file_edit_summary_from_value(value: &Value) -> Option<FileEditSummary> {
    let edits = value.get("edits")?.as_array()?.clone();
    Some(FileEditSummary {
        source: string_field(value, "source"),
        edits,
    })
}

fn merged_file_edit_update(summaries: &[FileEditSummary]) -> Value {
    let edits = merge_file_edit_items(
        summaries
            .iter()
            .flat_map(|summary| summary.edits.iter().cloned()),
    );
    let source = summaries
        .iter()
        .find_map(|summary| summary.source.clone())
        .unwrap_or_else(|| "session".to_string());
    json!({
        "source": source,
        "files": edits.len(),
        "additions": sum_edit_number(&edits, "additions"),
        "deletions": sum_edit_number(&edits, "deletions"),
        "edits": edits,
    })
}

fn merge_file_edit_items<I>(items: I) -> Vec<Value>
where
    I: IntoIterator<Item = Value>,
{
    let mut edits = Vec::new();
    for edit in items {
        merge_file_edit_item(&mut edits, edit);
    }
    edits
}

fn merge_file_edit_item(edits: &mut Vec<Value>, next: Value) {
    let key = file_edit_key(&next);
    if let Some(existing) = edits.iter_mut().find(|edit| file_edit_key(edit) == key) {
        merge_file_edit_values(existing, &next);
    } else {
        edits.push(next);
    }
}

fn file_edit_key(value: &Value) -> String {
    string_field(value, "path")
        .or_else(|| string_field(value, "displayPath"))
        .unwrap_or_else(|| "(unknown file)".to_string())
}

fn merge_file_edit_values(existing: &mut Value, next: &Value) {
    let existing_kind = string_field(existing, "kind");
    let next_kind = string_field(next, "kind");
    let Some(existing_object) = existing.as_object_mut() else {
        return;
    };
    if existing_kind.is_some() && next_kind.is_some() && existing_kind != next_kind {
        existing_object.insert("kind".to_string(), Value::String("mixed".to_string()));
    }
    set_number_sum(existing_object, "additions", next);
    set_number_sum(existing_object, "deletions", next);
    if let Some(detail) = merge_optional_text(
        existing_object.get("detail").and_then(Value::as_str),
        next.get("detail").and_then(Value::as_str),
    ) {
        existing_object.insert("detail".to_string(), Value::String(detail));
    }
    preserve_existing_single_string(existing_object, "patches", "patch");
    merge_string_array(existing_object, "patches", next, "patch");
    merge_string_array(existing_object, "patches", next, "patches");
    existing_object.remove("patch");
    preserve_existing_content_diff_fields(existing_object);
    merge_value_array(existing_object, "contentDiffs", next, "contentDiffs");
    merge_content_diff_fields(existing_object, next);
    existing_object.remove("oldContent");
    existing_object.remove("newContent");
}

fn set_number_sum(object: &mut Map<String, Value>, key: &str, next: &Value) {
    let sum = object.get(key).and_then(Value::as_i64).unwrap_or(0)
        + next.get(key).and_then(Value::as_i64).unwrap_or(0);
    object.insert(key.to_string(), Value::Number(sum.into()));
}

fn merge_optional_text(left: Option<&str>, right: Option<&str>) -> Option<String> {
    let left = left.unwrap_or("").trim();
    let right = right.unwrap_or("").trim();
    match (left.is_empty(), right.is_empty()) {
        (true, true) => None,
        (true, false) => Some(right.to_string()),
        (false, true) => Some(left.to_string()),
        (false, false) => Some(format!("{left}\n\n{right}")),
    }
}

fn merge_string_array(
    object: &mut Map<String, Value>,
    target_key: &str,
    source: &Value,
    source_key: &str,
) {
    let mut values = object
        .get(target_key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(text) = source.get(source_key).and_then(Value::as_str) {
        if !text.trim().is_empty() {
            values.push(Value::String(text.to_string()));
        }
    }
    if let Some(items) = source.get(source_key).and_then(Value::as_array) {
        values.extend(
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(|text| Value::String(text.to_string())),
        );
    }
    if !values.is_empty() {
        object.insert(target_key.to_string(), Value::Array(values));
    }
}

fn preserve_existing_single_string(
    object: &mut Map<String, Value>,
    target_key: &str,
    source_key: &str,
) {
    let Some(text) = object.get(source_key).and_then(Value::as_str) else {
        return;
    };
    if text.trim().is_empty() {
        return;
    }
    let mut values = object
        .get(target_key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    values.push(Value::String(text.to_string()));
    object.insert(target_key.to_string(), Value::Array(values));
}

fn merge_value_array(
    object: &mut Map<String, Value>,
    target_key: &str,
    source: &Value,
    source_key: &str,
) {
    let mut values = object
        .get(target_key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(items) = source.get(source_key).and_then(Value::as_array) {
        values.extend(items.iter().cloned());
    }
    if !values.is_empty() {
        object.insert(target_key.to_string(), Value::Array(values));
    }
}

fn preserve_existing_content_diff_fields(object: &mut Map<String, Value>) {
    if object.get("oldContent").is_none() && object.get("newContent").is_none() {
        return;
    }
    let mut values = object
        .get("contentDiffs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    values.push(json!({
        "oldContent": object.get("oldContent").cloned().unwrap_or(Value::Null),
        "newContent": object.get("newContent").cloned().unwrap_or(Value::Null),
    }));
    object.insert("contentDiffs".to_string(), Value::Array(values));
}

fn merge_content_diff_fields(object: &mut Map<String, Value>, source: &Value) {
    if source.get("oldContent").is_none() && source.get("newContent").is_none() {
        return;
    }
    let mut values = object
        .get("contentDiffs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    values.push(json!({
        "oldContent": source.get("oldContent").cloned().unwrap_or(Value::Null),
        "newContent": source.get("newContent").cloned().unwrap_or(Value::Null),
    }));
    object.insert("contentDiffs".to_string(), Value::Array(values));
}

fn sum_edit_number(edits: &[Value], key: &str) -> i64 {
    edits
        .iter()
        .filter_map(|edit| edit.get(key).and_then(Value::as_i64))
        .sum()
}

fn normalize_content_blocks(value: &Value, role: Option<&str>) -> Vec<SessionContentBlock> {
    if let Some(array) = value.as_array() {
        return array
            .iter()
            .flat_map(|item| normalize_content_blocks(item, role))
            .collect();
    }
    let Some(record) = value.as_object() else {
        return Vec::new();
    };
    if let Some(content) = record.get("content") {
        return normalize_content_blocks(content, role);
    }
    if record.get("type").and_then(Value::as_str).is_some() {
        return normalize_content_block(value, role).into_iter().collect();
    }
    Vec::new()
}

fn normalize_user_content_blocks(value: &Value) -> Vec<SessionContentBlock> {
    dedupe_user_attachment_blocks(
        normalize_content_blocks(value, Some("user"))
            .into_iter()
            .flat_map(clean_user_content_block)
            .collect(),
    )
}

fn clean_user_content_block(mut block: SessionContentBlock) -> Vec<SessionContentBlock> {
    match block.kind.as_str() {
        "text" => {
            let text = block.text.take().unwrap_or_default();
            let cleaned = strip_injected_context(&sanitize_user_text_for_display(&text));
            if cleaned.trim().is_empty() || is_system_noise(&cleaned) {
                Vec::new()
            } else {
                trim_user_text_blocks(text_content_blocks(&cleaned))
            }
        }
        "resource" | "resource_link" => vec![block],
        _ => vec![block],
    }
}

fn sanitize_user_text_for_display(text: &str) -> String {
    sanitize_sessio_attachment_text(&strip_image_placeholder_tags(text))
}

fn trim_user_text_blocks(mut blocks: Vec<SessionContentBlock>) -> Vec<SessionContentBlock> {
    let attachment_flags = blocks
        .iter()
        .map(|block| matches!(block.kind.as_str(), "image" | "resource" | "resource_link"))
        .collect::<Vec<_>>();
    for (index, block) in blocks.iter_mut().enumerate() {
        if block.kind != "text" {
            continue;
        }
        let trim_start = index > 0 && attachment_flags[index - 1];
        let trim_end = attachment_flags.get(index + 1).copied().unwrap_or(false);
        if trim_start || trim_end {
            if let Some(current) = block.text.take() {
                let text = match (trim_start, trim_end) {
                    (true, true) => current.trim().to_string(),
                    (true, false) => current.trim_start().to_string(),
                    (false, true) => current.trim_end().to_string(),
                    (false, false) => current,
                };
                block.text = Some(text);
            }
        }
    }
    blocks
        .into_iter()
        .filter(|block| {
            block.kind != "text" || block.text.as_deref().is_some_and(|text| !text.is_empty())
        })
        .collect()
}

fn dedupe_user_attachment_blocks(blocks: Vec<SessionContentBlock>) -> Vec<SessionContentBlock> {
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::with_capacity(blocks.len());
    for block in blocks {
        if matches!(block.kind.as_str(), "image" | "resource" | "resource_link") {
            let key = format!(
                "{}\u{1f}{}\u{1f}{}",
                block.kind,
                block.uri.as_deref().unwrap_or_default(),
                block.name.as_deref().unwrap_or_default()
            );
            if !seen.insert(key) {
                continue;
            }
        }
        deduped.push(block);
    }
    deduped
}

fn normalize_content_block(value: &Value, role: Option<&str>) -> Option<SessionContentBlock> {
    let type_name = string_field(value, "type").unwrap_or_else(|| "unknown".to_string());
    let meta = value_field(value, "meta")
        .or_else(|| value_field(value, "_meta"))
        .unwrap_or(Value::Null);
    match type_name.as_str() {
        "text" => {
            let text = string_field(value, "text").unwrap_or_default();
            Some(SessionContentBlock {
                kind: "text".to_string(),
                text: Some(if role == Some("user") {
                    sanitize_sessio_attachment_text(&text)
                } else {
                    text
                }),
                uri: None,
                data: None,
                mime_type: None,
                name: None,
                title: None,
                description: None,
                size: None,
                blob: None,
                resource: None,
                annotations: value_field(value, "annotations"),
                meta: Some(meta),
            })
        }
        "image" | "audio" => Some(SessionContentBlock {
            kind: type_name,
            text: None,
            uri: string_field(value, "uri"),
            data: string_field(value, "data"),
            mime_type: string_field(value, "mimeType"),
            name: None,
            title: None,
            description: None,
            size: None,
            blob: None,
            resource: None,
            annotations: value_field(value, "annotations"),
            meta: Some(meta),
        }),
        "resource_link" => {
            let uri = string_field(value, "uri").unwrap_or_default();
            let name = string_field(value, "name");
            let mime_type = string_field(value, "mimeType");
            Some(SessionContentBlock {
                kind: "resource_link".to_string(),
                text: None,
                uri: Some(uri),
                data: None,
                mime_type,
                name,
                title: string_field(value, "title"),
                description: string_field(value, "description"),
                size: number_field(value, "size"),
                blob: None,
                resource: None,
                annotations: value_field(value, "annotations"),
                meta: Some(meta),
            })
        }
        "resource" => {
            let resource = value_field(value, "resource").unwrap_or(Value::Null);
            let uri = string_field(&resource, "uri").or_else(|| string_field(value, "uri"));
            let name =
                string_field(value, "name").or_else(|| uri.as_deref().and_then(basename_from_uri));
            let mime_type =
                string_field(&resource, "mimeType").or_else(|| string_field(value, "mimeType"));
            let text = string_field(&resource, "text").or_else(|| string_field(value, "text"));
            let blob = string_field(&resource, "blob").or_else(|| string_field(value, "blob"));
            Some(SessionContentBlock {
                kind: "resource".to_string(),
                text,
                uri,
                data: None,
                mime_type,
                name,
                title: None,
                description: None,
                size: None,
                blob,
                resource: Some(resource),
                annotations: value_field(value, "annotations"),
                meta: Some(meta),
            })
        }
        other => Some(SessionContentBlock {
            kind: "unknown".to_string(),
            text: None,
            uri: string_field(value, "uri"),
            data: None,
            mime_type: string_field(value, "mimeType"),
            name: string_field(value, "name"),
            title: string_field(value, "title"),
            description: string_field(value, "description"),
            size: number_field(value, "size"),
            blob: string_field(value, "blob"),
            resource: Some(json!({ "originalType": other, "value": value })),
            annotations: value_field(value, "annotations"),
            meta: Some(meta),
        }),
    }
}

fn optimistic_user_content_blocks(
    text: &str,
    attachments: &[AgentAttachment],
) -> Vec<SessionContentBlock> {
    let mut blocks = clean_user_content_block(SessionContentBlock::text(text.to_string()));
    for attachment in attachments {
        match attachment.kind {
            AgentAttachmentKind::Image => blocks.push(SessionContentBlock::image(
                attachment.path.clone(),
                attachment.mime_type.clone(),
            )),
            AgentAttachmentKind::File => {
                let name = attachment
                    .display_name
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| basename_from_uri(&attachment.path));
                blocks.push(SessionContentBlock::resource(
                    Some(attachment.path.clone()),
                    name,
                    attachment.mime_type.clone(),
                ));
            }
        }
    }
    blocks
}

fn merge_adjacent_text_blocks(blocks: Vec<SessionContentBlock>) -> Vec<SessionContentBlock> {
    let mut merged: Vec<SessionContentBlock> = Vec::new();
    for block in blocks {
        let Some(previous) = merged.last_mut() else {
            merged.push(block);
            continue;
        };
        if previous.kind == "text"
            && block.kind == "text"
            && previous.uri == block.uri
            && previous.data == block.data
            && previous.mime_type == block.mime_type
            && previous.name == block.name
            && previous.title == block.title
            && previous.description == block.description
            && previous.size == block.size
            && previous.blob == block.blob
            && previous.resource == block.resource
            && previous.annotations == block.annotations
            && previous.meta == block.meta
        {
            let mut text = previous.text.take().unwrap_or_default();
            text.push_str(block.text.as_deref().unwrap_or_default());
            previous.text = Some(text);
        } else {
            merged.push(block);
        }
    }
    merged
}

fn tool_from_value(value: &Value, timestamp: i64) -> SessionHistoryToolCall {
    let title = string_field(value, "title").unwrap_or_else(|| "tool".to_string());
    let raw_input = value_field(value, "rawInput").unwrap_or(Value::Null);
    let tool_id = string_field(value, "toolCallId").unwrap_or_else(|| format!("tool-{timestamp}"));
    SessionHistoryToolCall {
        tool_id,
        title: title.clone(),
        kind: string_field(value, "kind").unwrap_or_else(|| "other".to_string()),
        status: string_field(value, "status").unwrap_or_else(|| "pending".to_string()),
        content: array_field(value, "content")
            .iter()
            .map(normalize_tool_call_content)
            .collect(),
        locations: array_field(value, "locations").to_vec(),
        raw_input: normalize_tool_raw_input(&title, raw_input),
        raw_output: value_field(value, "rawOutput").unwrap_or(Value::Null),
        meta: value_field(value, "meta").unwrap_or(Value::Null),
        raw: value.clone(),
        updated_at: timestamp,
    }
}

fn normalize_tool_raw_input(title: &str, raw_input: Value) -> Value {
    if matches!(title, "TodoWrite" | "todo_write") {
        normalize_task_list_update_value(raw_input, "todos")
    } else {
        raw_input
    }
}

fn normalize_tool_call_content(value: &Value) -> Value {
    let type_name = string_field(value, "type").unwrap_or_else(|| "unknown".to_string());
    match type_name.as_str() {
        "content" => {
            let mut out = as_object(value).clone();
            let content = out.get("content").cloned().unwrap_or(Value::Null);
            out.insert(
                "content".to_string(),
                normalize_content_block(&content, None)
                    .and_then(|block| serde_json::to_value(block).ok())
                    .unwrap_or(Value::Null),
            );
            out.insert(
                "meta".to_string(),
                value_field(value, "meta")
                    .or_else(|| value_field(value, "_meta"))
                    .unwrap_or(Value::Null),
            );
            Value::Object(out)
        }
        "diff" => {
            let mut out = as_object(value).clone();
            if !out.contains_key("path") {
                if let Some(file_path) = string_field(value, "filePath") {
                    out.insert("path".to_string(), Value::String(file_path));
                }
            }
            if !out.contains_key("oldText") {
                if let Some(old_text) = string_field(value, "old_text") {
                    out.insert("oldText".to_string(), Value::String(old_text));
                }
            }
            if !out.contains_key("newText") {
                out.insert(
                    "newText".to_string(),
                    Value::String(
                        string_field(value, "new_text")
                            .or_else(|| string_field(value, "newText"))
                            .unwrap_or_default(),
                    ),
                );
            }
            out.insert(
                "meta".to_string(),
                value_field(value, "meta")
                    .or_else(|| value_field(value, "_meta"))
                    .unwrap_or(Value::Null),
            );
            Value::Object(out)
        }
        "terminal" => {
            let mut out = as_object(value).clone();
            if !out.contains_key("terminalId") {
                if let Some(terminal_id) = string_field(value, "terminal_id") {
                    out.insert("terminalId".to_string(), Value::String(terminal_id));
                }
            }
            out.insert(
                "meta".to_string(),
                value_field(value, "meta")
                    .or_else(|| value_field(value, "_meta"))
                    .unwrap_or(Value::Null),
            );
            Value::Object(out)
        }
        _ => {
            let mut out = as_object(value).clone();
            out.insert("type".to_string(), Value::String("unknown".to_string()));
            out.insert("originalType".to_string(), Value::String(type_name));
            out.insert(
                "meta".to_string(),
                value_field(value, "meta")
                    .or_else(|| value_field(value, "_meta"))
                    .unwrap_or(Value::Null),
            );
            Value::Object(out)
        }
    }
}

fn upsert_tool(turn: &mut SessionHistoryTurn, next_tool: SessionHistoryToolCall) {
    let Some(index) = turn
        .tools
        .iter()
        .position(|tool| tool.tool_id == next_tool.tool_id)
    else {
        turn.tools.push(next_tool);
        return;
    };
    let current = turn.tools[index].clone();
    turn.tools[index] = SessionHistoryToolCall {
        title: if next_tool.title == "tool" {
            current.title
        } else {
            next_tool.title
        },
        kind: if next_tool.kind == "other" {
            current.kind
        } else {
            next_tool.kind
        },
        status: if next_tool.status == "pending" && !current.status.is_empty() {
            current.status
        } else {
            next_tool.status
        },
        content: if next_tool.content.is_empty() {
            current.content
        } else {
            next_tool.content
        },
        locations: if next_tool.locations.is_empty() {
            current.locations
        } else {
            next_tool.locations
        },
        raw_input: if next_tool.raw_input.is_null() {
            current.raw_input
        } else {
            next_tool.raw_input
        },
        raw_output: if next_tool.raw_output.is_null() {
            current.raw_output
        } else {
            next_tool.raw_output
        },
        tool_id: next_tool.tool_id,
        meta: next_tool.meta,
        raw: next_tool.raw,
        updated_at: next_tool.updated_at,
    };
}

fn permission_from_message(
    message: &AcpProtocolMessage,
) -> Option<SessionHistoryPermissionRequest> {
    let data = as_object(&message.data);
    let tool_call = data.get("toolCall").cloned().unwrap_or(Value::Null);
    let request_id = message
        .request_id
        .clone()
        .or_else(|| string_field(&tool_call, "toolCallId"))?;
    let fields = value_field(&tool_call, "fields").unwrap_or_else(|| tool_call.clone());
    Some(SessionHistoryPermissionRequest {
        request_id,
        tool_call,
        tool_name: string_field(&fields, "title").unwrap_or_else(|| "tool".to_string()),
        input: value_field(&fields, "rawInput").unwrap_or(Value::Null),
        options: array_field(&message.data, "options")
            .iter()
            .map(|item| SessionHistoryPermissionOption {
                option_id: string_field(item, "optionId").unwrap_or_default(),
                name: string_field(item, "name").unwrap_or_else(|| "Option".to_string()),
                kind: string_field(item, "kind").unwrap_or_else(|| "unknown".to_string()),
                meta: value_field(item, "meta").unwrap_or(Value::Null),
            })
            .collect(),
        selected_option_id: None,
        cancelled: false,
        raw: message.data.clone(),
    })
}

fn permission_from_runtime_event(
    request_id: &str,
    tool_name: &str,
    input: Option<Value>,
    data: Value,
) -> SessionHistoryPermissionRequest {
    SessionHistoryPermissionRequest {
        request_id: request_id.to_string(),
        tool_call: value_field(&data, "toolCall").unwrap_or(Value::Null),
        tool_name: tool_name.to_string(),
        input: input.unwrap_or(Value::Null),
        options: array_field(&data, "options")
            .iter()
            .map(|item| SessionHistoryPermissionOption {
                option_id: string_field(item, "optionId").unwrap_or_default(),
                name: string_field(item, "name").unwrap_or_else(|| "Option".to_string()),
                kind: string_field(item, "kind").unwrap_or_else(|| "unknown".to_string()),
                meta: value_field(item, "meta").unwrap_or(Value::Null),
            })
            .collect(),
        selected_option_id: None,
        cancelled: false,
        raw: data,
    }
}

fn upsert_permission(
    turn: &mut SessionHistoryTurn,
    permission: SessionHistoryPermissionRequest,
    timestamp: i64,
) {
    let request_id = permission.request_id.clone();
    if let Some(index) = turn
        .permissions
        .iter()
        .position(|item| item.request_id == permission.request_id)
    {
        turn.permissions[index] = permission;
    } else {
        turn.permissions.push(permission);
    }
    ensure_permission_block(turn, request_id, timestamp);
}

fn normalize_plan(value: &Value) -> AcpPlan {
    AcpPlan {
        entries: array_field(value, "entries")
            .iter()
            .map(|entry| AcpPlanEntry {
                content: string_field(entry, "content").unwrap_or_default(),
                priority: string_field(entry, "priority").unwrap_or_else(|| "medium".to_string()),
                status: string_field(entry, "status").unwrap_or_else(|| "pending".to_string()),
                meta: value_field(entry, "meta")
                    .or_else(|| value_field(entry, "_meta"))
                    .unwrap_or(Value::Null),
            })
            .collect(),
        meta: value_field(value, "meta")
            .or_else(|| value_field(value, "_meta"))
            .unwrap_or(Value::Null),
    }
}

fn normalize_task_list_update_value(value: Value, source: &str) -> Value {
    let entries_source = value
        .get("entries")
        .cloned()
        .or_else(|| value.get("todos").cloned())
        .or_else(|| value.get("plan").cloned())
        .unwrap_or_else(|| value.clone());
    let entries = entries_source
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let content = string_field(item, "content")
                        .or_else(|| string_field(item, "step"))
                        .or_else(|| string_field(item, "activeForm"))
                        .unwrap_or_default();
                    json!({
                        "content": content,
                        "activeForm": string_field(item, "activeForm"),
                        "status": string_field(item, "status").unwrap_or_else(|| "pending".to_string()),
                        "priority": string_field(item, "priority").unwrap_or_else(|| "medium".to_string()),
                        "meta": value_field(item, "meta").or_else(|| value_field(item, "_meta")).unwrap_or(Value::Null),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "entries": entries,
        "source": source,
        "meta": value_field(&value, "meta").or_else(|| value_field(&value, "_meta")).unwrap_or(Value::Null),
        "raw": value,
    })
}

fn normalize_available_command(value: &Value) -> AcpAvailableCommand {
    AcpAvailableCommand {
        name: string_field(value, "name").unwrap_or_else(|| "command".to_string()),
        description: string_field(value, "description").unwrap_or_default(),
        input: normalize_available_command_input(value_field(value, "input")),
        meta: value_field(value, "meta")
            .or_else(|| value_field(value, "_meta"))
            .unwrap_or(Value::Null),
    }
}

fn normalize_available_command_input(value: Option<Value>) -> Value {
    let Some(value) = value else {
        return Value::Null;
    };
    let hint = string_field(&value, "hint");
    json!({
        "kind": if hint.is_some() { "unstructured" } else { "unknown" },
        "hint": hint,
        "meta": value_field(&value, "meta").or_else(|| value_field(&value, "_meta")).unwrap_or(Value::Null),
        "raw": value
    })
}

fn normalize_session_config_option(value: &Value) -> AcpSessionConfigOption {
    let option_type = string_field(value, "type");
    let (options, groups) = if option_type.as_deref() == Some("select") {
        normalize_config_choices(value_field(value, "options"))
    } else {
        (Vec::new(), Vec::new())
    };
    AcpSessionConfigOption {
        id: string_field(value, "id").unwrap_or_default(),
        name: string_field(value, "name").unwrap_or_else(|| "Option".to_string()),
        description: string_field(value, "description"),
        category: string_field(value, "category"),
        option_type,
        current_value: value_field(value, "currentValue").unwrap_or(Value::Null),
        options,
        groups,
        meta: value_field(value, "meta")
            .or_else(|| value_field(value, "_meta"))
            .unwrap_or(Value::Null),
        raw: value.clone(),
    }
}

fn normalize_config_choices(value: Option<Value>) -> (Vec<Value>, Vec<Value>) {
    let Some(Value::Array(items)) = value else {
        return (Vec::new(), Vec::new());
    };
    if items
        .first()
        .and_then(Value::as_object)
        .map(|first| first.contains_key("options") && first.contains_key("group"))
        .unwrap_or(false)
    {
        return (Vec::new(), items);
    }
    (items, Vec::new())
}

fn normalize_mode_config_option(value: Option<Value>) -> Option<AcpSessionConfigOption> {
    let value = value?;
    let modes = array_field(&value, "availableModes");
    if modes.is_empty() {
        return None;
    }
    Some(AcpSessionConfigOption {
        id: "mode".to_string(),
        name: "Mode".to_string(),
        description: None,
        category: Some("mode".to_string()),
        option_type: Some("select".to_string()),
        current_value: string_field(&value, "currentModeId")
            .map(Value::String)
            .unwrap_or_else(|| Value::String(String::new())),
        options: modes
            .iter()
            .map(|mode| {
                let id = string_field(mode, "id").unwrap_or_default();
                json!({
                    "value": id,
                    "name": string_field(mode, "name").unwrap_or_else(|| if id.is_empty() { "Mode".to_string() } else { id.clone() }),
                    "description": string_field(mode, "description"),
                    "meta": value_field(mode, "meta").or_else(|| value_field(mode, "_meta")).unwrap_or(Value::Null)
                })
            })
            .collect(),
        groups: Vec::new(),
        meta: value_field(&value, "meta")
            .or_else(|| value_field(&value, "_meta"))
            .unwrap_or(Value::Null),
        raw: value,
    })
}

fn normalize_model_config_option(value: Option<Value>) -> Option<AcpSessionConfigOption> {
    let value = value?;
    let models = array_field(&value, "availableModels");
    if models.is_empty() {
        return None;
    }
    Some(AcpSessionConfigOption {
        id: "model".to_string(),
        name: "Model".to_string(),
        description: None,
        category: Some("model".to_string()),
        option_type: Some("select".to_string()),
        current_value: string_field(&value, "currentModelId")
            .map(Value::String)
            .unwrap_or_else(|| Value::String(String::new())),
        options: models
            .iter()
            .map(|model| {
                let id = string_field(model, "modelId")
                    .or_else(|| string_field(model, "id"))
                    .unwrap_or_default();
                json!({
                    "value": id,
                    "name": string_field(model, "name").unwrap_or_else(|| if id.is_empty() { "Model".to_string() } else { id.clone() }),
                    "description": string_field(model, "description"),
                    "meta": value_field(model, "meta").or_else(|| value_field(model, "_meta")).unwrap_or(Value::Null)
                })
            })
            .collect(),
        groups: Vec::new(),
        meta: value_field(&value, "meta")
            .or_else(|| value_field(&value, "_meta"))
            .unwrap_or(Value::Null),
        raw: value,
    })
}

fn normalize_session_info(value: &Value) -> AcpSessionInfo {
    AcpSessionInfo {
        title: string_field(value, "title"),
        updated_at: string_field(value, "updatedAt"),
        meta: value_field(value, "meta")
            .or_else(|| value_field(value, "_meta"))
            .unwrap_or(Value::Null),
        raw: value.clone(),
    }
}

fn dedupe_session_config_options(
    options: Vec<AcpSessionConfigOption>,
) -> Vec<AcpSessionConfigOption> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for option in options {
        let key = session_config_option_identity(&option);
        if seen.insert(key) {
            out.push(option);
        }
    }
    out
}

fn session_config_option_identity(option: &AcpSessionConfigOption) -> String {
    if !option.id.trim().is_empty() {
        return format!("id:{}", option.id.trim());
    }
    if let Some(category) = option
        .category
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return format!("category:{}", category.trim());
    }
    format!("name:{}", option.name.trim())
}

fn session_update_type(update: &Value, fallback: Option<&str>) -> Option<String> {
    let value =
        string_field(update, "sessionUpdate").or_else(|| fallback.map(ToString::to_string))?;
    Some(
        match value.as_str() {
            "plan_update" => "plan",
            "available_commands_update" => "available_commands",
            "current_mode_update" => "current_mode",
            "config_options_update" => "config_options",
            "session_info_update" => "session_info",
            other => other,
        }
        .to_string(),
    )
}

fn append_protocol_message(
    messages: &[AcpProtocolMessage],
    message: &AcpProtocolMessage,
) -> Vec<AcpProtocolMessage> {
    let mut next = messages.to_vec();
    next.push(message.clone());
    if next.len() > MAX_PROTOCOL_MESSAGES {
        next.drain(0..next.len() - MAX_PROTOCOL_MESSAGES);
    }
    next
}

fn append_turn_protocol_message(messages: &[Value], message: &AcpProtocolMessage) -> Vec<Value> {
    let mut next = messages.to_vec();
    next.push(serde_json::to_value(message).unwrap_or(Value::Null));
    if next.len() > MAX_PROTOCOL_MESSAGES {
        next.drain(0..next.len() - MAX_PROTOCOL_MESSAGES);
    }
    next
}

fn selected_permission_option_id(value: &Value) -> Option<String> {
    let outcome = value_field(value, "outcome")?;
    if string_field(&outcome, "outcome").as_deref() == Some("cancelled") {
        return None;
    }
    string_field(&outcome, "optionId")
}

fn runtime_turn_status(status: RuntimeTurnStatus) -> &'static str {
    match status {
        RuntimeTurnStatus::Pending => "pending",
        RuntimeTurnStatus::Streaming => "streaming",
        RuntimeTurnStatus::Cancelling => "cancelling",
        RuntimeTurnStatus::Completed => "completed",
        RuntimeTurnStatus::Failed => "failed",
        RuntimeTurnStatus::Cancelled => "cancelled",
    }
}

fn sanitize_sessio_attachment_text(text: &str) -> String {
    replace_context_tags(&replace_sessio_upload_file_tags(
        &remove_file_markdown_links(text),
    ))
}

fn remove_file_markdown_links(text: &str) -> String {
    let mut out = text.to_string();
    let mut search_from = 0usize;
    while let Some(close_label_rel) = out[search_from..].find("](") {
        let close_label = search_from + close_label_rel;
        let Some(open_label_rel) = out[search_from..close_label].rfind('[') else {
            search_from = close_label + 2;
            continue;
        };
        let open_label = search_from + open_label_rel;
        let target_start = close_label + 2;
        let Some(close_target_rel) = out[target_start..].find(')') else {
            break;
        };
        let close_target = target_start + close_target_rel;
        let target = out[target_start..close_target]
            .trim()
            .trim_matches(['<', '>']);
        let label = &out[open_label + 1..close_label];
        let is_at_prefix = label.trim_start().starts_with('@');
        let is_cross_context = target.contains("sessio-cross-context");
        if !target.starts_with("file://") || (!is_at_prefix && !is_cross_context) {
            search_from = close_target + 1;
            continue;
        }
        let drop_start = if open_label > 0 && out.as_bytes()[open_label - 1] == b'!' {
            open_label - 1
        } else {
            open_label
        };
        out.replace_range(drop_start..close_target + 1, "");
        search_from = drop_start;
    }
    collapse_blank_lines(&out)
}

fn strip_image_placeholder_tags(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.starts_with("<image") && trimmed.ends_with(">")) && trimmed != "</image>"
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn replace_sessio_upload_file_tags(text: &str) -> String {
    replace_xmlish_blocks(text, "sessio-upload-file", |attrs| {
        let uri = attrs
            .get("uri")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let name = attrs
            .get("name")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| uri.as_deref().and_then(basename_from_uri));
        file_marker(name.as_deref(), uri.as_deref())
    })
}

fn replace_context_tags(text: &str) -> String {
    replace_xmlish_blocks(text, "context", |attrs| {
        let uri = attrs
            .get("ref")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let name = uri.as_deref().and_then(basename_from_uri);
        file_marker(name.as_deref(), uri.as_deref())
    })
}

fn replace_xmlish_blocks(
    text: &str,
    tag: &str,
    marker: impl Fn(&Map<String, Value>) -> String,
) -> String {
    let mut out = String::new();
    let mut rest = text;
    let open_prefix = format!("<{tag}");
    let close_tag = format!("</{tag}>");
    loop {
        let Some(start) = rest.find(&open_prefix) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let after_start = &rest[start..];
        let Some(open_end) = after_start.find('>') else {
            out.push_str(after_start);
            break;
        };
        let tag_text = &after_start[..=open_end];
        let attrs = parse_attrs(tag_text);
        if tag_text.ends_with("/>") {
            out.push_str(&marker(&attrs));
            rest = &after_start[open_end + 1..];
            continue;
        }
        let body_start = open_end + 1;
        let Some(close) = after_start[body_start..].find(&close_tag) else {
            out.push_str(after_start);
            break;
        };
        out.push_str(&marker(&attrs));
        rest = &after_start[body_start + close + close_tag.len()..];
    }
    collapse_blank_lines(&out)
}

fn parse_attrs(tag_text: &str) -> Map<String, Value> {
    let mut attrs = Map::new();
    let mut rest = tag_text;
    while let Some(eq) = rest.find('=') {
        let before = &rest[..eq];
        let key = before
            .split_whitespace()
            .last()
            .unwrap_or("")
            .trim_matches(['<', '/', '>']);
        let after = rest[eq + 1..].trim_start();
        let Some(quote) = after.chars().next().filter(|ch| *ch == '"' || *ch == '\'') else {
            rest = after;
            continue;
        };
        let after_quote = &after[quote.len_utf8()..];
        let Some(end) = after_quote.find(quote) else {
            break;
        };
        if !key.is_empty() {
            attrs.insert(
                key.to_string(),
                Value::String(after_quote[..end].to_string()),
            );
        }
        rest = &after_quote[end + quote.len_utf8()..];
    }
    attrs
}

fn file_marker(name: Option<&str>, uri: Option<&str>) -> String {
    match (name, uri) {
        (Some(name), Some(uri)) if !name.is_empty() && !uri.is_empty() => {
            format!("[file: {}|{uri}]", sessio_attachment_marker_name(name))
        }
        (Some(name), _) if !name.is_empty() => {
            format!("[file: {}]", sessio_attachment_marker_name(name))
        }
        (_, Some(uri)) if !uri.is_empty() => {
            format!(
                "[file: {}|{uri}]",
                sessio_attachment_marker_name("attachment")
            )
        }
        _ => String::new(),
    }
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::new();
    let mut blank_count = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                out.push('\n');
            }
            continue;
        }
        blank_count = 0;
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

fn basename_from_uri(uri: &str) -> Option<String> {
    if uri.is_empty() {
        return None;
    }
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    path.split(['/', '\\']).rfind(|part| !part.is_empty())
        .map(ToString::to_string)
}

fn as_object(value: &Value) -> &Map<String, Value> {
    value.as_object().unwrap_or_else(|| {
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        EMPTY.get_or_init(Map::new)
    })
}

fn value_field(value: &Value, key: &str) -> Option<Value> {
    value.as_object()?.get(key).cloned()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .as_object()?
        .get(key)?
        .as_str()
        .map(ToString::to_string)
}

fn number_field(value: &Value, key: &str) -> Option<i64> {
    value.as_object()?.get(key)?.as_i64()
}

fn array_field<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::sources::types::{HistoryAcpMessage, SourceLocation};

    fn row(message: AcpProtocolMessage, timestamp: Option<i64>) -> HistoryAcpMessage {
        HistoryAcpMessage {
            message,
            timestamp,
            location: SourceLocation::file("/tmp/session.jsonl"),
            synthetic: true,
        }
    }

    #[test]
    fn history_turns_emit_acp_like_blocks_and_tools() {
        let messages = vec![
            row(
                history_user_message(
                    "review\n[file: __sessio_attachment__:spec.md|file:///tmp/spec.md]",
                    Some(10),
                ),
                Some(10),
            ),
            row(
                history_tool_call_message(
                    Some("tool-1".to_string()),
                    "Read",
                    json!({ "path": "spec.md" }),
                    Some(20),
                ),
                Some(20),
            ),
            row(
                history_tool_result_message(
                    Some("tool-1".to_string()),
                    Value::String("contents".to_string()),
                    Some(21),
                ),
                Some(21),
            ),
            row(
                history_session_update_message("file_edit", json!({ "edits": [] }), Some(22)),
                Some(22),
            ),
            row(history_assistant_message("done", Some(30)), Some(30)),
        ];
        let turns = session_history_turns_from_acp_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_id, "history-turn-0");
        assert_eq!(turns[0].blocks.len(), 4);
        assert_eq!(turns[0].tools.len(), 1);
        assert_eq!(turns[0].blocks[0].kind, "user");
        assert_eq!(turns[0].blocks[0].blocks[1].kind, "resource");
        assert_eq!(turns[0].blocks[1].kind, "tool");
        assert_eq!(turns[0].blocks[2].kind, "sessionUpdate");
        assert_eq!(turns[0].blocks[2].update_type.as_deref(), Some("file_edit"));
        assert_eq!(turns[0].blocks[3].kind, "assistant");
        assert_eq!(turns[0].blocks[3].blocks[0].text.as_deref(), Some("done"));
        assert_eq!(turns[0].tools[0].tool_id, "tool-1");
        assert_eq!(
            turns[0].tools[0].raw_output,
            Value::String("contents".to_string())
        );
    }

    #[test]
    fn history_turns_render_assistant_content_arrays() {
        let messages = vec![
            row(history_user_message("tell me a joke", Some(10)), Some(10)),
            row(history_assistant_message("here is one", Some(20)), Some(20)),
        ];
        let turns = session_history_turns_from_acp_messages(&messages);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].status, "completed");
        assert_eq!(turns[0].blocks.len(), 2);
        assert_eq!(turns[0].blocks[1].kind, "assistant");
        assert_eq!(
            turns[0].blocks[1].blocks[0].text.as_deref(),
            Some("here is one")
        );
        assert_eq!(turns[0].updated_at, 20);
    }

    #[test]
    fn history_builder_emits_canonical_task_entries_for_todos_and_plans() {
        let messages = vec![
            row(history_user_message("organize work", Some(10)), Some(10)),
            row(
                history_todo_message(
                    json!([
                        { "content": "Verify Claude todos", "status": "completed" },
                        { "activeForm": "Render TodoWrite", "status": "in_progress" }
                    ]),
                    Some(20),
                    Some("todo-1".to_string()),
                ),
                Some(20),
            ),
            row(
                history_session_update_message(
                    "plan",
                    json!({
                        "sessionUpdate": "plan_update",
                        "entries": [
                            { "content": "Parse Codex plan", "status": "completed", "priority": "medium" },
                            { "content": "Render like todos", "status": "in_progress", "priority": "medium" }
                        ]
                    }),
                    Some(30),
                ),
                Some(30),
            ),
        ];

        let turns = session_history_turns_from_acp_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].tools.len(), 1);
        assert_eq!(turns[0].tools[0].title, "TodoWrite");
        assert_eq!(turns[0].tools[0].kind, "task_list");
        assert_eq!(
            turns[0].tools[0].raw_input["entries"][0]["content"],
            "Verify Claude todos"
        );
        assert_eq!(
            turns[0].tools[0].raw_input["entries"][1]["content"],
            "Render TodoWrite"
        );

        let plan_block = turns[0]
            .blocks
            .iter()
            .find(|block| block.update_type.as_deref() == Some("plan"))
            .expect("plan block");
        assert_eq!(plan_block.kind, "sessionUpdate");
        assert_eq!(
            plan_block.data.as_ref().unwrap()["entries"][0]["content"],
            "Parse Codex plan"
        );
        assert_eq!(
            plan_block.data.as_ref().unwrap()["entries"][1]["content"],
            "Render like todos"
        );
    }

    #[test]
    fn history_builder_emits_structured_permission_requests() {
        let mut messages = vec![row(history_user_message("edit file", Some(10)), Some(10))];
        messages.extend(
            history_permission_request_message(
                Some("perm-1".to_string()),
                "apply_patch",
                json!({ "path": "src/lib.rs" }),
                vec![json!({ "optionId": "allow", "name": "Allow", "kind": "allow" })],
                Some("allow".to_string()),
                Some(false),
                Some(json!({
                    "toolCallId": "tool-1",
                    "fields": {
                        "title": "apply_patch",
                        "rawInput": { "path": "src/lib.rs" }
                    }
                })),
                json!({ "source": "history-permission" }),
                Some(20),
            )
            .into_iter()
            .map(|message| row(message, Some(20))),
        );

        let turns = session_history_turns_from_acp_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].blocks[1].kind, "permission");
        assert_eq!(turns[0].blocks[1].request_id.as_deref(), Some("perm-1"));
        assert_eq!(turns[0].permissions.len(), 1);
        assert_eq!(turns[0].permissions[0].request_id, "perm-1");
        assert_eq!(turns[0].permissions[0].tool_name, "apply_patch");
        assert_eq!(turns[0].permissions[0].input["path"], "src/lib.rs");
        assert_eq!(turns[0].permissions[0].options[0].option_id, "allow");
        assert_eq!(
            turns[0].permissions[0].selected_option_id.as_deref(),
            Some("allow")
        );
        assert!(!turns[0].permissions[0].cancelled);
    }

    #[test]
    fn builder_merges_terminal_polling_tools() {
        let messages = vec![
            row(history_user_message("run command", Some(10)), Some(10)),
            row(
                history_tool_call_message(
                    Some("run-1".to_string()),
                    "exec_command",
                    json!({ "cmd": "long-running" }),
                    Some(20),
                ),
                Some(20),
            ),
            row(
                history_tool_result_message(
                    Some("run-1".to_string()),
                    Value::String("Process running with session ID 42\ninitial output".to_string()),
                    Some(21),
                ),
                Some(21),
            ),
            row(
                history_tool_call_message(
                    Some("poll-1".to_string()),
                    "write_stdin",
                    json!({ "session_id": 42 }),
                    Some(30),
                ),
                Some(30),
            ),
            row(
                history_tool_result_message(
                    Some("poll-1".to_string()),
                    Value::String("poll output".to_string()),
                    Some(31),
                ),
                Some(31),
            ),
        ];

        let turns = session_history_turns_from_acp_messages(&messages);
        assert_eq!(turns[0].tools.len(), 1);
        assert_eq!(turns[0].tools[0].tool_id, "run-1");
        assert_eq!(
            turns[0].tools[0].raw_output.as_str(),
            Some("Process running with session ID 42\ninitial output\n\npoll output")
        );
        assert!(turns[0]
            .blocks
            .iter()
            .all(|block| block.tool_id.as_deref() != Some("poll-1")));
    }

    #[test]
    fn builder_merges_file_edit_session_updates() {
        let messages = vec![
            row(history_user_message("edit files", Some(10)), Some(10)),
            row(
                history_session_update_message(
                    "file_edit",
                    json!({
                        "source": "codex",
                        "edits": [{
                            "path": "src/lib.rs",
                            "kind": "edit",
                            "additions": 1,
                            "deletions": 2,
                            "patch": "@@ -1 +1 @@\n-old\n+new"
                        }]
                    }),
                    Some(20),
                ),
                Some(20),
            ),
            row(
                history_session_update_message(
                    "file_edit",
                    json!({
                        "source": "codex",
                        "edits": [{
                            "path": "src/lib.rs",
                            "kind": "edit",
                            "additions": 3,
                            "deletions": 4,
                            "detail": "second edit"
                        }]
                    }),
                    Some(30),
                ),
                Some(30),
            ),
        ];

        let turns = session_history_turns_from_acp_messages(&messages);
        let file_edits: Vec<_> = turns[0]
            .blocks
            .iter()
            .filter(|block| block.update_type.as_deref() == Some("file_edit"))
            .collect();
        assert_eq!(file_edits.len(), 1);
        let data = file_edits[0].data.as_ref().unwrap();
        assert_eq!(data["files"], 1);
        assert_eq!(data["additions"], 4);
        assert_eq!(data["deletions"], 6);
        assert_eq!(data["edits"][0]["patches"].as_array().unwrap().len(), 1);
        assert_eq!(data["edits"][0]["detail"], "second edit");
    }

    #[test]
    fn runtime_builder_emits_file_edit_block_from_tool_diff_on_completion() {
        let mut state = RuntimeTurnState::new(
            "runtime-1",
            Agent::Codex,
            "agent-1",
            RuntimeTransportKind::Acp,
            "/tmp/project",
            RuntimeCapabilitySet::fake(),
        );
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::TurnStarted {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
            10,
        );
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::AcpProtocolMessage {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                message: history_session_update_message(
                    "tool_call",
                    json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "tool-1",
                        "title": "edit",
                        "status": "completed",
                        "content": [{
                            "type": "diff",
                            "path": "src/main.rs",
                            "oldText": "old\n",
                            "newText": "new\nmore\n"
                        }]
                    }),
                    Some(20),
                ),
            },
            20,
        );
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::TurnCompleted {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: "turn-1".to_string(),
                result: None,
            },
            30,
        );

        let file_edit = state.turns[0]
            .blocks
            .iter()
            .find(|block| block.update_type.as_deref() == Some("file_edit"))
            .expect("file_edit block");
        let data = file_edit.data.as_ref().unwrap();
        assert_eq!(data["files"], 1);
        assert_eq!(data["edits"][0]["path"], "src/main.rs");
        assert_eq!(data["edits"][0]["additions"], 2);
        assert_eq!(data["edits"][0]["deletions"], 1);
    }

    #[test]
    fn runtime_builder_merges_tool_diff_file_edits_by_file() {
        let mut state = RuntimeTurnState::new(
            "runtime-1",
            Agent::Codex,
            "agent-1",
            RuntimeTransportKind::Acp,
            "/tmp/project",
            RuntimeCapabilitySet::fake(),
        );
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::TurnStarted {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
            10,
        );
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::AcpProtocolMessage {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                message: history_session_update_message(
                    "tool_call",
                    json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "tool-1",
                        "title": "edit",
                        "status": "completed",
                        "content": [
                            {
                                "type": "diff",
                                "path": "src/main.rs",
                                "oldText": "old\n",
                                "newText": "new\nmore\n"
                            },
                            {
                                "type": "diff",
                                "path": "src/main.rs",
                                "oldText": "before\nagain\n",
                                "newText": "after\n"
                            }
                        ]
                    }),
                    Some(20),
                ),
            },
            20,
        );
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::TurnCompleted {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: "turn-1".to_string(),
                result: None,
            },
            30,
        );

        let file_edit = state.turns[0]
            .blocks
            .iter()
            .find(|block| block.update_type.as_deref() == Some("file_edit"))
            .expect("file_edit block");
        let data = file_edit.data.as_ref().unwrap();
        assert_eq!(data["files"], 1);
        assert_eq!(data["additions"], 3);
        assert_eq!(data["deletions"], 3);
        assert_eq!(data["edits"].as_array().unwrap().len(), 1);
        assert_eq!(data["edits"][0]["path"], "src/main.rs");
    }

    #[test]
    fn runtime_builder_skips_failed_tool_diff_file_edits() {
        let mut state = RuntimeTurnState::new(
            "runtime-1",
            Agent::Codex,
            "agent-1",
            RuntimeTransportKind::Acp,
            "/tmp/project",
            RuntimeCapabilitySet::fake(),
        );
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::TurnStarted {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
            10,
        );
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::AcpProtocolMessage {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                message: history_session_update_message(
                    "tool_call",
                    json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "tool-1",
                        "title": "edit",
                        "status": "completed",
                        "content": [{
                            "type": "diff",
                            "path": "src/main.rs",
                            "oldText": "old\n",
                            "newText": "new\n"
                        }]
                    }),
                    Some(20),
                ),
            },
            20,
        );
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::AcpProtocolMessage {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                message: history_session_update_message(
                    "tool_call_update",
                    json!({
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": "tool-1",
                        "status": "completed",
                        "rawOutput": "aborted by user after 560.5s"
                    }),
                    Some(21),
                ),
            },
            21,
        );
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::TurnCompleted {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: "turn-1".to_string(),
                result: None,
            },
            30,
        );

        assert!(state.turns[0]
            .blocks
            .iter()
            .all(|block| block.update_type.as_deref() != Some("file_edit")));
    }

    #[test]
    fn history_and_live_builders_are_isomorphic_for_same_acp_messages() {
        let messages = vec![
            history_user_message(
                "review\n[file: __sessio_attachment__:spec.md|file:///tmp/spec.md]",
                Some(10),
            ),
            history_todo_message(
                json!([{ "content": "Check todo", "status": "in_progress" }]),
                Some(20),
                Some("todo-1".to_string()),
            ),
            history_session_update_message(
                "plan",
                json!({
                    "sessionUpdate": "plan_update",
                    "entries": [{ "content": "Check plan", "status": "pending" }]
                }),
                Some(30),
            ),
            history_permission_request_message(
                Some("perm-1".to_string()),
                "apply_patch",
                json!({ "path": "src/lib.rs" }),
                vec![json!({ "optionId": "allow", "name": "Allow", "kind": "allow" })],
                None,
                Some(false),
                None,
                json!({ "source": "permission" }),
                Some(40),
            )[0]
            .clone(),
            history_session_update_message(
                "file_edit",
                json!({
                    "source": "codex",
                    "edits": [{
                        "path": "src/lib.rs",
                        "kind": "edit",
                        "additions": 1,
                        "deletions": 0
                    }]
                }),
                Some(50),
            ),
            history_assistant_message("done", Some(60)),
        ];
        let rows = messages
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, message)| row(message, Some(((index + 1) * 10) as i64)))
            .collect::<Vec<_>>();
        let history = session_history_turns_from_acp_messages(&rows);

        let mut live = RuntimeTurnState::new(
            "runtime-1",
            Agent::Codex,
            "agent-1",
            RuntimeTransportKind::Acp,
            "/tmp/project",
            RuntimeCapabilitySet::fake(),
        );
        apply_runtime_event_to_state(
            &mut live,
            &AgentRuntimeEventPayload::TurnStarted {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: "live-turn".to_string(),
            },
            10,
        );
        for (index, message) in messages.into_iter().enumerate() {
            apply_runtime_event_to_state(
                &mut live,
                &AgentRuntimeEventPayload::AcpProtocolMessage {
                    sessio_runtime_session_id: "runtime-1".to_string(),
                    turn_id: Some("live-turn".to_string()),
                    message,
                },
                ((index + 1) * 10) as i64,
            );
        }
        apply_runtime_event_to_state(
            &mut live,
            &AgentRuntimeEventPayload::TurnCompleted {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: "live-turn".to_string(),
                result: None,
            },
            70,
        );

        assert_eq!(history.len(), 1);
        assert_eq!(live.turns.len(), 1);
        assert_eq!(
            comparable_turn(&history[0]),
            comparable_turn(&live.turns[0])
        );
    }

    fn comparable_turn(turn: &SessionHistoryTurn) -> Value {
        json!({
            "blocks": turn.blocks,
            "tools": turn.tools,
            "permissions": turn.permissions,
            "stopReason": turn.stop_reason,
            "error": turn.error,
        })
    }

    #[test]
    fn runtime_acp_prompt_builds_user_block() {
        let mut state = RuntimeTurnState::new(
            "runtime-1",
            Agent::Codex,
            "agent-1",
            RuntimeTransportKind::Acp,
            "/tmp",
            RuntimeCapabilitySet::fake(),
        );
        let message = AcpProtocolMessage {
            direction: "client_to_agent".to_string(),
            message_kind: "request".to_string(),
            method: "session/prompt".to_string(),
            protocol_version: Some("1".to_string()),
            acp_session_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            request_id: None,
            update_type: None,
            data: json!({ "prompt": [{ "type": "text", "text": "hello" }] }),
        };
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::AcpProtocolMessage {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                message,
            },
            42,
        );
        assert_eq!(state.turns.len(), 1);
        assert_eq!(state.turns[0].blocks[0].kind, "user");
        assert_eq!(
            state.turns[0].blocks[0].blocks[0].text.as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn runtime_acp_prompt_extracts_marked_attachments_from_user_text() {
        let mut state = RuntimeTurnState::new(
            "runtime-1",
            Agent::Codex,
            "agent-1",
            RuntimeTransportKind::Acp,
            "/tmp",
            RuntimeCapabilitySet::fake(),
        );
        let message = AcpProtocolMessage {
            direction: "client_to_agent".to_string(),
            message_kind: "request".to_string(),
            method: "session/prompt".to_string(),
            protocol_version: Some("1".to_string()),
            acp_session_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            request_id: None,
            update_type: None,
            data: json!({
                "prompt": [{
                    "type": "text",
                    "text": "图片和文档有关杀没\n![__sessio_attachment__:image/png](file:///Users/alex/Downloads/screen.png)\n[file: __sessio_attachment__:test.md|file:///Users/alex/Downloads/test.md]"
                }]
            }),
        };
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::AcpProtocolMessage {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                message,
            },
            42,
        );
        let blocks = &state.turns[0].blocks[0].blocks;
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].kind, "text");
        assert_eq!(blocks[0].text.as_deref(), Some("图片和文档有关杀没"));
        assert_eq!(blocks[1].kind, "image");
        assert_eq!(blocks[1].mime_type.as_deref(), Some("image/png"));
        assert_eq!(
            blocks[1].uri.as_deref(),
            Some("file:///Users/alex/Downloads/screen.png")
        );
        assert_eq!(blocks[2].kind, "resource");
        assert_eq!(blocks[2].name.as_deref(), Some("test.md"));
        assert_eq!(
            blocks[2].uri.as_deref(),
            Some("file:///Users/alex/Downloads/test.md")
        );
    }

    #[test]
    fn runtime_acp_prompt_keeps_marked_attachments_inside_code_as_text() {
        let mut state = RuntimeTurnState::new(
            "runtime-1",
            Agent::Codex,
            "agent-1",
            RuntimeTransportKind::Acp,
            "/tmp",
            RuntimeCapabilitySet::fake(),
        );
        let text = "`[file: __sessio_attachment__:test.md|file:///x]`\n`![__sessio_attachment__:image/png](file:///x.png)`\n\n```md\n[file: __sessio_attachment__:test.md|file:///x]\n![__sessio_attachment__:image/png](file:///x.png)\n```";
        let message = AcpProtocolMessage {
            direction: "client_to_agent".to_string(),
            message_kind: "request".to_string(),
            method: "session/prompt".to_string(),
            protocol_version: Some("1".to_string()),
            acp_session_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            request_id: None,
            update_type: None,
            data: json!({ "prompt": [{ "type": "text", "text": text }] }),
        };
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::AcpProtocolMessage {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                message,
            },
            42,
        );

        let blocks = &state.turns[0].blocks[0].blocks;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "text");
        assert_eq!(blocks[0].text.as_deref(), Some(text.trim()));
    }

    #[test]
    fn runtime_acp_prompt_keeps_marked_attachments_inside_unclosed_outer_fence_as_text() {
        let mut state = RuntimeTurnState::new(
            "runtime-1",
            Agent::Codex,
            "agent-1",
            RuntimeTransportKind::Acp,
            "/tmp",
            RuntimeCapabilitySet::fake(),
        );
        let text = "```\n`[file: __sessio_attachment__:test.md|file:///x]`\n`![__sessio_attachment__:image/png](file:///x.png)`\n\n```md\n[file: __sessio_attachment__:test.md|file:///x]\n![__sessio_attachment__:image/png](file:///x.png)\n```  这种下面的图片还是会被提取\n";
        let message = AcpProtocolMessage {
            direction: "client_to_agent".to_string(),
            message_kind: "request".to_string(),
            method: "session/prompt".to_string(),
            protocol_version: Some("1".to_string()),
            acp_session_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            request_id: None,
            update_type: None,
            data: json!({ "prompt": [{ "type": "text", "text": text }] }),
        };
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::AcpProtocolMessage {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                message,
            },
            42,
        );

        let blocks = &state.turns[0].blocks[0].blocks;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "text");
        assert_eq!(blocks[0].text.as_deref(), Some(text.trim()));
    }

    #[test]
    fn runtime_acp_prompt_keeps_cross_context_as_user_attachment() {
        let mut state = RuntimeTurnState::new(
            "runtime-1",
            Agent::Codex,
            "agent-1",
            RuntimeTransportKind::Acp,
            "/tmp",
            RuntimeCapabilitySet::fake(),
        );
        let message = AcpProtocolMessage {
            direction: "client_to_agent".to_string(),
            message_kind: "request".to_string(),
            method: "session/prompt".to_string(),
            protocol_version: Some("1".to_string()),
            acp_session_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            request_id: None,
            update_type: None,
            data: json!({
                "prompt": [
                    {
                        "type": "text",
                        "text": "继续\n[@sessio-cross-context-parent.md](file:///tmp/.cross-context/sessio-cross-context-parent.md)\n<context ref=\"file:///tmp/.cross-context/sessio-cross-context-parent.md\"><sessio-upload-file uri=\"file:///tmp/.cross-context/sessio-cross-context-parent.md\" name=\"sessio-cross-context-parent.md\">replay</sessio-upload-file></context>"
                    },
                    {
                        "type": "resource",
                        "uri": "file:///tmp/.cross-context/sessio-cross-context-parent.md",
                        "name": "sessio-cross-context-parent.md"
                    },
                    {
                        "type": "resource",
                        "uri": "file:///tmp/spec.md",
                        "name": "spec.md"
                    }
                ]
            }),
        };
        apply_runtime_event_to_state(
            &mut state,
            &AgentRuntimeEventPayload::AcpProtocolMessage {
                sessio_runtime_session_id: "runtime-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                message,
            },
            42,
        );

        let blocks = &state.turns[0].blocks[0].blocks;
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].text.as_deref(), Some("继续"));
        assert_eq!(blocks[1].kind, "resource");
        assert_eq!(
            blocks[1].name.as_deref(),
            Some("sessio-cross-context-parent.md")
        );
        assert_eq!(
            blocks[1].uri.as_deref(),
            Some("file:///tmp/.cross-context/sessio-cross-context-parent.md")
        );
        assert_eq!(blocks[2].kind, "resource");
        assert_eq!(blocks[2].name.as_deref(), Some("spec.md"));
    }

    #[test]
    fn optimistic_user_message_keeps_cross_context_attachment() {
        let mut state = RuntimeTurnState::new(
            "runtime-1",
            Agent::Codex,
            "agent-1",
            RuntimeTransportKind::Acp,
            "/tmp",
            RuntimeCapabilitySet::fake(),
        );
        apply_optimistic_user_message(
            &mut state,
            "turn-1",
            "继续",
            &[
                AgentAttachment {
                    path: "file:///tmp/.cross-context/sessio-cross-context-parent.md".to_string(),
                    mime_type: Some("text/markdown".to_string()),
                    kind: AgentAttachmentKind::File,
                    display_name: Some("sessio-cross-context-parent.md".to_string()),
                },
                AgentAttachment {
                    path: "file:///tmp/spec.md".to_string(),
                    mime_type: Some("text/markdown".to_string()),
                    kind: AgentAttachmentKind::File,
                    display_name: Some("spec.md".to_string()),
                },
            ],
            42,
        );

        let blocks = &state.turns[0].blocks[0].blocks;
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].text.as_deref(), Some("继续"));
        assert_eq!(blocks[1].kind, "resource");
        assert_eq!(
            blocks[1].name.as_deref(),
            Some("sessio-cross-context-parent.md")
        );
        assert_eq!(
            blocks[1].uri.as_deref(),
            Some("file:///tmp/.cross-context/sessio-cross-context-parent.md")
        );
        assert_eq!(blocks[2].kind, "resource");
        assert_eq!(blocks[2].name.as_deref(), Some("spec.md"));
    }

    #[test]
    fn optimistic_user_message_keeps_marked_attachments_inside_code_as_text() {
        let mut state = RuntimeTurnState::new(
            "runtime-1",
            Agent::Codex,
            "agent-1",
            RuntimeTransportKind::Acp,
            "/tmp",
            RuntimeCapabilitySet::fake(),
        );
        let text = "`[file: __sessio_attachment__:test.md|file:///x]`\n`![__sessio_attachment__:image/png](file:///x.png)`\n\n```md\n[file: __sessio_attachment__:test.md|file:///x]\n![__sessio_attachment__:image/png](file:///x.png)\n```";
        apply_optimistic_user_message(&mut state, "turn-1", text, &[], 42);

        let blocks = &state.turns[0].blocks[0].blocks;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "text");
        assert_eq!(blocks[0].text.as_deref(), Some(text));
    }
}
