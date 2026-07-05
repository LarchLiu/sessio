use std::collections::{HashMap, HashSet, VecDeque};

use serde::Deserialize;
use serde_yaml::Value as YamlValue;

use super::artifact_roles::{
    normalize_artifact_role_catalog, resolve_artifact_role, resolve_artifact_roles,
};
use super::{short_hash, AstraOrchestration, AstraRun, AstraRunIntent, AstraTaskCompletion};
use crate::astra::types::{AstraTaskProposal, AstraTaskRisk};
use crate::models::{Agent, PlanRoundMode, ThreadInfo, ThreadKind};

#[derive(Debug, Clone)]
pub(super) struct AstraOrchestrationParseFailure {
    pub code: &'static str,
    pub message: String,
}

impl AstraOrchestrationParseFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct RawAstraOrchestration {
    summary: Option<String>,
    run_intent: Option<AstraRunIntent>,
    reason: Option<String>,
    mode: Option<PlanRoundMode>,
    #[serde(default)]
    tasks: Vec<RawAstraTask>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct RawAstraTask {
    #[serde(rename = "id")]
    id: Option<String>,
    title: Option<String>,
    assistant_id: Option<String>,
    target_stage_id: Option<String>,
    target_agent: Option<String>,
    prompt: Option<String>,
    expected_output: Option<String>,
    risk: Option<String>,
    #[serde(default)]
    depends_on: Option<Vec<String>>,
    artifact_role: Option<String>,
    #[serde(default)]
    uses_artifact_roles: Vec<String>,
}

pub(super) fn parse_astra_orchestration_response_with_role_catalog(
    response: &str,
    run: &AstraRun,
    thread: &ThreadInfo,
    round_index: u32,
    completions: &[AstraTaskCompletion],
    artifact_role_catalog: &[String],
) -> Result<AstraOrchestration, AstraOrchestrationParseFailure> {
    let value = parse_yaml_mapping(response)?;
    reject_legacy_orchestration_fields(&value)?;
    let artifact_role_catalog =
        normalize_artifact_role_catalog(artifact_role_catalog).map_err(|error| {
            AstraOrchestrationParseFailure::new("validation_failed", error.to_string())
        })?;
    let raw: RawAstraOrchestration = serde_yaml::from_value(value)
        .map_err(|error| AstraOrchestrationParseFailure::new("invalid_yaml", error.to_string()))?;
    let RawAstraOrchestration {
        summary,
        run_intent,
        reason,
        mode,
        tasks: raw_tasks,
    } = raw;

    let run_intent = run_intent.ok_or_else(|| {
        AstraOrchestrationParseFailure::new("validation_failed", "orchestration missing runIntent")
    })?;
    if run_intent == AstraRunIntent::Continue && thread.kind != ThreadKind::Teamwork {
        return Err(AstraOrchestrationParseFailure::new(
            "validation_failed",
            "Astra automatic orchestration is only supported for teamwork threads",
        ));
    }
    let reason = reason
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_orchestration_reason(run_intent, completions.len()));

    let mut raw_id_to_idx: HashMap<String, usize> = HashMap::new();
    let mut ambiguous_ids: HashSet<String> = HashSet::new();
    for (idx, task) in raw_tasks.iter().enumerate() {
        if let Some(id) = task
            .id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            if raw_id_to_idx.insert(id.to_string(), idx).is_some() {
                ambiguous_ids.insert(id.to_string());
            }
        }
    }

    let mut raw_deps = Vec::with_capacity(raw_tasks.len());
    let mut tasks = Vec::new();
    let mut invalid_messages = Vec::new();
    for (idx, mut task) in raw_tasks.into_iter().enumerate() {
        raw_deps.push(task.depends_on.take());
        match sanitize_astra_task(task, run, thread, round_index, idx, &artifact_role_catalog) {
            Ok(task) => tasks.push(task),
            Err(error) => invalid_messages.push(error.message),
        }
    }
    if !invalid_messages.is_empty() {
        return Err(AstraOrchestrationParseFailure::new(
            "validation_failed",
            format!(
                "invalid Astra orchestrator task(s): {}",
                invalid_messages.join("; ")
            ),
        ));
    }
    resolve_task_dependencies(&mut tasks, &raw_deps, &raw_id_to_idx, &ambiguous_ids)?;
    validate_orchestration_contract(thread, run_intent, mode, &tasks)?;

    Ok(AstraOrchestration {
        summary: summary
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                format!(
                    "Astra orchestrator handled {} completion(s) with intent {} and selected {} task(s).",
                    completions.len(),
                    run_intent.as_str(),
                    tasks.len()
                )
            }),
        run_intent,
        reason,
        mode,
        tasks,
        diagnostics: Vec::new(),
    })
}

