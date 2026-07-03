use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{Agent, ChannelSessionInfo};
use crate::store::ChannelSessionRecord;

fn channel_session_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ChannelSessionRecord> {
    let agent_str: String = row.get(7)?;
    let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
    Ok(ChannelSessionRecord {
        platform: row.get(0)?,
        channel_id: row.get(1)?,
        channel_type: row.get(2)?,
        user_id: row.get(3)?,
        team_id: row.get(4)?,
        thread_id: row.get(5)?,
        display_name: row.get(6)?,
        agent,
        agent_session_id: row.get(8)?,
        sessio_runtime_session_id: row.get(9)?,
        workspace_path: row.get(10)?,
        metadata_json: row.get(11)?,
        last_update_id: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        last_activity_at: row.get(15)?,
        ended_at: row.get(16)?,
    })
}

fn channel_session_info_from_record(record: ChannelSessionRecord) -> ChannelSessionInfo {
    let metadata = serde_json::from_str::<serde_json::Value>(&record.metadata_json)
        .unwrap_or_else(|_| serde_json::json!({}));
    ChannelSessionInfo {
        platform: record.platform,
        channel_id: record.channel_id,
        channel_type: record.channel_type,
        user_id: record.user_id,
        team_id: record.team_id,
        thread_id: record.thread_id,
        display_name: record.display_name,
        agent: record.agent,
        agent_session_id: record.agent_session_id,
        sessio_runtime_session_id: record.sessio_runtime_session_id,
        workspace_path: record.workspace_path,
        metadata,
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_activity_at: record.last_activity_at,
        ended_at: record.ended_at,
    }
}

fn select_channel_session_columns() -> &'static str {
    "platform, channel_id, channel_type, user_id, team_id, thread_id, display_name,
     agent, agent_session_id, sessio_runtime_session_id, workspace_path, metadata_json,
     last_update_id, created_at, updated_at, last_activity_at, ended_at"
}

pub(super) fn list_channel_sessions(conn: &Connection) -> Result<Vec<ChannelSessionInfo>> {
    let sql = format!(
        "SELECT {} FROM channel_sessions ORDER BY updated_at DESC",
        select_channel_session_columns()
    );
    let mut stmt = conn.prepare(&sql)?;
    let records = stmt
        .query_map([], channel_session_record_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(records
        .into_iter()
        .map(channel_session_info_from_record)
        .collect())
}

pub(super) fn get_active_channel_session(
    conn: &Connection,
    platform: &str,
    channel_id: &str,
) -> Result<Option<ChannelSessionRecord>> {
    let sql = format!(
        "SELECT {} FROM channel_sessions
         WHERE platform = ? AND channel_id = ? AND ended_at IS NULL
         ORDER BY last_activity_at DESC, updated_at DESC
         LIMIT 1",
        select_channel_session_columns()
    );
    conn.query_row(
        &sql,
        params![platform, channel_id],
        channel_session_record_from_row,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn upsert_channel_session(
    conn: &Connection,
    record: &ChannelSessionRecord,
) -> Result<()> {
    if record.ended_at.is_none() {
        conn.execute(
            "UPDATE channel_sessions
             SET ended_at = ?, updated_at = ?
             WHERE platform = ? AND channel_id = ? AND ended_at IS NULL
               AND NOT (agent = ? AND agent_session_id = ?)",
            params![
                record.updated_at,
                record.updated_at,
                record.platform.as_str(),
                record.channel_id.as_str(),
                record.agent.as_str(),
                record.agent_session_id.as_str(),
            ],
        )?;
    }
    conn.execute(
        "INSERT INTO channel_sessions (
            platform, channel_id, channel_type, user_id, team_id, thread_id, display_name,
            agent, agent_session_id, sessio_runtime_session_id, workspace_path, metadata_json,
            last_update_id, created_at, updated_at, last_activity_at, ended_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(platform, channel_id, agent, agent_session_id) DO UPDATE SET
            channel_type = excluded.channel_type,
            user_id = excluded.user_id,
            team_id = excluded.team_id,
            thread_id = excluded.thread_id,
            display_name = excluded.display_name,
            sessio_runtime_session_id = excluded.sessio_runtime_session_id,
            workspace_path = excluded.workspace_path,
            metadata_json = excluded.metadata_json,
            last_update_id = CASE
                WHEN excluded.last_update_id IS NULL THEN channel_sessions.last_update_id
                WHEN channel_sessions.last_update_id IS NULL THEN excluded.last_update_id
                WHEN excluded.last_update_id > channel_sessions.last_update_id THEN excluded.last_update_id
                ELSE channel_sessions.last_update_id
            END,
            updated_at = excluded.updated_at,
            last_activity_at = excluded.last_activity_at,
            ended_at = excluded.ended_at",
        params![
            record.platform.as_str(),
            record.channel_id.as_str(),
            record.channel_type.as_deref(),
            record.user_id.as_deref(),
            record.team_id.as_deref(),
            record.thread_id.as_deref(),
            record.display_name.as_deref(),
            record.agent.as_str(),
            record.agent_session_id.as_str(),
            record.sessio_runtime_session_id.as_str(),
            record.workspace_path.as_str(),
            record.metadata_json.as_str(),
            record.last_update_id,
            record.created_at,
            record.updated_at,
            record.last_activity_at,
            record.ended_at,
        ],
    )?;
    Ok(())
}

pub(super) fn update_channel_session_activity(
    conn: &Connection,
    platform: &str,
    channel_id: &str,
    last_update_id: Option<i64>,
    last_activity_at: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE channel_sessions
         SET last_update_id = CASE
                WHEN ? IS NULL THEN last_update_id
                WHEN last_update_id IS NULL THEN ?
                WHEN ? > last_update_id THEN ?
                ELSE last_update_id
             END,
             last_activity_at = ?,
             updated_at = ?
         WHERE platform = ? AND channel_id = ? AND ended_at IS NULL",
        params![
            last_update_id,
            last_update_id,
            last_update_id,
            last_update_id,
            last_activity_at,
            last_activity_at,
            platform,
            channel_id,
        ],
    )?;
    Ok(())
}

pub(super) fn mark_channel_session_ended(
    conn: &Connection,
    platform: &str,
    channel_id: &str,
    agent: Agent,
    agent_session_id: &str,
    ended_at: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE channel_sessions
         SET ended_at = COALESCE(ended_at, ?), updated_at = ?
         WHERE platform = ? AND channel_id = ? AND agent = ? AND agent_session_id = ?",
        params![
            ended_at,
            ended_at,
            platform,
            channel_id,
            agent.as_str(),
            agent_session_id,
        ],
    )?;
    Ok(())
}
