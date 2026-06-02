use crate::agents::sources::shared::convert::project_key_for_path_or_name;
use crate::config;
use crate::memory::build::MemoryBuildOptions;
use crate::memory::qmd;
use crate::memory::records::safe_id_part;
use crate::memory::service::MemoryService;
use crate::memory::{MemoryRecord, MemorySearchOptions, MemoryStore, RecordContinuation};
use crate::models::{Agent, SessionHistoryBlock, SessionHistoryTurn, StageStatus};
use crate::store::sqlite::SqliteStore;
use crate::store::SessionStore;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug)]
struct Cli {
    command: Command,
}

#[derive(Debug)]
enum Command {
    Sessions(SessionsCommand),
    Memory(MemoryCommand),
    Config(ConfigCommand),
    Thread(ThreadCommand),
    Stage(StageCommand),
    Help,
}

#[derive(Debug)]
enum ThreadCommand {
    List {
        project: Option<String>,
        db_path: Option<String>,
        json: bool,
    },
    Show {
        id: String,
        db_path: Option<String>,
        json: bool,
    },
}

#[derive(Debug)]
enum StageCommand {
    List {
        thread_id: String,
        db_path: Option<String>,
        json: bool,
    },
    Show {
        id: String,
        db_path: Option<String>,
        json: bool,
    },
    SetStatus {
        id: String,
        status: String,
        summary: Option<String>,
        outcome: Option<String>,
        db_path: Option<String>,
        json: bool,
    },
}

#[derive(Debug)]
enum ConfigCommand {
    Show {
        json: bool,
    },
    MemorySet {
        binary: Option<String>,
        index: Option<String>,
        artifacts_root: Option<String>,
        auto_embed: Option<bool>,
        install_command: Option<String>,
        json: bool,
    },
}

#[derive(Debug)]
enum MemoryCommand {
    Status {
        binary: Option<String>,
        json: bool,
    },
    Sync {
        project_key: String,
        artifacts_root: Option<String>,
        binary: Option<String>,
        index: String,
        embed: bool,
        json: bool,
    },
    Build {
        project: String,
        artifacts_root: Option<String>,
        db_path: Option<String>,
        json: bool,
    },
    Resolve {
        record_id: String,
        db_path: Option<String>,
        include_source_excerpt: bool,
        json: bool,
    },
    CoveredBy {
        record_id: String,
        db_path: Option<String>,
        json: bool,
    },
    Base {
        record_id: String,
        db_path: Option<String>,
        json: bool,
    },
    Search {
        project_key: Option<String>,
        project: Option<String>,
        query: String,
        db_path: Option<String>,
        include_raw: bool,
        json: bool,
    },
    Jobs {
        project_key: String,
        status: Option<String>,
        db_path: Option<String>,
        json: bool,
    },
}