fn reject_legacy_orchestration_fields(
    value: &YamlValue,
) -> Result<(), AstraOrchestrationParseFailure> {
    let Some(object) = value.as_mapping() else {
        return Ok(());
    };
    for key in [
        "decisions",
        "decision",
        "action",
        "outcome",
        "issueStatus",
        "targetStageId",
        "stage",
        "issue",
        "retry",
    ] {
        let key_value = YamlValue::String(key.to_string());
        if object.contains_key(&key_value) {
            return Err(AstraOrchestrationParseFailure::new(
                "validation_failed",
                format!("legacy Astra orchestration field is not supported: {key}"),
            ));
        }
    }
    Ok(())
}

fn resolve_task_dependencies(
    tasks: &mut [AstraTaskProposal],
    raw_deps: &[Option<Vec<String>>],
    raw_id_to_idx: &HashMap<String, usize>,
    ambiguous_ids: &HashSet<String>,
) -> Result<(), AstraOrchestrationParseFailure> {
    let mut dep_indices: Vec<Vec<usize>> = vec![Vec::new(); tasks.len()];
    for (idx, deps) in raw_deps.iter().enumerate() {
        let Some(deps) = deps else { continue };
        let mut seen = HashSet::new();
        for reference in deps {
            let reference = reference.trim();
            if reference.is_empty() {
                return Err(AstraOrchestrationParseFailure::new(
                    "validation_failed",
                    "task dependsOn contains an empty id",
                ));
            }
            if ambiguous_ids.contains(reference) {
                return Err(AstraOrchestrationParseFailure::new(
                    "validation_failed",
                    format!("duplicate task id referenced by dependsOn: {reference}"),
                ));
            }
            let Some(&dep_idx) = raw_id_to_idx.get(reference) else {
                return Err(AstraOrchestrationParseFailure::new(
                    "validation_failed",
                    format!("task dependsOn references unknown task id: {reference}"),
                ));
            };
            if dep_idx == idx {
                return Err(AstraOrchestrationParseFailure::new(
                    "validation_failed",
                    format!("task must not depend on itself: {reference}"),
                ));
            }
            if seen.insert(dep_idx) {
                dep_indices[idx].push(dep_idx);
            }
        }
    }

    let mut in_degree = dep_indices.iter().map(Vec::len).collect::<Vec<_>>();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); tasks.len()];
    for (idx, deps) in dep_indices.iter().enumerate() {
        for &dep_idx in deps {
            dependents[dep_idx].push(idx);
        }
    }
    let mut queue = in_degree
        .iter()
        .enumerate()
        .filter(|(_, degree)| **degree == 0)
        .map(|(idx, _)| idx)
        .collect::<VecDeque<_>>();
    let mut visited = 0usize;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        for &dependent in &dependents[node] {
            in_degree[dependent] -= 1;
            if in_degree[dependent] == 0 {
                queue.push_back(dependent);
            }
        }
    }
    if visited != tasks.len() {
        return Err(AstraOrchestrationParseFailure::new(
            "validation_failed",
            "task dependsOn contains a cycle",
        ));
    }

    let task_ids = tasks.iter().map(|task| task.id.clone()).collect::<Vec<_>>();
    for (idx, deps) in dep_indices.into_iter().enumerate() {
        tasks[idx].depends_on = deps
            .into_iter()
            .map(|dep_idx| task_ids[dep_idx].clone())
            .collect();
    }
    Ok(())
}

fn validate_orchestration_contract(
    thread: &ThreadInfo,
    run_intent: AstraRunIntent,
    mode: Option<PlanRoundMode>,
    tasks: &[AstraTaskProposal],
) -> Result<(), AstraOrchestrationParseFailure> {
    match run_intent {
        AstraRunIntent::Continue => {
            if thread.kind != ThreadKind::Teamwork {
                return Err(AstraOrchestrationParseFailure::new(
                    "validation_failed",
                    "Astra automatic orchestration is only supported for teamwork threads",
                ));
            }
            if mode.is_none() {
                return Err(AstraOrchestrationParseFailure::new(
                    "validation_failed",
                    "continue runIntent requires mode",
                ));
            }
            if tasks.is_empty() {
                return Err(AstraOrchestrationParseFailure::new(
                    "validation_failed",
                    "continue runIntent requires at least one task",
                ));
            }
            if mode == Some(PlanRoundMode::Sequential)
                && tasks.iter().any(|task| !task.depends_on.is_empty())
            {
                return Err(AstraOrchestrationParseFailure::new(
                    "validation_failed",
                    "dependsOn is only supported with mode: parallel",
                ));
            }
        }
        AstraRunIntent::Complete | AstraRunIntent::WaitForHuman | AstraRunIntent::Error => {
            if mode.is_some() {
                return Err(AstraOrchestrationParseFailure::new(
                    "validation_failed",
                    "terminal runIntent must not include mode",
                ));
            }
            if !tasks.is_empty() {
                return Err(AstraOrchestrationParseFailure::new(
                    "validation_failed",
                    "terminal runIntent must not include tasks",
                ));
            }
        }
    }
    Ok(())
}

