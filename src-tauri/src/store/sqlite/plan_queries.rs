use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{
    Agent, PlanRoundInfo, PlanRoundMode, PlanRoundSource, PlanRoundStatus, PlanTaskInfo,
    PlanTaskRisk, PlanTaskSessionInfo, PlanTaskSessionRole, PlanTaskStatus,
};

pub(super) fn plan_round_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanRoundInfo> {
    let mode_raw: String = row.get(5)?;
    let source_raw: String = row.get(6)?;
    let status_raw: String = row.get(7)?;
    Ok(PlanRoundInfo {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        astra_run_id: row.get(2)?,
        round_index: row.get(3)?,
        summary: row.get(4)?,
        mode: PlanRoundMode::from_db_str(&mode_raw).unwrap_or(PlanRoundMode::Parallel),
        source: PlanRoundSource::from_db_str(&source_raw).unwrap_or(PlanRoundSource::Manual),
        status: PlanRoundStatus::from_db_str(&status_raw).unwrap_or(PlanRoundStatus::Planned),
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        tasks: Vec::new(),
    })
}

fn plan_task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanTaskInfo> {
    let target_agent_raw: String = row.get(5)?;
    let risk_raw: String = row.get(12)?;
    let status_raw: String = row.get(14)?;
    Ok(PlanTaskInfo {
        id: row.get(0)?,
        round_id: row.get(1)?,
        thread_stage_id: row.get(2)?,
        assistant_id: row.get(3)?,
        agent_participant_id: row.get(4)?,
        target_agent: Agent::from_db_str(&target_agent_raw).unwrap_or(Agent::Codex),
        stage_snapshot_json: row.get(6)?,
        assistant_snapshot_json: row.get(7)?,
        agent_snapshot_json: row.get(8)?,
        title: row.get(9)?,
        prompt: row.get(10)?,
        expected_output: row.get(11)?,
        risk: PlanTaskRisk::from_db_str(&risk_raw).unwrap_or(PlanTaskRisk::Medium),
        sort_order: row.get(13)?,
        status: PlanTaskStatus::from_db_str(&status_raw).unwrap_or(PlanTaskStatus::Planned),
        result_summary: row.get(15)?,
        error: row.get(16)?,
        started_at: row.get(17)?,
        completed_at: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
        sessions: Vec::new(),
    })
}

pub(super) fn plan_task_session_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PlanTaskSessionInfo> {
    let agent_raw: String = row.get(1)?;
    let role_raw: String = row.get(3)?;
    Ok(PlanTaskSessionInfo {
        task_id: row.get(0)?,
        agent: Agent::from_db_str(&agent_raw).unwrap_or(Agent::Codex),
        session_id: row.get(2)?,
        role: PlanTaskSessionRole::from_db_str(&role_raw).unwrap_or(PlanTaskSessionRole::Runtime),
        attempt_id: row.get(4)?,
        attempt_count: row.get(5)?,
        superseded_at: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

pub(super) fn load_plan_round_by_id(conn: &Connection, round_id: &str) -> Result<PlanRoundInfo> {
    let mut round = conn
        .query_row(
            "SELECT id, thread_id, astra_run_id, round_index, summary, mode, source, status, created_at, updated_at
             FROM thread_plan_rounds
             WHERE id = ?",
            params![round_id],
            plan_round_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("plan round not found: {round_id}"))?;
    round.tasks = load_plan_tasks(conn, &round.id)?;
    Ok(round)
}

pub(super) fn load_plan_task_by_id(conn: &Connection, task_id: &str) -> Result<PlanTaskInfo> {
    let mut task = conn
        .query_row(
            "SELECT id, round_id, thread_stage_id, assistant_id, agent_participant_id, target_agent,
                    stage_snapshot_json, assistant_snapshot_json, agent_snapshot_json,
                    title, prompt, expected_output, risk, sort_order, status,
                    result_summary, error, started_at, completed_at, created_at, updated_at
             FROM thread_plan_tasks
             WHERE id = ?",
            params![task_id],
            plan_task_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("plan task not found: {task_id}"))?;
    task.sessions = load_plan_task_sessions(conn, &task.id)?;
    Ok(task)
}

pub(super) fn load_plan_tasks(conn: &Connection, round_id: &str) -> Result<Vec<PlanTaskInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, round_id, thread_stage_id, assistant_id, agent_participant_id, target_agent,
                stage_snapshot_json, assistant_snapshot_json, agent_snapshot_json,
                title, prompt, expected_output, risk, sort_order, status,
                result_summary, error, started_at, completed_at, created_at, updated_at
         FROM thread_plan_tasks
         WHERE round_id = ?
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let mut tasks = stmt
        .query_map(params![round_id], plan_task_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for task in tasks.iter_mut() {
        task.sessions = load_plan_task_sessions(conn, &task.id)?;
    }
    Ok(tasks)
}

pub(super) fn load_plan_task_sessions(
    conn: &Connection,
    task_id: &str,
) -> Result<Vec<PlanTaskSessionInfo>> {
    let mut stmt = conn.prepare(
        "SELECT task_id, agent, session_id, role, attempt_id, attempt_count, superseded_at, created_at, updated_at
         FROM thread_plan_task_sessions
         WHERE task_id = ?
         ORDER BY attempt_count ASC, created_at ASC, role ASC, agent ASC, session_id ASC",
    )?;
    let rows = stmt.query_map(params![task_id], plan_task_session_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
