use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashSet;

use crate::agents::runtime::types::RuntimeTransportKind;
use crate::models::{
    Agent, AgentAiProviderInfo, AgentCommandsInfo, AgentInfo, AgentType, AssistantAgentInfo,
    RuntimeAgentOptionMetadata, StageType,
};
use crate::store::now_ms;

use super::{
    load_agent_by_id, runtime_agent_display_name, runtime_agent_name, runtime_agent_order,
    runtime_option, runtime_options, runtime_options_json, selected_ai_provider_id,
    stable_process_template_builtin_assistant_id, stage_has_assistants, transport_kind_to_db,
};

/// Seed all builtin data in dependency order: process templates, their stages,
/// runtime agents and assistants, then the process-template stage assistant
/// bindings. Idempotent -- every insert uses INSERT OR IGNORE / ON CONFLICT
/// DO NOTHING, so re-running never clobbers user edits.
pub(crate) fn seed_builtins(conn: &Connection) -> Result<()> {
    let now = now_ms();
    seed_builtin_process_templates(conn, now)?;
    seed_builtin_process_template_stages(conn, now)?;
    seed_builtin_agents(conn, now)?;
    seed_astra_config(conn, now)?;
    seed_builtin_process_template_stage_assistants(conn, now)?;
    Ok(())
}

fn seed_builtin_process_templates(conn: &Connection, now: i64) -> Result<()> {
    for (id, name, description) in BUILTIN_PROCESS_TEMPLATE_SEEDS {
        conn.execute(
            "INSERT OR IGNORE INTO process_templates (id, name, description, type, created_at, updated_at)
             VALUES (?, ?, ?, 'builtin', ?, ?)",
            params![id, name, description, now, now],
        )?;
    }
    Ok(())
}

fn seed_builtin_process_template_stages(conn: &Connection, now: i64) -> Result<()> {
    for (process_template_id, _, _) in BUILTIN_PROCESS_TEMPLATE_SEEDS {
        for (index, (kind, description)) in
            builtin_process_template_stage_seeds(process_template_id)
                .iter()
                .copied()
                .enumerate()
        {
            let id = format!("stage-builtin-{}-{}", process_template_id, kind.as_str());
            let allow_empty_assistants = matches!(kind, StageType::Human | StageType::Done);
            conn.execute(
                "INSERT OR IGNORE INTO stages (id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at)
                 VALUES (?, NULL, 'builtin', ?, ?, NULL, ?, NULL, ?, 1, ?, ?, ?)",
                params![
                    id,
                    process_template_id,
                    kind.as_str(),
                    description,
                    (index as i64 + 1) * 1000,
                    allow_empty_assistants as i64,
                    now,
                    now
                ],
            )?;
        }
    }
    Ok(())
}

fn seed_builtin_process_template_stage_assistants(conn: &Connection, now: i64) -> Result<()> {
    let existing_bindings: i64 =
        conn.query_row("SELECT count(*) FROM stage_assistants", [], |row| {
            row.get(0)
        })?;
    if existing_bindings > 0 {
        return Ok(());
    }
    for (process_template_id, _, _) in BUILTIN_PROCESS_TEMPLATE_SEEDS {
        for (kind, _) in builtin_process_template_stage_seeds(process_template_id) {
            if matches!(kind, StageType::Human | StageType::Done) {
                continue;
            }
            let stage_id = format!("stage-builtin-{}-{}", process_template_id, kind.as_str());
            if stage_has_assistants(conn, &stage_id)? {
                continue;
            }
            let assistant_seed = builtin_assistant_seed_for_kind(kind);
            let assistant_id = stable_process_template_builtin_assistant_id(
                process_template_id,
                assistant_seed.id,
            );
            seed_process_template_builtin_assistant(
                conn,
                process_template_id,
                assistant_seed.id,
                now,
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO stage_assistants (stage_id, assistant_id, sort_order, created_at, updated_at)
                 VALUES (?, ?, 0, ?, ?)",
                params![stage_id, assistant_id, now, now],
            )?;
        }
    }
    Ok(())
}

const BUILTIN_PROCESS_TEMPLATE_SEEDS: [(&str, &str, &str); 5] = [
    ("code", "Code", "process_template.description.code"),
    ("writing", "Writing", "process_template.description.writing"),
    (
        "research",
        "Research",
        "process_template.description.research",
    ),
    ("general", "General", "process_template.description.general"),
    (
        "video_production",
        "Video production",
        "process_template.description.video_production",
    ),
];