fn default_orchestration_reason(intent: AstraRunIntent, completion_count: usize) -> String {
    match intent {
        AstraRunIntent::Continue => "continue_with_next_plan_round",
        AstraRunIntent::Complete => "orchestration_complete",
        AstraRunIntent::WaitForHuman => "waiting_for_human",
        AstraRunIntent::Error => "orchestration_error",
    }
    .to_string()
        + &format!("_after_{}_completion(s)", completion_count)
}

fn sanitize_astra_task(
    raw: RawAstraTask,
    run: &AstraRun,
    thread: &ThreadInfo,
    round_index: u32,
    idx: usize,
    artifact_role_catalog: &[String],
) -> Result<AstraTaskProposal, AstraOrchestrationParseFailure> {
    let _raw_id = raw.id;
    let prompt = raw
        .prompt
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            AstraOrchestrationParseFailure::new("validation_failed", "task missing prompt")
        })?;
    let assistant_id = raw.assistant_id.filter(|value| !value.trim().is_empty());
    let stage_id = raw.target_stage_id.filter(|value| !value.trim().is_empty());
    if thread.kind != ThreadKind::Teamwork {
        return Err(AstraOrchestrationParseFailure::new(
            "validation_failed",
            "Astra automatic orchestration tasks are only supported for teamwork threads",
        ));
    }
    if stage_id.is_some() {
        return Err(AstraOrchestrationParseFailure::new(
            "validation_failed",
            "teamwork task must not include targetStageId",
        ));
    }
    let assistant_id = assistant_id.ok_or_else(|| {
        AstraOrchestrationParseFailure::new(
            "validation_failed",
            "teamwork task missing assistantId",
        )
    })?;
    let assistant = thread
        .assistants
        .iter()
        .find(|assistant| assistant.assistant_id == assistant_id)
        .ok_or_else(|| {
            AstraOrchestrationParseFailure::new("validation_failed", "unknown assistantId")
        })?;
    let assistant_agent = Agent::from_db_str(&assistant.agent.id).ok_or_else(|| {
        AstraOrchestrationParseFailure::new(
            "validation_failed",
            "assistant has no valid runtime agent",
        )
    })?;
    let target_agent = raw
        .target_agent
        .as_deref()
        .and_then(Agent::from_db_str)
        .unwrap_or(assistant_agent);
    if target_agent != assistant_agent {
        return Err(AstraOrchestrationParseFailure::new(
            "validation_failed",
            "task targetAgent does not match assistantId",
        ));
    }
    let title = raw
        .title
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("{} teamwork task", assistant.name));
    let artifact_role = raw
        .artifact_role
        .as_deref()
        .map(|role| {
            resolve_artifact_role(role, artifact_role_catalog).map_err(|error| {
                AstraOrchestrationParseFailure::new("validation_failed", error.to_string())
            })
        })
        .transpose()?;
    let uses_artifact_roles =
        resolve_artifact_roles(&raw.uses_artifact_roles, artifact_role_catalog).map_err(
            |error| AstraOrchestrationParseFailure::new("validation_failed", error.to_string()),
        )?;
    let id = format!(
        "task-{}",
        short_hash(&format!(
            "{}:{}:{}:{}:{}",
            run.run_id, run.thread_id, assistant_id, round_index, idx
        ))
    );
    Ok(AstraTaskProposal {
        id,
        plan_task_id: None,
        assistant_id: Some(assistant_id),
        agent_participant_id: None,
        title,
        target_stage_id: None,
        target_agent,
        prompt,
        expected_output: raw
            .expected_output
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                "Teamwork task result, concrete progress, decisions, and verification notes."
                    .to_string()
            }),
        risk: parse_task_risk(raw.risk.as_deref()),
        depends_on: Vec::new(),
        artifact_role,
        uses_artifact_roles,
    })
}

