use crate::agents::sources::shared::convert::project_key_for_path_or_name;
use crate::app_paths;
use crate::config;
use crate::memory::build::MemoryBuildOptions;
use crate::memory::qmd;
use crate::memory::records::safe_id_part;
use crate::memory::service::MemoryService;
use crate::memory::{MemoryRecord, MemorySearchOptions, MemoryStore, RecordContinuation};
use crate::models::{
    Agent, IssueSeverity, IssueStatus, SessionHistoryBlock, SessionHistoryTurn, StageStatus,
    ThreadKind,
};
use crate::store::sqlite::SqliteStore;
use crate::store::SessionStore;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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
    ComputerUse(ComputerUseCommand),
    Help,
}

#[derive(Debug, Clone)]
struct CuConnection {
    url: Option<String>,
    token: Option<String>,
    json: bool,
}

#[derive(Debug)]
enum ComputerUseCommand {
    Tool {
        connection: CuConnection,
        name: String,
        arguments: Value,
    },
}

#[derive(Debug)]
enum ThreadCommand {
    Create {
        project: String,
        goal: String,
        description: Option<String>,
        kind: ThreadKind,
        assistant_ids: Vec<String>,
        db_path: Option<String>,
        json: bool,
    },
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
    Update {
        id: String,
        goal: Option<String>,
        description: Option<String>,
        kind: Option<ThreadKind>,
        enabled: Option<bool>,
        assistant_ids: Option<Vec<String>>,
        db_path: Option<String>,
        json: bool,
    },
    SetStage {
        thread_id: String,
        stage_id: String,
        db_path: Option<String>,
        json: bool,
    },
    LinkSession {
        thread_id: String,
        agent: Agent,
        session_id: String,
        db_path: Option<String>,
        json: bool,
    },
    UnlinkSession {
        thread_id: String,
        agent: Agent,
        session_id: String,
        db_path: Option<String>,
        json: bool,
    },
}

#[derive(Debug)]
enum StageCommand {
    Catalog {
        project: String,
        db_path: Option<String>,
        json: bool,
    },
    Add {
        thread_id: String,
        stage_id: String,
        assistant_ids: Vec<String>,
        db_path: Option<String>,
        json: bool,
    },
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
    Update {
        id: String,
        status: Option<String>,
        summary: Option<String>,
        outcome: Option<String>,
        db_path: Option<String>,
        json: bool,
    },
    Configure {
        id: String,
        assistant_ids: Option<Vec<String>>,
        order: Option<i64>,
        enabled: Option<bool>,
        db_path: Option<String>,
        json: bool,
    },
    LinkSession {
        stage_id: String,
        agent: Agent,
        session_id: String,
        db_path: Option<String>,
        json: bool,
    },
    UnlinkSession {
        stage_id: String,
        agent: Agent,
        session_id: String,
        db_path: Option<String>,
        json: bool,
    },
    Issue(IssueCommand),
}

#[derive(Debug)]
enum IssueCommand {
    Add {
        stage_id: String,
        title: String,
        description: Option<String>,
        severity: String,
        db_path: Option<String>,
        json: bool,
    },
    List {
        stage_id: String,
        db_path: Option<String>,
        json: bool,
    },
    Set {
        id: String,
        status: Option<String>,
        severity: Option<String>,
        title: Option<String>,
        description: Option<String>,
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
        Command::ComputerUse(cmd) => run_computer_use(cmd),
        Command::Help => {
            print_help();
            Ok(())
        }
    }
}

fn run_computer_use(cmd: ComputerUseCommand) -> Result<()> {
    match cmd {
        ComputerUseCommand::Tool {
            connection,
            name,
            arguments,
        } => {
            let response = call_computer_use_tool(&connection, &name, arguments)?;
            print_computer_use_response(&response, connection.json)
        }
    }
}

fn call_computer_use_tool(
    connection: &CuConnection,
    name: &str,
    arguments: Value,
) -> Result<Value> {
    let resolved = resolve_cu_connection(connection)?;
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
        }
    });
    let response = reqwest::blocking::Client::new()
        .post(&resolved.url)
        .bearer_auth(&resolved.token)
        .header("Accept", "application/json, text/event-stream")
        .json(&body)
        .send()
        .with_context(|| {
            format!("computer-use host is not reachable at {}; start Sessio desktop or pass --url/--token", resolved.url)
        })?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        bail!("computer-use host rejected request ({status}): {text}");
    }
    serde_json::from_str(&text).with_context(|| "computer-use host returned invalid JSON")
}

struct ResolvedCuConnection {
    url: String,
    token: String,
}

fn resolve_cu_connection(connection: &CuConnection) -> Result<ResolvedCuConnection> {
    match (connection.url.clone(), connection.token.clone()) {
        (Some(url), Some(token)) => {
            return Ok(ResolvedCuConnection {
                url: normalize_cu_url(&url),
                token,
            });
        }
        (Some(_), None) | (None, Some(_)) => {
            bail!("--url and --token must be provided together");
        }
        (None, None) => {}
    }

    match (
        env::var("SESSIO_CU_URL").ok(),
        env::var("SESSIO_CU_TOKEN").ok(),
    ) {
        (Some(url), Some(token)) => {
            return Ok(ResolvedCuConnection {
                url: normalize_cu_url(&url),
                token,
            });
        }
        (Some(_), None) | (None, Some(_)) => {
            bail!("SESSIO_CU_URL and SESSIO_CU_TOKEN must be set together");
        }
        (None, None) => {}
    }

    if let Some(session) = crate::computer_use::broker::read_session()? {
        if validate_cu_session(&session) {
            return Ok(ResolvedCuConnection {
                url: normalize_cu_url(&session.mcp_url),
                token: session.token,
            });
        }
        crate::computer_use::broker::remove_session();
    }

    let discovery = crate::computer_use::broker::read_discovery()?.context(
        "no computer-use broker discovered: start Sessio desktop, or pass --url/--token",
    )?;
    let attach = attach_to_cu_broker(&discovery.broker_url)?;
    let session = crate::computer_use::broker::session_from_attach(discovery.broker_url, attach);
    crate::computer_use::broker::write_session(&session)?;
    Ok(ResolvedCuConnection {
        url: normalize_cu_url(&session.mcp_url),
        token: session.token,
    })
}

fn attach_to_cu_broker(
    broker_url: &str,
) -> Result<crate::computer_use::broker::ExternalAttachResponse> {
    let url = format!("{}/attach", broker_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "clientName": "sessio-cu",
        "clientPid": std::process::id(),
    });
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .context("failed to build computer-use broker client")?
        .post(&url)
        .json(&body)
        .send()
        .with_context(|| format!("computer-use broker is not reachable at {url}"))?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        bail!("computer-use broker rejected attach ({status}): {text}");
    }
    serde_json::from_str(&text).with_context(|| "computer-use broker returned invalid JSON")
}

fn validate_cu_session(session: &crate::computer_use::broker::ExternalSession) -> bool {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    client
        .post(normalize_cu_url(&session.mcp_url))
        .bearer_auth(&session.token)
        .header("Accept", "application/json, text/event-stream")
        .json(&body)
        .send()
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

fn normalize_cu_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.ends_with("/mcp") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/mcp")
    }
}

