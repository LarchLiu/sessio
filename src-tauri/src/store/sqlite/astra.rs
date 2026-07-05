use anyhow::Result;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};

use crate::models::{Agent, AstraConfig, PlanTaskSessionRole, ThreadAstraArtifactInfo};
use crate::store::{
    now_ms, AstraConfigPatch, AstraRunRecord, AstraRunSessionRecord, NewThreadAstraArtifact,
};

use super::identity::{downgrade_session_origin_when_unlinked, upgrade_session_origin_to_thread};

const ASTRA_RUN_SELECT: &str = "run_id, thread_id, project_id, project_path, status, mode,
    planner_backend, round_index, round_limit, terminal_reason,
    last_error_code, last_error_message, run_diagnostics_json, error, created_at, updated_at";
const ACTIVE_ASTRA_RUN_STATUS_SQL: &str =
    "'planning', 'thinking', 'awaiting_approval', 'dispatching', 'running'";
const ASTRA_ARTIFACT_SELECT: &str = "id, thread_id, astra_run_id, source_task_id, role, title,
    path, summary, is_current, created_at, updated_at, superseded_at";

fn astra_run_from_row_without_sessions(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<AstraRunRecord> {
    Ok(AstraRunRecord {
        run_id: row.get(0)?,
        thread_id: row.get(1)?,
        project_id: row.get(2)?,
        project_path: row.get(3)?,
        status: row.get(4)?,
        mode: row.get(5)?,
        planner_backend: row.get(6)?,
        round_index: row.get(7)?,
        round_limit: row.get(8)?,
        terminal_reason: row.get(9)?,
        last_error_code: row.get(10)?,
        last_error_message: row.get(11)?,
        internal_planner_sessions: Vec::new(),
        run_diagnostics_json: row.get(12)?,
        error: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

fn astra_artifact_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadAstraArtifactInfo> {
    let is_current: i64 = row.get(8)?;
    Ok(ThreadAstraArtifactInfo {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        astra_run_id: row.get(2)?,
        source_task_id: row.get(3)?,
        role: row.get(4)?,
        title: row.get(5)?,
        path: row.get(6)?,
        summary: row.get(7)?,
        is_current: is_current != 0,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        superseded_at: row.get(11)?,
    })
}

fn astra_run_session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AstraRunSessionRecord> {
    let agent_raw: String = row.get(1)?;
    let role_raw: String = row.get(3)?;
    Ok(AstraRunSessionRecord {
        run_id: row.get(0)?,
        agent: Agent::from_db_str(&agent_raw).unwrap_or(Agent::Pi),
        session_id: row.get(2)?,
        role: PlanTaskSessionRole::from_db_str(&role_raw).unwrap_or(PlanTaskSessionRole::Planner),
        sort_order: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

pub(super) fn get_astra_config(conn: &Connection) -> Result<AstraConfig> {
    let mut stmt = conn.prepare(
        "SELECT agent, model, effort, permission_mode, created_at, updated_at
         FROM astra_config WHERE id = 1",
    )?;
    let config = stmt.query_row([], |row| {
        Ok(AstraConfig {
            agent: row.get(0)?,
            model: row.get(1)?,
            effort: row.get(2)?,
            permission_mode: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })?;
    Ok(config)
}

pub(super) fn update_astra_config(
    conn: &Connection,
    patch: AstraConfigPatch<'_>,
) -> Result<AstraConfig> {
    let now = now_ms();

    let mut updates = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(value) = patch.agent {
        updates.push("agent = ?");
        params.push(Box::new(value));
    }
    if let Some(value) = patch.model {
        updates.push("model = ?");
        params.push(Box::new(value));
    }
    if let Some(value) = patch.effort {
        updates.push("effort = ?");
        params.push(Box::new(value));
    }
    if let Some(value) = patch.permission_mode {
        updates.push("permission_mode = ?");
        params.push(Box::new(value));
    }

    if updates.is_empty() {
        return get_astra_config(conn);
    }

    updates.push("updated_at = ?");
    params.push(Box::new(now));

    let sql = format!(
        "UPDATE astra_config SET {} WHERE id = 1",
        updates.join(", ")
    );
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    conn.execute(&sql, param_refs.as_slice())?;

    get_astra_config(conn)
}

pub(super) fn list_astra_run_sessions(
    conn: &Connection,
    run_id: &str,
) -> Result<Vec<AstraRunSessionRecord>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, agent, session_id, role, sort_order, created_at, updated_at
         FROM astra_run_sessions
         WHERE run_id = ?
         ORDER BY sort_order ASC, created_at ASC, session_id ASC",
    )?;
    let rows = stmt.query_map(params![run_id], astra_run_session_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn hydrate_astra_run_sessions(conn: &Connection, runs: &mut [AstraRunRecord]) -> Result<()> {
    if runs.is_empty() {
        return Ok(());
    }
    let run_ids = runs
        .iter()
        .map(|run| run.run_id.clone())
        .collect::<Vec<_>>();
    let mut sql = String::from(
        "SELECT run_id, agent, session_id, role, sort_order, created_at, updated_at
         FROM astra_run_sessions
         WHERE run_id IN (",
    );
    let mut values = Vec::<SqlValue>::with_capacity(run_ids.len());
    for (index, run_id) in run_ids.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push('?');
        values.push(SqlValue::from(run_id.clone()));
    }
    sql.push_str(") ORDER BY run_id ASC, sort_order ASC, created_at ASC, session_id ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values.iter()), astra_run_session_from_row)?;
    let mut grouped = HashMap::<String, Vec<AstraRunSessionRecord>>::new();
    for row in rows {
        let record = row?;
        grouped
            .entry(record.run_id.clone())
            .or_default()
            .push(record);
    }
    for run in runs {
        run.internal_planner_sessions = grouped.remove(&run.run_id).unwrap_or_default();
    }
    Ok(())
}

pub(super) fn replace_astra_run_sessions(
    conn: &Connection,
    run_id: &str,
    sessions: &[AstraRunSessionRecord],
) -> Result<()> {
    // Capture the prior (agent, session_id) set before the DELETE. We only
    // need to downgrade entries that don't reappear in `sessions`; the new
    // INSERTs below re-upgrade everything that's still listed. Computing the
    // set difference up front avoids redundant `still_linked` queries when
    // prior and sessions overlap heavily.
    let prior: HashSet<(Agent, String)> = {
        let mut stmt =
            conn.prepare("SELECT agent, session_id FROM astra_run_sessions WHERE run_id = ?")?;
        let rows = stmt
            .query_map(params![run_id], |row| {
                let agent_str: String = row.get(0)?;
                let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
                Ok((agent, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        rows
    };
    let next: HashSet<(Agent, String)> = sessions
        .iter()
        .map(|session| (session.agent, session.session_id.clone()))
        .collect();
    conn.execute(
        "DELETE FROM astra_run_sessions WHERE run_id = ?",
        params![run_id],
    )?;
    if !sessions.is_empty() {
        let mut stmt = conn.prepare(
            "INSERT INTO astra_run_sessions (
                run_id, agent, session_id, role, sort_order, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )?;
        for session in sessions {
            stmt.execute(params![
                run_id,
                session.agent.as_str(),
                session.session_id,
                session.role.as_str(),
                session.sort_order,
                session.created_at,
                session.updated_at,
            ])?;
            upgrade_session_origin_to_thread(conn, session.agent, &session.session_id)?;
        }
    }
    for (agent, session_id) in prior.difference(&next) {
        downgrade_session_origin_when_unlinked(conn, *agent, session_id)?;
    }
    Ok(())
}

pub(super) fn list_astra_run_sessions_for_thread(
    conn: &Connection,
    thread_id: &str,
) -> Result<Vec<AstraRunSessionRecord>> {
    let mut stmt = conn.prepare(
        "SELECT s.run_id, s.agent, s.session_id, s.role, s.sort_order, s.created_at, s.updated_at
         FROM astra_run_sessions s
         INNER JOIN astra_runs r ON r.run_id = s.run_id
         WHERE r.thread_id = ?
         ORDER BY r.updated_at DESC, r.created_at DESC, s.sort_order ASC, s.created_at ASC",
    )?;
    let rows = stmt.query_map(params![thread_id], astra_run_session_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(super) fn upsert_astra_run(conn: &mut Connection, run: &AstraRunRecord) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO astra_runs (
            run_id, thread_id, project_id, project_path, status, mode,
            planner_backend, round_index, round_limit, terminal_reason,
            last_error_code, last_error_message, run_diagnostics_json,
            error, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(run_id) DO UPDATE SET
            thread_id = excluded.thread_id,
            project_id = excluded.project_id,
            project_path = excluded.project_path,
            status = excluded.status,
            mode = excluded.mode,
            planner_backend = excluded.planner_backend,
            round_index = excluded.round_index,
            round_limit = excluded.round_limit,
            terminal_reason = excluded.terminal_reason,
            last_error_code = excluded.last_error_code,
            last_error_message = excluded.last_error_message,
            run_diagnostics_json = excluded.run_diagnostics_json,
            error = excluded.error,
            updated_at = excluded.updated_at",
        params![
            run.run_id,
            run.thread_id,
            run.project_id,
            run.project_path,
            run.status,
            run.mode,
            run.planner_backend,
            run.round_index,
            run.round_limit,
            run.terminal_reason,
            run.last_error_code,
            run.last_error_message,
            run.run_diagnostics_json,
            run.error,
            run.created_at,
            run.updated_at,
        ],
    )?;
    replace_astra_run_sessions(&tx, &run.run_id, &run.internal_planner_sessions)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn get_astra_run(conn: &Connection, run_id: &str) -> Result<Option<AstraRunRecord>> {
    let sql = format!("SELECT {ASTRA_RUN_SELECT} FROM astra_runs WHERE run_id = ?");
    let mut run = conn
        .query_row(&sql, params![run_id], astra_run_from_row_without_sessions)
        .optional()
        .map_err(anyhow::Error::from)?;
    if let Some(run) = run.as_mut() {
        run.internal_planner_sessions = list_astra_run_sessions(conn, &run.run_id)?;
    }
    Ok(run)
}

pub(super) fn get_active_astra_run(
    conn: &Connection,
    thread_id: &str,
) -> Result<Option<AstraRunRecord>> {
    let sql = format!(
        "SELECT {ASTRA_RUN_SELECT}
         FROM astra_runs
         WHERE thread_id = ?
           AND status IN ({ACTIVE_ASTRA_RUN_STATUS_SQL})
         ORDER BY updated_at DESC
         LIMIT 1"
    );
    let mut run = conn
        .query_row(
            &sql,
            params![thread_id],
            astra_run_from_row_without_sessions,
        )
        .optional()
        .map_err(anyhow::Error::from)?;
    if let Some(run) = run.as_mut() {
        run.internal_planner_sessions = list_astra_run_sessions(conn, &run.run_id)?;
    }
    Ok(run)
}

pub(super) fn list_astra_runs(conn: &Connection, thread_id: &str) -> Result<Vec<AstraRunRecord>> {
    let sql = format!(
        "SELECT {ASTRA_RUN_SELECT}
         FROM astra_runs
         WHERE thread_id = ?
         ORDER BY updated_at DESC, created_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![thread_id], astra_run_from_row_without_sessions)?;
    let mut runs = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    hydrate_astra_run_sessions(conn, &mut runs)?;
    Ok(runs)
}

pub(super) fn list_current_astra_artifacts(
    conn: &Connection,
    thread_id: &str,
) -> Result<Vec<ThreadAstraArtifactInfo>> {
    let sql = format!(
        "SELECT {ASTRA_ARTIFACT_SELECT}
         FROM thread_astra_artifacts
         WHERE thread_id = ? AND is_current = 1
         ORDER BY role ASC, updated_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![thread_id], astra_artifact_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(super) fn list_astra_artifacts_by_role(
    conn: &Connection,
    thread_id: &str,
    role: &str,
) -> Result<Vec<ThreadAstraArtifactInfo>> {
    let sql = format!(
        "SELECT {ASTRA_ARTIFACT_SELECT}
         FROM thread_astra_artifacts
         WHERE thread_id = ? AND role = ?
         ORDER BY is_current DESC, updated_at DESC, created_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![thread_id, role], astra_artifact_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(super) fn register_current_astra_artifact(
    conn: &mut Connection,
    artifact: NewThreadAstraArtifact<'_>,
) -> Result<ThreadAstraArtifactInfo> {
    let now = now_ms();
    let id = format!(
        "artifact-{}-{}",
        crate::astra::short_hash(&format!(
            "{}:{}:{}:{}:{}:{}",
            artifact.thread_id,
            artifact.astra_run_id,
            artifact.source_task_id,
            artifact.role,
            artifact.path,
            artifact.title
        )),
        now
    );
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE thread_astra_artifacts
         SET is_current = 0,
             superseded_at = COALESCE(superseded_at, ?),
             updated_at = ?
         WHERE thread_id = ? AND role = ? AND is_current = 1",
        params![now, now, artifact.thread_id, artifact.role],
    )?;
    tx.execute(
        "INSERT INTO thread_astra_artifacts (
            id, thread_id, astra_run_id, source_task_id, role, title, path, summary,
            is_current, created_at, updated_at, superseded_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, NULL)",
        params![
            id,
            artifact.thread_id,
            artifact.astra_run_id,
            artifact.source_task_id,
            artifact.role,
            artifact.title,
            artifact.path,
            artifact.summary,
            now,
            now,
        ],
    )?;
    let sql = format!("SELECT {ASTRA_ARTIFACT_SELECT} FROM thread_astra_artifacts WHERE id = ?");
    let inserted = tx.query_row(&sql, params![id], astra_artifact_from_row)?;
    tx.commit()?;
    Ok(inserted)
}

pub(super) fn interrupt_active_astra_runs(conn: &mut Connection) -> Result<Vec<AstraRunRecord>> {
    let tx = conn.transaction()?;
    let now = now_ms();
    let mut active: Vec<AstraRunRecord> = {
        let sql = format!(
            "SELECT {ASTRA_RUN_SELECT}
             FROM astra_runs
             WHERE status IN ({ACTIVE_ASTRA_RUN_STATUS_SQL})"
        );
        let mut stmt = tx.prepare(&sql)?;
        let rows = stmt.query_map([], astra_run_from_row_without_sessions)?;
        let mut runs = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        hydrate_astra_run_sessions(&tx, &mut runs)?;
        runs
    };
    let mut placeholder_session_ids = HashSet::new();
    for run in &active {
        for session_id in run
            .internal_planner_sessions
            .iter()
            .map(|session| session.session_id.as_str())
        {
            if !session_id.trim().is_empty() {
                placeholder_session_ids.insert(session_id.to_string());
            }
        }
        let mut stmt = tx.prepare(
            "SELECT DISTINCT s.session_id
             FROM thread_plan_task_sessions s
             INNER JOIN thread_plan_tasks t ON t.id = s.task_id
             INNER JOIN thread_plan_rounds r ON r.id = t.round_id
             WHERE r.astra_run_id = ?",
        )?;
        let rows = stmt.query_map(params![run.run_id], |row| row.get::<_, String>(0))?;
        for session_id in rows.collect::<rusqlite::Result<Vec<_>>>()? {
            if !session_id.trim().is_empty() {
                placeholder_session_ids.insert(session_id);
            }
        }
    }
    let update_active_runs_sql = format!(
        "UPDATE astra_runs
         SET status = 'interrupted',
             terminal_reason = COALESCE(terminal_reason, 'process_recovered_active_run'),
             last_error_code = COALESCE(last_error_code, 'worker_interrupted'),
             last_error_message = COALESCE(last_error_message, 'Astra run was active during startup recovery'),
             error = COALESCE(error, 'Astra run was active during startup recovery'),
             updated_at = ?
         WHERE status IN ({ACTIVE_ASTRA_RUN_STATUS_SQL})"
    );
    tx.execute(&update_active_runs_sql, params![now])?;
    for run in &active {
        tx.execute(
            "UPDATE thread_plan_tasks
             SET status = 'errored',
                 error = COALESCE(error, 'Astra task was active during startup recovery'),
                 result_summary = COALESCE(result_summary, 'Interrupted during startup recovery'),
                 completed_at = COALESCE(completed_at, ?),
                 updated_at = ?
             WHERE status = 'running'
               AND round_id IN (
                   SELECT id
                   FROM thread_plan_rounds
                   WHERE astra_run_id = ?
               )",
            params![now, now, run.run_id],
        )?;
        tx.execute(
            "UPDATE thread_plan_rounds
             SET status = 'errored',
                 updated_at = ?
             WHERE astra_run_id = ?
               AND status IN ('planned', 'running')",
            params![now, run.run_id],
        )?;
    }
    for session_id in &placeholder_session_ids {
        tx.execute(
            "UPDATE sessions
             SET available = 0, archived = 1, last_indexed_at = ?
             WHERE session_id = ?
               AND partial = 1
               AND file_size = 0
               AND available = 1",
            params![now, session_id],
        )?;
    }
    tx.commit()?;
    for run in &mut active {
        run.status = "interrupted".to_string();
        if run.terminal_reason.is_none() {
            run.terminal_reason = Some("process_recovered_active_run".to_string());
        }
        if run.last_error_code.is_none() {
            run.last_error_code = Some("worker_interrupted".to_string());
        }
        if run.last_error_message.is_none() {
            run.last_error_message =
                Some("Astra run was active during startup recovery".to_string());
        }
        if run.error.is_none() {
            run.error = Some("Astra run was active during startup recovery".to_string());
        }
        run.updated_at = now;
    }
    Ok(active)
}

pub(super) fn cleanup_partial_astra_sessions(
    conn: &mut Connection,
    session_ids: &[String],
) -> Result<usize> {
    if session_ids.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let mut changed = 0usize;
    for session_id in session_ids {
        changed += tx.execute(
            "UPDATE sessions
             SET available = 0, archived = 1, last_indexed_at = ?
             WHERE session_id = ?
               AND partial = 1
               AND file_size = 0
               AND available = 1",
            params![now_ms(), session_id],
        )?;
    }
    tx.commit()?;
    Ok(changed)
}
