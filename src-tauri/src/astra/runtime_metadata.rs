use serde_json::Value;

use crate::agents::runtime::types::{RuntimeMetadata, RuntimeTransportKind, StartAgentSession};
use crate::mcp::SELECTED_MCP_IDS_OPTION;
use crate::models::ThreadInfo;
use crate::skills::SELECTED_SKILL_IDS_OPTION;
use crate::store::SessionStore;

use super::AstraTaskProposal;

pub(super) fn hydrate_start_request_for_astra(
    req: &mut StartAgentSession,
    store: &dyn SessionStore,
) -> anyhow::Result<()> {
    let Some(agent) = store
        .list_agents()?
        .into_iter()
        .find(|agent| agent.id == req.agent.as_str())
    else {
        return Ok(());
    };
    insert_option_if_missing(&mut req.options, "model", agent.model);
    insert_option_if_missing(&mut req.options, "effort", agent.effort);
    insert_option_if_missing(&mut req.options, "permissionMode", agent.permission_mode);
    insert_option_if_missing(
        &mut req.options,
        "transport",
        Some(runtime_transport_option(agent.transport)),
    );
    if !req.options.contains_key("command") && !req.options.contains_key("acpCommand") {
        if let Some(command) = agent.commands.session.first().cloned() {
            insert_option_if_missing(&mut req.options, "command", Some(command));
        }
    }
    Ok(())
}

pub(super) fn insert_option_if_missing(
    options: &mut RuntimeMetadata,
    key: &str,
    value: Option<String>,
) {
    if options.contains_key(key) {
        return;
    }
    if let Some(value) = value.map(|value| value.trim().to_string()) {
        if !value.is_empty() {
            options.insert(key.to_string(), Value::String(value));
        }
    }
}

pub(super) fn insert_assistant_resource_options_from_thread(
    options: &mut RuntimeMetadata,
    thread: &ThreadInfo,
    task: &AstraTaskProposal,
) {
    if options.contains_key(SELECTED_SKILL_IDS_OPTION)
        && options.contains_key(SELECTED_MCP_IDS_OPTION)
    {
        return;
    }
    let snapshot = task
        .assistant_id
        .as_deref()
        .and_then(|assistant_id| {
            thread
                .assistants
                .iter()
                .find(|assistant| assistant.assistant_id == assistant_id)
                .and_then(|assistant| serde_json::to_value(assistant).ok())
        })
        .or_else(|| {
            thread
                .stages
                .iter()
                .flat_map(|stage| stage.assistants.iter())
                .find(|assistant| assistant.agent.id == task.target_agent.as_str())
                .and_then(|assistant| serde_json::to_value(assistant).ok())
        });
    if let Some(snapshot) = snapshot {
        insert_assistant_resource_options_from_value(options, &snapshot);
    }
}

pub(super) fn insert_assistant_resource_options_from_json(
    options: &mut RuntimeMetadata,
    assistant_snapshot_json: Option<&str>,
) {
    let Some(snapshot) =
        assistant_snapshot_json.and_then(|value| serde_json::from_str::<Value>(value).ok())
    else {
        return;
    };
    insert_assistant_resource_options_from_value(options, &snapshot);
}

fn insert_assistant_resource_options_from_value(options: &mut RuntimeMetadata, snapshot: &Value) {
    insert_string_array_option_if_missing(
        options,
        SELECTED_SKILL_IDS_OPTION,
        snapshot.get("selectedSkillIds"),
    );
    insert_string_array_option_if_missing(
        options,
        SELECTED_MCP_IDS_OPTION,
        snapshot.get("selectedMcpIds"),
    );
}

fn insert_string_array_option_if_missing(
    options: &mut RuntimeMetadata,
    key: &str,
    value: Option<&Value>,
) {
    if options.contains_key(key) {
        return;
    }
    let Some(values) = value.and_then(Value::as_array) else {
        return;
    };
    let values = values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .fold(Vec::<Value>::new(), |mut out, value| {
            if !out.iter().any(|existing| existing.as_str() == Some(value)) {
                out.push(Value::String(value.to_string()));
            }
            out
        });
    if !values.is_empty() {
        options.insert(key.to_string(), Value::Array(values));
    }
}

pub(super) fn non_empty_option(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn runtime_transport_option(transport: RuntimeTransportKind) -> String {
    match transport {
        RuntimeTransportKind::Acp => "acp",
        RuntimeTransportKind::PiRpc => "piRpc",
        RuntimeTransportKind::Fake => "fake",
    }
    .to_string()
}