fn print_computer_use_response(response: &Value, json: bool) -> Result<()> {
    if let Some(error) = response.get("error") {
        bail!("computer-use JSON-RPC error: {error}");
    }
    let result = response.get("result").context("missing MCP result")?;
    if result
        .get("isError")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        bail!(
            "{}",
            tool_text(result).unwrap_or_else(|| result.to_string())
        );
    }
    if json {
        let payload = result.get("structuredContent").unwrap_or(result);
        println!("{}", serde_json::to_string_pretty(payload)?);
    } else if let Some(text) = tool_text(result) {
        println!("{text}");
    } else {
        println!("{}", serde_json::to_string_pretty(result)?);
    }
    Ok(())
}

fn tool_text(result: &Value) -> Option<String> {
    result
        .get("content")
        .and_then(|content| content.as_array())
        .and_then(|content| content.first())
        .and_then(|block| block.get("text"))
        .and_then(|text| text.as_str())
        .map(|text| text.to_string())
}

fn serialize_app_config(config: &config::AppConfig) -> String {
    config::serialize_app_config(config)
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
            let mut memory = match config::load_memory_config() {
                Ok(memory) => memory,
                Err(_) => crate::config::MemoryConfig {
                    backend: "qmd".to_string(),
                    qmd: crate::config::QmdBackendConfig {
                        binary: None,
                        index: index
                            .clone()
                            .context("memory is not configured; pass --index to create it")?,
                        artifacts_root: config::expand_path(artifacts_root.as_deref().context(
                            "memory is not configured; pass --artifacts-root to create it",
                        )?)?,
                        auto_embed: auto_embed
                            .context("memory is not configured; pass --auto-embed to create it")?,
                        install_command: install_command.clone().context(
                            "memory is not configured; pass --install-command to create it",
                        )?,
                    },
                },
            };
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
                println!(
                    "saved memory config to {}/config.toml",
                    app_paths::app_home_display()
                );
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
        ThreadCommand::Create {
            project,
            goal,
            description,
            kind,
            assistant_ids,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let project_id = resolve_project_id(&store, &project)?;
            let thread = store.create_thread_with_options(
                &project_id,
                &goal,
                description.as_deref(),
                kind,
                &assistant_ids,
                &[],
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&thread)?);
            } else {
                println!("thread\t{}\t{}", thread.id, thread.goal);
            }
            Ok(())
        }
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
                stages.sort_by_key(|a| a.order);
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
        ThreadCommand::Update {
            id,
            goal,
            description,
            kind,
            enabled,
            assistant_ids,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let description = description.as_ref().map(|value| Some(value.as_str()));
            let thread = store.update_thread_with_options(
                &id,
                goal.as_deref(),
                description,
                enabled,
                kind,
                assistant_ids.as_deref(),
                None,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&thread)?);
            } else {
                println!("thread\t{}\t{}", thread.id, thread.goal);
            }
            Ok(())
        }
        ThreadCommand::SetStage {
            thread_id,
            stage_id,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let thread = store.set_thread_stage(&thread_id, &stage_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&thread)?);
            } else {
                println!("thread\t{}\tactive_stage\t{}", thread.id, stage_id);
            }
            Ok(())
        }
        ThreadCommand::LinkSession {
            thread_id,
            agent,
            session_id,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let thread = store.link_thread_session(&thread_id, agent, &session_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&thread)?);
            } else {
                println!(
                    "thread_session\t{}\t{}\t{}",
                    thread.id,
                    agent.as_str(),
                    session_id
                );
            }
            Ok(())
        }
        ThreadCommand::UnlinkSession {
            thread_id,
            agent,
            session_id,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let thread = store.unlink_thread_session(&thread_id, agent, &session_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&thread)?);
            } else {
                println!(
                    "thread_session_unlinked\t{}\t{}\t{}",
                    thread.id,
                    agent.as_str(),
                    session_id
                );
            }
            Ok(())
        }
    }
}

fn run_stage(cmd: StageCommand) -> Result<()> {
    match cmd {
        StageCommand::Catalog {
            project,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let project_id = resolve_project_id(&store, &project)?;
            let stages = store.list_project_stages(&project_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&stages)?);
            } else {
                for stage in &stages {
                    println!(
                        "{}\t{}\t{}",
                        stage.id,
                        stage.enabled,
                        project_stage_display_name(stage)
                    );
                }
            }
            Ok(())
        }
        StageCommand::Add {
            thread_id,
            stage_id,
            assistant_ids,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let stage = store.add_thread_stage(&thread_id, &stage_id, &assistant_ids)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&stage)?);
            } else {
                println!("stage\t{}\t{}", stage.id, stage_display_name(&stage));
            }
            Ok(())
        }
        StageCommand::List {
            thread_id,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let thread = find_thread_by_id(&store, &thread_id)?;
            let mut stages = thread.stages;
            stages.sort_by_key(|a| a.order);
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
        StageCommand::Update {
            id,
            status,
            summary,
            outcome,
            db_path,
            json,
        } => {
            let parsed = match status.as_deref() {
                Some(value) => Some(
                    StageStatus::from_db_str(value)
                        .with_context(|| format!("invalid stage status: {value}"))?,
                ),
                None => None,
            };
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let stage = store.update_thread_stage_state(
                &id,
                parsed,
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
        StageCommand::Configure {
            id,
            assistant_ids,
            order,
            enabled,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let stage = store.update_thread_stage(&id, assistant_ids.as_deref(), order, enabled)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&stage)?);
            } else {
                println!("stage\t{}\t{}", stage.id, stage_display_name(&stage));
            }
            Ok(())
        }
        StageCommand::LinkSession {
            stage_id,
            agent,
            session_id,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let stage = store.link_stage_session(&stage_id, agent, &session_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&stage)?);
            } else {
                println!(
                    "stage_session\t{}\t{}\t{}",
                    stage.id,
                    agent.as_str(),
                    session_id
                );
            }
            Ok(())
        }
        StageCommand::UnlinkSession {
            stage_id,
            agent,
            session_id,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let stage = store.unlink_stage_session(&stage_id, agent, &session_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&stage)?);
            } else {
                println!(
                    "stage_session_unlinked\t{}\t{}\t{}",
                    stage.id,
                    agent.as_str(),
                    session_id
                );
            }
            Ok(())
        }
        StageCommand::Issue(cmd) => run_stage_issue(cmd),
    }
}