fn builtin_process_template_stage_seeds(
    process_template_id: &str,
) -> Vec<(StageType, &'static str)> {
    match process_template_id {
        "code" => vec![
            (
                StageType::Research,
                "Gather technical context, codebase constraints, dependencies, and open questions before implementation.",
            ),
            (
                StageType::Plan,
                "Turn the engineering goal into a concrete implementation plan with scope, sequencing, and validation.",
            ),
            (
                StageType::Develop,
                "Implement the planned code changes and keep the thread moving toward a working result.",
            ),
            (
                StageType::Review,
                "Inspect the code for correctness, regressions, edge cases, and missing validation.",
            ),
            (
                StageType::Human,
                "Pause for human input, approval, product judgment, or external information.",
            ),
            (
                StageType::Done,
                "Close the thread after the goal has been completed and verified.",
            ),
        ],
        "writing" => vec![
            (
                StageType::Research,
                "Gather references, audience context, constraints, and source material before drafting.",
            ),
            (
                StageType::Plan,
                "Shape the writing brief into structure, angle, outline, and acceptance criteria.",
            ),
            (
                StageType::Writing,
                "Draft the content in the selected voice, structure, and level of detail.",
            ),
            (
                StageType::Editing,
                "Revise the draft for clarity, flow, accuracy, and fit to the intended audience.",
            ),
            (
                StageType::Proofreading,
                "Check grammar, spelling, formatting, terminology, and final polish before delivery.",
            ),
            (
                StageType::Human,
                "Pause for human input, approval, editorial judgment, or external information.",
            ),
            (
                StageType::Done,
                "Close the thread after the writing goal has been completed and verified.",
            ),
        ],
        "video_production" => vec![
            (
                StageType::Research,
                "Gather references, audience context, production constraints, and creative direction.",
            ),
            (
                StageType::Plan,
                "Turn the video goal into production scope, sequencing, responsibilities, and success criteria.",
            ),
            (
                StageType::Screenplay,
                "Write or refine the script, scenes, narration, dialogue, and beats.",
            ),
            (
                StageType::Storyboard,
                "Map scenes into shots, visual flow, timing, framing, and transitions.",
            ),
            (
                StageType::Design,
                "Define visual style, assets, graphics, motion language, and production look.",
            ),
            (
                StageType::Production,
                "Produce the video assets and assemble the planned shots into the working result.",
            ),
            (
                StageType::Review,
                "Review the cut for story, pacing, accuracy, visual quality, and delivery requirements.",
            ),
            (
                StageType::Human,
                "Pause for human input, approval, creative judgment, or external information.",
            ),
            (
                StageType::Done,
                "Close the thread after the video production goal has been completed and verified.",
            ),
        ],
        _ => vec![
            (
                StageType::Research,
                "Gather context, constraints, references, and open questions before committing to an approach.",
            ),
            (
                StageType::Plan,
                "Turn the goal into a concrete execution plan with scope, sequencing, and success criteria.",
            ),
            (
                StageType::Build,
                "Implement the planned work and keep the thread moving toward a working result.",
            ),
            (
                StageType::Review,
                "Inspect the result for correctness, regressions, edge cases, and missing validation.",
            ),
            (
                StageType::Human,
                "Pause for human input, approval, product judgment, or external information.",
            ),
            (
                StageType::Done,
                "Close the thread after the goal has been completed and verified.",
            ),
        ],
    }
}

struct BuiltinAgentSeed {
    model: Option<&'static str>,
    models: Vec<RuntimeAgentOptionMetadata>,
    effort: Option<&'static str>,
    efforts: Vec<RuntimeAgentOptionMetadata>,
    permission_mode: Option<&'static str>,
    permission_modes: Vec<RuntimeAgentOptionMetadata>,
    enabled: bool,
    transport: RuntimeTransportKind,
    commands: AgentCommandsInfo,
    ai_providers: Vec<AgentAiProviderInfo>,
}

fn seed_builtin_agent(
    conn: &Connection,
    agent: Agent,
    seed: BuiltinAgentSeed,
    now: i64,
) -> Result<()> {
    let BuiltinAgentSeed {
        model,
        models,
        effort,
        efforts,
        permission_mode,
        permission_modes,
        enabled,
        transport,
        commands,
        ai_providers,
    } = seed;
    let id = agent.as_str();
    let ai_provider = selected_ai_provider_id(&ai_providers, None);
    let ai_providers_json = serde_json::to_string(&ai_providers)?;
    conn.execute(
        "INSERT OR IGNORE INTO agents (
            id, name, display_name, icon, ai_provider, ai_providers_json, ai_api, api_base_url, api_key,
            model, models_json, effort, efforts_json,
            permission_mode, permission_modes_json, type, enabled, transport,
            commands_json, sort_order, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            id,
            runtime_agent_name(agent),
            runtime_agent_display_name(agent),
            id,
            ai_provider.as_deref(),
            ai_providers_json,
            Option::<&str>::None,
            Option::<&str>::None,
            Option::<&str>::None,
            model,
            runtime_options_json(&models)?,
            effort,
            serde_json::to_string(&efforts)?,
            permission_mode,
            serde_json::to_string(&permission_modes)?,
            AgentType::Builtin.as_str(),
            enabled as i64,
            transport_kind_to_db(transport),
            serde_json::to_string(&commands)?,
            runtime_agent_order(agent),
            now,
            now,
        ],
    )?;
    Ok(())
}