#[derive(Debug)]
enum SessionsCommand {
    List {
        project: Option<String>,
        db_path: Option<String>,
        json: bool,
    },
    Messages {
        agent: Agent,
        session_id: Option<String>,
        file_path: Option<String>,
        json: bool,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CliError {
    error: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemorySearchHit {
    record_id: String,
    title: String,
    summary: Option<String>,
    score: Option<f64>,
    snippet: Option<String>,
    sources: Vec<crate::memory::MemorySource>,
    continuation: Option<MemoryContinuationSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryBaseHit {
    record_id: String,
    record: Option<MemoryRecord>,
    continuation: MemoryContinuationSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryCoveredByResult {
    record_id: String,
    record: MemoryRecord,
    base_record_id: Option<String>,
    base_record: Option<MemoryRecord>,
    continuation: Option<MemoryContinuationSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryContinuationSummary {
    covered_by: String,
    base_file_path: String,
    base_turn_range: String,
    base_line_range: Option<String>,
    base_byte_range: Option<String>,
    candidate_trim_start: String,
    candidate_file_path: String,
}

pub fn run_from_env() {
    if let Err(err) = run() {
        let payload = CliError {
            error: err.to_string(),
        };
        match serde_json::to_string_pretty(&payload) {
            Ok(json) => eprintln!("{json}"),
            Err(_) => eprintln!("error: {err}"),
        }
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = parse_args(env::args().skip(1).collect())?;
    match cli.command {
        Command::Sessions(cmd) => run_sessions(cmd),
        Command::Memory(cmd) => run_memory(cmd),
        Command::Config(cmd) => run_config(cmd),
        Command::Thread(cmd) => run_thread(cmd),
        Command::Stage(cmd) => run_stage(cmd),
        Command::Help => {
            print_help();
            Ok(())
        }
    }
}

fn run_config(cmd: ConfigCommand) -> Result<()> {
    match cmd {
        ConfigCommand::Show { json } => {
            let config = config::load_config()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&config)?);
            } else {
                println!("{}", serialize_app_config(&config));
            }
            Ok(())
        }
        ConfigCommand::MemorySet {
            binary,
            index,
            artifacts_root,
            auto_embed,
            install_command,
            json,
        } => {
            let mut memory = config::load_memory_config()?;
            if let Some(binary) = binary {
                memory.qmd.binary = Some(binary);
            }
            if let Some(index) = index {
                memory.qmd.index = index;
            }
            if let Some(artifacts_root) = artifacts_root {
                memory.qmd.artifacts_root = config::expand_path(&artifacts_root)?;
            }
            if let Some(auto_embed) = auto_embed {
                memory.qmd.auto_embed = auto_embed;
            }
            if let Some(install_command) = install_command {
                memory.qmd.install_command = install_command;
            }
            config::save_memory_config(&memory)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&memory)?);
            } else {
                println!("saved memory config to ~/.sessio/config.toml");
            }
            Ok(())
        }
    }
}

fn run_memory(cmd: MemoryCommand) -> Result<()> {
    match cmd {
        MemoryCommand::Status { binary, json } => {
            // Honor an explicit CLI --binary override by transiently setting
            // the env var the config layer reads. Keeps `memory status` going
            // through MemoryService without re-introducing a qmd-specific
            // status path in the CLI.
            if let Some(binary) = binary.as_deref() {
                std::env::set_var("SESSIO_QMD_BINARY", binary);
            }
            let service = build_cli_service(None)?;
            let status = service.backend_status();
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else if status.available {
                let version = status
                    .details
                    .as_ref()
                    .and_then(|d| d.get("version"))
                    .and_then(|v| v.as_str())
                    .map(|v| format!(" ({v})"))
                    .unwrap_or_default();
                println!("memory backend available: {}{version}", status.backend);
            } else {
                println!(
                    "memory backend unavailable: {}",
                    status.error.as_deref().unwrap_or("unknown error")
                );
            }
            Ok(())
        }
        MemoryCommand::Sync {
            project_key,
            artifacts_root,
            binary,
            index,
            embed,
            json,
        } => {
            if let Some(binary) = binary.as_deref() {
                std::env::set_var("SESSIO_QMD_BINARY", binary);
            }
            if !index.is_empty() && index != "sessio" {
                std::env::set_var("SESSIO_QMD_INDEX", &index);
            }
            if let Some(root) = artifacts_root.as_deref() {
                std::env::set_var("SESSIO_QMD_ARTIFACTS_ROOT", root);
            }
            let service = build_cli_service(None)?;
            let sync = service.sync_project(&project_key)?;
            let embed_result = if embed {
                Some(qmd::embed_index(&qmd::QmdOptions {
                    binary: std::env::var("SESSIO_QMD_BINARY").ok(),
                    index: std::env::var("SESSIO_QMD_INDEX")
                        .ok()
                        .filter(|v| !v.is_empty())
                        .unwrap_or_else(|| "sessio".to_string()),
                    install_command: std::env::var("SESSIO_QMD_INSTALL_COMMAND")
                        .ok()
                        .filter(|v| !v.is_empty())
                        .unwrap_or_else(|| "npm install -g @tobilu/qmd".to_string()),
                })?)
            } else {
                None
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "projectKey": project_key,
                        "artifactsRoot": service.backend_artifacts_root(),
                        "backend": sync.backend,
                        "sync": sync,
                        "embed": embed_result,
                    }))?
                );
            } else {
                println!(
                    "synced memory backend {} for project {}",
                    sync.backend, project_key
                );
            }
            Ok(())
        }
        MemoryCommand::Build {
            project,
            artifacts_root,
            db_path,
            json,
        } => {
            if let Some(root) = artifacts_root.as_deref() {
                std::env::set_var("SESSIO_QMD_ARTIFACTS_ROOT", root);
            }
            let service = build_cli_service(db_path.as_deref())?;
            let artifacts_root = service.backend_artifacts_root();
            let (summary, sync_result) = service.build_project_and_sync(MemoryBuildOptions {
                project_path: PathBuf::from(project),
                artifacts_root,
            })?;
            let (backend_sync, backend_error) = match sync_result {
                Some(Ok(sync)) => (Some(sync), None),
                Some(Err(e)) => (None, Some(e.to_string())),
                None => (None, None),
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "summary": summary,
                        "backend": service.backend_name(),
                        "backendSync": backend_sync,
                        "backendError": backend_error,
                    }))?
                );
            } else {
                println!(
                    "built {} memory records from {} sources",
                    summary.records_written, summary.sources_built
                );
                if let Some(error) = backend_error {
                    println!("memory backend sync unavailable: {error}");
                }
            }
            Ok(())
        }
        MemoryCommand::Resolve {
            record_id,
            db_path,
            include_source_excerpt,
            json,
        } => {
            let service = build_cli_service(db_path.as_deref())?;
            let response = service.resolve_full(&record_id)?;
            let payload_sources: Vec<serde_json::Value> = response
                .sources
                .iter()
                .map(|source| {
                    let mut value = serde_json::to_value(source).unwrap_or(serde_json::json!({}));
                    if include_source_excerpt {
                        let excerpt = match crate::memory::resolve::read_source_excerpt(source) {
                            Ok(text) => text,
                            Err(e) => {
                                eprintln!(
                                    "sessio: read excerpt for {} failed: {e}",
                                    source.file_path
                                );
                                None
                            }
                        };
                        if let Some(map) = value.as_object_mut() {
                            map.insert("excerpt".to_string(), serde_json::json!(excerpt));
                        }
                    }
                    value
                })
                .collect();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "recordId": response.record_id,
                        "record": response.record,
                        "sources": payload_sources,
                        "continuation": response.continuation,
                        "continuationSummary": response.continuation
                            .as_ref()
                            .map(continuation_summary)
                    }))?
                );
            } else {
                for source in &response.sources {
                    println!(
                        "{}\t{}\t{}",
                        source.agent, source.session_id, source.file_path
                    );
                }
                if let Some(continuation) = &response.continuation {
                    print_continuation_summary(continuation);
                }
            }
            Ok(())
        }
        MemoryCommand::CoveredBy {
            record_id,
            db_path,
            json,
        } => {
            let service = build_cli_service(db_path.as_deref())?;
            let resolved = service.resolve_full(&record_id)?;
            let Some(record) = resolved.record else {
                bail!("record not found: {record_id}");
            };
            let continuation = resolved.continuation;
            let base_record_id = continuation.as_ref().map(base_record_id_for_continuation);
            let base_record = match base_record_id.as_deref() {
                Some(id) => service.resolve(id)?,
                None => None,
            };
            let payload = MemoryCoveredByResult {
                record_id: resolved.record_id,
                record,
                base_record_id,
                base_record,
                continuation: continuation.as_ref().map(continuation_summary),
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else if let Some(continuation) = continuation {
                println!(
                    "base record: {}",
                    base_record_id_for_continuation(&continuation)
                );
                print_continuation_summary(&continuation);
            } else {
                println!("no continuation provenance recorded");
            }
            Ok(())
        }
        MemoryCommand::Base {
            record_id,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let Some(record) = store.record_by_id(&record_id)? else {
                bail!("record not found: {record_id}");
            };
            let sources = store.sources_for_record(&record.record_id)?;
            let Some(base_source) = sources.first() else {
                bail!("base record has no source refs: {record_id}");
            };
            let continuations =
                store.continuations_for_base(&base_source.agent, &base_source.session_id)?;
            let hits: Vec<MemoryBaseHit> = continuations
                .into_iter()
                .map(|continuation| {
                    let record_id = continuation.record_id.clone();
                    let summary = continuation_summary(&continuation);
                    let record = store.record_by_id(&record_id).ok().flatten();
                    MemoryBaseHit {
                        record_id,
                        record,
                        continuation: summary,
                    }
                })
                .collect();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "baseRecordId": record.record_id,
                        "baseRecord": record,
                        "baseSource": base_source,
                        "hits": hits,
                    }))?
                );
            } else {
                println!("base record: {}", record.record_id);
                println!(
                    "base source: {} {}",
                    base_source.agent, base_source.session_id
                );
                for hit in hits {
                    println!(
                        "{}\t{}\t{}",
                        hit.record_id,
                        hit.continuation.covered_by,
                        hit.continuation.candidate_trim_start
                    );
                }
            }
            Ok(())
        }
        MemoryCommand::Search {
            project_key,
            project,
            query,
            db_path,
            include_raw,
            json,
        } => {
            let project_key = resolve_project_key(project_key, project.as_deref())?;
            let service = build_cli_service(db_path.as_deref())?;
            let search_result =
                service.search_full(&project_key, &query, MemorySearchOptions { include_raw });
            let (backend_name, raw, hits, backend_error) = match search_result {
                Ok(response) => (
                    response.backend,
                    response.raw,
                    response
                        .hits
                        .into_iter()
                        .map(|hit| MemorySearchHit {
                            record_id: hit.record.record_id.clone(),
                            title: hit.record.title.clone(),
                            summary: hit.record.summary.clone(),
                            score: hit.score,
                            snippet: hit.snippet,
                            sources: hit.sources,
                            continuation: hit.continuation.as_ref().map(continuation_summary),
                        })
                        .collect::<Vec<_>>(),
                    None,
                ),
                Err(e) => (
                    service.backend_name().to_string(),
                    None,
                    Vec::new(),
                    Some(e.to_string()),
                ),
            };
            if json {
                let mut payload = serde_json::json!({
                    "projectKey": project_key,
                    "query": query,
                    "backend": backend_name,
                    "hits": hits,
                    "backendError": backend_error,
                });
                if let Some(raw) = raw {
                    payload
                        .as_object_mut()
                        .expect("payload is a JSON object")
                        .insert("raw".to_string(), raw);
                }
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else if let Some(error) = backend_error {
                println!("memory search backend unavailable: {error}");
            } else if include_raw {
                println!("{}", serde_json::to_string_pretty(&raw)?);
            } else {
                for hit in &hits {
                    println!(
                        "{}\t{}\t{}",
                        hit.record_id,
                        hit.score
                            .map(|s| format!("{s:.3}"))
                            .unwrap_or_else(|| "-".to_string()),
                        hit.title,
                    );
                    if let Some(continuation) = &hit.continuation {
                        println!(
                            "  continuation: {} | base {} | trim {}",
                            continuation.covered_by,
                            continuation.base_turn_range,
                            continuation.candidate_trim_start,
                        );
                    }
                }
            }
            Ok(())
        }
        MemoryCommand::Jobs {
            project_key,
            status,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let jobs = store.list_memory_jobs(&project_key, status.as_deref())?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "projectKey": project_key,
                        "status": status,
                        "jobs": jobs
                    }))?
                );
            } else {
                for job in jobs {
                    println!(
                        "{}\t{}\t{}\t{}",
                        job.status,
                        job.kind,
                        job.scope,
                        job.error.unwrap_or_default()
                    );
                }
            }
            Ok(())
        }
    }
}

