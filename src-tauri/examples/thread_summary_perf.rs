use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use app_lib::store::sqlite::SqliteStore;
use app_lib::store::SessionStore;
use app_lib::thread_chat_summary::ThreadChatSummaryCache;

#[derive(Debug)]
struct Args {
    db_path: Option<PathBuf>,
    project_id: Option<String>,
    thread_id: Option<String>,
    iterations: usize,
    in_place: bool,
}

fn main() -> Result<()> {
    let args = parse_args(env::args().skip(1).collect())?;
    let source_db_path = source_db_path(args.db_path.as_deref())?;
    let working_db_path = prepare_db_path(&source_db_path, args.in_place)?;
    let store = Arc::new(SqliteStore::open(&working_db_path)?);
    store.init()?;
    let cache = ThreadChatSummaryCache::new(store.clone());

    let projects = store.list_projects()?;
    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    let project_id = args
        .project_id
        .clone()
        .or_else(|| projects.first().map(|project| project.id.clone()))
        .context("no project id available")?;
    let threads = store.list_threads(&project_id)?;
    let thread_id = args
        .thread_id
        .clone()
        .or_else(|| threads.first().map(|thread| thread.id.clone()));

    println!("Source DB: {}", source_db_path.display());
    if working_db_path != source_db_path {
        println!("Working DB: {}", working_db_path.display());
    }
    println!("Project: {project_id}");
    println!("Threads in project: {}", threads.len());
    if let Some(thread_id) = &thread_id {
        println!("Replay thread: {thread_id}");
    } else {
        println!("Replay thread: <none>");
    }
    println!("Iterations: {}", args.iterations);
    println!();

    measure("cache.warm", args.iterations, || cache.warm())?;
    measure("refresh_all", args.iterations, || cache.refresh_all())?;
    measure("refresh_project", args.iterations, || cache.refresh_project(&project_id))?;
    measure("list_project", args.iterations, || cache.list_project(&project_id).map(|_| ()))?;

    if let Some(thread_id) = &thread_id {
        measure("get_thread_replay", args.iterations, || {
            store.get_thread_replay(thread_id).map(|_| ())
        })?;
    }

    Ok(())
}

fn measure<F>(label: &str, iterations: usize, mut op: F) -> Result<()>
where
    F: FnMut() -> Result<()>,
{
    let mut total = Duration::ZERO;
    let mut best = Duration::MAX;
    let mut worst = Duration::ZERO;
    for _ in 0..iterations {
        let start = Instant::now();
        op()?;
        let elapsed = start.elapsed();
        total += elapsed;
        best = best.min(elapsed);
        worst = worst.max(elapsed);
    }
    let average = total / iterations as u32;
    println!(
        "{label:<18} avg={:>8} ms  best={:>8} ms  worst={:>8} ms",
        average.as_millis(),
        best.as_millis(),
        worst.as_millis()
    );
    Ok(())
}

fn parse_args(args: Vec<String>) -> Result<Args> {
    let mut db_path = None;
    let mut project_id = None;
    let mut thread_id = None;
    let mut iterations = 5usize;
    let mut in_place = false;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--db-path" => {
                i += 1;
                db_path = Some(PathBuf::from(
                    args.get(i).context("missing value for --db-path")?,
                ));
            }
            "--project-id" => {
                i += 1;
                project_id = Some(args.get(i).context("missing value for --project-id")?.clone());
            }
            "--thread-id" => {
                i += 1;
                thread_id = Some(args.get(i).context("missing value for --thread-id")?.clone());
            }
            "--iterations" => {
                i += 1;
                iterations = args
                    .get(i)
                    .context("missing value for --iterations")?
                    .parse()
                    .context("invalid --iterations value")?;
                if iterations == 0 {
                    anyhow::bail!("--iterations must be greater than 0");
                }
            }
            "--in-place" => {
                in_place = true;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
        i += 1;
    }
    Ok(Args {
        db_path,
        project_id,
        thread_id,
        iterations,
        in_place,
    })
}

fn print_help() {
    println!(
        "Usage: cargo run --example thread_summary_perf --manifest-path src-tauri/Cargo.toml -- [options]\n\
         \n\
         Options:\n\
         \t--db-path <path>         Override the Sessio SQLite DB path\n\
         \t--project-id <id>        Measure a specific project for refresh_project/list_project\n\
         \t--thread-id <id>         Measure a specific thread for get_thread_replay\n\
         \t--iterations <count>     Number of iterations per operation (default: 5)\n\
         \t--in-place               Open the source DB directly instead of copying it to a temp file\n\
         \t-h, --help               Show this help"
    );
}

fn source_db_path(db_path: Option<&std::path::Path>) -> Result<PathBuf> {
    if let Some(db_path) = db_path {
        return Ok(db_path.to_path_buf());
    }
    let data_dir = dirs::home_dir()
        .context("home directory unavailable")?
        .join(".sessio")
        .join("db-data");
    std::fs::create_dir_all(&data_dir).ok();
    Ok(data_dir.join("sessio-index.db"))
}

fn prepare_db_path(source: &std::path::Path, in_place: bool) -> Result<PathBuf> {
    if in_place {
        return Ok(source.to_path_buf());
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let copied = env::temp_dir().join(format!(
        "sessio-thread-summary-perf-{}-{}.db",
        std::process::id(),
        nanos
    ));
    std::fs::copy(source, &copied).with_context(|| {
        format!(
            "copy source DB {} to temporary benchmark DB {}",
            source.display(),
            copied.display()
        )
    })?;
    Ok(copied)
}
