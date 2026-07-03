use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{Agent, SessionInfo, SessionOrigin};
use crate::store::{file_mtime_for, is_real_session_file_path, now_ms};

#[derive(Debug, Clone)]
pub(super) struct ExistingSessionRow {
    pub(super) scope: String,
    pub(super) file_path: String,
    pub(super) partial: i64,
    pub(super) available: i64,
    pub(super) archived: i64,
    pub(super) message_count: i64,
    pub(super) rename_title: Option<String>,
    pub(super) title: Option<String>,
    pub(super) first_user_message: Option<String>,
    pub(super) forked_from_agent: Option<Agent>,
    pub(super) forked_from_id: Option<String>,
    /// `origin` is provenance plus sidebar routing. Link/unlink paths may
    /// upgrade/downgrade `chat <-> thread`, and merge logic carries the
    /// existing non-chat value forward so a later parser pass can't downgrade
    /// `thread`/`channel` back to `chat`.
    pub(super) origin: SessionOrigin,
    /// Sticky for the same reason: auto task placeholder rows write this and
    /// we want it preserved when the indexer later replaces the row.
    pub(super) scheduled_task_id: Option<String>,
    /// Sticky-OR: once any row in the identity set is auxiliary, the merged
    /// write keeps it set. Auxiliary rows never appear in the sidebar.
    pub(super) is_auxiliary: i64,
}

pub(super) struct MergedSessionProvenance {
    pub(super) origin: SessionOrigin,
    pub(super) scheduled_task_id: Option<String>,
    pub(super) is_auxiliary: i64,
}