fn seed_astra_config(conn: &Connection, now: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO astra_config (
            id, agent, model, effort, permission_mode, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![
            1,
            Some(Agent::Codex.as_str()),
            Option::<&str>::None, // model
            Option::<&str>::None, // effort
            Option::<&str>::None, // permission_mode
            now,
            now,
        ],
    )?;
    Ok(())
}

fn seed_builtin_agents(conn: &Connection, now: i64) -> Result<()> {
    seed_builtin_agent(
        conn,
        Agent::Codex,
        BuiltinAgentSeed {
            model: Some("gpt-5.5"),
            models: runtime_options(vec![
                runtime_option("gpt-5.5", "5.5"),
                runtime_option("gpt-5.4", "5.4"),
                runtime_option("gpt-5.3-codex", "5.3 Codex"),
            ]),
            effort: Some("high"),
            efforts: vec![
                runtime_option("low", "Low"),
                runtime_option("medium", "Medium"),
                runtime_option("high", "High"),
                runtime_option("xhigh", "Extra High"),
            ],
            permission_mode: Some("read-only"),
            permission_modes: vec![
                runtime_option("read-only", "Ask for approval"),
                runtime_option("agent", "Approve for me"),
                runtime_option("agent-full-access", "Full access"),
            ],
            enabled: true,
            transport: RuntimeTransportKind::Acp,
            commands: AgentCommandsInfo {
                session: vec!["npx -y @agentclientprotocol/codex-acp@latest".to_string()],
                version: vec!["npm view @agentclientprotocol/codex-acp version".to_string()],
            },
            ai_providers: vec![],
        },
        now,
    )?;
    seed_builtin_agent(
        conn,
        Agent::Pi,
        BuiltinAgentSeed {
            model: None,
            models: Vec::new(),
            effort: Some("medium"),
            efforts: vec![
                runtime_option("off", "Off"),
                runtime_option("minimal", "Minimal"),
                runtime_option("low", "Low"),
                runtime_option("medium", "Medium"),
                runtime_option("high", "High"),
                runtime_option("xhigh", "Extra High"),
            ],
            permission_mode: None,
            permission_modes: Vec::new(),
            enabled: false,
            transport: RuntimeTransportKind::PiRpc,
            commands: AgentCommandsInfo {
                session: vec!["pi --mode rpc".to_string()],
                version: vec!["pi --version".to_string()],
            },
            ai_providers: vec![],
        },
        now,
    )?;
    sync_pi_builtin_agent_defaults(conn, now)?;
    seed_builtin_agent(
        conn,
        Agent::Claude,
        BuiltinAgentSeed {
            model: Some("claude-opus-4-8"),
            models: runtime_options(vec![
                runtime_option("claude-opus-4-8", "Opus 4.8"),
                runtime_option("claude-opus-4-7", "Opus 4.7"),
                runtime_option("claude-opus-4-6", "Opus 4.6"),
            ]),
            effort: Some("high"),
            efforts: vec![
                runtime_option("low", "Low"),
                runtime_option("medium", "Medium"),
                runtime_option("high", "High"),
                runtime_option("xhigh", "Extra High"),
                runtime_option("max", "Max"),
            ],
            permission_mode: Some("default"),
            permission_modes: vec![
                runtime_option("default", "Ask before edits"),
                runtime_option("acceptEdits", "Edit automatically"),
                runtime_option("plan", "Plan mode"),
                runtime_option("dontAsk", "Don't Ask"),
            ],
            enabled: true,
            transport: RuntimeTransportKind::Acp,
            commands: AgentCommandsInfo {
                session: vec!["npx -y @agentclientprotocol/claude-agent-acp@latest".to_string()],
                version: vec!["npm view @agentclientprotocol/claude-agent-acp version".to_string()],
            },
            ai_providers: vec![],
        },
        now,
    )?;
    seed_opencode_builtin_agent(conn, now)?;
    seed_builtin_assistants(conn, now)?;
    Ok(())
}