fn parse_task_risk(value: Option<&str>) -> AstraTaskRisk {
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "high" => AstraTaskRisk::High,
        "medium" => AstraTaskRisk::Medium,
        _ => AstraTaskRisk::Low,
    }
}

fn parse_yaml_mapping(response: &str) -> Result<YamlValue, AstraOrchestrationParseFailure> {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return Err(AstraOrchestrationParseFailure::new(
            "empty_response",
            "Astra orchestrator returned an empty response",
        ));
    }
    if trimmed.contains("```") {
        return Err(AstraOrchestrationParseFailure::new(
            "invalid_yaml",
            "Astra orchestration response must be a plain YAML mapping, not a markdown code fence",
        ));
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Err(AstraOrchestrationParseFailure::new(
            "invalid_yaml",
            "JSON Astra orchestration responses are not supported; return a YAML mapping",
        ));
    }
    let value: YamlValue = serde_yaml::from_str(trimmed)
        .map_err(|error| AstraOrchestrationParseFailure::new("invalid_yaml", error.to_string()))?;
    if !value.is_mapping() {
        return Err(AstraOrchestrationParseFailure::new(
            "invalid_yaml",
            "Astra orchestration response YAML must be a mapping",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astra::AstraRunStatus;
    use crate::models::{AssistantAgentInfo, ThreadAssistantInfo, ThreadOrigin};

    fn run() -> AstraRun {
        AstraRun {
            run_id: "run-1".to_string(),
            thread_id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            project_path: "/tmp/project".to_string(),
            status: AstraRunStatus::Planning,
            mode: "auto".to_string(),
            planner_backend: None,
            round_index: None,
            round_limit: 3,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_session_ids: Vec::new(),
            run_diagnostics: Vec::new(),
            error: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn teamwork_thread() -> ThreadInfo {
        ThreadInfo {
            id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            goal: "Build the outline".to_string(),
            description: None,
            stage_id: None,
            kind: ThreadKind::Teamwork,
            enabled: true,
            origin: ThreadOrigin::Manual,
            scheduled_task_id: None,
            artifact_role_catalog: Vec::new(),
            created_at: 1,
            updated_at: 1,
            assistants: vec![ThreadAssistantInfo {
                assistant_id: "assistant-codex".to_string(),
                name: "Builder".to_string(),
                color: None,
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: String::new(),
                    mode: String::new(),
                    effort: String::new(),
                },
                system_prompt: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                order: 0,
            }],
            agent_participants: Vec::new(),
            stages: Vec::new(),
            sessions: Vec::new(),
        }
    }

    #[test]
    fn parser_normalizes_artifact_roles() {
        let parsed = parse_astra_orchestration_response_with_role_catalog(
            r#"
summary: continue work
runIntent: continue
reason: continue
mode: parallel
tasks:
  - id: t1
    title: Update brief
    assistantId: assistant-codex
    targetAgent: codex
    prompt: Update the brief.
    expectedOutput: Updated brief.
    risk: low
    artifactRole: Research-Brief
    usesArtifactRoles: [outline, research brief, outline]
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
            &[],
        )
        .unwrap();

        assert_eq!(
            parsed.tasks[0].artifact_role.as_deref(),
            Some("research_brief")
        );
        assert_eq!(
            parsed.tasks[0].uses_artifact_roles,
            vec!["outline", "research_brief"]
        );
    }

    #[test]
    fn parser_rejects_undeclared_artifact_roles() {
        let error = parse_astra_orchestration_response_with_role_catalog(
            r#"
summary: continue work
runIntent: continue
reason: continue
mode: parallel
tasks:
  - id: t1
    title: Update typo
    assistantId: assistant-codex
    targetAgent: codex
    prompt: Update the typo role.
    expectedOutput: Updated typo role.
    risk: low
    artifactRole: outlien
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("undeclared artifact role"));
    }

    #[test]
    fn parser_accepts_catalog_declared_custom_artifact_roles() {
        let parsed = parse_astra_orchestration_response_with_role_catalog(
            r#"
summary: continue work
runIntent: continue
reason: continue
mode: parallel
tasks:
  - id: t1
    title: Update custom artifact
    assistantId: assistant-codex
    targetAgent: codex
    prompt: Update the custom artifact.
    expectedOutput: Updated custom artifact.
    risk: low
    artifactRole: character sheet
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
            &["character_sheet".to_string()],
        )
        .unwrap();

        assert_eq!(
            parsed.tasks[0].artifact_role.as_deref(),
            Some("character_sheet")
        );
    }
}