fn run_thread(cmd: ThreadCommand) -> Result<()> {
    match cmd {
        ThreadCommand::List {
            project,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let projects = store.list_projects()?;
            let selected: Vec<_> = match project.as_deref() {
                Some(filter) => {
                    let wanted = normalize_project_filter(filter);
                    projects
                        .into_iter()
                        .filter(|p| normalize_project_filter(&p.path) == wanted)
                        .collect()
                }
                None => projects,
            };
            let mut threads = Vec::new();
            for project in &selected {
                threads.extend(store.list_threads(&project.id)?);
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&threads)?);
            } else {
                for thread in &threads {
                    println!(
                        "{}\t{}\t{} stages",
                        thread.id,
                        thread.goal,
                        thread.stages.len()
                    );
                }
            }
            Ok(())
        }
        ThreadCommand::Show { id, db_path, json } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let thread = find_thread_by_id(&store, &id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&thread)?);
            } else {
                println!("thread\t{}\t{}", thread.id, thread.goal);
                if let Some(description) = &thread.description {
                    println!("description\t{description}");
                }
                let mut stages = thread.stages;
                stages.sort_by(|a, b| a.order.cmp(&b.order));
                for stage in &stages {
                    println!(
                        "stage\t{}\t{}\t{}",
                        stage.id,
                        stage.status.as_str(),
                        stage_display_name(stage)
                    );
                }
            }
            Ok(())
        }
    }
}

