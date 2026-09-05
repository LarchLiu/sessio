use anyhow::Result;
use rusqlite::{params, Connection};

use crate::models::Agent;
use crate::store::{now_ms, SessioAppRecord};

pub(super) fn sync_apps(
    conn: &mut Connection,
    root_path: &str,
    apps: &[SessioAppRecord],
) -> Result<()> {
    let now = now_ms();
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE apps SET available = 0, updated_at = ? WHERE root_path = ?",
        params![now, root_path],
    )?;
    for app in apps {
        tx.execute(
            "INSERT INTO apps (id, root_path, path, slug, html_path, available, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 1, ?, ?)
             ON CONFLICT(path) DO UPDATE SET
                id = excluded.id,
                root_path = excluded.root_path,
                slug = excluded.slug,
                html_path = excluded.html_path,
                available = 1,
                updated_at = excluded.updated_at",
            params![
                app.id,
                app.root_path,
                app.directory_path,
                app.slug,
                app.html_path,
                now,
                now,
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn link_app_session(
    conn: &Connection,
    app_id: &str,
    agent: Agent,
    session_id: &str,
) -> Result<()> {
    let now = now_ms();
    let changed = conn.execute(
        "INSERT INTO app_sessions (app_id, agent, session_id, created_at, updated_at)
         SELECT id, ?, ?, ?, ? FROM apps WHERE id = ? AND available = 1
         ON CONFLICT(app_id, agent, session_id) DO UPDATE SET updated_at = excluded.updated_at",
        params![agent.as_str(), session_id, now, now, app_id],
    )?;
    if changed == 0 {
        anyhow::bail!("app not found: {app_id}");
    }
    Ok(())
}

pub(super) fn list_app_session_refs(
    conn: &Connection,
    app_id: &str,
) -> Result<Vec<(Agent, String)>> {
    let mut stmt = conn.prepare(
        "SELECT agent, session_id
         FROM app_sessions
         WHERE app_id = ?
         ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map(params![app_id], |row| {
        let agent: String = row.get(0)?;
        let session_id: String = row.get(1)?;
        Ok((agent, session_id))
    })?;
    let mut refs = Vec::new();
    for row in rows {
        let (agent, session_id) = row?;
        if let Some(agent) = Agent::from_db_str(&agent) {
            refs.push((agent, session_id));
        }
    }
    Ok(refs)
}

#[cfg(test)]
pub(super) fn app_session_count(
    conn: &Connection,
    app_id: &str,
    agent: Agent,
    session_id: &str,
) -> Result<usize> {
    let count = conn.query_row(
        "SELECT COUNT(*) FROM app_sessions WHERE app_id = ? AND agent = ? AND session_id = ?",
        params![app_id, agent.as_str(), session_id],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(count as usize)
}
