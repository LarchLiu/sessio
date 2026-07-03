use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{AssistantAgentInfo, AssistantInfo, AssistantType};
use crate::store::{now_ms, NewAssistant};

use super::projects::load_project_by_id;
use super::{
    ensure_assistant_can_be_disabled, ensure_process_template_exists, load_agent_by_id,
    parse_string_array_json,
};

fn stable_assistant_id(
    assistant_type: AssistantType,
    process_template_id: Option<&str>,
    project_id: Option<&str>,
    name: &str,
    model: &str,
    now: i64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(assistant_type.as_str().as_bytes());
    hasher.update(process_template_id.unwrap_or("").as_bytes());
    hasher.update(project_id.unwrap_or("").as_bytes());
    hasher.update(name.as_bytes());
    hasher.update(model.as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!("assistant-{}", &hex::encode(hasher.finalize())[..16])
}

pub(super) fn assistant_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssistantInfo> {
    let agent_json: String = row.get(2)?;
    let selected_skill_ids_json: String = row.get(5)?;
    let selected_mcp_ids_json: String = row.get(6)?;
    let assistant_type_raw: String = row.get(7)?;
    let process_template_id_raw: Option<String> = row.get(8)?;
    Ok(AssistantInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        agent: serde_json::from_str::<AssistantAgentInfo>(&agent_json).unwrap_or_else(|_| {
            AssistantAgentInfo {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                model: String::new(),
                mode: String::new(),
                effort: String::new(),
            }
        }),
        system_prompt: row.get(3)?,
        color: row.get(4)?,
        selected_skill_ids: parse_string_array_json(&selected_skill_ids_json),
        selected_mcp_ids: parse_string_array_json(&selected_mcp_ids_json),
        assistant_type: AssistantType::from_db_str(&assistant_type_raw)
            .unwrap_or(AssistantType::Custom),
        process_template_id: process_template_id_raw,
        project_id: row.get(9)?,
        enabled: row.get::<_, i64>(10)? != 0,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn normalize_string_ids(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .fold(Vec::<String>::new(), |mut out, item| {
            if !out.contains(&item) {
                out.push(item);
            }
            out
        })
}

fn string_ids_json(values: Vec<String>) -> Result<String> {
    Ok(serde_json::to_string(&normalize_string_ids(values))?)
}

pub(super) fn load_assistant_by_id(conn: &Connection, assistant_id: &str) -> Result<AssistantInfo> {
    conn.query_row(
        "SELECT id, name, agent_json, system_prompt, color, selected_skill_ids_json, selected_mcp_ids_json, type, process_template_id, project_id, enabled, created_at, updated_at
         FROM assistants
         WHERE id = ?",
        params![assistant_id],
        assistant_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("assistant not found: {assistant_id}"))
}

pub(super) fn list_assistants(
    conn: &Connection,
    project_id: Option<&str>,
) -> Result<Vec<AssistantInfo>> {
    let assistants = if let Some(project_id) = project_id {
        load_project_by_id(conn, project_id)?;
        let mut stmt = conn.prepare(
            "SELECT id, name, agent_json, system_prompt, color, selected_skill_ids_json, selected_mcp_ids_json, type, process_template_id, project_id, enabled, created_at, updated_at
             FROM assistants
             WHERE project_id = ?
             ORDER BY type ASC, updated_at DESC, name COLLATE NOCASE ASC",
        )?;
        let rows = stmt.query_map(params![project_id], assistant_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, name, agent_json, system_prompt, color, selected_skill_ids_json, selected_mcp_ids_json, type, process_template_id, project_id, enabled, created_at, updated_at
             FROM assistants
             ORDER BY type ASC, updated_at DESC, name COLLATE NOCASE ASC",
        )?;
        let rows = stmt.query_map([], assistant_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(assistants)
}

pub(super) fn create_assistant(
    conn: &Connection,
    assistant: NewAssistant<'_>,
) -> Result<AssistantInfo> {
    let NewAssistant {
        name,
        agent,
        system_prompt,
        color,
        selected_skill_ids,
        selected_mcp_ids,
        assistant_type,
        process_template_id,
        project_id,
    } = assistant;
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("assistant name cannot be empty");
    }
    let mut agent = agent;
    agent.id = agent.id.trim().to_string();
    agent.name = agent.name.trim().to_string();
    agent.model = agent.model.trim().to_string();
    agent.mode = agent.mode.trim().to_string();
    agent.effort = agent.effort.trim().to_string();
    if agent.id.is_empty() {
        anyhow::bail!("assistant agent id cannot be empty");
    }
    if agent.name.is_empty() {
        anyhow::bail!("assistant agent name cannot be empty");
    }
    if agent.model.is_empty() {
        anyhow::bail!("assistant model cannot be empty");
    }
    if agent.mode.is_empty() {
        anyhow::bail!("assistant permission mode cannot be empty");
    }
    if agent.effort.is_empty() {
        anyhow::bail!("assistant effort cannot be empty");
    }
    let system_prompt = system_prompt.map(str::trim).filter(|s| !s.is_empty());
    let color = color.map(str::trim).filter(|s| !s.is_empty());
    let db_agent = load_agent_by_id(conn, &agent.id)?;
    agent.name = db_agent.name;
    let project = project_id
        .map(|project_id| load_project_by_id(conn, project_id))
        .transpose()?;
    let resolved_process_template_id = process_template_id.or_else(|| {
        project
            .as_ref()
            .map(|project| project.process_template_id.clone())
    });
    match assistant_type {
        AssistantType::Builtin => {
            if project_id.is_some() {
                anyhow::bail!("builtin assistant cannot be linked to a project");
            }
        }
        AssistantType::Custom => {}
    }
    if let Some(process_template_id) = resolved_process_template_id.as_deref() {
        ensure_process_template_exists(conn, process_template_id)?;
    }
    let now = now_ms();
    let id = stable_assistant_id(
        assistant_type,
        resolved_process_template_id.as_deref(),
        project_id,
        name,
        &agent.model,
        now,
    );
    let agent_json = serde_json::to_string(&agent)?;
    let selected_skill_ids_json = string_ids_json(selected_skill_ids)?;
    let selected_mcp_ids_json = string_ids_json(selected_mcp_ids)?;
    conn.execute(
        "INSERT INTO assistants (
            id, name, agent_json, system_prompt, color, selected_skill_ids_json, selected_mcp_ids_json, type, process_template_id, project_id, enabled, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
        params![
            id,
            name,
            agent_json,
            system_prompt,
            color,
            selected_skill_ids_json,
            selected_mcp_ids_json,
            assistant_type.as_str(),
            resolved_process_template_id.as_deref(),
            project_id,
            now,
            now,
        ],
    )?;
    load_assistant_by_id(conn, &id)
}

pub(super) fn update_assistant(
    conn: &Connection,
    assistant_id: &str,
    name: Option<&str>,
    agent: Option<AssistantAgentInfo>,
    system_prompt: Option<Option<&str>>,
    color: Option<Option<&str>>,
    selected_skill_ids: Option<Vec<String>>,
    selected_mcp_ids: Option<Vec<String>>,
    enabled: Option<bool>,
) -> Result<AssistantInfo> {
    let current = load_assistant_by_id(conn, assistant_id)?;
    let next_agent = match agent {
        Some(mut value) => {
            value.id = value.id.trim().to_string();
            value.name = value.name.trim().to_string();
            value.model = value.model.trim().to_string();
            value.mode = value.mode.trim().to_string();
            value.effort = value.effort.trim().to_string();
            if value.id.is_empty() {
                anyhow::bail!("assistant agent id cannot be empty");
            }
            if value.model.is_empty() {
                anyhow::bail!("assistant model cannot be empty");
            }
            if value.mode.is_empty() {
                anyhow::bail!("assistant permission mode cannot be empty");
            }
            if value.effort.is_empty() {
                anyhow::bail!("assistant effort cannot be empty");
            }
            let db_agent = load_agent_by_id(conn, &value.id)?;
            value.name = db_agent.name;
            value
        }
        None => current.agent,
    };
    let next_name = match name {
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                anyhow::bail!("assistant name cannot be empty");
            }
            value.to_string()
        }
        None => current.name,
    };
    let next_system_prompt = match system_prompt {
        Some(Some(value)) => {
            if value.trim().is_empty() {
                None
            } else {
                Some(value.trim().to_string())
            }
        }
        Some(None) => None,
        None => current.system_prompt,
    };
    let next_color = match color {
        Some(Some(value)) => {
            if value.trim().is_empty() {
                None
            } else {
                Some(value.trim().to_string())
            }
        }
        Some(None) => None,
        None => current.color,
    };
    let next_enabled = enabled.unwrap_or(current.enabled);
    let next_selected_skill_ids = selected_skill_ids.unwrap_or(current.selected_skill_ids);
    let next_selected_mcp_ids = selected_mcp_ids.unwrap_or(current.selected_mcp_ids);
    if current.enabled && !next_enabled {
        ensure_assistant_can_be_disabled(conn, assistant_id)?;
    }
    let next_agent_json = serde_json::to_string(&next_agent)?;
    let next_selected_skill_ids_json = string_ids_json(next_selected_skill_ids)?;
    let next_selected_mcp_ids_json = string_ids_json(next_selected_mcp_ids)?;
    conn.execute(
        "UPDATE assistants
         SET name = ?, agent_json = ?, system_prompt = ?, color = ?, selected_skill_ids_json = ?, selected_mcp_ids_json = ?, enabled = ?, updated_at = ?
         WHERE id = ?",
        params![
            next_name,
            next_agent_json,
            next_system_prompt,
            next_color,
            next_selected_skill_ids_json,
            next_selected_mcp_ids_json,
            next_enabled as i64,
            now_ms(),
            assistant_id,
        ],
    )?;
    load_assistant_by_id(conn, assistant_id)
}

pub(super) fn delete_assistant(conn: &Connection, assistant_id: &str) -> Result<()> {
    load_assistant_by_id(conn, assistant_id)?;
    let stage_count: i64 = conn.query_row(
        "SELECT
            (SELECT count(*) FROM thread_stage_assistants WHERE assistant_id = ?) +
            (SELECT count(*) FROM stage_assistants WHERE assistant_id = ?) +
            (SELECT count(*) FROM thread_assistants WHERE assistant_id = ?)",
        params![assistant_id, assistant_id, assistant_id],
        |row| row.get(0),
    )?;
    if stage_count > 0 {
        anyhow::bail!("assistant is used by stages or threads");
    }
    conn.execute("DELETE FROM assistants WHERE id = ?", params![assistant_id])?;
    Ok(())
}