fn run_stage(cmd: StageCommand) -> Result<()> {
    match cmd {
        StageCommand::List {
            thread_id,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let thread = find_thread_by_id(&store, &thread_id)?;
            let mut stages = thread.stages;
            stages.sort_by(|a, b| a.order.cmp(&b.order));
            if json {
                println!("{}", serde_json::to_string_pretty(&stages)?);
            } else {
                for stage in &stages {
                    println!(
                        "{}\t{}\t{}",
                        stage.id,
                        stage.status.as_str(),
                        stage_display_name(stage)
                    );
                }
            }
            Ok(())
        }
        StageCommand::Show { id, db_path, json } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let stage = find_stage_by_id(&store, &id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&stage)?);
            } else {
                println!("stage\t{}\t{}", stage.id, stage.status.as_str());
                println!("name\t{}", stage_display_name(&stage));
                if let Some(summary) = &stage.summary {
                    println!("summary\t{summary}");
                }
                if let Some(outcome) = &stage.outcome {
                    println!("outcome\t{outcome}");
                }
                for session in &stage.sessions {
                    println!("session\t{}\t{}", session.agent.as_str(), session.id);
                }
            }
            Ok(())
        }
        StageCommand::SetStatus {
            id,
            status,
            summary,
            outcome,
            db_path,
            json,
        } => {
            let parsed = StageStatus::from_db_str(&status)
                .with_context(|| format!("invalid stage status: {status}"))?;
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let stage = store.update_thread_stage_state(
                &id,
                Some(parsed),
                summary.map(Some),
                outcome.map(Some),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&stage)?);
            } else {
                println!("stage\t{}\t{}", stage.id, stage.status.as_str());
            }
            Ok(())
        }
    }
}

fn find_thread_by_id(store: &SqliteStore, thread_id: &str) -> Result<crate::models::ThreadInfo> {
    for project in store.list_projects()? {
        if let Some(thread) = store
            .list_threads(&project.id)?
            .into_iter()
            .find(|thread| thread.id == thread_id)
        {
            return Ok(thread);
        }
    }
    bail!("thread not found: {thread_id}")
}

fn find_stage_by_id(store: &SqliteStore, thread_stage_id: &str) -> Result<crate::models::StageInfo> {
    for project in store.list_projects()? {
        for thread in store.list_threads(&project.id)? {
            if let Some(stage) = thread
                .stages
                .into_iter()
                .find(|stage| stage.id == thread_stage_id)
            {
                return Ok(stage);
            }
        }
    }
    bail!("thread stage not found: {thread_stage_id}")
}

fn stage_display_name(stage: &crate::models::StageInfo) -> String {
    if let Some(name) = &stage.name {
        return name.clone();
    }
    if let Some(kind) = &stage.kind {
        return kind.as_str().to_string();
    }
    stage.stage_id.clone()
}

fn run_sessions(cmd: SessionsCommand) -> Result<()> {
    match cmd {
        SessionsCommand::List {
            project,
            db_path,
            json,
        } => {
            let mut sessions = load_sessions_from_store_or_scan(db_path.as_deref())?;
            if let Some(project) = project {
                let wanted = normalize_project_filter(&project);
                sessions.retain(|s| {
                    s.project_path
                        .as_deref()
                        .map(normalize_project_filter)
                        .as_deref()
                        == Some(wanted.as_str())
                });
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&sessions)?);
            } else {
                for s in sessions {
                    println!(
                        "{}\t{}\t{}\t{}",
                        s.agent.as_str(),
                        s.id,
                        s.project_path.as_deref().unwrap_or(""),
                        s.file_path
                    );
                }
            }
            Ok(())
        }
        SessionsCommand::Messages {
            agent,
            session_id,
            file_path,
            json,
        } => {
            let file_path = match file_path {
                Some(path) => path,
                None => resolve_session_file(agent, session_id.as_deref())?,
            };
            let result =
                crate::read_session_history_result(agent, &file_path, session_id.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_session_turns(&result.turns);
            }
            Ok(())
        }
    }
}