fn run_stage_issue(cmd: IssueCommand) -> Result<()> {
    match cmd {
        IssueCommand::Add {
            stage_id,
            title,
            description,
            severity,
            db_path,
            json,
        } => {
            let parsed = IssueSeverity::from_db_str(&severity)
                .with_context(|| format!("invalid issue severity: {severity}"))?;
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let issue = store.create_thread_stage_issue(
                &stage_id,
                &title,
                description.as_deref(),
                parsed,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&issue)?);
            } else {
                println!(
                    "issue\t{}\t{}\t{}",
                    issue.id,
                    issue.severity.as_str(),
                    issue.title
                );
            }
            Ok(())
        }
        IssueCommand::List {
            stage_id,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let issues = store.list_thread_stage_issues(&stage_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&issues)?);
            } else {
                for issue in &issues {
                    println!(
                        "issue\t{}\t{}\t{}\t{}",
                        issue.id,
                        issue.status.as_str(),
                        issue.severity.as_str(),
                        issue.title
                    );
                }
            }
            Ok(())
        }
        IssueCommand::Set {
            id,
            status,
            severity,
            title,
            description,
            db_path,
            json,
        } => {
            let status = match status.as_deref() {
                Some(value) => Some(
                    IssueStatus::from_db_str(value)
                        .with_context(|| format!("invalid issue status: {value}"))?,
                ),
                None => None,
            };
            let severity = match severity.as_deref() {
                Some(value) => Some(
                    IssueSeverity::from_db_str(value)
                        .with_context(|| format!("invalid issue severity: {value}"))?,
                ),
                None => None,
            };
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let issue = store.update_thread_stage_issue(
                &id,
                title.as_deref(),
                description.as_deref().map(Some),
                status,
                severity,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&issue)?);
            } else {
                println!(
                    "issue\t{}\t{}\t{}",
                    issue.id,
                    issue.status.as_str(),
                    issue.title
                );
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

fn find_stage_by_id(
    store: &SqliteStore,
    thread_stage_id: &str,
) -> Result<crate::models::StageInfo> {
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

fn resolve_project_id(store: &SqliteStore, project: &str) -> Result<String> {
    let normalized = normalize_project_filter(project);
    let mut path_matches = Vec::new();
    for candidate in store.list_projects()? {
        if candidate.id == project {
            return Ok(candidate.id);
        }
        if normalize_project_filter(&candidate.path) == normalized {
            path_matches.push(candidate.id);
        }
    }
    match path_matches.len() {
        1 => Ok(path_matches.remove(0)),
        0 => bail!("project not found: {project}"),
        _ => bail!("project path is ambiguous: {project}"),
    }
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

fn project_stage_display_name(stage: &crate::models::ProjectStageInfo) -> String {
    if let Some(name) = &stage.name {
        return name.clone();
    }
    if let Some(kind) = &stage.kind {
        return kind.as_str().to_string();
    }
    stage.id.clone()
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
        "cu" | "computer-use" => parse_computer_use(&args[1..]),
        other => bail!("unknown command '{other}'"),
    }
}

fn parse_computer_use(args: &[String]) -> Result<Cli> {
    let Some(subcommand) = args.first() else {
        bail!("missing cu subcommand");
    };
    let (connection, args) = parse_cu_connection(&args[1..])?;
    let tool = |name: &'static str, arguments: Value| {
        Ok(Cli {
            command: Command::ComputerUse(ComputerUseCommand::Tool {
                connection: connection.clone(),
                name: name.to_string(),
                arguments,
            }),
        })
    };
    match subcommand.as_str() {
        "status" => {
            ensure_no_args(&args, "cu status")?;
            tool("computer_status", serde_json::json!({}))
        }
        "permissions" => {
            ensure_no_args(&args, "cu permissions")?;
            tool("computer_permissions", serde_json::json!({}))
        }
        "grant" => {
            ensure_known_options(&args, &["--permission"])?;
            tool(
                "computer_grant",
                serde_json::json!({ "permission": required_option(&args, "--permission")? }),
            )
        }
        "list-apps" => {
            ensure_known_options(&args, &["--days"])?;
            let mut value = serde_json::Map::new();
            if let Some(days) = optional_i64_option(&args, "--days")? {
                value.insert("days".into(), Value::Number(days.into()));
            }
            tool("computer_list_apps", Value::Object(value))
        }
        "start" => tool("computer_start", target_args(&args, true)?),
        "launch-app" => tool("computer_launch_app", target_args(&args, true)?),
        "raise" => tool("computer_raise_app", target_args(&args, true)?),
        "get-app-state" => tool("computer_get_app_state", target_args(&args, false)?),
        "click" => tool("computer_click", click_action_args(&args)?),
        "click-element" => tool("computer_click_element", {
            let args = normalize_ref_positional_args(&args)?;
            ensure_known_options(&args, &["--snapshot-id", "--element-id", "--ref"])?;
            serde_json::json!({
                "snapshotId": required_option(&args, "--snapshot-id")?,
                "elementId": required_option_any(&args, &["--element-id", "--ref"])?,
            })
        }),
        "click-at" => tool("computer_click_at", point_action_args(&args)?),
        "secondary-click" => tool("computer_secondary_click", secondary_action_args(&args)?),
        "perform-secondary-action" => tool(
            "computer_perform_secondary_action",
            secondary_action_args(&args)?,
        ),
        "double-click" => tool("computer_double_click", point_action_args(&args)?),
        "drag" => tool("computer_drag", drag_action_args(&args)?),
        "set-value" => tool("computer_set_value", {
            let args = normalize_ref_positional_args(&args)?;
            ensure_known_options(
                &args,
                &["--snapshot-id", "--element-id", "--ref", "--value"],
            )?;
            serde_json::json!({
                "snapshotId": required_option(&args, "--snapshot-id")?,
                "elementId": required_option_any(&args, &["--element-id", "--ref"])?,
                "value": required_option(&args, "--value")?,
            })
        }),
        "type-text" => tool("computer_type_text", {
            ensure_known_options(&args, &["--snapshot-id", "--text"])?;
            serde_json::json!({
                "snapshotId": required_option(&args, "--snapshot-id")?,
                "text": required_option(&args, "--text")?,
            })
        }),
        "press-key" => tool("computer_press_key", {
            ensure_known_options(&args, &["--snapshot-id", "--key"])?;
            serde_json::json!({
                "snapshotId": required_option(&args, "--snapshot-id")?,
                "key": required_option(&args, "--key")?,
            })
        }),
        "scroll" => tool("computer_scroll", scroll_action_args(&args)?),
        "stop" => {
            ensure_no_args(&args, "cu stop")?;
            tool("computer_stop", serde_json::json!({}))
        }
        "call" => {
            ensure_known_options(&args, &["--tool", "--args-json"])?;
            let name = required_option(&args, "--tool")?;
            let args_json =
                optional_option(&args, "--args-json")?.unwrap_or_else(|| "{}".to_string());
            let arguments: Value =
                serde_json::from_str(&args_json).context("invalid --args-json")?;
            Ok(Cli {
                command: Command::ComputerUse(ComputerUseCommand::Tool {
                    connection,
                    name,
                    arguments,
                }),
            })
        }
        other => bail!("unknown cu subcommand '{other}'"),
    }
}

fn parse_cu_connection(args: &[String]) -> Result<(CuConnection, Vec<String>)> {
    let mut connection = CuConnection {
        url: None,
        token: None,
        json: false,
    };
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--url" => {
                i += 1;
                connection.url = Some(args.get(i).context("missing value for --url")?.clone());
            }
            "--token" => {
                i += 1;
                connection.token = Some(args.get(i).context("missing value for --token")?.clone());
            }
            "--json" => connection.json = true,
            other => rest.push(other.to_string()),
        }
        i += 1;
    }
    Ok((connection, rest))
}

