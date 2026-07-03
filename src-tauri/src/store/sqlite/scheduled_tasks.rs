use anyhow::Result;
use rusqlite::{params, params_from_iter, Connection};

use crate::models::Agent;
use crate::store::{
    ScheduledTaskRecord, ScheduledTaskRunRecord, SCHEDULED_TASK_RUN_HISTORY_LIMIT_PER_TASK,
};

fn scheduled_task_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ScheduledTaskRecord> {
    Ok(ScheduledTaskRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        status: row.get(2)?,
        schedule_json: row.get(3)?,
        target_json: row.get(4)?,
        project_id: row.get(5)?,
        mode: row.get(6)?,
        sort_order: row.get(7)?,
        created_at_ms: row.get(8)?,
        updated_at_ms: row.get(9)?,
        last_run_at_ms: row.get(10)?,
    })
}

fn scheduled_task_run_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ScheduledTaskRunRecord> {
    let session_agent_raw: Option<String> = row.get(10)?;
    Ok(ScheduledTaskRunRecord {
        id: row.get(0)?,
        task_id: row.get(1)?,
        mode: row.get(2)?,
        trigger: row.get(3)?,
        status: row.get(4)?,
        started_at_ms: row.get(5)?,
        scheduled_for_ms: row.get(6)?,
        completed_at_ms: row.get(7)?,
        task_name: row.get(8)?,
        target_json: row.get(9)?,
        session_agent: session_agent_raw.as_deref().and_then(Agent::from_db_str),
        session_id: row.get(11)?,
        agent_session_id: row.get(12)?,
        thread_id: row.get(13)?,
        astra_run_id: row.get(14)?,
        push_platform: row.get(15)?,
        push_chat_id: row.get(16)?,
        push_status: row.get(17)?,
        push_summary: row.get(18)?,
        push_error: row.get(19)?,
        push_sent_at_ms: row.get(20)?,
        error: row.get(21)?,
    })
}

pub(super) fn list_scheduled_tasks(conn: &Connection) -> Result<Vec<ScheduledTaskRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, status, schedule_json, target_json, project_id, mode,
                sort_order, created_at_ms, updated_at_ms, last_run_at_ms
         FROM scheduled_tasks
         ORDER BY sort_order ASC, created_at_ms ASC",
    )?;
    let records = stmt
        .query_map([], scheduled_task_record_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(records)
}

pub(super) fn list_scheduled_task_runs(conn: &Connection) -> Result<Vec<ScheduledTaskRunRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, mode, trigger, status, started_at_ms, scheduled_for_ms, completed_at_ms,
                task_name, target_json, session_agent, session_id, agent_session_id,
                thread_id, astra_run_id, push_platform, push_chat_id,
                push_status, push_summary, push_error, push_sent_at_ms, error
         FROM (
            SELECT id, task_id, mode, trigger, status, started_at_ms, scheduled_for_ms, completed_at_ms,
                   task_name, target_json, session_agent, session_id, agent_session_id,
                   thread_id, astra_run_id, push_platform, push_chat_id,
                   push_status, push_summary, push_error, push_sent_at_ms, error,
                   ROW_NUMBER() OVER (
                       PARTITION BY task_id
                       ORDER BY started_at_ms DESC, id DESC
                   ) AS run_rank
            FROM scheduled_task_runs
         )
         WHERE run_rank <= ?
            OR status = 'running'
            OR push_status IN ('pending', 'summarizing')
         ORDER BY started_at_ms DESC, id DESC",
    )?;
    let records = stmt
        .query_map(
            params![SCHEDULED_TASK_RUN_HISTORY_LIMIT_PER_TASK as i64],
            scheduled_task_run_record_from_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(records)
}

pub(super) fn list_scheduled_task_runs_requiring_update(
    conn: &Connection,
) -> Result<Vec<ScheduledTaskRunRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, mode, trigger, status, started_at_ms, scheduled_for_ms, completed_at_ms,
                task_name, target_json, session_agent, session_id, agent_session_id,
                thread_id, astra_run_id, push_platform, push_chat_id,
                push_status, push_summary, push_error, push_sent_at_ms, error
         FROM scheduled_task_runs
         WHERE status = 'running'
            OR push_status IN ('pending', 'summarizing')
         ORDER BY started_at_ms DESC, id DESC",
    )?;
    let records = stmt
        .query_map([], scheduled_task_run_record_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(records)
}

