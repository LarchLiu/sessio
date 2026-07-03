use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{
    CanvasBlockKind, CanvasBlockRecord, CanvasBlockSourceType, CanvasContextAnchor,
    CanvasDocumentInfo, CanvasDocumentState, CanvasRevisionInfo,
};
use crate::store::{now_ms, UpsertCanvasBlockRecord};

use super::unique_nonce;

fn stable_canvas_id(session_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(session_id.as_bytes());
    format!("canvas-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_canvas_revision_id(canvas_id: &str, revision: i64, now: i64, nonce: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(canvas_id.as_bytes());
    hasher.update(revision.to_string().as_bytes());
    hasher.update(now.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("canvas-revision-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_canvas_block_record_id(canvas_id: &str, block_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(canvas_id.as_bytes());
    hasher.update(block_id.as_bytes());
    format!("canvas-block-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_canvas_anchor_id(
    canvas_id: &str,
    selection_block_ids_json: &str,
    selection_element_ids_json: &str,
    turn_id: &str,
    now: i64,
    nonce: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(canvas_id.as_bytes());
    hasher.update(selection_block_ids_json.as_bytes());
    hasher.update(selection_element_ids_json.as_bytes());
    hasher.update(turn_id.as_bytes());
    hasher.update(now.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("canvas-anchor-{}", &hex::encode(hasher.finalize())[..16])
}

fn canvas_document_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CanvasDocumentInfo> {
    Ok(CanvasDocumentInfo {
        id: row.get(0)?,
        session_id: row.get(1)?,
        title: row.get(2)?,
        current_saved_revision: row.get(3)?,
        draft_snapshot_path: row.get(4)?,
        draft_snapshot_hash: row.get(5)?,
        draft_updated_at: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn canvas_revision_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CanvasRevisionInfo> {
    Ok(CanvasRevisionInfo {
        id: row.get(0)?,
        canvas_id: row.get(1)?,
        revision: row.get(2)?,
        snapshot_path: row.get(3)?,
        snapshot_hash: row.get(4)?,
        snapshot_size_bytes: row.get(5)?,
        source: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn canvas_block_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CanvasBlockRecord> {
    let kind_raw: String = row.get(3)?;
    let source_type_raw: String = row.get(4)?;
    Ok(CanvasBlockRecord {
        id: row.get(0)?,
        canvas_id: row.get(1)?,
        block_id: row.get(2)?,
        block_kind: CanvasBlockKind::from_db_str(&kind_raw).unwrap_or(CanvasBlockKind::Note),
        source_type: CanvasBlockSourceType::from_db_str(&source_type_raw)
            .unwrap_or(CanvasBlockSourceType::Note),
        source_key: row.get(5)?,
        source_path: row.get(6)?,
        metadata_json: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn canvas_anchor_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CanvasContextAnchor> {
    Ok(CanvasContextAnchor {
        id: row.get(0)?,
        canvas_id: row.get(1)?,
        anchor_block_id: row.get(2)?,
        selection_block_ids_json: row.get(3)?,
        selection_element_ids_json: row.get(4)?,
        turn_id: row.get(5)?,
        summary: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn default_canvas_title(session_id: &str, requested_title: Option<&str>) -> String {
    requested_title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("Canvas {session_id}"))
}

fn get_canvas_document_by_session(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<CanvasDocumentInfo>> {
    conn.query_row(
        "SELECT id, session_id, title, current_saved_revision, draft_snapshot_path,
                draft_snapshot_hash, draft_updated_at, created_at, updated_at
         FROM canvases
         WHERE session_id = ?",
        params![session_id],
        canvas_document_from_row,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn upsert_canvas_document_title(
    conn: &Connection,
    session_id: &str,
    title: Option<&str>,
) -> Result<CanvasDocumentInfo> {
    let now = now_ms();
    let canvas_id = stable_canvas_id(session_id);
    let title_value = default_canvas_title(session_id, title);
    conn.execute(
        "INSERT INTO canvases (
            id, session_id, title, current_saved_revision, draft_snapshot_path,
            draft_snapshot_hash, draft_updated_at, created_at, updated_at
         ) VALUES (?, ?, ?, NULL, NULL, NULL, NULL, ?, ?)
         ON CONFLICT(session_id) DO UPDATE SET
            title = CASE
                WHEN excluded.title <> '' THEN excluded.title
                ELSE canvases.title
            END,
            updated_at = excluded.updated_at",
        params![canvas_id, session_id, title_value, now, now],
    )?;
    get_canvas_document_by_session(conn, session_id)?
        .ok_or_else(|| anyhow::anyhow!("canvas document missing after upsert for {session_id}"))
}

fn latest_canvas_revision(
    conn: &Connection,
    canvas_id: &str,
) -> Result<Option<CanvasRevisionInfo>> {
    conn.query_row(
        "SELECT id, canvas_id, revision, snapshot_path, snapshot_hash, snapshot_size_bytes, source, created_at
         FROM canvas_revisions
         WHERE canvas_id = ?
         ORDER BY revision DESC, created_at DESC
         LIMIT 1",
        params![canvas_id],
        canvas_revision_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn stale_canvas_revision_paths(
    conn: &Connection,
    canvas_id: &str,
    keep_latest: usize,
) -> Result<Vec<String>> {
    let keep_latest = i64::try_from(keep_latest).unwrap_or(i64::MAX);
    let mut stmt = conn.prepare(
        "SELECT snapshot_path
         FROM canvas_revisions
         WHERE canvas_id = ?
         ORDER BY revision DESC, created_at DESC
         LIMIT -1 OFFSET ?",
    )?;
    let rows = stmt.query_map(params![canvas_id, keep_latest], |row| {
        row.get::<_, String>(0)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_canvas_block_records(conn: &Connection, canvas_id: &str) -> Result<Vec<CanvasBlockRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, canvas_id, block_id, block_kind, source_type, source_key, source_path,
                metadata_json, created_at, updated_at
         FROM canvas_blocks
         WHERE canvas_id = ?
         ORDER BY updated_at DESC, created_at DESC, block_id ASC",
    )?;
    let rows = stmt.query_map(params![canvas_id], canvas_block_record_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_canvas_anchors(conn: &Connection, canvas_id: &str) -> Result<Vec<CanvasContextAnchor>> {
    let mut stmt = conn.prepare(
        "SELECT id, canvas_id, anchor_block_id, selection_block_ids_json, selection_element_ids_json, turn_id, summary, created_at
         FROM canvas_context_anchors
         WHERE canvas_id = ?
         ORDER BY created_at DESC, id DESC",
    )?;
    let rows = stmt.query_map(params![canvas_id], canvas_anchor_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(super) fn load_canvas_document_state(
    conn: &Connection,
    session_id: &str,
) -> Result<CanvasDocumentState> {
    let document = upsert_canvas_document_title(conn, session_id, None)?;
    let saved_revision = latest_canvas_revision(conn, &document.id)?;
    let block_records = load_canvas_block_records(conn, &document.id)?;
    let anchors = load_canvas_anchors(conn, &document.id)?;
    Ok(CanvasDocumentState {
        document,
        draft_snapshot: None,
        saved_revision,
        saved_snapshot: None,
        block_records,
        anchors,
    })
}

pub(super) fn save_canvas_draft(
    conn: &Connection,
    session_id: &str,
    title: Option<&str>,
    draft_snapshot_path: &str,
    draft_snapshot_hash: &str,
) -> Result<CanvasDocumentInfo> {
    let now = now_ms();
    let document = upsert_canvas_document_title(conn, session_id, title)?;
    conn.execute(
        "UPDATE canvases
         SET draft_snapshot_path = ?, draft_snapshot_hash = ?, draft_updated_at = ?, updated_at = ?
         WHERE id = ?",
        params![
            draft_snapshot_path,
            draft_snapshot_hash,
            now,
            now,
            document.id,
        ],
    )?;
    get_canvas_document_by_session(conn, session_id)?
        .ok_or_else(|| anyhow::anyhow!("canvas document missing after draft save for {session_id}"))
}

pub(super) fn save_canvas_revision(
    conn: &mut Connection,
    session_id: &str,
    title: Option<&str>,
    snapshot_path: &str,
    snapshot_hash: &str,
    snapshot_size_bytes: i64,
    source: &str,
) -> Result<(CanvasDocumentInfo, CanvasRevisionInfo)> {
    let now = now_ms();
    let nonce = unique_nonce();
    let tx = conn.transaction()?;
    let document = upsert_canvas_document_title(&tx, session_id, title)?;
    let next_revision = tx.query_row(
        "SELECT COALESCE(MAX(revision), 0) + 1 FROM canvas_revisions WHERE canvas_id = ?",
        params![document.id.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    let revision_id = stable_canvas_revision_id(&document.id, next_revision, now, &nonce);
    tx.execute(
        "INSERT INTO canvas_revisions (
            id, canvas_id, revision, snapshot_path, snapshot_hash, snapshot_size_bytes, source, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            revision_id,
            document.id.as_str(),
            next_revision,
            snapshot_path,
            snapshot_hash,
            snapshot_size_bytes,
            source,
            now,
        ],
    )?;
    tx.execute(
        "UPDATE canvases
         SET current_saved_revision = ?, draft_snapshot_path = ?, draft_snapshot_hash = ?, draft_updated_at = ?, updated_at = ?
         WHERE id = ?",
        params![
            next_revision,
            snapshot_path,
            snapshot_hash,
            now,
            now,
            document.id.as_str(),
        ],
    )?;
    let updated_document = get_canvas_document_by_session(&tx, session_id)?.ok_or_else(|| {
        anyhow::anyhow!("canvas document missing after revision save for {session_id}")
    })?;
    let revision = tx.query_row(
        "SELECT id, canvas_id, revision, snapshot_path, snapshot_hash, snapshot_size_bytes, source, created_at
         FROM canvas_revisions
         WHERE id = ?",
        params![revision_id],
        canvas_revision_from_row,
    )?;
    tx.commit()?;
    Ok((updated_document, revision))
}

pub(super) fn prune_canvas_revisions(
    conn: &mut Connection,
    session_id: &str,
    keep_latest: usize,
) -> Result<Vec<String>> {
    let tx = conn.transaction()?;
    let document = upsert_canvas_document_title(&tx, session_id, None)?;
    let stale_paths = stale_canvas_revision_paths(&tx, &document.id, keep_latest)?;
    if !stale_paths.is_empty() {
        let keep_latest = i64::try_from(keep_latest).unwrap_or(i64::MAX);
        tx.execute(
            "DELETE FROM canvas_revisions
             WHERE id IN (
                SELECT id
                FROM canvas_revisions
                WHERE canvas_id = ?
                ORDER BY revision DESC, created_at DESC
                LIMIT -1 OFFSET ?
             )",
            params![document.id.as_str(), keep_latest],
        )?;
    }
    tx.commit()?;
    Ok(stale_paths)
}

pub(super) fn replace_canvas_blocks(
    conn: &mut Connection,
    session_id: &str,
    blocks: &[UpsertCanvasBlockRecord],
) -> Result<Vec<CanvasBlockRecord>> {
    let now = now_ms();
    let tx = conn.transaction()?;
    let document = upsert_canvas_document_title(&tx, session_id, None)?;
    tx.execute(
        "DELETE FROM canvas_blocks WHERE canvas_id = ?",
        params![document.id.as_str()],
    )?;
    for item in blocks {
        let id = stable_canvas_block_record_id(&document.id, &item.block_id);
        tx.execute(
            "INSERT INTO canvas_blocks (
                id, canvas_id, block_id, block_kind, source_type, source_key, source_path,
                metadata_json, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                document.id.as_str(),
                item.block_id.as_str(),
                item.block_kind.as_str(),
                item.source_type.as_str(),
                item.source_key.as_deref(),
                item.source_path.as_deref(),
                item.metadata_json.as_str(),
                now,
                now,
            ],
        )?;
    }
    let loaded = load_canvas_block_records(&tx, &document.id)?;
    tx.commit()?;
    Ok(loaded)
}

pub(super) fn create_canvas_context_anchor(
    conn: &Connection,
    session_id: &str,
    anchor_block_id: Option<&str>,
    selection_block_ids_json: &str,
    selection_element_ids_json: &str,
    turn_id: &str,
    summary: Option<&str>,
) -> Result<CanvasContextAnchor> {
    let now = now_ms();
    let nonce = unique_nonce();
    let document = upsert_canvas_document_title(conn, session_id, None)?;
    let id = stable_canvas_anchor_id(
        &document.id,
        selection_block_ids_json,
        selection_element_ids_json,
        turn_id,
        now,
        &nonce,
    );
    conn.execute(
        "INSERT INTO canvas_context_anchors (
            id, canvas_id, anchor_block_id, selection_block_ids_json, selection_element_ids_json, turn_id, summary, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            id,
            document.id.as_str(),
            anchor_block_id,
            selection_block_ids_json,
            selection_element_ids_json,
            turn_id,
            summary,
            now,
        ],
    )?;
    conn.query_row(
        "SELECT id, canvas_id, anchor_block_id, selection_block_ids_json, selection_element_ids_json, turn_id, summary, created_at
         FROM canvas_context_anchors
         WHERE id = ?",
        params![id],
        canvas_anchor_from_row,
    )
    .map_err(Into::into)
}