/// Seed (`INSERT OR IGNORE`) on every init so the builtin OpenCode row is
/// always present without clobbering user edits.
pub(crate) fn seed_opencode_builtin_agent(conn: &Connection, now: i64) -> Result<()> {
    seed_builtin_agent(
        conn,
        Agent::Opencode,
        BuiltinAgentSeed {
            model: None,
            models: Vec::new(),
            effort: Some("high"),
            efforts: vec![
                runtime_option("low", "Low"),
                runtime_option("medium", "Medium"),
                runtime_option("high", "High"),
            ],
            permission_mode: None,
            permission_modes: Vec::new(),
            enabled: false,
            transport: RuntimeTransportKind::Acp,
            commands: AgentCommandsInfo {
                session: vec!["opencode acp".to_string()],
                version: vec!["opencode --version".to_string()],
            },
            ai_providers: vec![],
        },
        now,
    )
}

fn sync_pi_builtin_agent_defaults(conn: &Connection, now: i64) -> Result<()> {
    let commands_json = serde_json::to_string(&AgentCommandsInfo {
        session: vec!["pi --mode rpc".to_string()],
        version: vec!["pi --version".to_string()],
    })?;
    conn.execute(
        "UPDATE agents
         SET transport = ?, commands_json = ?, updated_at = ?
         WHERE id = ? AND (transport <> ? OR commands_json <> ?)",
        params![
            transport_kind_to_db(RuntimeTransportKind::PiRpc),
            commands_json,
            now,
            Agent::Pi.as_str(),
            transport_kind_to_db(RuntimeTransportKind::PiRpc),
            commands_json,
        ],
    )?;
    Ok(())
}

fn assistant_agent_from_db_agent(agent: &AgentInfo) -> Option<AssistantAgentInfo> {
    let model = agent
        .model
        .clone()
        .or_else(|| agent.models.first().map(|option| option.value.clone()))?;
    let mode = agent.permission_mode.clone().or_else(|| {
        agent
            .permission_modes
            .first()
            .map(|option| option.value.clone())
    })?;
    let effort = agent
        .effort
        .clone()
        .or_else(|| agent.efforts.first().map(|option| option.value.clone()))
        .unwrap_or_default();
    Some(AssistantAgentInfo {
        id: agent.id.clone(),
        name: agent.name.clone(),
        model,
        mode,
        effort,
    })
}

fn seed_builtin_assistants(conn: &Connection, now: i64) -> Result<()> {
    let codex_agent = load_agent_by_id(conn, Agent::Codex.as_str())?;
    let Some(assistant_agent) = assistant_agent_from_db_agent(&codex_agent) else {
        return Ok(());
    };
    for seed in builtin_assistant_seeds() {
        upsert_builtin_assistant(conn, seed, &assistant_agent, now)?;
    }
    Ok(())
}

fn seed_process_template_builtin_assistant(
    conn: &Connection,
    process_template_id: &str,
    source_assistant_id: &str,
    now: i64,
) -> Result<()> {
    let process_template_assistant_id =
        stable_process_template_builtin_assistant_id(process_template_id, source_assistant_id);
    conn.execute(
        "INSERT INTO assistants (
            id, name, agent_json, system_prompt, color, selected_skill_ids_json, selected_mcp_ids_json, type, process_template_id, project_id, enabled, created_at, updated_at
         )
         SELECT ?, name, agent_json, system_prompt, color, selected_skill_ids_json, selected_mcp_ids_json, type, ?, NULL, enabled, ?, ?
         FROM assistants
         WHERE id = ?
         ON CONFLICT(id) DO NOTHING",
        params![
            process_template_assistant_id,
            process_template_id,
            now,
            now,
            source_assistant_id
        ],
    )?;
    Ok(())
}

struct BuiltinAssistantSeed {
    id: &'static str,
    name: &'static str,
    color: &'static str,
    system_prompt: &'static str,
}

fn builtin_assistant_seeds() -> Vec<BuiltinAssistantSeed> {
    builtin_assistant_kinds()
        .into_iter()
        .map(builtin_assistant_seed_for_kind)
        .collect()
}

fn builtin_assistant_kinds() -> Vec<StageType> {
    let mut seen = HashSet::new();
    BUILTIN_PROCESS_TEMPLATE_SEEDS
        .iter()
        .flat_map(|(process_template_id, _, _)| {
            builtin_process_template_stage_seeds(process_template_id)
        })
        .map(|(kind, _)| kind)
        .filter(|kind| !matches!(kind, StageType::Human | StageType::Done))
        .filter(|kind| seen.insert(*kind))
        .collect()
}