pub(super) fn replace_scheduled_tasks(
    conn: &mut Connection,
    tasks: &[ScheduledTaskRecord],
) -> Result<()> {
    let tx = conn.transaction()?;
    if tasks.is_empty() {
        tx.execute("DELETE FROM scheduled_tasks", [])?;
    } else {
        let placeholders = vec!["?"; tasks.len()].join(", ");
        let sql = format!("DELETE FROM scheduled_tasks WHERE id NOT IN ({placeholders})");
        let ids = tasks
            .iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>();
        tx.execute(&sql, params_from_iter(ids))?;
    }
    {
        let mut stmt = tx.prepare(
            "INSERT INTO scheduled_tasks (
                id, name, status, schedule_json, target_json, project_id, mode,
                sort_order, created_at_ms, updated_at_ms, last_run_at_ms
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                status = excluded.status,
                schedule_json = excluded.schedule_json,
                target_json = excluded.target_json,
                project_id = excluded.project_id,
                mode = excluded.mode,
                sort_order = excluded.sort_order,
                updated_at_ms = excluded.updated_at_ms,
                last_run_at_ms = excluded.last_run_at_ms",
        )?;
        for (index, task) in tasks.iter().enumerate() {
            stmt.execute(params![
                task.id.as_str(),
                task.name.as_str(),
                task.status.as_str(),
                task.schedule_json.as_str(),
                task.target_json.as_str(),
                task.project_id.as_str(),
                task.mode.as_str(),
                index as i64,
                task.created_at_ms,
                task.updated_at_ms,
                task.last_run_at_ms,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn insert_scheduled_task_run(
    conn: &Connection,
    run: &ScheduledTaskRunRecord,
) -> Result<()> {
    conn.execute(
        "INSERT INTO scheduled_task_runs (
            id, task_id, mode, trigger, status, started_at_ms, scheduled_for_ms, completed_at_ms,
            task_name, target_json, session_agent, session_id, agent_session_id, thread_id, astra_run_id,
            push_platform, push_chat_id, push_status, push_summary, push_error, push_sent_at_ms, error
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO NOTHING",
        params![
            run.id.as_str(),
            run.task_id.as_str(),
            run.mode.as_str(),
            run.trigger.as_str(),
            run.status.as_str(),
            run.started_at_ms,
            run.scheduled_for_ms,
            run.completed_at_ms,
            run.task_name.as_deref(),
            run.target_json.as_deref(),
            run.session_agent.map(|agent| agent.as_str().to_string()),
            run.session_id.as_deref(),
            run.agent_session_id.as_deref(),
            run.thread_id.as_deref(),
            run.astra_run_id.as_deref(),
            run.push_platform.as_deref(),
            run.push_chat_id.as_deref(),
            run.push_status.as_deref(),
            run.push_summary.as_deref(),
            run.push_error.as_deref(),
            run.push_sent_at_ms,
            run.error.as_deref(),
        ],
    )?;
    Ok(())
}

pub(super) fn update_scheduled_task_run_status(
    conn: &Connection,
    run_id: &str,
    status: &str,
    completed_at_ms: Option<i64>,
    error: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE scheduled_task_runs
         SET status = ?,
             completed_at_ms = CASE WHEN ? IS NULL THEN completed_at_ms ELSE ? END,
             error = CASE WHEN ? IS NULL THEN error ELSE ? END
         WHERE id = ?",
        params![
            status,
            completed_at_ms,
            completed_at_ms,
            error,
            error,
            run_id
        ],
    )?;
    Ok(())
}

pub(super) fn update_scheduled_task_run_agent_session_id(
    conn: &Connection,
    run_id: &str,
    agent_session_id: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE scheduled_task_runs
         SET agent_session_id = ?
         WHERE id = ?",
        params![agent_session_id, run_id],
    )?;
    Ok(())
}

pub(super) fn update_scheduled_task_run_push(
    conn: &Connection,
    run_id: &str,
    push_status: &str,
    push_summary: Option<&str>,
    push_error: Option<&str>,
    push_sent_at_ms: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE scheduled_task_runs
         SET push_status = ?,
             push_summary = CASE WHEN ? IS NULL THEN push_summary ELSE ? END,
             push_error = ?,
             push_sent_at_ms = CASE WHEN ? IS NULL THEN push_sent_at_ms ELSE ? END
         WHERE id = ?",
        params![
            push_status,
            push_summary,
            push_summary,
            push_error,
            push_sent_at_ms,
            push_sent_at_ms,
            run_id,
        ],
    )?;
    Ok(())
}

pub(super) fn update_scheduled_task_last_run(
    conn: &Connection,
    task_id: &str,
    when_ms: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE scheduled_tasks
         SET last_run_at_ms = ?
         WHERE id = ?",
        params![when_ms, task_id],
    )?;
    Ok(())
}

pub(super) fn fail_interrupted_task_run_pushes(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE scheduled_task_runs
         SET push_status = 'failed',
             push_error = COALESCE(push_error, 'push interrupted by app restart')
         WHERE push_status = 'summarizing'",
        [],
    )?;
    Ok(())
}
