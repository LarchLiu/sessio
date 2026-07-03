use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{
    Agent, AssistantAgentInfo, ThreadAgentInfo, ThreadAssistantInfo, ThreadInfo, ThreadKind,
    ThreadOrigin,
};

use super::{load_thread_sessions, load_thread_stages, parse_string_array_json};

pub(super) fn thread_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadInfo> {
    let kind_raw: String = row.get(5)?;
    let origin_raw: String = row.get(9)?;
    Ok(ThreadInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        goal: row.get(2)?,
        description: row.get(3)?,
        stage_id: row.get(4)?,
        kind: ThreadKind::from_db_str(&kind_raw).unwrap_or_default(),
        enabled: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        origin: ThreadOrigin::from_db_str(&origin_raw).unwrap_or_default(),
        scheduled_task_id: row.get(10)?,
        assistants: Vec::new(),
        agent_participants: Vec::new(),
        stages: Vec::new(),
        sessions: Vec::new(),
    })
}

pub(super) fn load_thread_by_id(conn: &Connection, thread_id: &str) -> Result<ThreadInfo> {
    let mut thread = conn
        .query_row(
            "SELECT id, project_id, goal, description, stage_id, kind, enabled, created_at, updated_at,
                    origin, scheduled_task_id
             FROM threads
             WHERE id = ?",
            params![thread_id],
            thread_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
    thread.assistants = load_thread_assistants(conn, &thread.id)?;
    thread.agent_participants = load_thread_agents(conn, &thread.id)?;
    thread.stages = load_thread_stages(conn, &thread.id)?;
    thread.sessions = load_thread_sessions(conn, &thread.id)?;
    Ok(thread)
}

pub(super) fn load_thread_assistants(
    conn: &Connection,
    thread_id: &str,
) -> Result<Vec<ThreadAssistantInfo>> {
    let mut stmt = conn.prepare(
        "SELECT ta.assistant_id, a.name, a.color, a.agent_json, a.system_prompt, a.selected_skill_ids_json, a.selected_mcp_ids_json, ta.sort_order
         FROM thread_assistants ta
         INNER JOIN assistants a ON a.id = ta.assistant_id
         WHERE ta.thread_id = ?
         ORDER BY ta.sort_order ASC, ta.created_at ASC",
    )?;
    let rows = stmt.query_map(params![thread_id], |row| {
        let agent_json: String = row.get(3)?;
        Ok(ThreadAssistantInfo {
            assistant_id: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
            agent: serde_json::from_str::<AssistantAgentInfo>(&agent_json).unwrap_or_else(|_| {
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: String::new(),
                    mode: String::new(),
                    effort: String::new(),
                }
            }),
            system_prompt: row.get(4)?,
            selected_skill_ids: parse_string_array_json(&row.get::<_, String>(5)?),
            selected_mcp_ids: parse_string_array_json(&row.get::<_, String>(6)?),
            order: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(super) fn load_thread_agents(
    conn: &Connection,
    thread_id: &str,
) -> Result<Vec<ThreadAgentInfo>> {
    let mut stmt = conn.prepare(
        "SELECT participant_id, agent, model, effort, permission_mode, sort_order, created_at, updated_at
         FROM thread_agents
         WHERE thread_id = ?
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let rows = stmt.query_map(params![thread_id], |row| {
        let agent_raw: String = row.get(1)?;
        Ok(ThreadAgentInfo {
            participant_id: row.get(0)?,
            agent: Agent::from_db_str(&agent_raw).unwrap_or(Agent::Codex),
            model: row.get(2)?,
            effort: row.get(3)?,
            permission_mode: row.get(4)?,
            order: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
