use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;

use crate::models::{
    Agent, AssistantAgentInfo, AssistantInfo, AssistantType, IssueSeverity, IssueStatus,
    ProjectStageInfo, ProjectStageType, StageAssistantInfo, StageInfo, StageIssueInfo, StageStatus,
    StageType, ThreadInfo,
};
use crate::store::{now_ms, ProjectStagePatch};

use super::assistants::load_assistant_by_id;
use super::identity::{downgrade_session_origin_when_unlinked, upgrade_session_origin_to_thread};
use super::projects::load_project_by_id;
use super::thread_queries::{
    load_stage_sessions, load_thread_by_id, load_thread_stages, thread_stage_from_row,
};
use super::{
    ensure_process_template_exists, ensure_session_not_linked_to_thread_process, load_agent_by_id,
    parse_string_array_json, session_project_path, unique_nonce, usage_list,
    validate_assistant_for_project, validate_assistants_for_project,
};

fn stable_project_stage_id(
    process_template_id: Option<&str>,
    project_id: Option<&str>,
    stage_name: &str,
    order: i64,
    now: i64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(process_template_id.unwrap_or("").as_bytes());
    hasher.update(project_id.unwrap_or("").as_bytes());
    hasher.update(stage_name.as_bytes());
    hasher.update(order.to_string().as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!("stage-{}", &hex::encode(hasher.finalize())[..16])
}

pub(super) fn project_stage_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProjectStageInfo> {
    let stage_type_raw: String = row.get(2)?;
    let process_template_id_raw: Option<String> = row.get(3)?;
    let stage_kind_raw: Option<String> = row.get(4)?;
    Ok(ProjectStageInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        stage_type: ProjectStageType::from_db_str(&stage_type_raw)
            .unwrap_or(ProjectStageType::Custom),
        process_template_id: process_template_id_raw,
        kind: stage_kind_raw.and_then(|value| StageType::from_db_str(&value)),
        name: row.get(5)?,
        description: row.get(6)?,
        icon: row.get(7)?,
        order: row.get(8)?,
        enabled: row.get::<_, i64>(9)? != 0,
        allow_empty_assistants: row.get::<_, i64>(10)? != 0,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        assistants: Vec::new(),
    })
}

pub(super) fn load_project_stage_by_id(
    conn: &Connection,
    stage_id: &str,
) -> Result<ProjectStageInfo> {
    let mut stage = conn.query_row(
        "SELECT id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
         FROM stages
         WHERE id = ?",
        params![stage_id],
        project_stage_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("project stage not found: {stage_id}"))?;
    stage.assistants = load_project_stage_assistants(conn, &stage.id)?;
    Ok(stage)
}

fn validate_assistant_for_stage(
    conn: &Connection,
    stage: &ProjectStageInfo,
    assistant_id: &str,
) -> Result<AssistantInfo> {
    let assistant = load_assistant_by_id(conn, assistant_id)?;
    if !assistant.enabled {
        anyhow::bail!("assistant is disabled");
    }
    if stage.project_id.is_some() && assistant.project_id == stage.project_id {
        return Ok(assistant);
    }
    if stage.project_id.is_none()
        && stage.process_template_id.is_some()
        && assistant.project_id.is_none()
        && assistant.process_template_id == stage.process_template_id
    {
        return Ok(assistant);
    }
    if stage.project_id.is_none()
        && stage.process_template_id.is_some()
        && assistant.project_id.is_none()
        && assistant.process_template_id.is_none()
        && assistant.assistant_type == AssistantType::Custom
    {
        return Ok(assistant);
    }
    anyhow::bail!("assistant is not available for this stage")
}

fn validate_assistants_for_stage(
    conn: &Connection,
    stage: &ProjectStageInfo,
    assistant_ids: &[String],
) -> Result<Vec<AssistantInfo>> {
    let mut seen = HashSet::new();
    let mut assistants = Vec::new();
    for assistant_id in assistant_ids {
        let assistant_id = assistant_id.trim();
        if assistant_id.is_empty() || !seen.insert(assistant_id.to_string()) {
            continue;
        }
        assistants.push(validate_assistant_for_stage(conn, stage, assistant_id)?);
    }
    Ok(assistants)
}

