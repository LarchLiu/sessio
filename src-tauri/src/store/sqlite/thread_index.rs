use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};

use crate::models::{ThreadIndexItemInfo, ThreadKind, ThreadOrigin};

fn thread_index_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadIndexItemInfo> {
    let kind_raw: String = row.get(3)?;
    let origin_raw: String = row.get(7)?;
    Ok(ThreadIndexItemInfo {
        thread_id: row.get(0)?,
        project_id: row.get(1)?,
        goal: row.get(2)?,
        kind: ThreadKind::from_db_str(&kind_raw).unwrap_or_default(),
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        time: row.get(6)?,
        origin: ThreadOrigin::from_db_str(&origin_raw).unwrap_or_default(),
        scheduled_task_id: row.get(8)?,
        session_keys: Vec::new(),
    })
}

fn load_thread_index_session_keys(
    conn: &Connection,
    project_id: Option<&str>,
) -> Result<HashMap<String, HashSet<String>>> {
    let mut keys = HashMap::<String, HashSet<String>>::new();
    for sql in [
        "SELECT s.thread_id, s.agent, s.session_id
         FROM thread_sessions s
         INNER JOIN threads t ON t.id = s.thread_id
         WHERE (?1 IS NULL OR t.project_id = ?1)",
        "SELECT ts.thread_id, ss.agent, ss.session_id
         FROM stage_sessions ss
         INNER JOIN thread_stages ts ON ts.id = ss.thread_stage_id
         INNER JOIN threads t ON t.id = ts.thread_id
         WHERE (?1 IS NULL OR t.project_id = ?1)",
        "SELECT r.thread_id, s.agent, s.session_id
         FROM thread_plan_task_sessions s
         INNER JOIN thread_plan_tasks tk ON tk.id = s.task_id
         INNER JOIN thread_plan_rounds r ON r.id = tk.round_id
         INNER JOIN threads t ON t.id = r.thread_id
         WHERE s.superseded_at IS NULL AND (?1 IS NULL OR t.project_id = ?1)",
        "SELECT r.thread_id, s.agent, s.session_id
         FROM astra_run_sessions s
         INNER JOIN astra_runs r ON r.run_id = s.run_id
         INNER JOIN threads t ON t.id = r.thread_id
         WHERE (?1 IS NULL OR t.project_id = ?1)",
    ] {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![project_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (thread_id, agent, session_id) = row?;
            keys.entry(thread_id)
                .or_default()
                .insert(format!("{agent}:{session_id}"));
        }
    }
    Ok(keys)
}

pub(super) fn list_thread_index(
    conn: &Connection,
    project_id: Option<&str>,
) -> Result<Vec<ThreadIndexItemInfo>> {
    let mut stmt = conn.prepare(
        "WITH base AS (
            SELECT t.id, t.project_id, t.goal, t.kind, t.created_at, t.updated_at,
                   t.origin, t.scheduled_task_id
            FROM threads t
            INNER JOIN projects p ON p.id = t.project_id AND p.archived = 0
            WHERE (?1 IS NULL OR t.project_id = ?1)
         ), thread_times AS (
            SELECT id AS thread_id, created_at AS time FROM base
            UNION ALL SELECT id, updated_at FROM base
            UNION ALL SELECT b.id, ts.created_at FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id
            UNION ALL SELECT b.id, ts.updated_at FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id
            UNION ALL SELECT b.id, tss.created_at FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id INNER JOIN thread_stage_states tss ON tss.thread_stage_id = ts.id
            UNION ALL SELECT b.id, tss.updated_at FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id INNER JOIN thread_stage_states tss ON tss.thread_stage_id = ts.id
            UNION ALL SELECT b.id, s.created_at FROM base b INNER JOIN thread_sessions s ON s.thread_id = b.id
            UNION ALL SELECT b.id, COALESCE(sess.updated_at, sess.started_at) FROM base b INNER JOIN thread_sessions s ON s.thread_id = b.id INNER JOIN sessions sess ON sess.agent = s.agent AND sess.session_id = s.session_id
            UNION ALL SELECT b.id, ss.created_at FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id INNER JOIN stage_sessions ss ON ss.thread_stage_id = ts.id
            UNION ALL SELECT b.id, COALESCE(sess.updated_at, sess.started_at) FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id INNER JOIN stage_sessions ss ON ss.thread_stage_id = ts.id INNER JOIN sessions sess ON sess.agent = ss.agent AND sess.session_id = ss.session_id
            UNION ALL SELECT b.id, r.created_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id
            UNION ALL SELECT b.id, r.updated_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id
            UNION ALL SELECT b.id, t.created_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id INNER JOIN thread_plan_tasks t ON t.round_id = r.id
            UNION ALL SELECT b.id, t.updated_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id INNER JOIN thread_plan_tasks t ON t.round_id = r.id
            UNION ALL SELECT b.id, pts.created_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id INNER JOIN thread_plan_tasks t ON t.round_id = r.id INNER JOIN thread_plan_task_sessions pts ON pts.task_id = t.id AND pts.superseded_at IS NULL
            UNION ALL SELECT b.id, pts.updated_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id INNER JOIN thread_plan_tasks t ON t.round_id = r.id INNER JOIN thread_plan_task_sessions pts ON pts.task_id = t.id AND pts.superseded_at IS NULL
            UNION ALL SELECT b.id, ar.created_at FROM base b INNER JOIN astra_runs ar ON ar.thread_id = b.id
            UNION ALL SELECT b.id, ar.updated_at FROM base b INNER JOIN astra_runs ar ON ar.thread_id = b.id
            UNION ALL SELECT b.id, ars.created_at FROM base b INNER JOIN astra_runs ar ON ar.thread_id = b.id INNER JOIN astra_run_sessions ars ON ars.run_id = ar.run_id
            UNION ALL SELECT b.id, ars.updated_at FROM base b INNER JOIN astra_runs ar ON ar.thread_id = b.id INNER JOIN astra_run_sessions ars ON ars.run_id = ar.run_id
         )
         SELECT b.id, b.project_id, b.goal, b.kind, b.created_at, b.updated_at, MAX(tt.time) AS time,
                b.origin, b.scheduled_task_id
         FROM base b
         INNER JOIN thread_times tt ON tt.thread_id = b.id
         GROUP BY b.id, b.project_id, b.goal, b.kind, b.created_at, b.updated_at, b.origin, b.scheduled_task_id
         ORDER BY time DESC, b.updated_at DESC, b.created_at DESC",
    )?;
    let rows = stmt.query_map(params![project_id], thread_index_from_row)?;
    let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut keys_by_thread = load_thread_index_session_keys(conn, project_id)?;
    for item in items.iter_mut() {
        if let Some(keys) = keys_by_thread.remove(&item.thread_id) {
            let mut keys = keys.into_iter().collect::<Vec<_>>();
            keys.sort();
            item.session_keys = keys;
        }
    }
    Ok(items)
}