fn print_session_turns(turns: &[SessionHistoryTurn]) {
    for turn in turns {
        for block in &turn.blocks {
            if let Some((role, text)) = session_history_block_text(block) {
                println!("[{}]\n{}\n", role, text);
            }
        }
    }
}

fn session_history_block_text(block: &SessionHistoryBlock) -> Option<(&'static str, String)> {
    let role = match block.kind.as_str() {
        "user" => "user",
        "assistant" => "assistant",
        "thought" => "thinking",
        _ => return None,
    };
    let text = block
        .blocks
        .iter()
        .filter_map(|part| match part.kind.as_str() {
            "text" => part.text.clone(),
            "image" => part.uri.as_ref().map(|uri| {
                format!(
                    "![{}]({})",
                    part.mime_type.as_deref().unwrap_or("image"),
                    uri
                )
            }),
            "resource" | "resource_link" => {
                let name = part.name.as_deref().unwrap_or("attachment");
                let uri = part.uri.as_deref().unwrap_or("");
                Some(format!("[file: {name}|{uri}]"))
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some((role, text))
}

fn resolve_session_file(agent: Agent, session_id: Option<&str>) -> Result<String> {
    let session_id = session_id.context("missing --session-id or --file-path")?;
    crate::agents::sources::list_all()
        .into_iter()
        .find(|s| s.agent == agent && s.id == session_id)
        .map(|s| s.file_path)
        .with_context(|| format!("session not found for {}:{session_id}", agent.as_str()))
}

fn parse_args(args: Vec<String>) -> Result<Cli> {
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(Cli {
            command: Command::Help,
        });
    }

    match args[0].as_str() {
        "sessions" => parse_sessions(&args[1..]),
        "memory" => parse_memory(&args[1..]),
        "config" => parse_config(&args[1..]),
        "thread" => parse_thread(&args[1..]),
        "stage" => parse_stage(&args[1..]),
        other => bail!("unknown command '{other}'"),
    }
}

fn parse_config(args: &[String]) -> Result<Cli> {
    let Some(subcommand) = args.first() else {
        bail!("missing config subcommand");
    };
    match subcommand.as_str() {
        "show" => {
            let mut json = false;
            for arg in &args[1..] {
                match arg.as_str() {
                    "--json" => json = true,
                    other => bail!("unknown config show option '{other}'"),
                }
            }
            Ok(Cli {
                command: Command::Config(ConfigCommand::Show { json }),
            })
        }
        "memory" => parse_config_memory(&args[1..]),
        other => bail!("unknown config subcommand '{other}'"),
    }
}

fn parse_config_memory(args: &[String]) -> Result<Cli> {
    let Some(subcommand) = args.first() else {
        bail!("missing config memory subcommand");
    };
    match subcommand.as_str() {
        "set" => {
            let mut binary = None;
            let mut index = None;
            let mut artifacts_root = None;
            let mut auto_embed = None;
            let mut install_command = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--binary" => {
                        i += 1;
                        binary = Some(args.get(i).context("missing value for --binary")?.clone());
                    }
                    "--index" => {
                        i += 1;
                        index = Some(args.get(i).context("missing value for --index")?.clone());
                    }
                    "--artifacts-root" => {
                        i += 1;
                        artifacts_root = Some(
                            args.get(i)
                                .context("missing value for --artifacts-root")?
                                .clone(),
                        );
                    }
                    "--auto-embed" => {
                        i += 1;
                        auto_embed = Some(parse_config_bool(
                            args.get(i).context("missing value for --auto-embed")?,
                        )?);
                    }
                    "--install-command" => {
                        i += 1;
                        install_command = Some(
                            args.get(i)
                                .context("missing value for --install-command")?
                                .clone(),
                        );
                    }
                    "--json" => json = true,
                    other => bail!("unknown config memory set option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Config(ConfigCommand::MemorySet {
                    binary,
                    index,
                    artifacts_root,
                    auto_embed,
                    install_command,
                    json,
                }),
            })
        }
        other => bail!("unknown config memory subcommand '{other}'"),
    }
}