pub(super) fn ensure_project_stage_can_be_disabled(
    conn: &Connection,
    stage_id: &str,
) -> Result<()> {
    let stage = load_project_stage_by_id(conn, stage_id)?;
    let project_id = stage.project_id.as_deref();
    let thread_stage_usages = {
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(p.name, 'Unknown'),
                t.goal,
                COALESCE(s.name, s.kind, s.id)
             FROM thread_stages ts
             INNER JOIN threads t ON t.id = ts.thread_id
             INNER JOIN stages s ON s.id = ts.stage_id
             LEFT JOIN projects p ON p.id = t.project_id
             WHERE ts.stage_id = ?
               AND ((? IS NULL AND t.project_id IS NULL) OR t.project_id = ?)
             ORDER BY p.name COLLATE NOCASE ASC, t.updated_at DESC, ts.sort_order ASC",
        )?;
        let rows = stmt.query_map(params![stage_id, project_id, project_id], |row| {
            let project_name: String = row.get(0)?;
            let thread_goal: String = row.get(1)?;
            let stage_name: String = row.get(2)?;
            Ok(format!(
                "project \"{project_name}\" thread \"{thread_goal}\" stage \"{stage_name}\""
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    if !thread_stage_usages.is_empty() {
        anyhow::bail!(
            "stage is in use by {} thread stage(s); remove these stages from threads before disabling: {}",
            thread_stage_usages.len(),
            usage_list(&thread_stage_usages)
        );
    }
    Ok(())
}

pub(super) fn load_project_stage_assistants(
    conn: &Connection,
    stage_id: &str,
) -> Result<Vec<StageAssistantInfo>> {
    let mut stmt = conn.prepare(
        "SELECT sa.assistant_id, a.name, a.color, a.agent_json, a.system_prompt, a.selected_skill_ids_json, a.selected_mcp_ids_json, sa.sort_order
         FROM stage_assistants sa
         INNER JOIN assistants a ON a.id = sa.assistant_id
         WHERE sa.stage_id = ?
         ORDER BY sa.sort_order ASC, sa.created_at ASC",
    )?;
    let rows = stmt.query_map(params![stage_id], |row| {
        let agent_json: String = row.get(3)?;
        Ok(StageAssistantInfo {
            assistant_id: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
            agent: serde_json::from_str::<AssistantAgentInfo>(&agent_json).unwrap_or_else(|_| {
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: String::new(),
                    mode: String::new(),
                    effort: String::new(),
                }
            }),
            system_prompt: row.get(4)?,
            selected_skill_ids: parse_string_array_json(&row.get::<_, String>(5)?),
            selected_mcp_ids: parse_string_array_json(&row.get::<_, String>(6)?),
            order: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn replace_project_stage_assistants(
    conn: &Connection,
    stage_id: &str,
    assistants: &[AssistantInfo],
    now: i64,
) -> Result<()> {
    conn.execute(
        "DELETE FROM stage_assistants WHERE stage_id = ?",
        params![stage_id],
    )?;
    for (index, assistant) in assistants.iter().enumerate() {
        conn.execute(
            "INSERT INTO stage_assistants (stage_id, assistant_id, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
            params![stage_id, assistant.id, index as i64, now, now],
        )?;
    }
    Ok(())
}

fn reorder_project_stage_scope(
    conn: &Connection,
    stage_id: &str,
    process_template_id: &str,
    project_id: Option<&str>,
    target_order: i64,
) -> Result<i64> {
    let rows: Vec<(String, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT id, sort_order
             FROM stages
             WHERE process_template_id = ?
               AND ((project_id IS NULL AND ? IS NULL) OR project_id = ?)
             ORDER BY sort_order ASC, type ASC, project_id IS NOT NULL ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(
            params![process_template_id, project_id, project_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let Some(current_index) = rows.iter().position(|(id, _)| id == stage_id) else {
        anyhow::bail!("project stage not found in reorder scope: {stage_id}");
    };
    let Some(target_index) = rows
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != current_index)
        .find(|(_, (_, order))| *order == target_order)
        .map(|(index, _)| index)
    else {
        return Ok(rows[current_index].1);
    };
    if current_index == target_index {
        return Ok(rows[current_index].1);
    }

    let mut ids = rows.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    let id = ids.remove(current_index);
    ids.insert(target_index, id);

    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE stages SET sort_order = ? WHERE id = ?",
            params![-((index as i64) + 1), id],
        )?;
    }
    let mut next_order = 0;
    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE stages SET sort_order = ? WHERE id = ?",
            params![index as i64, id],
        )?;
        if id == stage_id {
            next_order = index as i64;
        }
    }
    Ok(next_order)
}

pub(super) fn list_project_stages(
    conn: &Connection,
    project_id: &str,
) -> Result<Vec<ProjectStageInfo>> {
    load_project_by_id(conn, project_id)?;
    let mut stmt = conn.prepare(
        "SELECT id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
         FROM stages
         WHERE project_id = ?
         ORDER BY sort_order ASC, type ASC, created_at ASC",
    )?;
    let rows = stmt.query_map(params![project_id], project_stage_from_row)?;
    let mut stages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for stage in stages.iter_mut() {
        stage.assistants = load_project_stage_assistants(conn, &stage.id)?;
    }
    Ok(stages)
}

pub(super) fn list_process_template_stages(
    conn: &Connection,
    process_template_id: &str,
) -> Result<Vec<ProjectStageInfo>> {
    ensure_process_template_exists(conn, process_template_id)?;
    let mut stmt = conn.prepare(
        "SELECT id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
         FROM stages
         WHERE project_id IS NULL AND process_template_id = ?
         ORDER BY sort_order ASC, type ASC, created_at ASC",
    )?;
    let rows = stmt.query_map(params![process_template_id], project_stage_from_row)?;
    let mut stages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for stage in stages.iter_mut() {
        stage.assistants = load_project_stage_assistants(conn, &stage.id)?;
    }
    Ok(stages)
}

pub(super) fn create_project_stage(
    conn: &Connection,
    project_id: &str,
    process_template_id: Option<String>,
    name: &str,
    description: Option<&str>,
    icon: Option<&str>,
) -> Result<ProjectStageInfo> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("project stage name cannot be empty");
    }
    let description = description.map(str::trim).filter(|value| !value.is_empty());
    let icon = icon.map(str::trim).filter(|value| !value.is_empty());
    let requested_process_template_id = process_template_id;
    let project = if requested_process_template_id.is_none() {
        Some(load_project_by_id(conn, project_id)?)
    } else if project_id.trim().is_empty() {
        None
    } else {
        Some(load_project_by_id(conn, project_id)?)
    };
    let resolved_process_template_id = requested_process_template_id
        .as_deref()
        .or_else(|| {
            project
                .as_ref()
                .map(|project| project.process_template_id.as_str())
        })
        .ok_or_else(|| anyhow::anyhow!("project stage requires a project or process template"))?;
    ensure_process_template_exists(conn, resolved_process_template_id)?;
    let template_project_id = if requested_process_template_id.is_some() {
        None
    } else {
        Some(project_id)
    };
    let next_order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM stages
             WHERE process_template_id = ?
               AND ((project_id IS NULL AND ? IS NULL) OR project_id = ?)",
            params![
                resolved_process_template_id,
                template_project_id,
                template_project_id
            ],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let now = now_ms();
    let id = stable_project_stage_id(
        Some(resolved_process_template_id),
        template_project_id,
        name,
        next_order,
        now,
    );
    conn.execute(
        "INSERT INTO stages (id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at)
         VALUES (?, ?, 'custom', ?, NULL, ?, ?, ?, ?, 1, 0, ?, ?)",
        params![
            id,
            template_project_id,
            resolved_process_template_id,
            name,
            description,
            icon,
            next_order,
            now,
            now
        ],
    )?;
    load_project_stage_by_id(conn, &id)
}

pub(super) fn update_project_stage(
    conn: &mut Connection,
    stage_id: &str,
    patch: ProjectStagePatch<'_>,
) -> Result<ProjectStageInfo> {
    let ProjectStagePatch {
        name,
        description,
        icon,
        order,
        enabled,
        allow_empty_assistants,
    } = patch;
    let tx = conn.transaction()?;
    let current = load_project_stage_by_id(&tx, stage_id)?;
    if current.stage_type != ProjectStageType::Custom && (name.is_some() || description.is_some()) {
        anyhow::bail!("builtin project stage details cannot be updated");
    }
    let Some(scope_process_template_id) = current.process_template_id else {
        anyhow::bail!("project stage requires a process template");
    };
    let scope_project_id = current.project_id.as_deref();
    let next_name = match name {
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                anyhow::bail!("project stage name cannot be empty");
            }
            value.to_string()
        }
        None => current.name.unwrap_or_default(),
    };
    let next_description = match description {
        Some(Some(value)) => {
            if value.trim().is_empty() {
                None
            } else {
                Some(value.trim().to_string())
            }
        }
        Some(None) => None,
        None => current.description,
    };
    let next_icon = match icon {
        Some(Some(value)) => {
            if value.trim().is_empty() {
                None
            } else {
                Some(value.trim().to_string())
            }
        }
        Some(None) => None,
        None => current.icon,
    };
    let next_order = match order {
        Some(target_order) if target_order != current.order => reorder_project_stage_scope(
            &tx,
            stage_id,
            scope_process_template_id.as_str(),
            scope_project_id,
            target_order,
        )?,
        _ => current.order,
    };
    let next_enabled = enabled.unwrap_or(current.enabled);
    if current.enabled && !next_enabled {
        ensure_project_stage_can_be_disabled(&tx, stage_id)?;
    }
    let next_allow_empty_assistants =
        allow_empty_assistants.unwrap_or(current.allow_empty_assistants);
    let now = now_ms();
    if current.stage_type == ProjectStageType::Custom {
        tx.execute(
            "UPDATE stages SET name = ?, description = ?, icon = ?, sort_order = ?, enabled = ?, allow_empty_assistants = ?, updated_at = ? WHERE id = ?",
            params![
                next_name,
                next_description,
                next_icon,
                next_order,
                next_enabled as i64,
                next_allow_empty_assistants as i64,
                now,
                stage_id
            ],
        )?;
    } else {
        tx.execute(
            "UPDATE stages SET icon = ?, sort_order = ?, enabled = ?, allow_empty_assistants = ?, updated_at = ? WHERE id = ?",
            params![
                next_icon,
                next_order,
                next_enabled as i64,
                next_allow_empty_assistants as i64,
                now,
                stage_id
            ],
        )?;
    }
    let stage = load_project_stage_by_id(&tx, stage_id)?;
    tx.commit()?;
    Ok(stage)
}

pub(super) fn update_project_stage_assistants(
    conn: &mut Connection,
    stage_id: &str,
    assistant_ids: &[String],
) -> Result<ProjectStageInfo> {
    let tx = conn.transaction()?;
    let stage = load_project_stage_by_id(&tx, stage_id)?;
    let assistants = validate_assistants_for_stage(&tx, &stage, assistant_ids)?;
    let now = now_ms();
    replace_project_stage_assistants(&tx, stage_id, &assistants, now)?;
    tx.execute(
        "UPDATE stages SET updated_at = ? WHERE id = ?",
        params![now, stage_id],
    )?;
    let stage = load_project_stage_by_id(&tx, stage_id)?;
    tx.commit()?;
    Ok(stage)
}

pub(super) fn delete_project_stage(conn: &Connection, stage_id: &str) -> Result<()> {
    let current = load_project_stage_by_id(conn, stage_id)?;
    if current.stage_type != ProjectStageType::Custom {
        anyhow::bail!("builtin project stage cannot be deleted");
    }
    let changed = conn.execute("DELETE FROM stages WHERE id = ?", params![stage_id])?;
    if changed == 0 {
        anyhow::bail!("project stage not found: {stage_id}");
    }
    Ok(())
}

fn stable_issue_id(thread_stage_id: &str, title: &str, now: i64, nonce: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(thread_stage_id.as_bytes());
    hasher.update(title.as_bytes());
    hasher.update(now.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("issue-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_thread_stage_id(
    thread_id: &str,
    stage_id: &str,
    assistant_id: &str,
    order: i64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(thread_id.as_bytes());
    hasher.update(stage_id.as_bytes());
    hasher.update(assistant_id.as_bytes());
    hasher.update(order.to_string().as_bytes());
    format!("thread-stage-{}", &hex::encode(hasher.finalize())[..16])
}

pub(super) fn load_thread_stage_by_id(
    conn: &Connection,
    thread_stage_id: &str,
) -> Result<StageInfo> {
    let mut stage = conn
        .query_row(
            "SELECT ts.id, ts.thread_id, ts.stage_id, t.project_id, s.type, s.process_template_id, s.kind, s.name, s.description, s.icon,
                    ts.sort_order, s.enabled, s.allow_empty_assistants, ts.created_at, ts.updated_at,
                    tss.status, tss.summary, tss.outcome
             FROM thread_stages ts
             INNER JOIN threads t ON t.id = ts.thread_id
             INNER JOIN stages s ON s.id = ts.stage_id
             LEFT JOIN thread_stage_states tss ON tss.thread_stage_id = ts.id
             WHERE ts.id = ?",
            params![thread_stage_id],
            thread_stage_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("thread stage not found: {thread_stage_id}"))?;
    let has_stored_state: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM thread_stage_states WHERE thread_stage_id = ? LIMIT 1",
            params![thread_stage_id],
            |row| row.get(0),
        )
        .optional()?;
    if has_stored_state.is_none() {
        let stages = load_thread_stages(conn, &stage.thread_id)?;
        if let Some(effective) = stages.into_iter().find(|item| item.id == stage.id) {
            stage.status = effective.status;
        }
    }
    stage.assistants = load_stage_assistants(conn, &stage.id)?;
    stage.assistant_ids = stage
        .assistants
        .iter()
        .map(|assistant| assistant.assistant_id.clone())
        .collect();
    stage.sessions = load_stage_sessions(conn, &stage.id)?;
    stage.issues = load_stage_issues(conn, &stage.id)?;
    Ok(stage)
}

pub(super) fn load_stage_assistants(
    conn: &Connection,
    thread_stage_id: &str,
) -> Result<Vec<StageAssistantInfo>> {
    let mut stmt = conn.prepare(
        "SELECT tsa.assistant_id, a.name, a.color, tsa.agent_json, a.system_prompt, a.selected_skill_ids_json, a.selected_mcp_ids_json, tsa.sort_order
         FROM thread_stage_assistants tsa
         INNER JOIN assistants a ON a.id = tsa.assistant_id
         WHERE tsa.thread_stage_id = ?
         ORDER BY tsa.sort_order ASC, tsa.created_at ASC",
    )?;
    let rows = stmt.query_map(params![thread_stage_id], |row| {
        let agent_json: String = row.get(3)?;
        Ok(StageAssistantInfo {
            assistant_id: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
            agent: serde_json::from_str::<AssistantAgentInfo>(&agent_json).unwrap_or_else(|_| {
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: String::new(),
                    mode: String::new(),
                    effort: String::new(),
                }
            }),
            system_prompt: row.get(4)?,
            selected_skill_ids: parse_string_array_json(&row.get::<_, String>(5)?),
            selected_mcp_ids: parse_string_array_json(&row.get::<_, String>(6)?),
            order: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn stage_assistant_from_assistant(assistant: AssistantInfo, order: i64) -> StageAssistantInfo {
    StageAssistantInfo {
        assistant_id: assistant.id,
        name: assistant.name,
        color: assistant.color,
        agent: assistant.agent,
        system_prompt: assistant.system_prompt,
        selected_skill_ids: assistant.selected_skill_ids,
        selected_mcp_ids: assistant.selected_mcp_ids,
        order,
    }
}

fn normalize_assistant_agent(
    conn: &Connection,
    mut agent: AssistantAgentInfo,
) -> Result<AssistantAgentInfo> {
    agent.id = agent.id.trim().to_string();
    agent.name = agent.name.trim().to_string();
    agent.model = agent.model.trim().to_string();
    agent.mode = agent.mode.trim().to_string();
    agent.effort = agent.effort.trim().to_string();
    if agent.id.is_empty() {
        anyhow::bail!("assistant agent id cannot be empty");
    }
    if agent.model.is_empty() {
        anyhow::bail!("assistant model cannot be empty");
    }
    if agent.mode.is_empty() {
        anyhow::bail!("assistant permission mode cannot be empty");
    }
    if agent.effort.is_empty() {
        anyhow::bail!("assistant effort cannot be empty");
    }
    let db_agent = load_agent_by_id(conn, &agent.id)?;
    agent.name = db_agent.name;
    Ok(agent)
}

fn replace_thread_stage_assistants(
    conn: &Connection,
    thread_stage_id: &str,
    assistants: &[StageAssistantInfo],
    now: i64,
) -> Result<()> {
    conn.execute(
        "DELETE FROM thread_stage_assistants WHERE thread_stage_id = ?",
        params![thread_stage_id],
    )?;
    for (index, assistant) in assistants.iter().enumerate() {
        let agent_json = serde_json::to_string(&assistant.agent)?;
        conn.execute(
            "INSERT INTO thread_stage_assistants (thread_stage_id, assistant_id, agent_json, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                thread_stage_id,
                assistant.assistant_id,
                agent_json,
                index as i64,
                now,
                now
            ],
        )?;
    }
    Ok(())
}

fn next_thread_stage_id(conn: &Connection, thread_id: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT id
         FROM thread_stages
         WHERE thread_id = ?
         ORDER BY sort_order ASC, created_at ASC
         LIMIT 1",
        params![thread_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn compact_stage_order(conn: &Connection, thread_id: &str) -> Result<()> {
    let ids: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id
             FROM thread_stages
             WHERE thread_id = ?
             ORDER BY sort_order ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![thread_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE thread_stages SET sort_order = ? WHERE id = ?",
            params![index as i64, id],
        )?;
    }
    Ok(())
}

pub(super) fn load_stage_issues(
    conn: &Connection,
    thread_stage_id: &str,
) -> Result<Vec<StageIssueInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, thread_stage_id, title, description, status, severity, created_at, updated_at
         FROM thread_stage_issues
         WHERE thread_stage_id = ?
         ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![thread_stage_id], |row| {
        let status_raw: String = row.get(4)?;
        let severity_raw: String = row.get(5)?;
        Ok(StageIssueInfo {
            id: row.get(0)?,
            thread_stage_id: row.get(1)?,
            title: row.get(2)?,
            description: row.get(3)?,
            status: IssueStatus::from_db_str(&status_raw).unwrap_or(IssueStatus::Open),
            severity: IssueSeverity::from_db_str(&severity_raw).unwrap_or(IssueSeverity::Medium),
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_stage_issue_by_id(conn: &Connection, issue_id: &str) -> Result<StageIssueInfo> {
    conn.query_row(
        "SELECT id, thread_stage_id, title, description, status, severity, created_at, updated_at
         FROM thread_stage_issues
         WHERE id = ?",
        params![issue_id],
        |row| {
            let status_raw: String = row.get(4)?;
            let severity_raw: String = row.get(5)?;
            Ok(StageIssueInfo {
                id: row.get(0)?,
                thread_stage_id: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                status: IssueStatus::from_db_str(&status_raw).unwrap_or(IssueStatus::Open),
                severity: IssueSeverity::from_db_str(&severity_raw)
                    .unwrap_or(IssueSeverity::Medium),
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("issue not found: {issue_id}"))
}

pub(super) fn add_thread_stage(
    conn: &mut Connection,
    thread_id: &str,
    stage_id: &str,
    assistant_ids: &[String],
) -> Result<StageInfo> {
    let tx = conn.transaction()?;
    let thread = load_thread_by_id(&tx, thread_id)?;
    if !thread.enabled {
        anyhow::bail!("thread is disabled");
    }
    let project = load_project_by_id(&tx, &thread.project_id)?;
    let project_stage = load_project_stage_by_id(&tx, stage_id)?;
    if !project_stage.enabled {
        anyhow::bail!("project stage is disabled");
    }
    if project_stage.project_id.as_deref() != Some(thread.project_id.as_str())
        || project_stage.process_template_id.as_deref()
            != Some(project.process_template_id.as_str())
    {
        anyhow::bail!("project stage does not belong to this thread's project");
    }
    let default_assistant_ids = if assistant_ids.is_empty() {
        project_stage
            .assistants
            .iter()
            .map(|assistant| assistant.assistant_id.clone())
            .collect::<Vec<_>>()
    } else {
        assistant_ids.to_vec()
    };
    let assistant_bindings =
        validate_assistants_for_project(&tx, &thread.project_id, &default_assistant_ids)?
            .into_iter()
            .enumerate()
            .map(|(index, assistant)| stage_assistant_from_assistant(assistant, index as i64))
            .collect::<Vec<_>>();
    let assistant_ids = assistant_bindings
        .iter()
        .map(|assistant| assistant.assistant_id.clone())
        .collect::<Vec<_>>();
    if assistant_ids.is_empty() && !project_stage.allow_empty_assistants {
        anyhow::bail!("stage does not allow empty assistants");
    }
    let next_order: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM thread_stages WHERE thread_id = ?",
            params![thread_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let now = now_ms();
    let id = stable_thread_stage_id(thread_id, stage_id, &assistant_ids.join(","), next_order);
    tx.execute(
        "INSERT INTO thread_stages (id, thread_id, stage_id, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        params![id, thread_id, stage_id, next_order, now, now],
    )?;
    replace_thread_stage_assistants(&tx, &id, &assistant_bindings, now)?;
    tx.execute(
        "UPDATE threads SET updated_at = ? WHERE id = ?",
        params![now, thread_id],
    )?;
    let stage = load_thread_stage_by_id(&tx, &id)?;
    tx.commit()?;
    Ok(stage)
}

pub(super) fn update_thread_stage(
    conn: &mut Connection,
    thread_stage_id: &str,
    assistant_ids: Option<&[String]>,
    order: Option<i64>,
    enabled: Option<bool>,
) -> Result<StageInfo> {
    let tx = conn.transaction()?;
    let current = load_thread_stage_by_id(&tx, thread_stage_id)?;
    let next_assistant_bindings = match assistant_ids {
        Some(ids) => {
            let bindings = validate_assistants_for_project(&tx, &current.project_id, ids)?
                .into_iter()
                .enumerate()
                .map(|(index, assistant)| stage_assistant_from_assistant(assistant, index as i64))
                .collect::<Vec<_>>();
            if bindings.is_empty() && !current.allow_empty_assistants {
                anyhow::bail!("stage does not allow empty assistants");
            }
            Some(bindings)
        }
        None => None,
    };
    let max_order: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(sort_order), 0) FROM thread_stages WHERE thread_id = ?",
            params![current.thread_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let next_order = order.unwrap_or(current.order).clamp(0, max_order);
    if next_order != current.order {
        let mut ids = {
            let mut stmt = tx.prepare(
                "SELECT id
                 FROM thread_stages
                 WHERE thread_id = ?
                 ORDER BY sort_order ASC, created_at ASC",
            )?;
            let rows = stmt.query_map(params![current.thread_id], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<Vec<String>>>()?
        };
        let Some(current_index) = ids.iter().position(|id| id == thread_stage_id) else {
            anyhow::bail!("thread stage not found in reorder scope: {thread_stage_id}");
        };
        let id = ids.remove(current_index);
        ids.insert(next_order as usize, id);
        for (index, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE thread_stages SET sort_order = ? WHERE id = ?",
                params![-((index as i64) + 1), id],
            )?;
        }
        for (index, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE thread_stages SET sort_order = ? WHERE id = ?",
                params![index as i64, id],
            )?;
        }
    }
    let now = now_ms();
    tx.execute(
        "UPDATE thread_stages
         SET sort_order = ?, updated_at = ?
         WHERE id = ?",
        params![next_order, now, thread_stage_id],
    )?;
    if let Some(next_assistant_bindings) = next_assistant_bindings {
        replace_thread_stage_assistants(&tx, thread_stage_id, &next_assistant_bindings, now)?;
    }
    if let Some(enabled) = enabled {
        if current.enabled && !enabled {
            ensure_project_stage_can_be_disabled(&tx, &current.stage_id)?;
        }
        tx.execute(
            "UPDATE stages SET enabled = ?, updated_at = ? WHERE id = ?",
            params![enabled as i64, now, current.stage_id],
        )?;
    }
    compact_stage_order(&tx, &current.thread_id)?;
    tx.execute(
        "UPDATE threads SET updated_at = ? WHERE id = ?",
        params![now, current.thread_id],
    )?;
    let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
    tx.commit()?;
    Ok(stage)
}

pub(super) fn update_thread_stage_state(
    conn: &mut Connection,
    thread_stage_id: &str,
    status: Option<StageStatus>,
    summary: Option<Option<String>>,
    outcome: Option<Option<String>>,
) -> Result<StageInfo> {
    let tx = conn.transaction()?;
    let current = load_thread_stage_by_id(&tx, thread_stage_id)?;
    let next_status = status.unwrap_or(current.status);
    let next_summary = match summary {
        Some(value) => value,
        None => current.summary.clone(),
    };
    let next_outcome = match outcome {
        Some(value) => value,
        None => current.outcome.clone(),
    };
    let now = now_ms();
    tx.execute(
        "INSERT INTO thread_stage_states
            (thread_stage_id, status, summary, outcome, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(thread_stage_id) DO UPDATE SET
            status = excluded.status,
            summary = excluded.summary,
            outcome = excluded.outcome,
            updated_at = excluded.updated_at",
        params![
            thread_stage_id,
            next_status.as_str(),
            next_summary,
            next_outcome,
            now,
            now
        ],
    )?;
    tx.execute(
        "UPDATE threads SET updated_at = ? WHERE id = ?",
        params![now, current.thread_id],
    )?;
    let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
    tx.commit()?;
    Ok(stage)
}

pub(super) fn list_thread_stage_issues(
    conn: &Connection,
    thread_stage_id: &str,
) -> Result<Vec<StageIssueInfo>> {
    load_stage_issues(conn, thread_stage_id)
}

pub(super) fn create_thread_stage_issue(
    conn: &Connection,
    thread_stage_id: &str,
    title: &str,
    description: Option<&str>,
    severity: IssueSeverity,
) -> Result<StageIssueInfo> {
    let title = title.trim();
    if title.is_empty() {
        anyhow::bail!("issue title cannot be empty");
    }
    let description = description.map(str::trim).filter(|s| !s.is_empty());
    let exists = conn
        .query_row(
            "SELECT 1 FROM thread_stages WHERE id = ?",
            params![thread_stage_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        anyhow::bail!("thread stage not found: {thread_stage_id}");
    }
    let now = now_ms();
    let id = stable_issue_id(thread_stage_id, title, now, &unique_nonce());
    conn.execute(
        "INSERT INTO thread_stage_issues (
            id, thread_stage_id, title, description, status, severity, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            id,
            thread_stage_id,
            title,
            description,
            IssueStatus::Open.as_str(),
            severity.as_str(),
            now,
            now,
        ],
    )?;
    load_stage_issue_by_id(conn, &id)
}

pub(super) fn update_thread_stage_issue(
    conn: &Connection,
    issue_id: &str,
    title: Option<&str>,
    description: Option<Option<&str>>,
    status: Option<IssueStatus>,
    severity: Option<IssueSeverity>,
) -> Result<StageIssueInfo> {
    let current = load_stage_issue_by_id(conn, issue_id)?;
    let next_title = match title {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                anyhow::bail!("issue title cannot be empty");
            }
            trimmed.to_string()
        }
        None => current.title,
    };
    let next_description = match description {
        Some(Some(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(None) => None,
        None => current.description,
    };
    let next_status = status.unwrap_or(current.status);
    let next_severity = severity.unwrap_or(current.severity);
    conn.execute(
        "UPDATE thread_stage_issues
         SET title = ?, description = ?, status = ?, severity = ?, updated_at = ?
         WHERE id = ?",
        params![
            next_title,
            next_description,
            next_status.as_str(),
            next_severity.as_str(),
            now_ms(),
            issue_id,
        ],
    )?;
    load_stage_issue_by_id(conn, issue_id)
}

pub(super) fn delete_thread_stage_issue(conn: &Connection, issue_id: &str) -> Result<()> {
    let changed = conn.execute(
        "DELETE FROM thread_stage_issues WHERE id = ?",
        params![issue_id],
    )?;
    if changed == 0 {
        anyhow::bail!("issue not found: {issue_id}");
    }
    Ok(())
}

pub(super) fn update_thread_stage_assistant_agent(
    conn: &mut Connection,
    thread_stage_id: &str,
    assistant_id: &str,
    agent: AssistantAgentInfo,
) -> Result<StageInfo> {
    let tx = conn.transaction()?;
    let current = load_thread_stage_by_id(&tx, thread_stage_id)?;
    validate_assistant_for_project(&tx, &current.project_id, assistant_id)?;
    let exists: i64 = tx.query_row(
        "SELECT count(*) FROM thread_stage_assistants WHERE thread_stage_id = ? AND assistant_id = ?",
        params![thread_stage_id, assistant_id],
        |row| row.get(0),
    )?;
    if exists == 0 {
        anyhow::bail!("assistant is not linked to this thread stage");
    }
    let agent = normalize_assistant_agent(&tx, agent)?;
    let agent_json = serde_json::to_string(&agent)?;
    let now = now_ms();
    tx.execute(
        "UPDATE thread_stage_assistants
         SET agent_json = ?, updated_at = ?
         WHERE thread_stage_id = ? AND assistant_id = ?",
        params![agent_json, now, thread_stage_id, assistant_id],
    )?;
    tx.execute(
        "UPDATE thread_stages SET updated_at = ? WHERE id = ?",
        params![now, thread_stage_id],
    )?;
    tx.execute(
        "UPDATE threads SET updated_at = ? WHERE id = ?",
        params![now, current.thread_id],
    )?;
    let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
    tx.commit()?;
    Ok(stage)
}

pub(super) fn delete_thread_stage(conn: &mut Connection, thread_stage_id: &str) -> Result<()> {
    let tx = conn.transaction()?;
    let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
    let session_refs = {
        let mut stmt = tx.prepare(
            "SELECT agent, session_id
             FROM stage_sessions
             WHERE thread_stage_id = ?",
        )?;
        let rows = stmt.query_map(params![thread_stage_id], |row| {
            let agent_str: String = row.get(0)?;
            let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
            Ok((agent, row.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    tx.execute(
        "DELETE FROM thread_stages WHERE id = ?",
        params![thread_stage_id],
    )?;
    compact_stage_order(&tx, &stage.thread_id)?;
    let current_stage_id: Option<String> = tx.query_row(
        "SELECT stage_id FROM threads WHERE id = ?",
        params![stage.thread_id],
        |row| row.get(0),
    )?;
    let next_stage_id = if current_stage_id.as_deref() == Some(thread_stage_id) {
        next_thread_stage_id(&tx, &stage.thread_id)?
    } else {
        current_stage_id
    };
    tx.execute(
        "UPDATE threads SET stage_id = ?, updated_at = ? WHERE id = ?",
        params![next_stage_id, now_ms(), stage.thread_id],
    )?;
    for (agent, session_id) in &session_refs {
        downgrade_session_origin_when_unlinked(&tx, *agent, session_id)?;
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn set_thread_stage(
    conn: &Connection,
    thread_id: &str,
    thread_stage_id: &str,
) -> Result<ThreadInfo> {
    let thread = load_thread_by_id(conn, thread_id)?;
    if !thread.enabled {
        anyhow::bail!("thread is disabled");
    }
    let stage = load_thread_stage_by_id(conn, thread_stage_id)?;
    if stage.thread_id != thread_id {
        anyhow::bail!("stage does not belong to this thread");
    }
    if !stage.enabled {
        anyhow::bail!("thread stage is disabled");
    }
    conn.execute(
        "UPDATE threads SET stage_id = ?, updated_at = ? WHERE id = ?",
        params![thread_stage_id, now_ms(), thread_id],
    )?;
    load_thread_by_id(conn, thread_id)
}

pub(super) fn link_stage_session(
    conn: &mut Connection,
    thread_stage_id: &str,
    agent: Agent,
    session_id: &str,
) -> Result<StageInfo> {
    let tx = conn.transaction()?;
    let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
    if !stage.enabled {
        anyhow::bail!("thread stage is disabled");
    }
    let thread = load_thread_by_id(&tx, &stage.thread_id)?;
    if !thread.enabled {
        anyhow::bail!("thread is disabled");
    }
    let project = load_project_by_id(&tx, &stage.project_id)?;
    let session_project_path = session_project_path(&tx, agent, session_id)?
        .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
    if session_project_path != project.path {
        anyhow::bail!("session does not belong to this stage's project");
    }
    ensure_session_not_linked_to_thread_process(&tx, agent, session_id)?;
    let now = now_ms();
    tx.execute(
        "INSERT OR IGNORE INTO stage_sessions (thread_stage_id, agent, session_id, created_at)
         VALUES (?, ?, ?, ?)",
        params![thread_stage_id, agent.as_str(), session_id, now],
    )?;
    upgrade_session_origin_to_thread(&tx, agent, session_id)?;
    tx.execute(
        "UPDATE thread_stages SET updated_at = ? WHERE id = ?",
        params![now, thread_stage_id],
    )?;
    tx.execute(
        "UPDATE threads SET updated_at = ? WHERE id = ?",
        params![now, stage.thread_id],
    )?;
    let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
    tx.commit()?;
    Ok(stage)
}

pub(super) fn unlink_stage_session(
    conn: &mut Connection,
    thread_stage_id: &str,
    agent: Agent,
    session_id: &str,
) -> Result<StageInfo> {
    let tx = conn.transaction()?;
    let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
    tx.execute(
        "DELETE FROM stage_sessions
         WHERE thread_stage_id = ? AND agent = ? AND session_id = ?",
        params![thread_stage_id, agent.as_str(), session_id],
    )?;
    downgrade_session_origin_when_unlinked(&tx, agent, session_id)?;
    let now = now_ms();
    tx.execute(
        "UPDATE thread_stages SET updated_at = ? WHERE id = ?",
        params![now, thread_stage_id],
    )?;
    tx.execute(
        "UPDATE threads SET updated_at = ? WHERE id = ?",
        params![now, stage.thread_id],
    )?;
    let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
    tx.commit()?;
    Ok(stage)
}
