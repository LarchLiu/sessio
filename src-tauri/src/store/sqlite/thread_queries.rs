use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;

use crate::models::{
    Agent, AssistantAgentInfo, ProjectStageType, SessionInfo, StageInfo, StageStatus, StageType,
    ThreadAgentInfo, ThreadAssistantInfo, ThreadInfo, ThreadKind, ThreadOrigin,
};

use super::{
    dedupe_sessions, load_all_subagents_grouped, load_stage_assistants, load_stage_issues,
    parse_string_array_json, session_info_from_row, SESSION_INFO_COLUMNS_S,
};

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

pub(super) fn thread_stage_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StageInfo> {
    let stage_type_raw: String = row.get(4)?;
    let process_template_id_raw: Option<String> = row.get(5)?;
    let stage_kind_raw: Option<String> = row.get(6)?;
    let status_raw: Option<String> = row.get(15)?;
    Ok(StageInfo {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        stage_id: row.get(2)?,
        project_id: row.get(3)?,
        assistant_ids: Vec::new(),
        assistants: Vec::new(),
        stage_type: ProjectStageType::from_db_str(&stage_type_raw)
            .unwrap_or(ProjectStageType::Custom),
        process_template_id: process_template_id_raw,
        kind: stage_kind_raw.and_then(|value| StageType::from_db_str(&value)),
        name: row.get(7)?,
        description: row.get(8)?,
        icon: row.get(9)?,
        order: row.get(10)?,
        status: status_raw
            .as_deref()
            .and_then(StageStatus::from_db_str)
            .unwrap_or(StageStatus::NotStarted),
        summary: row.get(16)?,
        outcome: row.get(17)?,
        enabled: row.get::<_, i64>(11)? != 0,
        allow_empty_assistants: row.get::<_, i64>(12)? != 0,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        sessions: Vec::new(),
        issues: Vec::new(),
    })
}

pub(super) fn load_thread_sessions(conn: &Connection, thread_id: &str) -> Result<Vec<SessionInfo>> {
    let mut subs_by_parent = load_all_subagents_grouped(conn)?;
    let sql = format!(
        "SELECT {SESSION_INFO_COLUMNS_S}
         FROM thread_sessions ts
         INNER JOIN sessions s ON s.agent = ts.agent AND s.session_id = ts.session_id
         WHERE ts.thread_id = ? AND s.available = 1
         ORDER BY ts.created_at ASC, s.updated_at DESC, s.started_at DESC",
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut sessions: Vec<SessionInfo> = stmt
        .query_map(params![thread_id], session_info_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    dedupe_sessions(&mut sessions);
    for session in sessions.iter_mut() {
        session.subagents = subs_by_parent
            .remove(&(session.agent, session.id.clone()))
            .unwrap_or_default();
    }
    Ok(sessions)
}

pub(super) fn load_stage_sessions(
    conn: &Connection,
    thread_stage_id: &str,
) -> Result<Vec<SessionInfo>> {
    let mut subs_by_parent = load_all_subagents_grouped(conn)?;
    let sql = format!(
        "SELECT {SESSION_INFO_COLUMNS_S}
         FROM stage_sessions ss
         INNER JOIN sessions s ON s.agent = ss.agent AND s.session_id = ss.session_id
         WHERE ss.thread_stage_id = ? AND s.available = 1
         ORDER BY ss.created_at ASC, s.updated_at DESC, s.started_at DESC",
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut sessions: Vec<SessionInfo> = stmt
        .query_map(params![thread_stage_id], session_info_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    dedupe_sessions(&mut sessions);
    for session in sessions.iter_mut() {
        session.subagents = subs_by_parent
            .remove(&(session.agent, session.id.clone()))
            .unwrap_or_default();
    }
    Ok(sessions)
}

pub(super) fn load_thread_stages(conn: &Connection, thread_id: &str) -> Result<Vec<StageInfo>> {
    let mut stmt = conn.prepare(
        "SELECT ts.id, ts.thread_id, ts.stage_id, t.project_id, s.type, s.process_template_id, s.kind, s.name, s.description, s.icon,
                ts.sort_order, s.enabled, s.allow_empty_assistants, ts.created_at, ts.updated_at,
                tss.status, tss.summary, tss.outcome
         FROM thread_stages ts
         INNER JOIN threads t ON t.id = ts.thread_id
         INNER JOIN stages s ON s.id = ts.stage_id
         LEFT JOIN thread_stage_states tss ON tss.thread_stage_id = ts.id
         WHERE ts.thread_id = ?
         ORDER BY ts.sort_order ASC, ts.created_at ASC",
    )?;
    let mut stages = stmt
        .query_map(params![thread_id], thread_stage_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Lazy default: thread stages without an explicit thread_stage_states row
    // get a status derived from their order relative to the active stage
    // (before -> completed, active -> in_progress, after -> not_started). This
    // keeps pre-V6 threads coherent without materializing rows on read.
    let stored: HashSet<String> = {
        let mut stmt = conn.prepare(
            "SELECT tss.thread_stage_id
             FROM thread_stage_states tss
             INNER JOIN thread_stages ts ON ts.id = tss.thread_stage_id
             WHERE ts.thread_id = ?",
        )?;
        let ids = stmt
            .query_map(params![thread_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<HashSet<String>>>()?;
        ids
    };
    let active_stage_id: Option<String> = conn
        .query_row(
            "SELECT stage_id FROM threads WHERE id = ?",
            params![thread_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let active_index = active_stage_id.as_deref().and_then(|active| {
        stages
            .iter()
            .position(|stage| stage.id == active || stage.stage_id == active)
    });
    for (index, stage) in stages.iter_mut().enumerate() {
        if !stored.contains(&stage.id) {
            stage.status = match active_index {
                Some(active) if index < active => StageStatus::Completed,
                Some(active) if index == active => StageStatus::InProgress,
                _ => StageStatus::NotStarted,
            };
        }
        stage.assistants = load_stage_assistants(conn, &stage.id)?;
        stage.assistant_ids = stage
            .assistants
            .iter()
            .map(|assistant| assistant.assistant_id.clone())
            .collect();
        stage.sessions = load_stage_sessions(conn, &stage.id)?;
        stage.issues = load_stage_issues(conn, &stage.id)?;
    }
    Ok(stages)
}