pub(super) fn load_identity_session_rows(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
) -> Result<Vec<ExistingSessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT scope, file_path, partial, available, archived,
                message_count, rename_title, title, first_user_message, forked_from_agent, forked_from_id,
                origin, scheduled_task_id, is_auxiliary
         FROM sessions
         WHERE agent = ? AND session_id = ?
         ORDER BY
           CASE WHEN file_path != '' AND file_path NOT LIKE 'astra://%' THEN 0 ELSE 1 END,
           partial ASC,
           updated_at DESC,
           last_indexed_at DESC",
    )?;
    let rows = stmt
        .query_map(params![agent.as_str(), session_id], |row| {
            let forked_agent = row
                .get::<_, Option<String>>(9)?
                .and_then(|value| Agent::from_db_str(&value));
            let origin_raw: String = row.get(11)?;
            Ok(ExistingSessionRow {
                scope: row.get(0)?,
                file_path: row.get(1)?,
                partial: row.get(2)?,
                available: row.get(3)?,
                archived: row.get(4)?,
                message_count: row.get(5)?,
                rename_title: row.get(6)?,
                title: row.get(7)?,
                first_user_message: row.get(8)?,
                forked_from_agent: forked_agent,
                forked_from_id: row.get(10)?,
                origin: SessionOrigin::from_db_str(&origin_raw).unwrap_or_default(),
                scheduled_task_id: row.get(12)?,
                is_auxiliary: row.get(13)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(super) fn choose_identity_title(
    rows: &[ExistingSessionRow],
    incoming: &SessionInfo,
    prefer_incoming_parsed: bool,
) -> Option<String> {
    let existing = rows.iter().find_map(|row| {
        row.title
            .as_ref()
            .map(|title| title.trim())
            .filter(|title| !title.is_empty())
            .map(ToString::to_string)
    });
    if prefer_incoming_parsed {
        incoming.title.clone().or(existing)
    } else {
        existing.or_else(|| incoming.title.clone())
    }
}

pub(super) fn choose_identity_rename_title(
    rows: &[ExistingSessionRow],
    incoming: &SessionInfo,
) -> Option<String> {
    rows.iter()
        .find_map(|row| row.rename_title.clone())
        .or_else(|| incoming.rename_title.clone())
}

pub(super) fn choose_identity_first_user(
    rows: &[ExistingSessionRow],
    incoming: &SessionInfo,
    prefer_incoming: bool,
) -> Option<String> {
    let existing = rows.iter().find_map(|row| row.first_user_message.clone());
    if prefer_incoming {
        incoming.first_user_message.clone().or(existing)
    } else {
        existing.or_else(|| incoming.first_user_message.clone())
    }
}

pub(super) fn merge_identity_lineage(
    rows: &[ExistingSessionRow],
    incoming: &SessionInfo,
) -> (Option<Agent>, Option<String>) {
    let mut forked_from_agent = None;
    let mut forked_from_id = None;
    for row in rows {
        let merged = merge_session_lineage(
            forked_from_agent,
            forked_from_id,
            row.forked_from_agent,
            row.forked_from_id.clone(),
        );
        forked_from_agent = merged.0;
        forked_from_id = merged.1;
    }
    merge_session_lineage(
        forked_from_agent,
        forked_from_id,
        incoming.forked_from_agent,
        incoming.forked_from_id.clone(),
    )
}

pub(super) fn merged_message_count(rows: &[ExistingSessionRow], incoming: &SessionInfo) -> i64 {
    rows.iter()
        .map(|row| row.message_count)
        .max()
        .unwrap_or_default()
        .max(incoming.message_count as i64)
}

/// Upgrade a session's origin from the default `chat` to `thread` for every
/// row sharing the `(agent, session_id)` identity. Channel-origin rows are
/// left intact (a channel-originated message that lands in a thread keeps its
/// `channel` provenance). Used by `link_thread_session`,
/// `link_stage_session`, and `link_plan_task_session` so the sidebar filter
/// hides any session attached to a thread workflow.
pub(super) fn upgrade_session_origin_to_thread(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE sessions
            SET origin = 'thread'
          WHERE agent = ? AND session_id = ? AND origin = 'chat'",
        params![agent.as_str(), session_id],
    )?;
    Ok(())
}

pub(super) fn mark_session_scheduled_task(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
    scheduled_task_id: &str,
    is_auxiliary: bool,
) -> Result<()> {
    // OR-ing into is_auxiliary keeps the sticky semantics: marking a session
    // auxiliary later in its lifetime is allowed, but a chat-mode task session
    // that was created with is_auxiliary=false must not flip to auxiliary just
    // because a later mark call lands.
    let aux_value = if is_auxiliary { 1 } else { 0 };
    conn.execute(
        "UPDATE sessions
            SET scheduled_task_id = ?,
                is_auxiliary = MAX(is_auxiliary, ?)
          WHERE agent = ? AND session_id = ?",
        params![scheduled_task_id, aux_value, agent.as_str(), session_id,],
    )?;
    Ok(())
}

pub(super) fn mark_session_origin(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
    origin: SessionOrigin,
) -> Result<()> {
    // Sticky origin: only upgrade rows whose stored origin is still the
    // default `chat`. A `thread` or `channel` row stays put. Marking with
    // `Chat` is a no-op.
    if origin == SessionOrigin::Chat {
        return Ok(());
    }
    conn.execute(
        "UPDATE sessions
            SET origin = ?
          WHERE agent = ? AND session_id = ? AND origin = 'chat'",
        params![origin.as_str(), agent.as_str(), session_id],
    )?;
    Ok(())
}

/// Symmetric counterpart to `upgrade_session_origin_to_thread`. Called from
/// every `unlink_*` / supersede path: if the `(agent, session_id)` identity has no
/// remaining thread / stage / plan-task / astra-run reference, downgrade
/// `origin = 'thread'` rows back to `'chat'` so the session reappears in the
/// sidebar. Channel-origin rows are not touched; auxiliary rows (Astra
/// delegated etc.) stay hidden via `is_auxiliary` independently of origin.
///
/// The pre-link reverse-join model recomputed visibility on every render,
/// so unlinking automatically restored sidebar presence. The new sticky
/// model needs this explicit downgrade to preserve that behaviour.
pub(super) fn downgrade_session_origin_when_unlinked(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
) -> Result<()> {
    let still_linked: i64 = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM thread_sessions
            WHERE agent = ?1 AND session_id = ?2
            UNION ALL
            SELECT 1 FROM stage_sessions
            WHERE agent = ?1 AND session_id = ?2
            UNION ALL
            SELECT 1 FROM thread_plan_task_sessions
            WHERE agent = ?1 AND session_id = ?2 AND superseded_at IS NULL
            UNION ALL
            SELECT 1 FROM astra_run_sessions
            WHERE agent = ?1 AND session_id = ?2
         )",
        params![agent.as_str(), session_id],
        |row| row.get(0),
    )?;
    if still_linked == 0 {
        conn.execute(
            "UPDATE sessions
                SET origin = 'chat'
              WHERE agent = ? AND session_id = ? AND origin = 'thread'",
            params![agent.as_str(), session_id],
        )?;
    }
    Ok(())
}

/// Preserve any already-recorded non-chat provenance. A `thread`/`channel`
/// incoming row may only upgrade an identity whose stored rows are still the
/// default `chat`, matching `mark_session_origin` / `upgrade_*` semantics.
fn merged_origin(rows: &[ExistingSessionRow], incoming: SessionOrigin) -> SessionOrigin {
    rows.iter()
        .map(|row| row.origin)
        .find(|origin| *origin != SessionOrigin::Chat)
        .unwrap_or(incoming)
}

/// Sticky scheduled_task_id merge: prefer the incoming value when set, else
/// preserve any existing value. Once a session is attached to a scheduled
/// task that link stays for its lifetime.
fn merged_scheduled_task_id(rows: &[ExistingSessionRow], incoming: Option<&str>) -> Option<String> {
    if let Some(value) = incoming {
        return Some(value.to_string());
    }
    rows.iter().find_map(|row| row.scheduled_task_id.clone())
}

/// Sticky-OR for auxiliary: incoming OR any existing row sets it. Once any
/// row in the identity set is auxiliary, the merged write keeps it set.
fn merged_is_auxiliary(rows: &[ExistingSessionRow], incoming: bool) -> i64 {
    (incoming || rows.iter().any(|row| row.is_auxiliary != 0)) as i64
}

pub(super) fn merge_session_provenance(
    rows: &[ExistingSessionRow],
    incoming: &SessionInfo,
) -> MergedSessionProvenance {
    MergedSessionProvenance {
        origin: merged_origin(rows, incoming.origin),
        scheduled_task_id: merged_scheduled_task_id(rows, incoming.scheduled_task_id.as_deref()),
        is_auxiliary: merged_is_auxiliary(rows, incoming.is_auxiliary),
    }
}

pub(super) fn delete_duplicate_session_rows(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
    keep_scope: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM sessions
         WHERE agent = ? AND session_id = ? AND scope != ?",
        params![agent.as_str(), session_id, keep_scope],
    )?;
    Ok(())
}