fn builtin_assistant_seed_for_kind(kind: StageType) -> BuiltinAssistantSeed {
    match kind {
        StageType::Research => BuiltinAssistantSeed {
            id: "assistant-builtin-research",
            name: "Researcher",
            color: "#0ea5e9",
            system_prompt: "Research the problem space before implementation. Gather relevant context, inspect existing project behavior, identify constraints and unknowns, and report concise findings with sources or file references when available.",
        },
        StageType::Plan => BuiltinAssistantSeed {
            id: "assistant-builtin-plan",
            name: "Planner",
            color: "#8b5cf6",
            system_prompt: "Create a clear execution plan from the thread goal. Break the work into ordered steps, call out dependencies and risks, and keep the plan focused on decisions that unblock implementation.",
        },
        StageType::Develop => BuiltinAssistantSeed {
            id: "assistant-builtin-develop",
            name: "Developer",
            color: "#22c55e",
            system_prompt: "Implement the planned code changes. Follow existing project patterns, keep behavior coherent across the stack, and verify the result with relevant checks.",
        },
        StageType::Build => BuiltinAssistantSeed {
            id: "assistant-builtin-build",
            name: "Builder",
            color: "#f59e0b",
            system_prompt: "Implement the planned work and keep the thread moving toward a working result. Make scoped changes and verify the result with the most relevant checks.",
        },
        StageType::Writing => BuiltinAssistantSeed {
            id: "assistant-builtin-writing",
            name: "Writer",
            color: "#ec4899",
            system_prompt: "Draft the requested content in the selected voice, structure, and level of detail while preserving the goal, audience, and constraints. Use the available file-editing tools to write the draft into the target file before finishing; do not only return the text in chat. If no target file is specified, inspect the project or thread context to find the appropriate document path, or create a clearly named draft file and report its path.",
        },
        StageType::Editing => BuiltinAssistantSeed {
            id: "assistant-builtin-editing",
            name: "Editor",
            color: "#f97316",
            system_prompt: "Revise the draft for clarity, flow, accuracy, structure, and fit to the intended audience while preserving the authorial intent.",
        },
        StageType::Review => BuiltinAssistantSeed {
            id: "assistant-builtin-review",
            name: "Reviewer",
            color: "#ef4444",
            system_prompt: "Review the completed work for correctness, regressions, data model consistency, edge cases, and missing tests. Prioritize actionable findings and confirm when no blocking issues remain.",
        },
        StageType::Proofreading => BuiltinAssistantSeed {
            id: "assistant-builtin-proofreading",
            name: "Proofreader",
            color: "#14b8a6",
            system_prompt: "Check grammar, spelling, formatting, terminology, consistency, and final polish before delivery.",
        },
        StageType::Screenplay => BuiltinAssistantSeed {
            id: "assistant-builtin-screenplay",
            name: "Screenwriter",
            color: "#6366f1",
            system_prompt: "Write or refine scripts, scenes, narration, dialogue, and beats for the video production goal.",
        },
        StageType::Storyboard => BuiltinAssistantSeed {
            id: "assistant-builtin-storyboard",
            name: "Storyboarder",
            color: "#a855f7",
            system_prompt: "Map scenes into shots, visual flow, timing, framing, transitions, and production notes.",
        },
        StageType::Design => BuiltinAssistantSeed {
            id: "assistant-builtin-design",
            name: "Designer",
            color: "#06b6d4",
            system_prompt: "Define visual style, assets, graphics, motion language, and production look for the project.",
        },
        StageType::Production => BuiltinAssistantSeed {
            id: "assistant-builtin-production",
            name: "Producer",
            color: "#84cc16",
            system_prompt: "Produce and assemble the planned assets or shots into a working result that matches the production plan.",
        },
        StageType::Human | StageType::Done => BuiltinAssistantSeed {
            id: "assistant-builtin-done",
            name: "Done",
            color: "#64748b",
            system_prompt: "Close the completed thread.",
        },
    }
}

fn upsert_builtin_assistant(
    conn: &Connection,
    seed: BuiltinAssistantSeed,
    assistant_agent: &AssistantAgentInfo,
    now: i64,
) -> Result<()> {
    let agent_json = serde_json::to_string(&assistant_agent)?;
    conn.execute(
        "INSERT INTO assistants (
            id, name, agent_json, system_prompt, color, type, process_template_id, project_id, enabled, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, 'builtin', NULL, NULL, 1, ?, ?)
         ON CONFLICT(id) DO NOTHING",
        params![
            seed.id,
            seed.name,
            agent_json,
            seed.system_prompt,
            seed.color,
            now,
            now
        ],
    )?;
    Ok(())
}