fn parse_memory(args: &[String]) -> Result<Cli> {
    let Some(subcommand) = args.first() else {
        bail!("missing memory subcommand");
    };
    match subcommand.as_str() {
        "status" => {
            let mut binary = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--binary" => {
                        i += 1;
                        binary = Some(args.get(i).context("missing value for --binary")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown memory status option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Memory(MemoryCommand::Status { binary, json }),
            })
        }
        "sync" => {
            let mut project_key = None;
            let mut artifacts_root = None;
            let mut binary = None;
            let mut index = "sessio".to_string();
            let mut embed = false;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--project-key" => {
                        i += 1;
                        project_key = Some(
                            args.get(i)
                                .context("missing value for --project-key")?
                                .clone(),
                        );
                    }
                    "--artifacts-root" => {
                        i += 1;
                        artifacts_root = Some(
                            args.get(i)
                                .context("missing value for --artifacts-root")?
                                .clone(),
                        );
                    }
                    "--binary" => {
                        i += 1;
                        binary = Some(args.get(i).context("missing value for --binary")?.clone());
                    }
                    "--index" => {
                        i += 1;
                        index = args.get(i).context("missing value for --index")?.clone();
                    }
                    "--embed" => embed = true,
                    "--json" => json = true,
                    other => bail!("unknown memory sync option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Memory(MemoryCommand::Sync {
                    project_key: project_key.context("missing --project-key")?,
                    artifacts_root,
                    binary,
                    index,
                    embed,
                    json,
                }),
            })
        }
        "build" => {
            let mut project = None;
            let mut artifacts_root = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--project" => {
                        i += 1;
                        project = Some(args.get(i).context("missing value for --project")?.clone());
                    }
                    "--artifacts-root" => {
                        i += 1;
                        artifacts_root = Some(
                            args.get(i)
                                .context("missing value for --artifacts-root")?
                                .clone(),
                        );
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown memory build option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Memory(MemoryCommand::Build {
                    project: project.context("missing --project")?,
                    artifacts_root,
                    db_path,
                    json,
                }),
            })
        }
        "base" => {
            let mut record_id = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--record-id" => {
                        i += 1;
                        record_id = Some(
                            args.get(i)
                                .context("missing value for --record-id")?
                                .clone(),
                        );
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown memory base option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Memory(MemoryCommand::Base {
                    record_id: record_id.context("missing --record-id")?,
                    db_path,
                    json,
                }),
            })
        }
        "resolve" => {
            let mut record_id = None;
            let mut db_path = None;
            let mut include_source_excerpt = false;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--record-id" => {
                        i += 1;
                        record_id = Some(
                            args.get(i)
                                .context("missing value for --record-id")?
                                .clone(),
                        );
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--include-source-excerpt" => include_source_excerpt = true,
                    "--json" => json = true,
                    other => bail!("unknown memory resolve option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Memory(MemoryCommand::Resolve {
                    record_id: record_id.context("missing --record-id")?,
                    db_path,
                    include_source_excerpt,
                    json,
                }),
            })
        }
        "covered-by" => {
            let mut record_id = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--record-id" => {
                        i += 1;
                        record_id = Some(
                            args.get(i)
                                .context("missing value for --record-id")?
                                .clone(),
                        );
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown memory covered-by option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Memory(MemoryCommand::CoveredBy {
                    record_id: record_id.context("missing --record-id")?,
                    db_path,
                    json,
                }),
            })
        }
        "search" => {
            let mut project_key = None;
            let mut project = None;
            let mut query_parts = Vec::new();
            let mut db_path = None;
            let mut include_raw = false;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--project-key" => {
                        i += 1;
                        project_key = Some(
                            args.get(i)
                                .context("missing value for --project-key")?
                                .clone(),
                        );
                    }
                    "--project" => {
                        i += 1;
                        project = Some(args.get(i).context("missing value for --project")?.clone());
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--include-raw" => include_raw = true,
                    "--json" => json = true,
                    value => query_parts.push(value.to_string()),
                }
                i += 1;
            }
            let query = query_parts.join(" ");
            if query.trim().is_empty() {
                bail!("missing query text");
            }
            Ok(Cli {
                command: Command::Memory(MemoryCommand::Search {
                    project_key,
                    project,
                    query,
                    db_path,
                    include_raw,
                    json,
                }),
            })
        }
        "jobs" => {
            let mut project_key = None;
            let mut status = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--project-key" => {
                        i += 1;
                        project_key = Some(
                            args.get(i)
                                .context("missing value for --project-key")?
                                .clone(),
                        );
                    }
                    "--status" => {
                        i += 1;
                        status = Some(args.get(i).context("missing value for --status")?.clone());
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown memory jobs option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Memory(MemoryCommand::Jobs {
                    project_key: project_key.context("missing --project-key")?,
                    status,
                    db_path,
                    json,
                }),
            })
        }
        other => bail!("unknown memory subcommand '{other}'"),
    }
}

fn parse_config_bool(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        other => bail!("invalid boolean value: {other}"),
    }
}

fn parse_thread(args: &[String]) -> Result<Cli> {
    let Some(subcommand) = args.first() else {
        bail!("missing thread subcommand");
    };
    match subcommand.as_str() {
        "list" => {
            let mut project = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--project" => {
                        i += 1;
                        project = Some(args.get(i).context("missing value for --project")?.clone());
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown thread list option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Thread(ThreadCommand::List {
                    project,
                    db_path,
                    json,
                }),
            })
        }
        "show" => {
            let mut id = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--id" => {
                        i += 1;
                        id = Some(args.get(i).context("missing value for --id")?.clone());
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown thread show option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Thread(ThreadCommand::Show {
                    id: id.context("missing --id")?,
                    db_path,
                    json,
                }),
            })
        }
        other => bail!("unknown thread subcommand '{other}'"),
    }
}

