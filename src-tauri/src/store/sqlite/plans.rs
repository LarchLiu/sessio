use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{
    Agent, PlanRoundInfo, PlanRoundMode, PlanRoundStatus, PlanTaskInfo, PlanTaskSessionInfo,
    PlanTaskSessionRole, PlanTaskStatus,
};
use crate::store::{now_ms, NewPlanRound, NewPlanTask, NewPlanTaskSession, PlanTaskStatusPatch};

use super::identity::{downgrade_session_origin_when_unlinked, upgrade_session_origin_to_thread};
use super::plan_queries::{
    load_plan_round_by_id, load_plan_task_by_id, load_plan_task_sessions, load_plan_tasks,
    plan_round_from_row, plan_task_session_from_row,
};
use super::thread_queries::load_thread_by_id;
use super::{load_assistant_by_id, unique_nonce};

fn stable_plan_round_id(thread_id: &str, round_index: i64, now: i64, nonce: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(thread_id.as_bytes());
    hasher.update(round_index.to_string().as_bytes());
    hasher.update(now.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("plan-round-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_plan_task_id(
    round_id: &str,
    title: &str,
    sort_order: i64,
    now: i64,
    nonce: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(round_id.as_bytes());
    hasher.update(title.as_bytes());
    hasher.update(sort_order.to_string().as_bytes());
    hasher.update(now.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("plan-task-{}", &hex::encode(hasher.finalize())[..16])
}

fn clean_required(value: &str, field: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("{field} cannot be empty");
    }
    Ok(value.to_string())
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn aggregate_round_status(statuses: &[PlanTaskStatus]) -> PlanRoundStatus {
    if statuses.contains(&PlanTaskStatus::Running) {
        return PlanRoundStatus::Running;
    }
    if statuses
        .iter()
        .any(|status| matches!(status, PlanTaskStatus::Failed | PlanTaskStatus::Errored))
    {
        return PlanRoundStatus::Errored;
    }
    if statuses
        .iter()
        .all(|status| *status == PlanTaskStatus::Cancelled)
    {
        return PlanRoundStatus::Cancelled;
    }
    if statuses.contains(&PlanTaskStatus::Planned) {
        return PlanRoundStatus::Planned;
    }
    PlanRoundStatus::Completed
}

fn validate_new_plan_round_invariants(round: &NewPlanRound<'_>) -> Result<()> {
    if round.mode != PlanRoundMode::Sequential {
        return Ok(());
    }
    let running_tasks = round
        .tasks
        .iter()
        .filter(|task| task.status == PlanTaskStatus::Running)
        .collect::<Vec<_>>();
    if running_tasks.len() > 1 {
        anyhow::bail!("sequential plan round cannot start multiple running tasks");
    }
    if let Some(running_task) = running_tasks.first() {
        let lower_planned_count = round
            .tasks
            .iter()
            .filter(|task| task.status == PlanTaskStatus::Planned)
            .filter(|task| task.sort_order < running_task.sort_order)
            .count();
        if lower_planned_count > 0 {
            anyhow::bail!("sequential plan round must start the lowest-order planned task");
        }
    }
    Ok(())
}

fn plan_task_statuses(conn: &Connection, round_id: &str) -> Result<Vec<PlanTaskStatus>> {
    let mut stmt = conn.prepare(
        "SELECT status
         FROM thread_plan_tasks
         WHERE round_id = ?
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let rows = stmt.query_map(params![round_id], |row| {
        let status_raw: String = row.get(0)?;
        Ok(PlanTaskStatus::from_db_str(&status_raw).unwrap_or(PlanTaskStatus::Planned))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn update_plan_round_status_from_tasks(
    conn: &Connection,
    round_id: &str,
    now: i64,
) -> Result<PlanRoundStatus> {
    let statuses = plan_task_statuses(conn, round_id)?;
    let status = aggregate_round_status(&statuses);
    conn.execute(
        "UPDATE thread_plan_rounds
         SET status = ?, updated_at = ?
         WHERE id = ?",
        params![status.as_str(), now, round_id],
    )?;
    Ok(status)
}

fn ensure_no_other_running_task(
    conn: &Connection,
    round_id: &str,
    task_id: Option<&str>,
) -> Result<()> {
    let count: i64 = match task_id {
        Some(task_id) => conn.query_row(
            "SELECT count(*)
             FROM thread_plan_tasks
             WHERE round_id = ? AND status = 'running' AND id != ?",
            params![round_id, task_id],
            |row| row.get(0),
        )?,
        None => conn.query_row(
            "SELECT count(*)
             FROM thread_plan_tasks
             WHERE round_id = ? AND status = 'running'",
            params![round_id],
            |row| row.get(0),
        )?,
    };
    if count > 0 {
        anyhow::bail!("sequential plan round already has a running task");
    }
    Ok(())
}

fn ensure_sequential_running_candidate(
    conn: &Connection,
    round_id: &str,
    task_id: &str,
) -> Result<()> {
    ensure_no_other_running_task(conn, round_id, Some(task_id))?;
    let candidate_order: i64 = conn.query_row(
        "SELECT sort_order
         FROM thread_plan_tasks
         WHERE id = ? AND round_id = ?",
        params![task_id, round_id],
        |row| row.get(0),
    )?;
    let lower_planned_count: i64 = conn.query_row(
        "SELECT count(*)
         FROM thread_plan_tasks
         WHERE round_id = ? AND status = 'planned' AND sort_order < ?",
        params![round_id, candidate_order],
        |row| row.get(0),
    )?;
    if lower_planned_count > 0 {
        anyhow::bail!("sequential plan round must start the lowest-order planned task");
    }
    Ok(())
}

fn ensure_plan_task_refs(conn: &Connection, thread_id: &str, task: &NewPlanTask<'_>) -> Result<()> {
    if let Some(thread_stage_id) = task.thread_stage_id {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM thread_stages WHERE id = ? AND thread_id = ? LIMIT 1",
                params![thread_stage_id, thread_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            anyhow::bail!("thread stage does not belong to thread: {thread_stage_id}");
        }
    }
    if let Some(assistant_id) = task.assistant_id {
        load_assistant_by_id(conn, assistant_id)?;
    }
    if let Some(participant_id) = task.agent_participant_id {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM thread_agents WHERE thread_id = ? AND participant_id = ? LIMIT 1",
                params![thread_id, participant_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            anyhow::bail!("thread agent participant does not belong to thread: {participant_id}");
        }
    }
    Ok(())
}

fn insert_plan_task(
    conn: &Connection,
    round_id: &str,
    thread_id: &str,
    task: &NewPlanTask<'_>,
    now: i64,
    nonce: &str,
) -> Result<()> {
    ensure_plan_task_refs(conn, thread_id, task)?;
    let title = clean_required(task.title, "plan task title")?;
    let prompt = clean_required(task.prompt, "plan task prompt")?;
    let agent_snapshot_json =
        clean_required(task.agent_snapshot_json, "plan task agent snapshot json")?;
    let expected_output = clean_optional(task.expected_output);
    let artifact_role = clean_optional(task.artifact_role);
    let uses_artifact_roles_json = serde_json::to_string(task.uses_artifact_roles)?;
    let stage_snapshot_json = clean_optional(task.stage_snapshot_json);
    let assistant_snapshot_json = clean_optional(task.assistant_snapshot_json);
    let started_at = if matches!(task.status, PlanTaskStatus::Running) || task.status.is_terminal()
    {
        Some(now)
    } else {
        None
    };
    let completed_at = if task.status.is_terminal() {
        Some(now)
    } else {
        None
    };
    let id = stable_plan_task_id(round_id, &title, task.sort_order, now, nonce);
    conn.execute(
        "INSERT INTO thread_plan_tasks (
            id, round_id, thread_stage_id, assistant_id, agent_participant_id, target_agent,
            stage_snapshot_json, assistant_snapshot_json, agent_snapshot_json,
            title, prompt, expected_output, artifact_role, uses_artifact_roles_json,
            risk, sort_order, status,
            result_summary, error, started_at, completed_at, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?)",
        params![
            id,
            round_id,
            task.thread_stage_id,
            task.assistant_id,
            task.agent_participant_id,
            task.target_agent.as_str(),
            stage_snapshot_json,
            assistant_snapshot_json,
            agent_snapshot_json,
            title,
            prompt,
            expected_output,
            artifact_role,
            uses_artifact_roles_json,
            task.risk.as_str(),
            task.sort_order,
            task.status.as_str(),
            started_at,
            completed_at,
            now,
            now,
        ],
    )?;
    Ok(())
}

fn apply_plan_task_status_patch(
    conn: &Connection,
    task_id: &str,
    patch: PlanTaskStatusPatch<'_>,
    now: i64,
) -> Result<PlanTaskInfo> {
    let current = load_plan_task_by_id(conn, task_id)?;
    let round = load_plan_round_by_id(conn, &current.round_id)?;
    if round.mode == PlanRoundMode::Sequential && patch.status == PlanTaskStatus::Running {
        ensure_sequential_running_candidate(conn, &current.round_id, task_id)?;
    }
    let result_summary = match patch.result_summary {
        Some(value) => clean_optional(value),
        None => current.result_summary,
    };
    let error = match patch.error {
        Some(value) => clean_optional(value),
        None => current.error,
    };
    let started_at = match patch.status {
        PlanTaskStatus::Planned => None,
        PlanTaskStatus::Running => current.started_at.or(Some(now)),
        status if status.is_terminal() => current.started_at.or(Some(now)),
        _ => current.started_at,
    };
    let completed_at = if patch.status.is_terminal() {
        current.completed_at.or(Some(now))
    } else {
        None
    };
    conn.execute(
        "UPDATE thread_plan_tasks
         SET status = ?,
             result_summary = ?,
             error = ?,
             started_at = ?,
             completed_at = ?,
             updated_at = ?
         WHERE id = ?",
        params![
            patch.status.as_str(),
            result_summary,
            error,
            started_at,
            completed_at,
            now,
            task_id,
        ],
    )?;
    update_plan_round_status_from_tasks(conn, &current.round_id, now)?;
    load_plan_task_by_id(conn, task_id)
}

pub(super) fn create_plan_round(
    conn: &mut Connection,
    round: NewPlanRound<'_>,
) -> Result<PlanRoundInfo> {
    validate_new_plan_round_invariants(&round)?;
    let tx = conn.transaction()?;
    load_thread_by_id(&tx, round.thread_id)?;
    if let Some(astra_run_id) = round.astra_run_id {
        let exists: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM astra_runs WHERE run_id = ? LIMIT 1",
                params![astra_run_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            anyhow::bail!("Astra run not found: {astra_run_id}");
        }
    }
    let round_index = match round.round_index {
        Some(value) if value < 0 => anyhow::bail!("round index cannot be negative"),
        Some(value) => value,
        None => tx.query_row(
            "SELECT COALESCE(MAX(round_index), -1) + 1
             FROM thread_plan_rounds
             WHERE thread_id = ?",
            params![round.thread_id],
            |row| row.get(0),
        )?,
    };
    let now = now_ms();
    let id = stable_plan_round_id(round.thread_id, round_index, now, &unique_nonce());
    let summary = clean_optional(round.summary);
    tx.execute(
        "INSERT INTO thread_plan_rounds (
            id, thread_id, astra_run_id, round_index, summary, mode, source, status, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            id,
            round.thread_id,
            round.astra_run_id,
            round_index,
            summary,
            round.mode.as_str(),
            round.source.as_str(),
            round.status.as_str(),
            now,
            now,
        ],
    )?;
    for task in &round.tasks {
        insert_plan_task(&tx, &id, round.thread_id, task, now, &unique_nonce())?;
    }
    if !round.tasks.is_empty() {
        update_plan_round_status_from_tasks(&tx, &id, now)?;
    }
    let loaded = load_plan_round_by_id(&tx, &id)?;
    tx.commit()?;
    Ok(loaded)
}

pub(super) fn get_plan_round(conn: &Connection, round_id: &str) -> Result<Option<PlanRoundInfo>> {
    let round = conn
        .query_row(
            "SELECT id, thread_id, astra_run_id, round_index, summary, mode, source, status, created_at, updated_at
             FROM thread_plan_rounds
             WHERE id = ?",
            params![round_id],
            plan_round_from_row,
        )
        .optional()?;
    match round {
        Some(mut round) => {
            round.tasks = load_plan_tasks(conn, &round.id)?;
            Ok(Some(round))
        }
        None => Ok(None),
    }
}

pub(super) fn list_plan_rounds(conn: &Connection, thread_id: &str) -> Result<Vec<PlanRoundInfo>> {
    load_thread_by_id(conn, thread_id)?;
    let mut stmt = conn.prepare(
        "SELECT id, thread_id, astra_run_id, round_index, summary, mode, source, status, created_at, updated_at
         FROM thread_plan_rounds
         WHERE thread_id = ?
         ORDER BY round_index ASC, created_at ASC",
    )?;
    let mut rounds = stmt
        .query_map(params![thread_id], plan_round_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for round in rounds.iter_mut() {
        round.tasks = load_plan_tasks(conn, &round.id)?;
    }
    Ok(rounds)
}

pub(super) fn get_plan_task_thread_id(conn: &Connection, task_id: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT r.thread_id
         FROM thread_plan_tasks t
         INNER JOIN thread_plan_rounds r ON r.id = t.round_id
         WHERE t.id = ?",
        params![task_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn update_plan_task_status(
    conn: &mut Connection,
    task_id: &str,
    patch: PlanTaskStatusPatch<'_>,
) -> Result<PlanTaskInfo> {
    let tx = conn.transaction()?;
    let task = apply_plan_task_status_patch(&tx, task_id, patch, now_ms())?;
    tx.commit()?;
    Ok(task)
}

pub(super) fn complete_plan_task_and_start_next(
    conn: &mut Connection,
    task_id: &str,
    patch: PlanTaskStatusPatch<'_>,
) -> Result<PlanRoundInfo> {
    if !patch.status.is_terminal() {
        anyhow::bail!("sequential transition requires a terminal task status");
    }
    let tx = conn.transaction()?;
    let current = load_plan_task_by_id(&tx, task_id)?;
    let round = load_plan_round_by_id(&tx, &current.round_id)?;
    if round.mode != PlanRoundMode::Sequential {
        anyhow::bail!("plan round is not sequential");
    }
    let now = now_ms();
    apply_plan_task_status_patch(&tx, task_id, patch, now)?;
    ensure_no_other_running_task(&tx, &current.round_id, None)?;
    let next_task_id: Option<String> = tx
        .query_row(
            "SELECT id
             FROM thread_plan_tasks
             WHERE round_id = ? AND status = 'planned'
             ORDER BY sort_order ASC, created_at ASC
             LIMIT 1",
            params![current.round_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(next_task_id) = next_task_id {
        apply_plan_task_status_patch(
            &tx,
            &next_task_id,
            PlanTaskStatusPatch {
                status: PlanTaskStatus::Running,
                result_summary: None,
                error: None,
            },
            now,
        )?;
    } else {
        update_plan_round_status_from_tasks(&tx, &current.round_id, now)?;
    }
    let loaded = load_plan_round_by_id(&tx, &current.round_id)?;
    tx.commit()?;
    Ok(loaded)
}

pub(super) fn link_plan_task_session(
    conn: &Connection,
    session: NewPlanTaskSession<'_>,
) -> Result<PlanTaskSessionInfo> {
    load_plan_task_by_id(conn, session.task_id)?;
    let now = now_ms();
    let attempt_count = session.attempt_count.max(1);
    let attempt_id = session
        .attempt_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let superseded_at = if session.role == PlanTaskSessionRole::Runtime {
        let delegated_exists = conn
            .query_row(
                "SELECT 1
                 FROM thread_plan_task_sessions
                 WHERE task_id = ? AND agent = ? AND role = 'delegated' AND attempt_count = ?
                   AND superseded_at IS NULL
                 LIMIT 1",
                params![session.task_id, session.agent.as_str(), attempt_count],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        delegated_exists.then_some(now)
    } else {
        None
    };
    let superseded_session_refs = {
        let mut stmt = conn.prepare(
            "SELECT agent, session_id
             FROM thread_plan_task_sessions
             WHERE task_id = ? AND agent = ? AND role = ? AND attempt_count = ?
               AND session_id != ? AND superseded_at IS NULL",
        )?;
        let refs = stmt
            .query_map(
                params![
                    session.task_id,
                    session.agent.as_str(),
                    session.role.as_str(),
                    attempt_count,
                    session.session_id,
                ],
                |row| {
                    let agent_str: String = row.get(0)?;
                    let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
                    Ok((agent, row.get::<_, String>(1)?))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        refs
    };
    conn.execute(
        "UPDATE thread_plan_task_sessions
         SET superseded_at = COALESCE(superseded_at, ?), updated_at = ?
         WHERE task_id = ? AND agent = ? AND role = ? AND attempt_count = ?
           AND session_id != ? AND superseded_at IS NULL",
        params![
            now,
            now,
            session.task_id,
            session.agent.as_str(),
            session.role.as_str(),
            attempt_count,
            session.session_id,
        ],
    )?;
    conn.execute(
        "INSERT INTO thread_plan_task_sessions (
            task_id, agent, session_id, role, attempt_id, attempt_count, superseded_at, created_at, updated_at
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(task_id, agent, session_id, role) DO UPDATE SET
            attempt_id = excluded.attempt_id,
            attempt_count = excluded.attempt_count,
            superseded_at = excluded.superseded_at,
            updated_at = excluded.updated_at",
        params![
            session.task_id,
            session.agent.as_str(),
            session.session_id,
            session.role.as_str(),
            attempt_id,
            attempt_count,
            superseded_at,
            now,
            now,
        ],
    )?;
    if superseded_at.is_some() {
        downgrade_session_origin_when_unlinked(conn, session.agent, session.session_id)?;
    } else {
        upgrade_session_origin_to_thread(conn, session.agent, session.session_id)?;
    }
    for (agent, session_id) in &superseded_session_refs {
        downgrade_session_origin_when_unlinked(conn, *agent, session_id)?;
    }
    conn.query_row(
        "SELECT task_id, agent, session_id, role, attempt_id, attempt_count, superseded_at, created_at, updated_at
         FROM thread_plan_task_sessions
         WHERE task_id = ? AND agent = ? AND session_id = ? AND role = ?",
        params![
            session.task_id,
            session.agent.as_str(),
            session.session_id,
            session.role.as_str(),
        ],
        plan_task_session_from_row,
    )
    .map_err(Into::into)
}

pub(super) fn relink_plan_task_session(
    conn: &mut Connection,
    from: NewPlanTaskSession<'_>,
    to_session_id: &str,
    to_role: PlanTaskSessionRole,
) -> Result<PlanTaskSessionInfo> {
    let tx = conn.transaction()?;
    load_plan_task_by_id(&tx, from.task_id)?;
    let now = now_ms();
    let existing_attempt = tx
        .query_row(
            "SELECT attempt_id, attempt_count, created_at
             FROM thread_plan_task_sessions
             WHERE task_id = ? AND agent = ? AND session_id = ? AND role = ?",
            params![
                from.task_id,
                from.agent.as_str(),
                from.session_id,
                from.role.as_str(),
            ],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let attempt_id = existing_attempt
        .as_ref()
        .and_then(|(attempt_id, _, _)| attempt_id.as_deref())
        .or_else(|| {
            from.attempt_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
        });
    let attempt_count = existing_attempt
        .as_ref()
        .map(|(_, attempt_count, _)| *attempt_count)
        .unwrap_or_else(|| from.attempt_count.max(1));
    let existing_created_at = existing_attempt
        .as_ref()
        .map(|(_, _, created_at)| *created_at)
        .unwrap_or(now);
    let superseded_session_refs = {
        let mut stmt = tx.prepare(
            "SELECT agent, session_id
             FROM thread_plan_task_sessions
             WHERE task_id = ? AND agent = ? AND role = ? AND attempt_count = ?
               AND session_id != ? AND superseded_at IS NULL",
        )?;
        let refs = stmt
            .query_map(
                params![
                    from.task_id,
                    from.agent.as_str(),
                    from.role.as_str(),
                    attempt_count,
                    to_session_id,
                ],
                |row| {
                    let agent_str: String = row.get(0)?;
                    let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
                    Ok((agent, row.get::<_, String>(1)?))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        refs
    };
    tx.execute(
        "UPDATE thread_plan_task_sessions
         SET superseded_at = COALESCE(superseded_at, ?), updated_at = ?
         WHERE task_id = ? AND agent = ? AND role = ? AND attempt_count = ?
           AND session_id != ? AND superseded_at IS NULL",
        params![
            now,
            now,
            from.task_id,
            from.agent.as_str(),
            from.role.as_str(),
            attempt_count,
            to_session_id,
        ],
    )?;
    tx.execute(
        "INSERT INTO thread_plan_task_sessions (
            task_id, agent, session_id, role, attempt_id, attempt_count, superseded_at, created_at, updated_at
         )
         VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?)
         ON CONFLICT(task_id, agent, session_id, role) DO UPDATE SET
            attempt_id = excluded.attempt_id,
            attempt_count = excluded.attempt_count,
            superseded_at = excluded.superseded_at,
            updated_at = excluded.updated_at",
        params![
            from.task_id,
            from.agent.as_str(),
            to_session_id,
            to_role.as_str(),
            attempt_id,
            attempt_count,
            existing_created_at,
            now,
        ],
    )?;
    upgrade_session_origin_to_thread(&tx, from.agent, to_session_id)?;
    for (agent, session_id) in &superseded_session_refs {
        downgrade_session_origin_when_unlinked(&tx, *agent, session_id)?;
    }
    let linked = tx.query_row(
        "SELECT task_id, agent, session_id, role, attempt_id, attempt_count, superseded_at, created_at, updated_at
         FROM thread_plan_task_sessions
         WHERE task_id = ? AND agent = ? AND session_id = ? AND role = ?",
        params![
            from.task_id,
            from.agent.as_str(),
            to_session_id,
            to_role.as_str(),
        ],
        plan_task_session_from_row,
    )?;
    tx.commit()?;
    Ok(linked)
}

pub(super) fn list_plan_task_sessions(
    conn: &Connection,
    task_id: &str,
) -> Result<Vec<PlanTaskSessionInfo>> {
    load_plan_task_by_id(conn, task_id)?;
    load_plan_task_sessions(conn, task_id)
}
