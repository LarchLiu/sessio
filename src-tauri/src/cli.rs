use crate::memory::build::{build_project_memory, default_output_root, MemoryBuildOptions};
use crate::memory::cards::safe_id_part;
use crate::memory::qmd;
use crate::memory::{CardContinuation, MemoryCard, MemoryStore};
use crate::models::Agent;
use crate::providers;
use crate::providers::shared::convert::project_key_for_path_or_name;
use crate::store::sqlite::SqliteStore;
use crate::store::SessionStore;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::env;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug)]
struct Cli {
    command: Command,
}

#[derive(Debug)]
enum Command {
    Sessions(SessionsCommand),
    Memory(MemoryCommand),
    Qmd(QmdCommand),
    Help,
}

#[derive(Debug)]
enum MemoryCommand {
    Build {
        project: String,
        output_root: Option<String>,
        db_path: Option<String>,
        json: bool,
    },
    Resolve {
        card_id: String,
        db_path: Option<String>,
        include_source_excerpt: bool,
        json: bool,
    },
    CoveredBy {
        card_id: String,
        db_path: Option<String>,
        json: bool,
    },
    Base {
        card_id: String,
        db_path: Option<String>,
        json: bool,
    },
    Search {
        project_key: Option<String>,
        project: Option<String>,
        query: String,
        binary: Option<String>,
        index: String,
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
enum QmdCommand {
    Status {
        binary: Option<String>,
        json: bool,
    },
    Sync {
        project_key: String,
        cards_root: String,
        binary: Option<String>,
        index: String,
        embed: bool,
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
    card_id: String,
    title: String,
    summary: Option<String>,
    qmd_path: String,
    score: Option<f64>,
    snippet: Option<String>,
    sources: Vec<crate::memory::MemorySource>,
    continuation: Option<MemoryContinuationSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryBaseHit {
    card_id: String,
    card: Option<MemoryCard>,
    continuation: MemoryContinuationSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryCoveredByResult {
    card_id: String,
    card: MemoryCard,
    base_card_id: Option<String>,
    base_card: Option<MemoryCard>,
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
        Command::Qmd(cmd) => run_qmd(cmd),
        Command::Help => {
            print_help();
            Ok(())
        }
    }
}

fn run_memory(cmd: MemoryCommand) -> Result<()> {
    match cmd {
        MemoryCommand::Build {
            project,
            output_root,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let registry = providers::builtin_providers();
            let output_root = match output_root {
                Some(path) => PathBuf::from(path),
                None => default_output_root()?,
            };
            let summary = build_project_memory(
                &registry,
                &store,
                &MemoryBuildOptions {
                    project_path: PathBuf::from(project),
                    output_root,
                },
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!(
                    "built {} memory cards from {} sources",
                    summary.cards_written, summary.sources_built
                );
            }
            Ok(())
        }
        MemoryCommand::Resolve {
            card_id,
            db_path,
            include_source_excerpt,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let card = store.card_by_id(&card_id)?;
            let sources = store.sources_for_card(&card_id)?;
            let continuation = store.continuation_for_card(&card_id)?;
            let payload_sources: Vec<serde_json::Value> = sources
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
                        "cardId": card_id,
                        "card": card,
                        "sources": payload_sources,
                        "continuation": continuation,
                        "continuationSummary": continuation
                            .as_ref()
                            .map(continuation_summary)
                    }))?
                );
            } else {
                for source in sources {
                    println!(
                        "{}\t{}\t{}",
                        source.agent, source.session_id, source.file_path
                    );
                }
                if let Some(continuation) = continuation {
                    print_continuation_summary(&continuation);
                }
            }
            Ok(())
        }
        MemoryCommand::CoveredBy {
            card_id,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let card = store.card_by_id(&card_id)?;
            let Some(card) = card else {
                bail!("card not found: {card_id}");
            };
            let continuation = store.continuation_for_card(&card_id)?;
            let base_card_id = continuation.as_ref().map(base_card_id_for_continuation);
            let base_card = match base_card_id.as_deref() {
                Some(base_card_id) => store.card_by_id(base_card_id)?,
                None => None,
            };
            let payload = MemoryCoveredByResult {
                card_id,
                card,
                base_card_id,
                base_card,
                continuation: continuation.as_ref().map(continuation_summary),
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else if let Some(continuation) = continuation {
                println!(
                    "base card: {}",
                    base_card_id_for_continuation(&continuation)
                );
                print_continuation_summary(&continuation);
            } else {
                println!("no continuation provenance recorded");
            }
            Ok(())
        }
        MemoryCommand::Base {
            card_id,
            db_path,
            json,
        } => {
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let card = store.card_by_id(&card_id)?;
            let Some(card) = card else {
                bail!("card not found: {card_id}");
            };
            let sources = store.sources_for_card(&card.card_id)?;
            let Some(base_source) = sources.first() else {
                bail!("base card has no source refs: {card_id}");
            };
            let continuations = store
                .continuations_for_base(&base_source.agent, &base_source.session_id)?;
            let hits: Vec<MemoryBaseHit> = continuations
                .into_iter()
                .filter_map(|continuation| {
                    let card_id = continuation.card_id.clone();
                    let summary = continuation_summary(&continuation);
                    let card = store.card_by_id(&card_id).ok().flatten();
                    Some(MemoryBaseHit {
                        card_id,
                        card,
                        continuation: summary,
                    })
                })
                .collect();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "baseCardId": card.card_id,
                        "baseCard": card,
                        "baseSource": base_source,
                        "hits": hits,
                    }))?
                );
            } else {
                println!("base card: {}", card.card_id);
                println!("base source: {} {}", base_source.agent, base_source.session_id);
                for hit in hits {
                    println!(
                        "{}\t{}\t{}",
                        hit.card_id,
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
            binary,
            index,
            db_path,
            include_raw,
            json,
        } => {
            let project_key = resolve_project_key(project_key, project.as_deref())?;
            let store = open_store(db_path.as_deref())?;
            store.init()?;
            let options = qmd::QmdOptions { binary, index };
            let search_result = qmd::search_project(&options, &project_key, &query);
            let (collection, raw, hits, backend_error) = match search_result {
                Ok(result) => {
                    let hits = map_qmd_hits_to_memory(&store, &project_key, &result.raw)?;
                    (result.collection, result.raw, hits, None)
                }
                Err(e) => (
                    qmd::collection_name(&project_key),
                    serde_json::Value::Null,
                    Vec::new(),
                    Some(e.to_string()),
                ),
            };
            if json {
                let mut payload = serde_json::json!({
                    "projectKey": project_key,
                    "query": query,
                    "collection": collection,
                    "hits": hits,
                    "backendError": backend_error,
                });
                if include_raw {
                    payload
                        .as_object_mut()
                        .expect("payload is a JSON object")
                        .insert("raw".to_string(), raw);
                }
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                if let Some(error) = backend_error {
                    println!("memory search backend unavailable: {error}");
                } else if include_raw {
                    println!("{}", serde_json::to_string_pretty(&raw)?);
                } else {
                    for hit in &hits {
                        println!(
                            "{}\t{}\t{}",
                            hit.card_id,
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

fn run_qmd(cmd: QmdCommand) -> Result<()> {
    match cmd {
        QmdCommand::Status { binary, json } => {
            let status = qmd::qmd_status(binary.as_deref());
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else if status.available {
                println!(
                    "qmd available: {}{}",
                    status.binary.as_deref().unwrap_or("qmd"),
                    status
                        .version
                        .as_deref()
                        .map(|v| format!(" ({v})"))
                        .unwrap_or_default()
                );
            } else {
                println!(
                    "qmd unavailable: {}",
                    status.error.as_deref().unwrap_or("unknown error")
                );
            }
            Ok(())
        }
        QmdCommand::Sync {
            project_key,
            cards_root,
            binary,
            index,
            embed,
            json,
        } => {
            let options = qmd::QmdOptions { binary, index };
            let ensure = qmd::ensure_project_collection(
                &options,
                &project_key,
                &PathBuf::from(&cards_root),
            )?;
            let update = qmd::update_index(&options)?;
            let embed_result = if embed {
                Some(qmd::embed_index(&options)?)
            } else {
                None
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "projectKey": project_key,
                        "cardsRoot": cards_root,
                        "collection": qmd::collection_name(&project_key),
                        "ensure": ensure,
                        "update": update,
                        "embed": embed_result
                    }))?
                );
            } else {
                println!(
                    "synced qmd collection {}",
                    qmd::collection_name(&project_key)
                );
            }
            Ok(())
        }
    }
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
            let path = PathBuf::from(&file_path);
            let messages = match agent {
                Agent::Codex => providers::codex::parser::read_messages(&path)?,
                Agent::Claude => providers::claude::parser::read_messages(&path)?,
                Agent::Gemini => providers::gemini::parser::read_messages(
                    &path,
                    session_id
                        .as_deref()
                        .context("gemini messages require --session-id")?,
                )?,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&messages)?);
            } else {
                for m in messages {
                    println!("[{}]\n{}\n", m.role, m.text);
                }
            }
            Ok(())
        }
    }
}

