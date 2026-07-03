use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashSet;

use crate::models::{Agent, AssistantInfo, ThreadAgentInfo, ThreadInfo, ThreadKind, ThreadOrigin};
use crate::store::now_ms;

use super::identity::downgrade_session_origin_when_unlinked;
use super::projects::load_project_by_id;
use super::thread_queries::{
    load_thread_agents, load_thread_assistants, load_thread_by_id, load_thread_sessions,
    load_thread_stages, thread_from_row,
};
use super::{load_agent_by_id, validate_assistants_for_project};

fn stable_thread_id(project_id: &str, goal: &str, now: i64) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(goal.as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!("thread-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_thread_agent_participant_id(
    thread_id: &str,
    agent: Agent,
    model: &str,
    order: i64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(thread_id.as_bytes());
    hasher.update(agent.as_str().as_bytes());
    hasher.update(model.as_bytes());
    hasher.update(order.to_string().as_bytes());
    format!("thread-agent-{}", &hex::encode(hasher.finalize())[..16])
}

fn replace_thread_assistants(
    conn: &Connection,
    thread_id: &str,
    assistants: &[AssistantInfo],
    now: i64,
) -> Result<()> {
    conn.execute(
        "DELETE FROM thread_assistants WHERE thread_id = ?",
        params![thread_id],
    )?;
    for (index, assistant) in assistants.iter().enumerate() {
        conn.execute(
            "INSERT INTO thread_assistants (thread_id, assistant_id, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
            params![thread_id, assistant.id, index as i64, now, now],
        )?;
    }
    Ok(())
}

fn normalize_thread_agents(
    conn: &Connection,
    thread_id: &str,
    participants: &[ThreadAgentInfo],
) -> Result<Vec<ThreadAgentInfo>> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for (index, participant) in participants.iter().enumerate() {
        load_agent_by_id(conn, participant.agent.as_str())?;
        let model = participant.model.trim();
        if model.is_empty() {
            anyhow::bail!("thread agent model cannot be empty");
        }
        let effort = participant.effort.trim();
        let permission_mode = participant.permission_mode.trim();
        let order = participant.order;
        let participant_id = participant.participant_id.trim();
        let participant_id = if participant_id.is_empty() {
            stable_thread_agent_participant_id(thread_id, participant.agent, model, index as i64)
        } else {
            participant_id.to_string()
        };
        if !seen.insert(participant_id.clone()) {
            anyhow::bail!("duplicate thread agent participant id: {participant_id}");
        }
        normalized.push(ThreadAgentInfo {
            participant_id,
            agent: participant.agent,
            model: model.to_string(),
            effort: effort.to_string(),
            permission_mode: permission_mode.to_string(),
            order,
            created_at: participant.created_at,
            updated_at: participant.updated_at,
        });
    }
    Ok(normalized)
}

fn replace_thread_agents(
    conn: &Connection,
    thread_id: &str,
    participants: &[ThreadAgentInfo],
    now: i64,
) -> Result<()> {
    let participants = normalize_thread_agents(conn, thread_id, participants)?;
    conn.execute(
        "DELETE FROM thread_agents WHERE thread_id = ?",
        params![thread_id],
    )?;
    for (index, participant) in participants.iter().enumerate() {
        conn.execute(
            "INSERT INTO thread_agents (
                thread_id, participant_id, agent, model, effort, permission_mode,
                sort_order, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                thread_id,
                participant.participant_id,
                participant.agent.as_str(),
                participant.model,
                participant.effort,
                participant.permission_mode,
                index as i64,
                now,
                now
            ],
        )?;
    }
    Ok(())
}