fn parse_stage(args: &[String]) -> Result<Cli> {
    let Some(subcommand) = args.first() else {
        bail!("missing stage subcommand");
    };
    match subcommand.as_str() {
        "list" => {
            let mut thread_id = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--thread-id" => {
                        i += 1;
                        thread_id =
                            Some(args.get(i).context("missing value for --thread-id")?.clone());
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown stage list option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Stage(StageCommand::List {
                    thread_id: thread_id.context("missing --thread-id")?,
                    db_path,
                    json,
                }),
            })
        }
        "show" => {
            let mut id = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--id" => {
                        i += 1;
                        id = Some(args.get(i).context("missing value for --id")?.clone());
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown stage show option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Stage(StageCommand::Show {
                    id: id.context("missing --id")?,
                    db_path,
                    json,
                }),
            })
        }
        "set-status" => {
            let mut id = None;
            let mut status = None;
            let mut summary = None;
            let mut outcome = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--id" => {
                        i += 1;
                        id = Some(args.get(i).context("missing value for --id")?.clone());
                    }
                    "--status" => {
                        i += 1;
                        status = Some(args.get(i).context("missing value for --status")?.clone());
                    }
                    "--summary" => {
                        i += 1;
                        summary = Some(args.get(i).context("missing value for --summary")?.clone());
                    }
                    "--outcome" => {
                        i += 1;
                        outcome = Some(args.get(i).context("missing value for --outcome")?.clone());
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown stage set-status option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Stage(StageCommand::SetStatus {
                    id: id.context("missing --id")?,
                    status: status.context("missing --status")?,
                    summary,
                    outcome,
                    db_path,
                    json,
                }),
            })
        }
        other => bail!("unknown stage subcommand '{other}'"),
    }
}

fn parse_sessions(args: &[String]) -> Result<Cli> {
    let Some(subcommand) = args.first() else {
        bail!("missing sessions subcommand");
    };
    match subcommand.as_str() {
        "list" => {
            let mut project = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--project" => {
                        i += 1;
                        project = Some(args.get(i).context("missing value for --project")?.clone());
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown sessions list option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Sessions(SessionsCommand::List {
                    project,
                    db_path,
                    json,
                }),
            })
        }
        "messages" => {
            let mut agent = None;
            let mut session_id = None;
            let mut file_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--agent" => {
                        i += 1;
                        agent = Some(parse_agent(
                            args.get(i).context("missing value for --agent")?,
                        )?);
                    }
                    "--session-id" => {
                        i += 1;
                        session_id = Some(
                            args.get(i)
                                .context("missing value for --session-id")?
                                .clone(),
                        );
                    }
                    "--file-path" => {
                        i += 1;
                        file_path = Some(
                            args.get(i)
                                .context("missing value for --file-path")?
                                .clone(),
                        );
                    }
                    "--json" => json = true,
                    other => bail!("unknown sessions messages option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Sessions(SessionsCommand::Messages {
                    agent: agent.context("missing --agent")?,
                    session_id,
                    file_path,
                    json,
                }),
            })
        }
        other => bail!("unknown sessions subcommand '{other}'"),
    }
}

fn parse_agent(s: &str) -> Result<Agent> {
    match s {
        "codex" => Ok(Agent::Codex),
        "claude" => Ok(Agent::Claude),
        "gemini" => Ok(Agent::Gemini),
        _ => bail!("unknown agent '{s}'"),
    }
}

fn normalize_project_filter(project: &str) -> String {
    std::fs::canonicalize(project)
        .unwrap_or_else(|_| PathBuf::from(project))
        .to_string_lossy()
        .to_string()
}

fn open_store(db_path: Option<&str>) -> Result<SqliteStore> {
    if let Some(db_path) = db_path {
        return SqliteStore::open(&PathBuf::from(db_path));
    }
    let data_dir = dirs::home_dir()
        .context("no home dir")?
        .join(".sessio")
        .join("db-data");
    std::fs::create_dir_all(&data_dir).ok();
    SqliteStore::open(&data_dir.join("sessio-index.db"))
}

fn build_cli_service(db_path: Option<&str>) -> Result<MemoryService> {
    let store = Arc::new(open_store(db_path)?);
    store.init()?;
    let memory_store: Arc<dyn MemoryStore> = store;
    MemoryService::new(
        memory_store,
        Arc::new(crate::agents::sources::builtin_agent_sources()),
    )
}

fn load_sessions_from_store_or_scan(
    db_path: Option<&str>,
) -> Result<Vec<crate::models::SessionInfo>> {
    match open_store(db_path) {
        Ok(store) => {
            match store.init().and_then(|()| store.list_sessions()) {
                Ok(sessions) if !sessions.is_empty() => Ok(sessions),
                Ok(_) => {
                    eprintln!(
                        "sessio: session index is empty, falling back to filesystem scan. \
                     Run the Sessio desktop app (or rebuild the index) for faster lookups."
                    );
                    Ok(crate::agents::sources::list_all())
                }
                Err(e) => {
                    eprintln!("sessio: failed to read session index ({e}), falling back to filesystem scan.");
                    Ok(crate::agents::sources::list_all())
                }
            }
        }
        Err(e) => {
            eprintln!(
                "sessio: failed to open session index ({e}), falling back to filesystem scan."
            );
            Ok(crate::agents::sources::list_all())
        }
    }
}

