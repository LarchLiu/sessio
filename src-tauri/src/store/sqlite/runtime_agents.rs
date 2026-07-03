use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::Agent;
use crate::store::{
    now_ms, RuntimeAgentCapabilityRecord, RuntimeAgentSelection, RuntimeAgentSessionConfigRecord,
};

use super::{
    normalize_adapter_version_key, transport_kind_from_db, transport_kind_to_db,
    RUNTIME_SELECTION_KEY,
};

fn runtime_agent_selection_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RuntimeAgentSelection> {
    let agent_str: String = row.get(0)?;
    let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
    Ok(RuntimeAgentSelection {
        agent,
        model: row.get(1)?,
        effort: row.get(2)?,
        permission_mode: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn runtime_agent_session_config_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RuntimeAgentSessionConfigRecord> {
    let agent_raw: String = row.get(0)?;
    Ok(RuntimeAgentSessionConfigRecord {
        agent: Agent::from_db_str(&agent_raw).unwrap_or(Agent::Codex),
        adapter_version: row.get(1)?,
        available_commands_json: row.get(2)?,
        config_options_json: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

pub(super) fn get_last_runtime_agent_selection(
    conn: &Connection,
) -> Result<Option<RuntimeAgentSelection>> {
    conn.query_row(
        "SELECT agent, model, effort, permission_mode, updated_at
         FROM runtime_agent_selections
         WHERE key = ?",
        params![RUNTIME_SELECTION_KEY],
        runtime_agent_selection_from_row,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn set_last_runtime_agent_selection(
    conn: &Connection,
    agent: Agent,
    model: Option<&str>,
    effort: Option<&str>,
    permission_mode: Option<&str>,
) -> Result<RuntimeAgentSelection> {
    let now = now_ms();
    let model = model.map(str::trim).filter(|value| !value.is_empty());
    let effort = effort.map(str::trim).filter(|value| !value.is_empty());
    let permission_mode = permission_mode
        .map(str::trim)
        .filter(|value| !value.is_empty());
    conn.execute(
        "INSERT INTO runtime_agent_selections (
            key, agent, model, effort, permission_mode, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET
            agent = excluded.agent,
            model = excluded.model,
            effort = excluded.effort,
            permission_mode = excluded.permission_mode,
            updated_at = excluded.updated_at",
        params![
            RUNTIME_SELECTION_KEY,
            agent.as_str(),
            model,
            effort,
            permission_mode,
            now,
        ],
    )?;
    Ok(RuntimeAgentSelection {
        agent,
        model: model.map(str::to_string),
        effort: effort.map(str::to_string),
        permission_mode: permission_mode.map(str::to_string),
        updated_at: now,
    })
}

pub(super) fn get_runtime_agent_capability(
    conn: &Connection,
    agent: Agent,
) -> Result<Option<RuntimeAgentCapabilityRecord>> {
    let mut stmt = conn.prepare(
        "SELECT transport_kind, detected_version, protocol_version,
                raw_initialize_response_json, raw_capabilities_json, updated_at
         FROM runtime_agent_capabilities
         WHERE agent = ?",
    )?;
    stmt.query_row(params![agent.as_str()], |row| {
        let transport_kind: String = row.get(0)?;
        Ok(RuntimeAgentCapabilityRecord {
            agent,
            transport: transport_kind_from_db(&transport_kind),
            version: row.get(1)?,
            protocol_version: row.get(2)?,
            raw_initialize_response_json: row.get(3)?,
            raw_capabilities_json: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })
    .optional()
    .map_err(Into::into)
}

pub(super) fn upsert_runtime_agent_capability(
    conn: &Connection,
    record: &RuntimeAgentCapabilityRecord,
) -> Result<()> {
    conn.execute(
        "INSERT INTO runtime_agent_capabilities (
            agent, transport_kind, detected_version, protocol_version,
            raw_initialize_response_json, raw_capabilities_json, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(agent) DO UPDATE SET
            transport_kind = excluded.transport_kind,
            detected_version = excluded.detected_version,
            protocol_version = excluded.protocol_version,
            raw_initialize_response_json = excluded.raw_initialize_response_json,
            raw_capabilities_json = excluded.raw_capabilities_json,
            updated_at = excluded.updated_at",
        params![
            record.agent.as_str(),
            transport_kind_to_db(record.transport),
            record.version,
            record.protocol_version,
            record.raw_initialize_response_json,
            record.raw_capabilities_json,
            record.updated_at,
        ],
    )?;
    Ok(())
}

pub(super) fn get_runtime_agent_session_config(
    conn: &Connection,
    agent: Agent,
    adapter_version: &str,
) -> Result<Option<RuntimeAgentSessionConfigRecord>> {
    let Some(adapter_version) = normalize_adapter_version_key(adapter_version) else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT agent, adapter_version, available_commands_json,
                config_options_json, created_at, updated_at
         FROM runtime_agent_session_configs
         WHERE agent = ? AND adapter_version = ?",
        params![agent.as_str(), adapter_version],
        runtime_agent_session_config_from_row,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn list_runtime_agent_session_configs(
    conn: &Connection,
    agent: Agent,
) -> Result<Vec<RuntimeAgentSessionConfigRecord>> {
    let mut stmt = conn.prepare(
        "SELECT agent, adapter_version, available_commands_json,
                config_options_json, created_at, updated_at
         FROM runtime_agent_session_configs
         WHERE agent = ?
         ORDER BY updated_at DESC, adapter_version ASC",
    )?;
    let rows = stmt.query_map(
        params![agent.as_str()],
        runtime_agent_session_config_from_row,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(super) fn mark_runtime_agent_session_config_needs_refresh(
    conn: &Connection,
    agent: Agent,
    adapter_version: &str,
) -> Result<()> {
    let Some(adapter_version) = normalize_adapter_version_key(adapter_version) else {
        return Ok(());
    };
    conn.execute(
        "DELETE FROM runtime_agent_session_configs
         WHERE agent = ? AND adapter_version = ?",
        params![agent.as_str(), adapter_version],
    )?;
    Ok(())
}

pub(super) fn upsert_runtime_agent_session_config(
    conn: &Connection,
    record: &RuntimeAgentSessionConfigRecord,
) -> Result<()> {
    let Some(adapter_version) = normalize_adapter_version_key(&record.adapter_version) else {
        return Ok(());
    };
    conn.execute(
        "INSERT INTO runtime_agent_session_configs (
            agent, adapter_version, available_commands_json,
            config_options_json, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(agent, adapter_version) DO UPDATE SET
            available_commands_json = excluded.available_commands_json,
            config_options_json = excluded.config_options_json,
            updated_at = excluded.updated_at",
        params![
            record.agent.as_str(),
            adapter_version,
            record.available_commands_json,
            record.config_options_json,
            record.created_at,
            record.updated_at,
        ],
    )?;
    Ok(())
}