fn target_args(args: &[String], app_required: bool) -> Result<Value> {
    ensure_known_options(args, &["--app-id", "--bundle", "--window-id"])?;
    let app_id = optional_option(args, "--app-id")?.or(optional_option(args, "--bundle")?);
    if app_required && app_id.is_none() {
        bail!("missing --app-id/--bundle");
    }
    let mut value = serde_json::Map::new();
    if let Some(app_id) = app_id {
        value.insert("appId".into(), Value::String(app_id));
    }
    if let Some(window_id) = optional_option(args, "--window-id")? {
        value.insert("windowId".into(), Value::String(window_id));
    }
    Ok(Value::Object(value))
}

fn click_action_args(args: &[String]) -> Result<Value> {
    let args = normalize_ref_positional_args(args)?;
    ensure_known_options(
        &args,
        &[
            "--snapshot-id",
            "--element-id",
            "--ref",
            "--x",
            "--y",
            "--coord-space",
        ],
    )?;
    let mut value = serde_json::json!({
        "snapshotId": required_option(&args, "--snapshot-id")?,
    });
    let element_id = optional_option(&args, "--element-id")?.or(optional_option(&args, "--ref")?);
    let x = optional_f64_option(&args, "--x")?;
    let y = optional_f64_option(&args, "--y")?;
    match (element_id, x, y) {
        (Some(element_id), _, _) => {
            value
                .as_object_mut()
                .expect("click args are an object")
                .insert("elementId".into(), Value::String(element_id));
        }
        (None, Some(x), Some(y)) => {
            let object = value.as_object_mut().expect("click args are an object");
            object.insert("x".into(), value_from_f64(x, "--x")?);
            object.insert("y".into(), value_from_f64(y, "--y")?);
            insert_coord_space(&mut value, &args)?;
        }
        (None, _, _) => bail!("cu click requires --element-id or both --x and --y"),
    }
    Ok(value)
}

fn secondary_action_args(args: &[String]) -> Result<Value> {
    click_action_args(args)
}

fn point_action_args(args: &[String]) -> Result<Value> {
    ensure_known_options(args, &["--snapshot-id", "--x", "--y", "--coord-space"])?;
    let mut value = serde_json::json!({
        "snapshotId": required_option(args, "--snapshot-id")?,
        "x": required_f64_option(args, "--x")?,
        "y": required_f64_option(args, "--y")?,
    });
    insert_coord_space(&mut value, args)?;
    Ok(value)
}

fn scroll_action_args(args: &[String]) -> Result<Value> {
    let args = normalize_ref_positional_args(args)?;
    ensure_known_options(
        &args,
        &[
            "--snapshot-id",
            "--element-id",
            "--ref",
            "--direction",
            "--amount",
        ],
    )?;
    let mut value = serde_json::json!({
        "snapshotId": required_option(&args, "--snapshot-id")?,
        "direction": required_option(&args, "--direction")?,
        "amount": optional_i64_option(&args, "--amount")?.unwrap_or(0),
    });
    if let Some(element_id) =
        optional_option(&args, "--element-id")?.or(optional_option(&args, "--ref")?)
    {
        value
            .as_object_mut()
            .expect("scroll args are an object")
            .insert("elementId".into(), Value::String(element_id));
    }
    Ok(value)
}

fn normalize_ref_positional_args(args: &[String]) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg.starts_with("--") {
            normalized.push(arg.clone());
            i += 1;
            if i >= args.len() {
                bail!("missing value for {arg}");
            }
            normalized.push(args[i].clone());
        } else {
            positional.push(arg.clone());
        }
        i += 1;
    }
    if positional.len() > 1 {
        bail!("expected at most one element ref argument");
    }
    if let Some(element_ref) = positional.pop() {
        normalized.push("--ref".into());
        normalized.push(element_ref);
    }
    Ok(normalized)
}

fn drag_action_args(args: &[String]) -> Result<Value> {
    ensure_known_options(
        args,
        &[
            "--snapshot-id",
            "--from-x",
            "--from-y",
            "--to-x",
            "--to-y",
            "--coord-space",
        ],
    )?;
    let mut value = serde_json::json!({
        "snapshotId": required_option(args, "--snapshot-id")?,
        "fromX": required_f64_option(args, "--from-x")?,
        "fromY": required_f64_option(args, "--from-y")?,
        "toX": required_f64_option(args, "--to-x")?,
        "toY": required_f64_option(args, "--to-y")?,
    });
    insert_coord_space(&mut value, args)?;
    Ok(value)
}

fn insert_coord_space(value: &mut Value, args: &[String]) -> Result<()> {
    if let Some(coord_space) = optional_option(args, "--coord-space")? {
        value
            .as_object_mut()
            .expect("coordinate args are an object")
            .insert("coordSpace".into(), Value::String(coord_space));
    }
    Ok(())
}

fn ensure_no_args(args: &[String], label: &str) -> Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        bail!("{label} does not accept arguments: {}", args.join(" "))
    }
}

fn ensure_known_options(args: &[String], allowed: &[&str]) -> Result<()> {
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        if !flag.starts_with("--") {
            bail!("unexpected argument '{flag}'");
        }
        if !allowed.contains(&flag) {
            bail!("unknown cu option '{flag}'");
        }
        i += 1;
        if i >= args.len() {
            bail!("missing value for {flag}");
        }
        i += 1;
    }
    Ok(())
}

fn ensure_known_options_with_flags(
    args: &[String],
    value_flags: &[&str],
    bool_flags: &[&str],
) -> Result<()> {
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        if !flag.starts_with("--") {
            bail!("unexpected argument '{flag}'");
        }
        if bool_flags.contains(&flag) {
            i += 1;
            continue;
        }
        if !value_flags.contains(&flag) {
            bail!("unknown option '{flag}'");
        }
        i += 1;
        if i >= args.len() {
            bail!("missing value for {flag}");
        }
        i += 1;
    }
    Ok(())
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn has_option(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn required_option(args: &[String], flag: &str) -> Result<String> {
    optional_option(args, flag)?.with_context(|| format!("missing {flag}"))
}

fn required_option_any(args: &[String], flags: &[&str]) -> Result<String> {
    for flag in flags {
        if let Some(value) = optional_option(args, flag)? {
            return Ok(value);
        }
    }
    bail!("missing {}", flags.join("|"))
}

fn optional_option(args: &[String], flag: &str) -> Result<Option<String>> {
    let mut found = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag {
            i += 1;
            found = Some(
                args.get(i)
                    .with_context(|| format!("missing value for {flag}"))?
                    .clone(),
            );
        }
        i += 1;
    }
    Ok(found)
}

