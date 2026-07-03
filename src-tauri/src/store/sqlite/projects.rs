use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

use crate::models::ProjectInfo;
use crate::store::now_ms;

use super::{
    ensure_process_template_exists, instantiate_project_assistants,
    instantiate_project_builtin_stages, link_project_stage_assistants,
};

fn canonical_project_path(path: &str) -> Result<String> {
    let path = Path::new(path);
    let meta = std::fs::metadata(path)
        .with_context(|| format!("project directory does not exist: {}", path.display()))?;
    if !meta.is_dir() {
        anyhow::bail!("project path is not a directory: {}", path.display());
    }
    Ok(std::fs::canonicalize(path)
        .with_context(|| format!("canonicalize project path {}", path.display()))?
        .to_string_lossy()
        .to_string())
}

fn clean_project_name(name: Option<&str>, path: &str) -> Result<String> {
    let from_name = name.map(str::trim).filter(|s| !s.is_empty());
    let value = from_name
        .map(str::to_string)
        .or_else(|| {
            Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| path.to_string());
    if value.trim().is_empty() {
        anyhow::bail!("project name cannot be empty");
    }
    Ok(value)
}

fn clean_child_project_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        anyhow::bail!("project name cannot be empty");
    }
    if trimmed.contains(std::path::MAIN_SEPARATOR) || trimmed == "." || trimmed == ".." {
        anyhow::bail!("project name must be a single directory name");
    }
    if cfg!(windows) && (trimmed.contains('/') || trimmed.contains('\\')) {
        anyhow::bail!("project name must be a single directory name");
    }
    Ok(trimmed.to_string())
}

fn stable_project_id(path: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    format!("project-{}", &hex::encode(hasher.finalize())[..16])
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectInfo> {
    Ok(ProjectInfo {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        process_template_id: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        session_count: row.get::<_, i64>(6)? as usize,
    })
}

pub(super) fn load_project_by_id(conn: &Connection, project_id: &str) -> Result<ProjectInfo> {
    conn.query_row(
        "SELECT p.id, p.path, p.name, p.process_template_id, p.created_at, p.updated_at,
                COUNT(s.session_id) AS session_count
         FROM projects p
         LEFT JOIN sessions s ON s.project_path = p.path AND s.available = 1
                              AND s.is_auxiliary = 0 AND s.origin IN ('chat', 'channel')
         WHERE p.id = ? AND p.archived = 0
         GROUP BY p.id",
        params![project_id],
        project_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("project not found: {project_id}"))
}

pub(super) fn list_projects(conn: &Connection) -> Result<Vec<ProjectInfo>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.path, p.name, p.process_template_id, p.created_at, p.updated_at,
                COUNT(s.session_id) AS session_count
         FROM projects p
         LEFT JOIN sessions s ON s.project_path = p.path AND s.available = 1
                              AND s.is_auxiliary = 0 AND s.origin IN ('chat', 'channel')
         WHERE p.archived = 0
         GROUP BY p.id
         ORDER BY p.updated_at DESC, p.name COLLATE NOCASE ASC",
    )?;
    let rows = stmt.query_map([], project_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(super) fn add_project(
    conn: &mut Connection,
    path: &str,
    name: Option<&str>,
    process_template_id: String,
    enabled_stage_ids: Option<&[String]>,
) -> Result<ProjectInfo> {
    let canonical = canonical_project_path(path)?;
    let name = clean_project_name(name, &canonical)?;
    let id = stable_project_id(&canonical);
    let now = now_ms();
    let tx = conn.transaction()?;
    ensure_process_template_exists(&tx, &process_template_id)?;
    tx.execute(
        "INSERT INTO projects (id, path, name, process_template_id, created_at, updated_at, archived)
         VALUES (?, ?, ?, ?, ?, ?, 0)",
        params![id, canonical, name, process_template_id.as_str(), now, now],
    )
    .with_context(|| "add project")?;
    instantiate_project_builtin_stages(&tx, &id, &process_template_id, enabled_stage_ids, now)?;
    instantiate_project_assistants(&tx, &id, &process_template_id, now)?;
    link_project_stage_assistants(&tx, &id, &process_template_id, now)?;
    let project = load_project_by_id(&tx, &id)?;
    tx.commit()?;
    Ok(project)
}

pub(super) fn create_project(
    conn: &mut Connection,
    parent_path: &str,
    name: &str,
    process_template_id: String,
    enabled_stage_ids: Option<&[String]>,
) -> Result<ProjectInfo> {
    let parent = canonical_project_path(parent_path)?;
    let clean_name = clean_child_project_name(name)?;
    let project_path = Path::new(&parent).join(&clean_name);
    if project_path.exists() {
        anyhow::bail!(
            "project directory already exists: {}",
            project_path.display()
        );
    }
    std::fs::create_dir(&project_path)
        .with_context(|| format!("create project directory {}", project_path.display()))?;
    let path = canonical_project_path(&project_path.to_string_lossy())?;
    add_project(
        conn,
        &path,
        Some(&clean_name),
        process_template_id,
        enabled_stage_ids,
    )
}

pub(super) fn update_project(
    conn: &mut Connection,
    project_id: &str,
    name: Option<&str>,
    process_template_id: Option<String>,
) -> Result<ProjectInfo> {
    let tx = conn.transaction()?;
    let current = load_project_by_id(&tx, project_id)?;
    let next_name = match name {
        Some(value) => clean_project_name(Some(value), &current.path)?,
        None => current.name,
    };
    let current_process_template_id = current.process_template_id.clone();
    let next_process_template_id =
        process_template_id.unwrap_or_else(|| current_process_template_id.clone());
    ensure_process_template_exists(&tx, &next_process_template_id)?;
    let process_template_changed = next_process_template_id != current_process_template_id;
    tx.execute(
        "UPDATE projects
         SET name = ?, process_template_id = ?, updated_at = ?
         WHERE id = ? AND archived = 0",
        params![
            next_name,
            next_process_template_id.as_str(),
            now_ms(),
            project_id
        ],
    )?;
    if process_template_changed {
        tx.execute(
            "DELETE FROM stages WHERE project_id = ? AND type = 'builtin'",
            params![project_id],
        )?;
        instantiate_project_builtin_stages(
            &tx,
            project_id,
            &next_process_template_id,
            None,
            now_ms(),
        )?;
        instantiate_project_assistants(&tx, project_id, &next_process_template_id, now_ms())?;
        link_project_stage_assistants(&tx, project_id, &next_process_template_id, now_ms())?;
    }
    let project = load_project_by_id(&tx, project_id)?;
    tx.commit()?;
    Ok(project)
}

pub(super) fn archive_project(conn: &Connection, project_id: &str) -> Result<()> {
    let changed = conn.execute(
        "UPDATE projects SET archived = 1, updated_at = ? WHERE id = ? AND archived = 0",
        params![now_ms(), project_id],
    )?;
    if changed == 0 {
        anyhow::bail!("project not found: {project_id}");
    }
    Ok(())
}