pub(super) fn list_threads(conn: &Connection, project_id: &str) -> Result<Vec<ThreadInfo>> {
    load_project_by_id(conn, project_id)?;
    let mut stmt = conn.prepare(
        "SELECT id, project_id, goal, description, stage_id, kind, enabled, created_at, updated_at,
                origin, scheduled_task_id
         FROM threads
         WHERE project_id = ?
         ORDER BY updated_at DESC, created_at DESC",
    )?;
    let mut threads = stmt
        .query_map(params![project_id], thread_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for thread in threads.iter_mut() {
        thread.assistants = load_thread_assistants(conn, &thread.id)?;
        thread.agent_participants = load_thread_agents(conn, &thread.id)?;
        thread.stages = load_thread_stages(conn, &thread.id)?;
        thread.sessions = load_thread_sessions(conn, &thread.id)?;
    }
    Ok(threads)
}

pub(super) fn create_thread(
    conn: &mut Connection,
    project_id: &str,
    goal: &str,
    description: Option<&str>,
) -> Result<ThreadInfo> {
    create_thread_with_options(
        conn,
        project_id,
        goal,
        description,
        ThreadKind::Process,
        &[],
        &[],
    )
}

pub(super) fn create_thread_with_options(
    conn: &mut Connection,
    project_id: &str,
    goal: &str,
    description: Option<&str>,
    kind: ThreadKind,
    assistant_ids: &[String],
    agent_participants: &[ThreadAgentInfo],
) -> Result<ThreadInfo> {
    create_thread_with_origin(
        conn,
        project_id,
        goal,
        description,
        kind,
        assistant_ids,
        agent_participants,
        ThreadOrigin::Manual,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_thread_with_origin(
    conn: &mut Connection,
    project_id: &str,
    goal: &str,
    description: Option<&str>,
    kind: ThreadKind,
    assistant_ids: &[String],
    agent_participants: &[ThreadAgentInfo],
    origin: ThreadOrigin,
    scheduled_task_id: Option<&str>,
) -> Result<ThreadInfo> {
    let goal = goal.trim();
    if goal.is_empty() {
        anyhow::bail!("thread goal cannot be empty");
    }
    let description = description.map(str::trim).filter(|s| !s.is_empty());
    let tx = conn.transaction()?;
    load_project_by_id(&tx, project_id)?;
    let assistants = validate_assistants_for_project(&tx, project_id, assistant_ids)?;
    let now = now_ms();
    let id = stable_thread_id(project_id, goal, now);
    tx.execute(
        "INSERT INTO threads (id, project_id, goal, description, stage_id, kind, enabled,
                              origin, scheduled_task_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, NULL, ?, 1, ?, ?, ?, ?)",
        params![
            id,
            project_id,
            goal,
            description,
            kind.as_str(),
            origin.as_str(),
            scheduled_task_id,
            now,
            now
        ],
    )?;
    replace_thread_assistants(&tx, &id, &assistants, now)?;
    replace_thread_agents(&tx, &id, agent_participants, now)?;
    let thread = load_thread_by_id(&tx, &id)?;
    tx.commit()?;
    Ok(thread)
}

pub(super) fn update_thread(
    conn: &mut Connection,
    thread_id: &str,
    goal: Option<&str>,
    description: Option<Option<&str>>,
    enabled: Option<bool>,
) -> Result<ThreadInfo> {
    update_thread_with_options(
        conn,
        thread_id,
        goal,
        description,
        enabled,
        None,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update_thread_with_options(
    conn: &mut Connection,
    thread_id: &str,
    goal: Option<&str>,
    description: Option<Option<&str>>,
    enabled: Option<bool>,
    kind: Option<ThreadKind>,
    assistant_ids: Option<&[String]>,
    agent_participants: Option<&[ThreadAgentInfo]>,
) -> Result<ThreadInfo> {
    let tx = conn.transaction()?;
    let current = load_thread_by_id(&tx, thread_id)?;
    let next_goal = match goal {
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                anyhow::bail!("thread goal cannot be empty");
            }
            value.to_string()
        }
        None => current.goal,
    };
    let next_description = match description {
        Some(Some(value)) => {
            if value.trim().is_empty() {
                None
            } else {
                Some(value.trim().to_string())
            }
        }
        Some(None) => None,
        None => current.description,
    };
    let next_enabled = enabled.unwrap_or(current.enabled);
    let next_kind = kind.unwrap_or(current.kind);
    let assistant_bindings = assistant_ids
        .map(|ids| validate_assistants_for_project(&tx, &current.project_id, ids))
        .transpose()?;
    let now = now_ms();
    tx.execute(
        "UPDATE threads
         SET goal = ?, description = ?, kind = ?, enabled = ?, updated_at = ?
         WHERE id = ?",
        params![
            next_goal,
            next_description,
            next_kind.as_str(),
            next_enabled as i64,
            now,
            thread_id
        ],
    )?;
    if let Some(assistants) = assistant_bindings.as_deref() {
        replace_thread_assistants(&tx, thread_id, assistants, now)?;
    }
    if let Some(participants) = agent_participants {
        replace_thread_agents(&tx, thread_id, participants, now)?;
    }
    let thread = load_thread_by_id(&tx, thread_id)?;
    tx.commit()?;
    Ok(thread)
}

pub(super) fn delete_thread(conn: &mut Connection, thread_id: &str) -> Result<()> {
    let tx = conn.transaction()?;
    // Collect every session this thread references through any link table.
    // ON DELETE CASCADE wipes those rows out; downgrading after the delete
    // restores sidebar visibility for sessions no longer attached anywhere.
    let mut session_refs: HashSet<(Agent, String)> = HashSet::new();
    for sql in [
        "SELECT agent, session_id FROM thread_sessions WHERE thread_id = ?",
        "SELECT s.agent, s.session_id FROM stage_sessions s
           INNER JOIN thread_stages ts ON ts.id = s.thread_stage_id
           WHERE ts.thread_id = ?",
        "SELECT s.agent, s.session_id FROM thread_plan_task_sessions s
           INNER JOIN thread_plan_tasks t ON t.id = s.task_id
           INNER JOIN thread_plan_rounds r ON r.id = t.round_id
           WHERE r.thread_id = ?",
        "SELECT s.agent, s.session_id FROM astra_run_sessions s
           INNER JOIN astra_runs r ON r.run_id = s.run_id
           WHERE r.thread_id = ?",
    ] {
        let mut stmt = tx.prepare(sql)?;
        let rows = stmt
            .query_map(params![thread_id], |row| {
                let agent_str: String = row.get(0)?;
                let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
                Ok((agent, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for entry in rows {
            session_refs.insert(entry);
        }
    }
    let changed = tx.execute("DELETE FROM threads WHERE id = ?", params![thread_id])?;
    if changed == 0 {
        anyhow::bail!("thread not found: {thread_id}");
    }
    for (agent, session_id) in &session_refs {
        downgrade_session_origin_when_unlinked(&tx, *agent, session_id)?;
    }
    tx.commit()?;
    Ok(())
}