fn repeated_option(args: &[String], flag: &str) -> Result<Vec<String>> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag {
            i += 1;
            values.push(
                args.get(i)
                    .with_context(|| format!("missing value for {flag}"))?
                    .clone(),
            );
        }
        i += 1;
    }
    Ok(values)
}

fn required_f64_option(args: &[String], flag: &str) -> Result<f64> {
    let value = required_option(args, flag)?;
    value
        .parse::<f64>()
        .with_context(|| format!("invalid number for {flag}: {value}"))
}

fn optional_f64_option(args: &[String], flag: &str) -> Result<Option<f64>> {
    optional_option(args, flag)?
        .map(|value| {
            value
                .parse::<f64>()
                .with_context(|| format!("invalid number for {flag}: {value}"))
        })
        .transpose()
}

fn value_from_f64(value: f64, flag: &str) -> Result<Value> {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .with_context(|| format!("invalid number for {flag}: {value}"))
}

fn optional_i64_option(args: &[String], flag: &str) -> Result<Option<i64>> {
    optional_option(args, flag)?
        .map(|value| {
            value
                .parse::<i64>()
                .with_context(|| format!("invalid integer for {flag}: {value}"))
        })
        .transpose()
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

fn parse_thread_kind(value: &str) -> Result<ThreadKind> {
    ThreadKind::from_db_str(value).with_context(|| format!("invalid thread kind: {value}"))
}

fn parse_thread(args: &[String]) -> Result<Cli> {
    let Some(subcommand) = args.first() else {
        bail!("missing thread subcommand");
    };
    match subcommand.as_str() {
        "create" => {
            ensure_known_options_with_flags(
                &args[1..],
                &[
                    "--project",
                    "--goal",
                    "--description",
                    "--kind",
                    "--assistant-id",
                    "--db-path",
                ],
                &["--json"],
            )?;
            let rest = &args[1..];
            Ok(Cli {
                command: Command::Thread(ThreadCommand::Create {
                    project: required_option(rest, "--project")?,
                    goal: required_option(rest, "--goal")?,
                    description: optional_option(rest, "--description")?,
                    kind: parse_thread_kind(
                        optional_option(rest, "--kind")?
                            .as_deref()
                            .unwrap_or(ThreadKind::Process.as_str()),
                    )?,
                    assistant_ids: repeated_option(rest, "--assistant-id")?,
                    db_path: optional_option(rest, "--db-path")?,
                    json: has_flag(rest, "--json"),
                }),
            })
        }
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
        "update" => {
            ensure_known_options_with_flags(
                &args[1..],
                &[
                    "--id",
                    "--goal",
                    "--description",
                    "--kind",
                    "--enabled",
                    "--assistant-id",
                    "--db-path",
                ],
                &["--json"],
            )?;
            let rest = &args[1..];
            let assistant_ids = if has_option(rest, "--assistant-id") {
                Some(repeated_option(rest, "--assistant-id")?)
            } else {
                None
            };
            Ok(Cli {
                command: Command::Thread(ThreadCommand::Update {
                    id: required_option(rest, "--id")?,
                    goal: optional_option(rest, "--goal")?,
                    description: optional_option(rest, "--description")?,
                    kind: optional_option(rest, "--kind")?
                        .as_deref()
                        .map(parse_thread_kind)
                        .transpose()?,
                    enabled: optional_option(rest, "--enabled")?
                        .as_deref()
                        .map(parse_config_bool)
                        .transpose()?,
                    assistant_ids,
                    db_path: optional_option(rest, "--db-path")?,
                    json: has_flag(rest, "--json"),
                }),
            })
        }
        "set-stage" => {
            ensure_known_options_with_flags(
                &args[1..],
                &["--thread-id", "--stage-id", "--db-path"],
                &["--json"],
            )?;
            let rest = &args[1..];
            Ok(Cli {
                command: Command::Thread(ThreadCommand::SetStage {
                    thread_id: required_option(rest, "--thread-id")?,
                    stage_id: required_option(rest, "--stage-id")?,
                    db_path: optional_option(rest, "--db-path")?,
                    json: has_flag(rest, "--json"),
                }),
            })
        }
        "link-session" | "unlink-session" => {
            ensure_known_options_with_flags(
                &args[1..],
                &["--thread-id", "--agent", "--session-id", "--db-path"],
                &["--json"],
            )?;
            let rest = &args[1..];
            let thread_id = required_option(rest, "--thread-id")?;
            let agent = parse_agent(&required_option(rest, "--agent")?)?;
            let session_id = required_option(rest, "--session-id")?;
            let db_path = optional_option(rest, "--db-path")?;
            let json = has_flag(rest, "--json");
            let command = if subcommand == "link-session" {
                ThreadCommand::LinkSession {
                    thread_id,
                    agent,
                    session_id,
                    db_path,
                    json,
                }
            } else {
                ThreadCommand::UnlinkSession {
                    thread_id,
                    agent,
                    session_id,
                    db_path,
                    json,
                }
            };
            Ok(Cli {
                command: Command::Thread(command),
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
        "catalog" => {
            ensure_known_options_with_flags(&args[1..], &["--project", "--db-path"], &["--json"])?;
            let rest = &args[1..];
            Ok(Cli {
                command: Command::Stage(StageCommand::Catalog {
                    project: required_option(rest, "--project")?,
                    db_path: optional_option(rest, "--db-path")?,
                    json: has_flag(rest, "--json"),
                }),
            })
        }
        "add" => {
            ensure_known_options_with_flags(
                &args[1..],
                &["--thread-id", "--stage-id", "--assistant-id", "--db-path"],
                &["--json"],
            )?;
            let rest = &args[1..];
            Ok(Cli {
                command: Command::Stage(StageCommand::Add {
                    thread_id: required_option(rest, "--thread-id")?,
                    stage_id: required_option(rest, "--stage-id")?,
                    assistant_ids: repeated_option(rest, "--assistant-id")?,
                    db_path: optional_option(rest, "--db-path")?,
                    json: has_flag(rest, "--json"),
                }),
            })
        }
        "list" => {
            let mut thread_id = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--thread-id" => {
                        i += 1;
                        thread_id = Some(
                            args.get(i)
                                .context("missing value for --thread-id")?
                                .clone(),
                        );
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
        "update" => {
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
                    other => bail!("unknown stage update option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Stage(StageCommand::Update {
                    id: id.context("missing --id")?,
                    status,
                    summary,
                    outcome,
                    db_path,
                    json,
                }),
            })
        }
        "configure" => {
            ensure_known_options_with_flags(
                &args[1..],
                &[
                    "--id",
                    "--assistant-id",
                    "--order",
                    "--enabled",
                    "--db-path",
                ],
                &["--json"],
            )?;
            let rest = &args[1..];
            let assistant_ids = if has_option(rest, "--assistant-id") {
                Some(repeated_option(rest, "--assistant-id")?)
            } else {
                None
            };
            Ok(Cli {
                command: Command::Stage(StageCommand::Configure {
                    id: required_option(rest, "--id")?,
                    assistant_ids,
                    order: optional_i64_option(rest, "--order")?,
                    enabled: optional_option(rest, "--enabled")?
                        .as_deref()
                        .map(parse_config_bool)
                        .transpose()?,
                    db_path: optional_option(rest, "--db-path")?,
                    json: has_flag(rest, "--json"),
                }),
            })
        }
        "link-session" | "unlink-session" => {
            ensure_known_options_with_flags(
                &args[1..],
                &["--stage-id", "--agent", "--session-id", "--db-path"],
                &["--json"],
            )?;
            let rest = &args[1..];
            let stage_id = required_option(rest, "--stage-id")?;
            let agent = parse_agent(&required_option(rest, "--agent")?)?;
            let session_id = required_option(rest, "--session-id")?;
            let db_path = optional_option(rest, "--db-path")?;
            let json = has_flag(rest, "--json");
            let command = if subcommand == "link-session" {
                StageCommand::LinkSession {
                    stage_id,
                    agent,
                    session_id,
                    db_path,
                    json,
                }
            } else {
                StageCommand::UnlinkSession {
                    stage_id,
                    agent,
                    session_id,
                    db_path,
                    json,
                }
            };
            Ok(Cli {
                command: Command::Stage(command),
            })
        }
        "issue" => parse_stage_issue(&args[1..]),
        other => bail!("unknown stage subcommand '{other}'"),
    }
}

fn parse_stage_issue(args: &[String]) -> Result<Cli> {
    let Some(subcommand) = args.first() else {
        bail!("missing stage issue subcommand");
    };
    match subcommand.as_str() {
        "add" => {
            let mut stage_id = None;
            let mut title = None;
            let mut description = None;
            let mut severity = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--stage-id" => {
                        i += 1;
                        stage_id =
                            Some(args.get(i).context("missing value for --stage-id")?.clone());
                    }
                    "--title" => {
                        i += 1;
                        title = Some(args.get(i).context("missing value for --title")?.clone());
                    }
                    "--description" => {
                        i += 1;
                        description = Some(
                            args.get(i)
                                .context("missing value for --description")?
                                .clone(),
                        );
                    }
                    "--severity" => {
                        i += 1;
                        severity =
                            Some(args.get(i).context("missing value for --severity")?.clone());
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown stage issue add option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Stage(StageCommand::Issue(IssueCommand::Add {
                    stage_id: stage_id.context("missing --stage-id")?,
                    title: title.context("missing --title")?,
                    description,
                    severity: severity.context("missing --severity")?,
                    db_path,
                    json,
                })),
            })
        }
        "list" => {
            let mut stage_id = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--stage-id" => {
                        i += 1;
                        stage_id =
                            Some(args.get(i).context("missing value for --stage-id")?.clone());
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown stage issue list option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Stage(StageCommand::Issue(IssueCommand::List {
                    stage_id: stage_id.context("missing --stage-id")?,
                    db_path,
                    json,
                })),
            })
        }
        "set" => {
            let mut id = None;
            let mut status = None;
            let mut severity = None;
            let mut title = None;
            let mut description = None;
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
                    "--severity" => {
                        i += 1;
                        severity =
                            Some(args.get(i).context("missing value for --severity")?.clone());
                    }
                    "--title" => {
                        i += 1;
                        title = Some(args.get(i).context("missing value for --title")?.clone());
                    }
                    "--description" => {
                        i += 1;
                        description = Some(
                            args.get(i)
                                .context("missing value for --description")?
                                .clone(),
                        );
                    }
                    "--db-path" => {
                        i += 1;
                        db_path = Some(args.get(i).context("missing value for --db-path")?.clone());
                    }
                    "--json" => json = true,
                    other => bail!("unknown stage issue set option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Stage(StageCommand::Issue(IssueCommand::Set {
                    id: id.context("missing --id")?,
                    status,
                    severity,
                    title,
                    description,
                    db_path,
                    json,
                })),
            })
        }
        other => bail!("unknown stage issue subcommand '{other}'"),
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
        "pi" => Ok(Agent::Pi),
        "opencode" => Ok(Agent::Opencode),
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
    let data_dir = app_paths::db_data_dir()?;
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
        Ok(store) => match store.init().and_then(|()| store.list_sessions()) {
            Ok(sessions) if !sessions.is_empty() => Ok(sessions),
            Ok(_) => {
                eprintln!(
                    "sessio: session index is empty, falling back to filesystem scan. \
                     Run the Sessio desktop app (or rebuild the index) for faster lookups."
                );
                Ok(crate::agents::sources::list_all())
            }
            Err(e) => {
                eprintln!(
                    "sessio: failed to read session index ({e}), falling back to filesystem scan."
                );
                Ok(crate::agents::sources::list_all())
            }
        },
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
  sessio sessions messages --agent <codex|claude|opencode|pi> [--session-id <id>] [--file-path <path>] [--json]
  sessio thread create --project <projectPathOrId> --goal <text> [--description <text>] [--kind <process|teamwork|brainstorm|debate>] [--assistant-id <assistantId> ...] [--db-path <path>] [--json]
  sessio thread list [--project <path>] [--db-path <path>] [--json]
  sessio thread show --id <threadId> [--db-path <path>] [--json]
  sessio thread update --id <threadId> [--goal <text>] [--description <text>] [--kind <process|teamwork|brainstorm|debate>] [--enabled <true|false>] [--assistant-id <assistantId> ...] [--db-path <path>] [--json]
  sessio thread set-stage --thread-id <threadId> --stage-id <threadStageId> [--db-path <path>] [--json]
  sessio thread link-session --thread-id <threadId> --agent <codex|claude|opencode|pi> --session-id <sessionId> [--db-path <path>] [--json]
  sessio thread unlink-session --thread-id <threadId> --agent <codex|claude|opencode|pi> --session-id <sessionId> [--db-path <path>] [--json]
  sessio stage catalog --project <projectPathOrId> [--db-path <path>] [--json]
  sessio stage add --thread-id <threadId> --stage-id <projectStageId> [--assistant-id <assistantId> ...] [--db-path <path>] [--json]
  sessio stage list --thread-id <threadId> [--db-path <path>] [--json]
  sessio stage show --id <threadStageId> [--db-path <path>] [--json]
  sessio stage configure --id <threadStageId> [--assistant-id <assistantId> ...] [--order <n>] [--enabled <true|false>] [--db-path <path>] [--json]
  sessio stage link-session --stage-id <threadStageId> --agent <codex|claude|opencode|pi> --session-id <sessionId> [--db-path <path>] [--json]
  sessio stage unlink-session --stage-id <threadStageId> --agent <codex|claude|opencode|pi> --session-id <sessionId> [--db-path <path>] [--json]
  sessio stage set-status --id <threadStageId> --status <not_started|in_progress|blocked|needs_review|completed|skipped> [--summary <text>] [--outcome <text>] [--db-path <path>] [--json]
  sessio stage update --id <threadStageId> [--status <not_started|in_progress|blocked|needs_review|completed|skipped>] [--summary <text>] [--outcome <text>] [--db-path <path>] [--json]
  sessio stage issue add --stage-id <threadStageId> --title <text> --severity <low|medium|high|critical> [--description <text>] [--db-path <path>] [--json]
  sessio stage issue list --stage-id <threadStageId> [--db-path <path>] [--json]
  sessio stage issue set --id <issueId> [--status <open|resolved|dismissed>] [--severity <low|medium|high|critical>] [--title <text>] [--description <text>] [--db-path <path>] [--json]
  sessio cu status [--url <mcp-url>] [--token <token>] [--json]
  sessio cu permissions [--url <mcp-url>] [--token <token>] [--json]
  sessio cu grant --permission <screenshots|accessibility> [--url <mcp-url>] [--token <token>] [--json]
  sessio cu list-apps [--days <n>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu start (--app-id <bundleId>|--bundle <bundleId>) [--window-id <id>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu launch-app (--app-id <bundleId>|--bundle <bundleId>) [--window-id <id>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu raise (--app-id <bundleId>|--bundle <bundleId>) [--window-id <id>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu get-app-state [--app-id <bundleId>|--bundle <bundleId>] [--window-id <id>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu click --snapshot-id <id> (<ref>|--element-id <id>|--x <px> --y <px>) [--coord-space <screenshot|screen>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu click-element --snapshot-id <id> (<ref>|--element-id <id>) [--url <mcp-url>] [--token <token>] [--json]
  sessio cu click-at --snapshot-id <id> --x <px> --y <px> [--coord-space <screenshot|screen>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu secondary-click --snapshot-id <id> (<ref>|--element-id <id>|--x <px> --y <px>) [--coord-space <screenshot|screen>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu perform-secondary-action --snapshot-id <id> (<ref>|--element-id <id>|--x <px> --y <px>) [--coord-space <screenshot|screen>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu double-click --snapshot-id <id> --x <px> --y <px> [--coord-space <screenshot|screen>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu drag --snapshot-id <id> --from-x <px> --from-y <px> --to-x <px> --to-y <px> [--coord-space <screenshot|screen>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu set-value --snapshot-id <id> (<ref>|--element-id <id>) --value <text> [--url <mcp-url>] [--token <token>] [--json]
  sessio cu type-text --snapshot-id <id> --text <text> [--url <mcp-url>] [--token <token>] [--json]
  sessio cu press-key --snapshot-id <id> --key <key-or-chord> [--url <mcp-url>] [--token <token>] [--json]
  sessio cu scroll --snapshot-id <id> [<ref>|--element-id <id>] --direction <up|down|left|right> [--amount <n>] [--url <mcp-url>] [--token <token>] [--json]
  sessio cu stop [--url <mcp-url>] [--token <token>] [--json]
  sessio cu call --tool <computer_tool_name> [--args-json <json>] [--url <mcp-url>] [--token <token>] [--json]
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
  cu auto-discovers a running Sessio desktop computer-use broker and attaches a scoped external session token on demand. SESSIO_CU_URL/SESSIO_CU_TOKEN and --url/--token remain available as advanced overrides.
  cu raise is the reliable recovery path for hidden or minimized app windows. If get-app-state reports no visible window, use raise and then retry; do not rely on generic launcher/activation shortcuts for this recovery path.
  sessions list reads from the Sessio index DB by default and falls back to a filesystem scan when the index is empty/unreadable; a stderr warning is printed when the fallback fires.
  memory search omits qmd's raw payload by default; pass --include-raw for debugging.
  memory resolve omits raw JSONL excerpts by default; pass --include-source-excerpt to attach the byte/line range each source points at.
  memory covered-by shows which base record covered a given record, if continuation provenance exists.
  memory base lists records covered by a given base record via record_continuations.
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_thread_create_workflow_command() {
        let cli = parse_args(args(&[
            "thread",
            "create",
            "--project",
            "/tmp/project",
            "--goal",
            "Ship the workflow",
            "--description",
            "Coordinate stages",
            "--kind",
            "process",
            "--assistant-id",
            "assistant-a",
            "--assistant-id",
            "assistant-b",
            "--json",
        ]))
        .unwrap();

        let Command::Thread(ThreadCommand::Create {
            project,
            goal,
            description,
            kind,
            assistant_ids,
            json,
            ..
        }) = cli.command
        else {
            panic!("expected thread create command");
        };
        assert_eq!(project, "/tmp/project");
        assert_eq!(goal, "Ship the workflow");
        assert_eq!(description.as_deref(), Some("Coordinate stages"));
        assert_eq!(kind, ThreadKind::Process);
        assert_eq!(assistant_ids, vec!["assistant-a", "assistant-b"]);
        assert!(json);
    }

    #[test]
    fn parses_thread_link_session_command() {
        let cli = parse_args(args(&[
            "thread",
            "link-session",
            "--thread-id",
            "thread-1",
            "--agent",
            "codex",
            "--session-id",
            "session-1",
            "--json",
        ]))
        .unwrap();

        let Command::Thread(ThreadCommand::LinkSession {
            thread_id,
            agent,
            session_id,
            json,
            ..
        }) = cli.command
        else {
            panic!("expected thread link-session command");
        };
        assert_eq!(thread_id, "thread-1");
        assert_eq!(agent, Agent::Codex);
        assert_eq!(session_id, "session-1");
        assert!(json);
    }

    #[test]
    fn parses_stage_add_configure_and_link_commands() {
        let catalog = parse_args(args(&[
            "stage",
            "catalog",
            "--project",
            "/tmp/project",
            "--json",
        ]))
        .unwrap();
        let Command::Stage(StageCommand::Catalog { project, json, .. }) = catalog.command else {
            panic!("expected stage catalog command");
        };
        assert_eq!(project, "/tmp/project");
        assert!(json);

        let add = parse_args(args(&[
            "stage",
            "add",
            "--thread-id",
            "thread-1",
            "--stage-id",
            "project-stage-1",
            "--assistant-id",
            "assistant-a",
        ]))
        .unwrap();
        let Command::Stage(StageCommand::Add {
            thread_id,
            stage_id,
            assistant_ids,
            ..
        }) = add.command
        else {
            panic!("expected stage add command");
        };
        assert_eq!(thread_id, "thread-1");
        assert_eq!(stage_id, "project-stage-1");
        assert_eq!(assistant_ids, vec!["assistant-a"]);

        let configure = parse_args(args(&[
            "stage",
            "configure",
            "--id",
            "thread-stage-1",
            "--order",
            "2",
            "--enabled",
            "false",
        ]))
        .unwrap();
        let Command::Stage(StageCommand::Configure {
            id, order, enabled, ..
        }) = configure.command
        else {
            panic!("expected stage configure command");
        };
        assert_eq!(id, "thread-stage-1");
        assert_eq!(order, Some(2));
        assert_eq!(enabled, Some(false));

        let link = parse_args(args(&[
            "stage",
            "link-session",
            "--stage-id",
            "thread-stage-1",
            "--agent",
            "claude",
            "--session-id",
            "session-2",
        ]))
        .unwrap();
        let Command::Stage(StageCommand::LinkSession {
            stage_id,
            agent,
            session_id,
            ..
        }) = link.command
        else {
            panic!("expected stage link-session command");
        };
        assert_eq!(stage_id, "thread-stage-1");
        assert_eq!(agent, Agent::Claude);
        assert_eq!(session_id, "session-2");
    }

    #[test]
    fn parses_computer_use_coordinate_command() {
        let cli = parse_args(args(&[
            "cu",
            "click-at",
            "--snapshot-id",
            "snap-1",
            "--x",
            "10",
            "--y",
            "20",
            "--coord-space",
            "screen",
            "--url",
            "http://127.0.0.1:9999/mcp",
            "--token",
            "token",
            "--json",
        ]))
        .unwrap();

        let Command::ComputerUse(ComputerUseCommand::Tool {
            connection,
            name,
            arguments,
        }) = cli.command
        else {
            panic!("expected computer-use command");
        };
        assert_eq!(connection.url.as_deref(), Some("http://127.0.0.1:9999/mcp"));
        assert_eq!(connection.token.as_deref(), Some("token"));
        assert!(connection.json);
        assert_eq!(name, "computer_click_at");
        assert_eq!(arguments["snapshotId"], "snap-1");
        assert_eq!(arguments["x"].as_f64(), Some(10.0));
        assert_eq!(arguments["y"].as_f64(), Some(20.0));
        assert_eq!(arguments["coordSpace"], "screen");
    }

    #[test]
    fn parses_computer_use_unified_click_element_command() {
        let cli = parse_args(args(&[
            "cu",
            "click",
            "--snapshot-id",
            "snap-1",
            "--element-id",
            "el-1",
            "--x",
            "10",
            "--y",
            "20",
            "--url",
            "http://127.0.0.1:9999/mcp",
            "--token",
            "token",
            "--json",
        ]))
        .unwrap();

        let Command::ComputerUse(ComputerUseCommand::Tool {
            name, arguments, ..
        }) = cli.command
        else {
            panic!("expected computer-use command");
        };
        assert_eq!(name, "computer_click");
        assert_eq!(arguments["snapshotId"], "snap-1");
        assert_eq!(arguments["elementId"], "el-1");
        assert!(arguments.get("x").is_none());
        assert!(arguments.get("y").is_none());
    }

    #[test]
    fn parses_computer_use_unified_click_coordinate_command() {
        let cli = parse_args(args(&[
            "cu",
            "click",
            "--snapshot-id",
            "snap-1",
            "--x",
            "10",
            "--y",
            "20",
            "--coord-space",
            "screen",
        ]))
        .unwrap();

        let Command::ComputerUse(ComputerUseCommand::Tool {
            name, arguments, ..
        }) = cli.command
        else {
            panic!("expected computer-use command");
        };
        assert_eq!(name, "computer_click");
        assert_eq!(arguments["snapshotId"], "snap-1");
        assert_eq!(arguments["x"].as_f64(), Some(10.0));
        assert_eq!(arguments["y"].as_f64(), Some(20.0));
        assert_eq!(arguments["coordSpace"], "screen");
    }

    #[test]
    fn parses_computer_use_click_positional_ref_command() {
        let cli = parse_args(args(&["cu", "click", "--snapshot-id", "snap-1", "ax-7"])).unwrap();

        let Command::ComputerUse(ComputerUseCommand::Tool {
            name, arguments, ..
        }) = cli.command
        else {
            panic!("expected computer-use command");
        };
        assert_eq!(name, "computer_click");
        assert_eq!(arguments["snapshotId"], "snap-1");
        assert_eq!(arguments["elementId"], "ax-7");
    }

    #[test]
    fn parses_computer_use_perform_secondary_action_ref_command() {
        let cli = parse_args(args(&[
            "cu",
            "perform-secondary-action",
            "--snapshot-id",
            "snap-1",
            "--ref",
            "ax-2",
        ]))
        .unwrap();

        let Command::ComputerUse(ComputerUseCommand::Tool {
            name, arguments, ..
        }) = cli.command
        else {
            panic!("expected computer-use command");
        };
        assert_eq!(name, "computer_perform_secondary_action");
        assert_eq!(arguments["snapshotId"], "snap-1");
        assert_eq!(arguments["elementId"], "ax-2");
    }

    #[test]
    fn parses_computer_use_scroll_ref_command() {
        let cli = parse_args(args(&[
            "cu",
            "scroll",
            "--snapshot-id",
            "snap-1",
            "ax-3",
            "--direction",
            "down",
            "--amount",
            "400",
        ]))
        .unwrap();

        let Command::ComputerUse(ComputerUseCommand::Tool {
            name, arguments, ..
        }) = cli.command
        else {
            panic!("expected computer-use command");
        };
        assert_eq!(name, "computer_scroll");
        assert_eq!(arguments["snapshotId"], "snap-1");
        assert_eq!(arguments["elementId"], "ax-3");
        assert_eq!(arguments["direction"], "down");
        assert_eq!(arguments["amount"], 400);
    }

    #[test]
    fn parses_computer_use_bundle_alias() {
        let cli = parse_args(args(&["cu", "start", "--bundle", "com.example.app"])).unwrap();

        let Command::ComputerUse(ComputerUseCommand::Tool {
            name, arguments, ..
        }) = cli.command
        else {
            panic!("expected computer-use command");
        };
        assert_eq!(name, "computer_start");
        assert_eq!(arguments["appId"], "com.example.app");
    }

    #[test]
    fn parses_computer_use_raise_command() {
        let cli = parse_args(args(&["cu", "raise", "--bundle", "com.example.app"])).unwrap();

        let Command::ComputerUse(ComputerUseCommand::Tool {
            name, arguments, ..
        }) = cli.command
        else {
            panic!("expected computer-use command");
        };
        assert_eq!(name, "computer_raise_app");
        assert_eq!(arguments["appId"], "com.example.app");
    }

    #[test]
    fn parses_computer_use_list_apps_days_hint() {
        let cli = parse_args(args(&["cu", "list-apps", "--days", "14"])).unwrap();

        let Command::ComputerUse(ComputerUseCommand::Tool {
            name, arguments, ..
        }) = cli.command
        else {
            panic!("expected computer-use command");
        };
        assert_eq!(name, "computer_list_apps");
        assert_eq!(arguments["days"], 14);
    }

    #[test]
    fn computer_use_parser_rejects_unknown_options() {
        let err = parse_args(args(&["cu", "start", "--app-id", "com.example", "--oops"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown cu option '--oops'"), "{err}");
    }

    #[test]
    fn computer_use_target_error_mentions_bundle_alias() {
        let err = parse_args(args(&["cu", "raise"])).unwrap_err().to_string();

        assert!(err.contains("missing --app-id/--bundle"), "{err}");
    }

    #[test]
    fn normalizes_computer_use_url_to_mcp_endpoint() {
        assert_eq!(
            normalize_cu_url("http://127.0.0.1:1234"),
            "http://127.0.0.1:1234/mcp"
        );
        assert_eq!(
            normalize_cu_url("http://127.0.0.1:1234/mcp"),
            "http://127.0.0.1:1234/mcp"
        );
    }
}
