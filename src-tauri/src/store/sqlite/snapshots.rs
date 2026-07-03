use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{Agent, SessionHistoryTurn};
use crate::store::{SessionHistorySnapshotRecord, ThreadWorkSnapshotRecord};

pub(super) fn load_session_history_snapshots(
    conn: &Connection,
    child_agent: Agent,
    child_session_id: &str,
) -> Result<Vec<SessionHistorySnapshotRecord>> {
    let mut stmt = conn.prepare(
        "SELECT ancestor_index, ancestor_agent, ancestor_session_id,
                history_cache_version, created_at
         FROM session_history_snapshots
         WHERE child_agent = ? AND child_session_id = ?
         ORDER BY ancestor_index ASC",
    )?;
    let rows = stmt.query_map(params![child_agent.as_str(), child_session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;

    let mut snapshots = Vec::new();
    for row in rows {
        let (
            ancestor_index,
            ancestor_agent,
            ancestor_session_id,
            history_cache_version,
            created_at,
        ) = row?;
        let Some(ancestor_agent) = Agent::from_db_str(&ancestor_agent) else {
            continue;
        };
        let mut turns_stmt = conn.prepare(
            "SELECT turn_json
             FROM session_history_snapshot_turns
             WHERE child_agent = ? AND child_session_id = ? AND ancestor_index = ?
             ORDER BY turn_index ASC",
        )?;
        let turn_rows = turns_stmt.query_map(
            params![child_agent.as_str(), child_session_id, ancestor_index],
            |row| row.get::<_, String>(0),
        )?;
        let mut turns = Vec::new();
        for turn_row in turn_rows {
            turns.push(serde_json::from_str::<SessionHistoryTurn>(&turn_row?)?);
        }
        snapshots.push(SessionHistorySnapshotRecord {
            child_agent,
            child_session_id: child_session_id.to_string(),
            ancestor_agent,
            ancestor_session_id,
            ancestor_index,
            history_cache_version,
            created_at,
            turns,
        });
    }

    Ok(snapshots)
}

pub(super) fn replace_session_history_snapshots(
    conn: &Connection,
    child_agent: Agent,
    child_session_id: &str,
    snapshots: &[SessionHistorySnapshotRecord],
) -> Result<()> {
    conn.execute(
        "DELETE FROM session_history_snapshots
         WHERE child_agent = ? AND child_session_id = ?",
        params![child_agent.as_str(), child_session_id],
    )?;
    {
        let mut header_stmt = conn.prepare(
            "INSERT INTO session_history_snapshots (
                child_agent, child_session_id, ancestor_index, ancestor_agent,
                ancestor_session_id, history_cache_version, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )?;
        let mut turn_stmt = conn.prepare(
            "INSERT INTO session_history_snapshot_turns (
                child_agent, child_session_id, ancestor_index, turn_index, turn_id,
                started_at, updated_at, turn_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        for snapshot in snapshots {
            header_stmt.execute(params![
                child_agent.as_str(),
                child_session_id,
                snapshot.ancestor_index,
                snapshot.ancestor_agent.as_str(),
                snapshot.ancestor_session_id.as_str(),
                snapshot.history_cache_version,
                snapshot.created_at,
            ])?;
            for (index, turn) in snapshot.turns.iter().enumerate() {
                turn_stmt.execute(params![
                    child_agent.as_str(),
                    child_session_id,
                    snapshot.ancestor_index,
                    index as i64,
                    turn.turn_id.as_str(),
                    turn.started_at,
                    turn.updated_at,
                    serde_json::to_string(turn)?,
                ])?;
            }
        }
    }
    Ok(())
}

pub(super) fn save_thread_work_snapshot(
    conn: &Connection,
    snapshot: &ThreadWorkSnapshotRecord,
) -> Result<()> {
    conn.execute(
        "INSERT INTO thread_work_snapshots
            (child_agent, child_session_id, thread_id, stage_id, snapshot_json, version, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(child_agent, child_session_id) DO UPDATE SET
            thread_id = excluded.thread_id,
            stage_id = excluded.stage_id,
            snapshot_json = excluded.snapshot_json,
            version = excluded.version,
            created_at = excluded.created_at",
        params![
            snapshot.child_agent.as_str(),
            snapshot.child_session_id,
            snapshot.thread_id,
            snapshot.stage_id,
            snapshot.snapshot_json,
            snapshot.version,
            snapshot.created_at,
        ],
    )?;
    Ok(())
}

pub(super) fn get_thread_work_snapshot(
    conn: &Connection,
    child_agent: Agent,
    child_session_id: &str,
) -> Result<Option<ThreadWorkSnapshotRecord>> {
    conn.query_row(
        "SELECT child_agent, child_session_id, thread_id, stage_id, snapshot_json, version, created_at
         FROM thread_work_snapshots
         WHERE child_agent = ? AND child_session_id = ?",
        params![child_agent.as_str(), child_session_id],
        |row| {
            let agent_raw: String = row.get(0)?;
            Ok(ThreadWorkSnapshotRecord {
                child_agent: Agent::from_db_str(&agent_raw).unwrap_or(child_agent),
                child_session_id: row.get(1)?,
                thread_id: row.get(2)?,
                stage_id: row.get(3)?,
                snapshot_json: row.get(4)?,
                version: row.get(5)?,
                created_at: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}