fn resolve_project_key(project_key: Option<String>, project: Option<&str>) -> Result<String> {
    if let Some(project_key) = project_key {
        return Ok(project_key);
    }
    let Some(project) = project else {
        bail!("missing --project or --project-key");
    };
    let path = std::fs::canonicalize(project).unwrap_or_else(|_| PathBuf::from(project));
    Ok(project_key_for_path_or_name(
        Some(&path.to_string_lossy()),
        path.file_name().and_then(|name| name.to_str()),
    ))
}

fn continuation_summary(continuation: &RecordContinuation) -> MemoryContinuationSummary {
    MemoryContinuationSummary {
        covered_by: format!(
            "{} {}",
            continuation.base_agent, continuation.base_session_id
        ),
        base_file_path: continuation.base_file_path.clone(),
        base_turn_range: format!(
            "turn {}..{}",
            continuation.base_start_turn_index, continuation.base_end_turn_index
        ),
        base_line_range: format_optional_range(
            "line",
            continuation.base_start_line_start,
            continuation.base_end_line_end,
        ),
        base_byte_range: format_optional_range(
            "byte",
            continuation.base_start_byte_start,
            continuation.base_end_byte_end,
        ),
        candidate_trim_start: format_trim_start(
            continuation.candidate_trim_turn_start,
            continuation.candidate_trim_line_start,
            continuation.candidate_trim_byte_start,
        ),
        candidate_file_path: continuation.candidate_file_path.clone(),
    }
}

fn base_record_id_for_continuation(continuation: &RecordContinuation) -> String {
    format!(
        "sessio-{}-{}",
        safe_id_part(&continuation.base_agent),
        safe_id_part(&continuation.base_session_id)
    )
}

fn format_optional_range(label: &str, start: Option<u64>, end: Option<u64>) -> Option<String> {
    match (start, end) {
        (Some(start), Some(end)) => Some(format!("{label} {start}..{end}")),
        (Some(start), None) => Some(format!("{label} {start}..")),
        (None, Some(end)) => Some(format!("{label} ..{end}")),
        (None, None) => None,
    }
}

fn format_trim_start(turn: usize, line: Option<u64>, byte: Option<u64>) -> String {
    let mut parts = vec![format!("turn {turn}")];
    if let Some(line) = line {
        parts.push(format!("line {line}"));
    }
    if let Some(byte) = byte {
        parts.push(format!("byte {byte}"));
    }
    parts.join(", ")
}

fn print_continuation_summary(continuation: &RecordContinuation) {
    let summary = continuation_summary(continuation);
    println!("continuation:");
    println!("  covered by: {}", summary.covered_by);
    println!("  base file: {}", summary.base_file_path);
    println!("  base coverage: {}", summary.base_turn_range);
    if let Some(line_range) = summary.base_line_range {
        println!("  base lines: {}", line_range);
    }
    if let Some(byte_range) = summary.base_byte_range {
        println!("  base bytes: {}", byte_range);
    }
    println!("  candidate trim starts: {}", summary.candidate_trim_start);
    println!("  candidate file: {}", summary.candidate_file_path);
}

fn print_help() {
    println!(
        r#"sessio

Usage:
  sessio sessions list [--project <path>] [--db-path <path>] [--json]
  sessio sessions messages --agent <codex|claude|gemini> [--session-id <id>] [--file-path <path>] [--json]
  sessio thread list [--project <path>] [--db-path <path>] [--json]
  sessio thread show --id <threadId> [--db-path <path>] [--json]
  sessio stage list --thread-id <threadId> [--db-path <path>] [--json]
  sessio stage show --id <threadStageId> [--db-path <path>] [--json]
  sessio stage set-status --id <threadStageId> --status <not_started|in_progress|blocked|needs_review|completed|skipped> [--summary <text>] [--outcome <text>] [--db-path <path>] [--json]
  sessio config show [--json]
  sessio config memory set [--binary <path>] [--index <name>] [--artifacts-root <path>] [--auto-embed <bool>] [--install-command <cmd>] [--json]
  sessio memory build --project <path> [--artifacts-root <path>] [--db-path <path>] [--json]
  sessio memory covered-by --record-id <id> [--db-path <path>] [--json]
  sessio memory base --record-id <id> [--db-path <path>] [--json]
  sessio memory status [--binary <path>] [--json]
  sessio memory sync --project-key <key> [--artifacts-root <path>] [--index sessio] [--binary <path>] [--embed] [--json]
  sessio memory search (--project <path>|--project-key <key>) <query> [--db-path <path>] [--include-raw] [--json]
  sessio memory resolve --record-id <id> [--db-path <path>] [--include-source-excerpt] [--json]
  sessio memory jobs --project-key <key> [--status <status>] [--db-path <path>] [--json]

Notes:
  --json emits stable machine-readable output for skills and agents.
  sessions list reads from the Sessio index DB by default and falls back to a filesystem scan when the index is empty/unreadable; a stderr warning is printed when the fallback fires.
  memory search omits qmd's raw payload by default; pass --include-raw for debugging.
  memory resolve omits raw JSONL excerpts by default; pass --include-source-excerpt to attach the byte/line range each source points at (Codex / Claude today; Gemini is session-level only).
  Gemini message lookup reads session JSONL files.
  memory covered-by shows which base record covered a given record, if continuation provenance exists.
  memory base lists records covered by a given base record via record_continuations.
"#
    );
}

fn serialize_app_config(config: &config::AppConfig) -> String {
    config::serialize_app_config(config)
}