fn merge_session_lineage(
    existing_agent: Option<Agent>,
    existing_id: Option<String>,
    parsed_agent: Option<Agent>,
    parsed_id: Option<String>,
) -> (Option<Agent>, Option<String>) {
    match (existing_agent, existing_id) {
        (Some(agent), Some(id)) => (Some(agent), Some(id)),
        (None, Some(id)) => {
            let agent = if parsed_id.as_deref() == Some(id.as_str()) {
                parsed_agent
            } else {
                None
            };
            (agent, Some(id))
        }
        (Some(agent), None) => (Some(agent), parsed_id),
        (None, None) => (parsed_agent, parsed_id),
    }
}

struct ExistingPlaceholder {
    session_id: String,
    scope: String,
}

pub(super) fn insert_session(conn: &Connection, scope: &str, s: &SessionInfo) -> Result<()> {
    let identity_rows = load_identity_session_rows(conn, s.agent, &s.id)?;
    let incoming_real = is_real_session_file_path(&s.file_path);
    let existing_real = identity_rows
        .iter()
        .find(|row| is_real_session_file_path(&row.file_path))
        .cloned();
    let existing_same_scope = identity_rows.iter().find(|row| row.scope == scope).cloned();

    if !incoming_real {
        if let Some(existing) = existing_real.clone() {
            let message_count = merged_message_count(&identity_rows, s);
            let partial = existing.partial;
            let available = (existing.available != 0 || s.available) as i64;
            let archived = existing.archived;
            let rename_title = choose_identity_rename_title(&identity_rows, s);
            let title = choose_identity_title(&identity_rows, s, false);
            let first_user_message = choose_identity_first_user(&identity_rows, s, false);
            let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
            let provenance = merge_session_provenance(&identity_rows, s);
            conn.execute(
                "UPDATE sessions
                 SET project_path = COALESCE(project_path, ?),
                     project_name = COALESCE(project_name, ?),
                     started_at = COALESCE(started_at, ?),
                     updated_at = COALESCE(updated_at, ?),
                     rename_title = ?,
                     title = ?,
                     first_user_message = ?,
                     message_count = ?,
                     partial = ?,
                     available = ?,
                     archived = ?,
                     forked_from_agent = ?,
                     forked_from_id = ?,
                     origin = ?,
                     scheduled_task_id = ?,
                     is_auxiliary = ?
                 WHERE agent = ? AND session_id = ? AND scope = ?",
                params![
                    s.project_path,
                    s.project_name,
                    s.started_at,
                    s.updated_at,
                    rename_title,
                    title,
                    first_user_message,
                    message_count,
                    partial,
                    available,
                    archived,
                    forked_from_agent.map(|agent| agent.as_str()),
                    forked_from_id,
                    provenance.origin.as_str(),
                    provenance.scheduled_task_id,
                    provenance.is_auxiliary,
                    s.agent.as_str(),
                    s.id,
                    existing.scope,
                ],
            )?;
            delete_duplicate_session_rows(conn, s.agent, &s.id, &existing.scope)?;
            return Ok(());
        }
    }

    if incoming_real {
        if let Some(existing) = existing_same_scope.clone() {
            let rename_title = choose_identity_rename_title(&identity_rows, s);
            let title = choose_identity_title(&identity_rows, s, true);
            let first_user_message = choose_identity_first_user(&identity_rows, s, true);
            let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
            let provenance = merge_session_provenance(&identity_rows, s);
            conn.execute(
                "UPDATE sessions
                 SET file_path = ?, project_path = ?, project_name = ?,
                     started_at = ?, updated_at = ?, rename_title = ?, title = ?, first_user_message = ?,
                     message_count = ?, file_size = ?, file_mtime = ?, partial = ?, available = ?, archived = ?,
                     last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?,
                     origin = ?, scheduled_task_id = ?, is_auxiliary = ?
                 WHERE agent = ? AND session_id = ? AND scope = ?",
                params![
                    s.file_path,
                    s.project_path,
                    s.project_name,
                    s.started_at,
                    s.updated_at,
                    rename_title,
                    title,
                    first_user_message,
                    merged_message_count(&identity_rows, s),
                    s.file_size as i64,
                    file_mtime_for(&s.file_path),
                    0,
                    s.available as i64,
                    s.archived as i64,
                    now_ms(),
                    forked_from_agent.map(|agent| agent.as_str()),
                    forked_from_id,
                    provenance.origin.as_str(),
                    provenance.scheduled_task_id,
                    provenance.is_auxiliary,
                    s.agent.as_str(),
                    s.id,
                    existing.scope,
                ],
            )?;
            delete_duplicate_session_rows(conn, s.agent, &s.id, scope)?;
            return Ok(());
        }
        if let Some(existing) = existing_real.clone().filter(|row| row.scope != scope) {
            let rename_title = choose_identity_rename_title(&identity_rows, s);
            let title = choose_identity_title(&identity_rows, s, true);
            let first_user_message = choose_identity_first_user(&identity_rows, s, true);
            let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
            let provenance = merge_session_provenance(&identity_rows, s);
            conn.execute(
                "UPDATE sessions
                 SET scope = ?, file_path = ?, project_path = ?, project_name = ?,
                     started_at = ?, updated_at = ?, rename_title = ?, title = ?, first_user_message = ?,
                     message_count = ?, file_size = ?, file_mtime = ?, partial = ?, available = ?, archived = ?,
                     last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?,
                     origin = ?, scheduled_task_id = ?, is_auxiliary = ?
                 WHERE agent = ? AND session_id = ? AND scope = ?",
                params![
                    scope,
                    s.file_path,
                    s.project_path,
                    s.project_name,
                    s.started_at,
                    s.updated_at,
                    rename_title,
                    title,
                    first_user_message,
                    merged_message_count(&identity_rows, s),
                    s.file_size as i64,
                    file_mtime_for(&s.file_path),
                    0,
                    s.available as i64,
                    s.archived as i64,
                    now_ms(),
                    forked_from_agent.map(|agent| agent.as_str()),
                    forked_from_id,
                    provenance.origin.as_str(),
                    provenance.scheduled_task_id,
                    provenance.is_auxiliary,
                    s.agent.as_str(),
                    s.id,
                    existing.scope,
                ],
            )?;
            delete_duplicate_session_rows(conn, s.agent, &s.id, scope)?;
            return Ok(());
        }
        if let Some(existing) = existing_placeholder(conn, s.agent, &s.id, scope, s)? {
            let rename_title = choose_identity_rename_title(&identity_rows, s);
            let title = choose_identity_title(&identity_rows, s, true);
            let first_user_message = choose_identity_first_user(&identity_rows, s, true);
            let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
            let provenance = merge_session_provenance(&identity_rows, s);
            if conn.query_row(
                "SELECT 1 FROM sessions WHERE agent = ? AND session_id = ? AND scope = ? LIMIT 1",
                params![s.agent.as_str(), s.id, scope],
                |_| Ok(()),
            ).optional()?.is_some() {
                conn.execute(
                    "UPDATE sessions
                     SET origin = ?,
                         scheduled_task_id = ?,
                         is_auxiliary = ?
                     WHERE agent = ? AND session_id = ? AND scope = ?",
                    params![
                        provenance.origin.as_str(),
                        provenance.scheduled_task_id,
                        provenance.is_auxiliary,
                        s.agent.as_str(),
                        s.id,
                        scope,
                    ],
                )?;
                conn.execute(
                    "DELETE FROM sessions
                     WHERE agent = ? AND session_id = ? AND scope = ?",
                    params![s.agent.as_str(), s.id, existing.scope],
                )?;
            } else {
                conn.execute(
                    "UPDATE sessions
                     SET session_id = ?, scope = ?, file_path = ?, project_path = ?, project_name = ?,
                         started_at = ?, updated_at = ?, rename_title = ?, title = ?, first_user_message = ?,
                         message_count = ?, file_size = ?, file_mtime = ?, partial = ?, available = ?, archived = ?,
                         last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?,
                         origin = ?, scheduled_task_id = ?, is_auxiliary = ?
                     WHERE agent = ? AND session_id = ? AND scope = ?",
                    params![
                        s.id,
                        scope,
                        s.file_path,
                        s.project_path,
                        s.project_name,
                        s.started_at,
                        s.updated_at,
                        rename_title,
                        title,
                        first_user_message,
                        merged_message_count(&identity_rows, s),
                        s.file_size as i64,
                        file_mtime_for(&s.file_path),
                        0,
                        s.available as i64,
                        s.archived as i64,
                        now_ms(),
                        forked_from_agent.map(|agent| agent.as_str()),
                        forked_from_id,
                        provenance.origin.as_str(),
                        provenance.scheduled_task_id,
                        provenance.is_auxiliary,
                        s.agent.as_str(),
                        existing.session_id,
                        existing.scope,
                    ],
                )?;
                delete_duplicate_session_rows(
                    conn,
                    s.agent,
                    &s.id,
                    scope,
                )?;
                return Ok(());
            }
        }
    }

    let identity_rows = load_identity_session_rows(conn, s.agent, &s.id)?;
    let prefer_incoming_parsed = incoming_real;
    let rename_title = choose_identity_rename_title(&identity_rows, s);
    let title = choose_identity_title(&identity_rows, s, prefer_incoming_parsed);
    let first_user_message = choose_identity_first_user(&identity_rows, s, prefer_incoming_parsed);
    if let Some(existing_same_scope) = identity_rows.iter().find(|row| row.scope == scope) {
        let message_count = merged_message_count(&identity_rows, s);
        let partial = if s.partial {
            existing_same_scope.partial
        } else {
            0
        };
        let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
        let provenance = merge_session_provenance(&identity_rows, s);
        conn.execute(
            "UPDATE sessions
             SET session_id = ?, scope = ?, file_path = ?, project_path = ?, project_name = ?,
                 started_at = ?, updated_at = ?, rename_title = ?, title = ?, first_user_message = ?,
                 message_count = ?, file_size = ?, file_mtime = ?, partial = ?, available = ?, archived = ?,
                 last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?,
                 origin = ?, scheduled_task_id = ?, is_auxiliary = ?
             WHERE agent = ? AND session_id = ? AND scope = ?",
            params![
                s.id,
                scope,
                s.file_path,
                s.project_path,
                s.project_name,
                s.started_at,
                s.updated_at,
                rename_title,
                title,
                first_user_message,
                message_count,
                s.file_size as i64,
                file_mtime_for(&s.file_path),
                partial,
                s.available as i64,
                s.archived as i64,
                now_ms(),
                forked_from_agent.map(|agent| agent.as_str()),
                forked_from_id,
                provenance.origin.as_str(),
                provenance.scheduled_task_id,
                provenance.is_auxiliary,
                s.agent.as_str(),
                s.id,
                existing_same_scope.scope,
            ],
        )?;
        delete_duplicate_session_rows(conn, s.agent, &s.id, scope)?;
        return Ok(());
    }
    let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
    let provenance = merge_session_provenance(&identity_rows, s);
    conn.execute(
        "INSERT OR REPLACE INTO sessions (
            agent, session_id, scope, file_path,
            project_path, project_name,
            started_at, updated_at,
            message_count, rename_title, title, first_user_message,
            file_size, file_mtime,
            partial, available, archived,
            last_indexed_at, forked_from_agent, forked_from_id,
            origin, scheduled_task_id, is_auxiliary
        ) VALUES (?,?,?,?, ?,?, ?,?, ?,?,?,?, ?,?, ?,?,?, ?,?,?, ?,?,?)",
        params![
            s.agent.as_str(),
            s.id,
            scope,
            s.file_path,
            s.project_path,
            s.project_name,
            s.started_at,
            s.updated_at,
            merged_message_count(&identity_rows, s),
            rename_title,
            title,
            first_user_message,
            s.file_size as i64,
            file_mtime_for(&s.file_path),
            s.partial as i64,
            s.available as i64,
            s.archived as i64,
            now_ms(),
            forked_from_agent.map(|agent| agent.as_str()),
            forked_from_id,
            provenance.origin.as_str(),
            provenance.scheduled_task_id,
            provenance.is_auxiliary,
        ],
    )?;
    delete_duplicate_session_rows(conn, s.agent, &s.id, scope)?;
    // Subagent rows are written through upsert_subagent so their lifecycle
    // is independent from the parent session's reindex.
    Ok(())
}

fn existing_placeholder(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
    next_scope: &str,
    _next: &SessionInfo,
) -> Result<Option<ExistingPlaceholder>> {
    if let Some(scope) = existing_placeholder_scope(conn, agent, session_id, next_scope)? {
        return Ok(Some(ExistingPlaceholder {
            session_id: session_id.to_string(),
            scope,
        }));
    }
    Ok(None)
}

fn existing_placeholder_scope(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
    next_scope: &str,
) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT scope FROM sessions
         WHERE agent = ? AND session_id = ? AND scope != ?
           AND file_size = 0 AND partial = 1
           AND (file_path = '' OR file_path LIKE 'astra://%')
         ORDER BY last_indexed_at DESC
         LIMIT 1",
    )?;
    let scope = stmt
        .query_row(params![agent.as_str(), session_id, next_scope], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(scope)
}