fn resolve_session_file(agent: Agent, session_id: Option<&str>) -> Result<String> {
    let session_id = session_id.context("missing --session-id or --file-path")?;
    providers::list_all()
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
        "qmd" => parse_qmd(&args[1..]),
        other => bail!("unknown command '{other}'"),
    }
}

fn parse_memory(args: &[String]) -> Result<Cli> {
    let Some(subcommand) = args.first() else {
        bail!("missing memory subcommand");
    };
    match subcommand.as_str() {
        "build" => {
            let mut project = None;
            let mut output_root = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--project" => {
                        i += 1;
                        project = Some(args.get(i).context("missing value for --project")?.clone());
                    }
                    "--output-root" => {
                        i += 1;
                        output_root = Some(
                            args.get(i)
                                .context("missing value for --output-root")?
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
                    output_root,
                    db_path,
                    json,
                }),
            })
        }
        "base" => {
            let mut card_id = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--card-id" => {
                        i += 1;
                        card_id = Some(args.get(i).context("missing value for --card-id")?.clone());
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
                    card_id: card_id.context("missing --card-id")?,
                    db_path,
                    json,
                }),
            })
        }
        "resolve" => {
            let mut card_id = None;
            let mut db_path = None;
            let mut include_source_excerpt = false;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--card-id" => {
                        i += 1;
                        card_id = Some(args.get(i).context("missing value for --card-id")?.clone());
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
                    card_id: card_id.context("missing --card-id")?,
                    db_path,
                    include_source_excerpt,
                    json,
                }),
            })
        }
        "covered-by" => {
            let mut card_id = None;
            let mut db_path = None;
            let mut json = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--card-id" => {
                        i += 1;
                        card_id = Some(args.get(i).context("missing value for --card-id")?.clone());
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
                    card_id: card_id.context("missing --card-id")?,
                    db_path,
                    json,
                }),
            })
        }
        "search" => {
            let mut project_key = None;
            let mut project = None;
            let mut query_parts = Vec::new();
            let mut binary = None;
            let mut index = "sessio".to_string();
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
                    "--binary" => {
                        i += 1;
                        binary = Some(args.get(i).context("missing value for --binary")?.clone());
                    }
                    "--index" => {
                        i += 1;
                        index = args.get(i).context("missing value for --index")?.clone();
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
                    binary,
                    index,
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

fn parse_qmd(args: &[String]) -> Result<Cli> {
    let Some(subcommand) = args.first() else {
        bail!("missing qmd subcommand");
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
                    other => bail!("unknown qmd status option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Qmd(QmdCommand::Status { binary, json }),
            })
        }
        "sync" => {
            let mut project_key = None;
            let mut cards_root = None;
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
                    "--cards-root" => {
                        i += 1;
                        cards_root = Some(
                            args.get(i)
                                .context("missing value for --cards-root")?
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
                    other => bail!("unknown qmd sync option '{other}'"),
                }
                i += 1;
            }
            Ok(Cli {
                command: Command::Qmd(QmdCommand::Sync {
                    project_key: project_key.context("missing --project-key")?,
                    cards_root: cards_root.context("missing --cards-root")?,
                    binary,
                    index,
                    embed,
                    json,
                }),
            })
        }
        other => bail!("unknown qmd subcommand '{other}'"),
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
                    Ok(providers::list_all())
                }
                Err(e) => {
                    eprintln!("sessio: failed to read session index ({e}), falling back to filesystem scan.");
                    Ok(providers::list_all())
                }
            }
        }
        Err(e) => {
            eprintln!(
                "sessio: failed to open session index ({e}), falling back to filesystem scan."
            );
            Ok(providers::list_all())
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

fn map_qmd_hits_to_memory(
    store: &dyn MemoryStore,
    project_key: &str,
    raw: &serde_json::Value,
) -> Result<Vec<MemorySearchHit>> {
    let cards = store
        .list_project_cards(project_key)?
        .into_iter()
        .filter(|card| card.available)
        .collect::<Vec<_>>();
    let candidates = qmd_hit_candidates(raw);
    let mut out = Vec::new();
    for candidate in candidates {
        let Some(card) = cards.iter().find(|card| {
            candidate
                .card_id
                .as_deref()
                .map(|id| id == card.card_id)
                .unwrap_or(false)
                || candidate
                    .path
                    .as_deref()
                    .map(|path| path_matches_card(path, &card.card_id, &card.qmd_path))
                    .unwrap_or(false)
        }) else {
            continue;
        };
        if out
            .iter()
            .any(|hit: &MemorySearchHit| hit.card_id == card.card_id)
        {
            continue;
        }
        out.push(MemorySearchHit {
            card_id: card.card_id.clone(),
            title: card.title.clone(),
            summary: card.summary.clone(),
            qmd_path: card.qmd_path.clone(),
            score: candidate.score,
            snippet: candidate.snippet,
            sources: store.sources_for_card(&card.card_id)?,
            continuation: store
                .continuation_for_card(&card.card_id)?
                .as_ref()
                .map(continuation_summary),
        });
    }
    Ok(out)
}

fn continuation_summary(continuation: &CardContinuation) -> MemoryContinuationSummary {
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

fn base_card_id_for_continuation(continuation: &CardContinuation) -> String {
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

fn print_continuation_summary(continuation: &CardContinuation) {
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

#[derive(Debug, Default)]
struct QmdHitCandidate {
    card_id: Option<String>,
    path: Option<String>,
    score: Option<f64>,
    snippet: Option<String>,
}

fn qmd_hit_candidates(raw: &serde_json::Value) -> Vec<QmdHitCandidate> {
    let mut out = Vec::new();
    collect_qmd_hit_candidates(raw, &mut out);
    out
}

fn collect_qmd_hit_candidates(value: &serde_json::Value, out: &mut Vec<QmdHitCandidate>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_qmd_hit_candidates(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            let candidate = QmdHitCandidate {
                card_id: first_string(map, &["cardId", "card_id", "id"])
                    .and_then(card_id_from_text),
                path: first_string(map, &["path", "file", "filePath", "filepath", "source"]),
                score: first_number(map, &["score", "rank", "similarity"]),
                snippet: first_string(map, &["snippet", "text", "content", "preview"]),
            };
            if candidate.card_id.is_some() || candidate.path.is_some() {
                out.push(candidate);
            }
            for key in ["results", "hits", "documents", "items", "matches"] {
                if let Some(child) = map.get(key) {
                    collect_qmd_hit_candidates(child, out);
                }
            }
        }
        _ => {}
    }
}

fn first_string(map: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = map.get(*key) else {
            continue;
        };
        if let Some(s) = value.as_str() {
            return Some(s.to_string());
        }
    }
    None
}

fn first_number(map: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(n) = map.get(*key).and_then(|value| value.as_f64()) {
            return Some(n);
        }
    }
    None
}

fn card_id_from_text(text: String) -> Option<String> {
    let path = Path::new(&text);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(&text);
    if stem.starts_with("sessio-") {
        Some(stem.to_string())
    } else {
        None
    }
}

fn path_matches_card(path: &str, card_id: &str, qmd_path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let dashed_card_id = card_id.replace('_', "-");
    let dashed_qmd_path = qmd_path.replace('_', "-");
    let slashless_qmd_path = qmd_path.replace('/', "-");
    normalized.ends_with(&format!("{card_id}.md"))
        || normalized.ends_with(qmd_path)
        || normalized.ends_with(&format!("{dashed_card_id}.md"))
        || normalized.ends_with(&dashed_qmd_path)
        || normalized.ends_with(&slashless_qmd_path)
        || normalized.contains(&format!("/cards/{card_id}.md"))
        || normalized.contains(&format!("/cards/{dashed_card_id}.md"))
}

fn print_help() {
    println!(
        r#"sessio

Usage:
  sessio sessions list [--project <path>] [--db-path <path>] [--json]
  sessio sessions messages --agent <codex|claude|gemini> [--session-id <id>] [--file-path <path>] [--json]
  sessio memory build --project <path> [--output-root <path>] [--db-path <path>] [--json]
  sessio memory covered-by --card-id <id> [--db-path <path>] [--json]
  sessio memory base --card-id <id> [--db-path <path>] [--json]
  sessio memory search (--project <path>|--project-key <key>) <query> [--index sessio] [--binary <path>] [--db-path <path>] [--include-raw] [--json]
  sessio memory resolve --card-id <id> [--db-path <path>] [--include-source-excerpt] [--json]
  sessio memory jobs --project-key <key> [--status <status>] [--db-path <path>] [--json]
  sessio qmd status [--binary <path>] [--json]
  sessio qmd sync --project-key <key> --cards-root <path> [--index sessio] [--binary <path>] [--embed] [--json]

Notes:
  --json emits stable machine-readable output for skills and agents.
  sessions list reads from the Sessio index DB by default and falls back to a filesystem scan when the index is empty/unreadable; a stderr warning is printed when the fallback fires.
  memory search omits qmd's raw payload by default; pass --include-raw for debugging.
  memory resolve omits raw JSONL excerpts by default; pass --include-source-excerpt to attach the byte/line range each source points at (Codex / Claude today; Gemini is session-level only).
  Gemini message lookup requires --session-id because multiple sessions can share one logs.json.
  memory covered-by shows which base card covered a given card, if continuation provenance exists.
  memory base lists cards covered by a given base card via card_continuations.
"#
    );
}
