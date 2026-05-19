use crate::memory::artifacts::MemoryArtifactSink;
use crate::memory::{
    MemoryBackendHit, MemoryBackendSearchResult, MemoryBackendStatus, MemoryIndexBackend,
    MemoryRecord, MemorySearchOptions, MemorySyncReport,
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QmdStatus {
    pub available: bool,
    pub binary: Option<String>,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct QmdOptions {
    pub binary: Option<String>,
    pub index: String,
}

impl Default for QmdOptions {
    fn default() -> Self {
        Self {
            binary: None,
            index: "sessio".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QmdCommandResult {
    pub ok: bool,
    pub command: Vec<String>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QmdSearchResult {
    pub collection: String,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct QmdBackend {
    options: QmdOptions,
    artifacts_root: PathBuf,
    auto_embed: bool,
}

impl QmdBackend {
    pub fn new(options: QmdOptions, artifacts_root: PathBuf) -> Self {
        Self {
            options,
            artifacts_root,
            auto_embed: false,
        }
    }

    pub fn with_auto_embed(mut self, auto_embed: bool) -> Self {
        self.auto_embed = auto_embed;
        self
    }

    pub fn options(&self) -> &QmdOptions {
        &self.options
    }

    fn project_sessions_root(&self, project_key: &str) -> PathBuf {
        self.artifacts_root
            .join(self.name())
            .join(project_key)
            .join("sessions")
    }
}

impl MemoryIndexBackend for QmdBackend {
    fn name(&self) -> &'static str {
        "qmd"
    }

    fn status(&self) -> MemoryBackendStatus {
        let status = qmd_status(self.options.binary.as_deref());
        let details = serde_json::json!({
            "binary": status.binary,
            "version": status.version,
        });
        MemoryBackendStatus {
            backend: self.name().to_string(),
            available: status.available,
            error: status.error,
            details: Some(details),
        }
    }

    fn sync_project(
        &self,
        project_key: &str,
        records: &[MemoryRecord],
        _artifacts: &dyn MemoryArtifactSink,
    ) -> Result<MemorySyncReport> {
        let sessions_root = self.project_sessions_root(project_key);
        ensure_project_collection(&self.options, project_key, &sessions_root)?;
        update_index(&self.options)?;
        let mut errors = Vec::new();
        if self.auto_embed {
            if let Err(e) = embed_index(&self.options) {
                errors.push(format!("qmd embed failed: {e}"));
            }
        }
        Ok(MemorySyncReport {
            backend: self.name().to_string(),
            project_key: project_key.to_string(),
            synced_records: records.iter().filter(|record| record.available).count(),
            removed_records: records.iter().filter(|record| !record.available).count(),
            errors,
        })
    }

    fn remove_project(&self, project_key: &str) -> Result<MemorySyncReport> {
        remove_collection(&self.options, &collection_name(project_key))?;
        Ok(MemorySyncReport {
            backend: self.name().to_string(),
            project_key: project_key.to_string(),
            synced_records: 0,
            removed_records: 0,
            errors: Vec::new(),
        })
    }

    fn search(
        &self,
        project_key: &str,
        query: &str,
        options: MemorySearchOptions,
    ) -> Result<MemoryBackendSearchResult> {
        let result = search_project(&self.options, project_key, query)?;
        let hits = qmd_hit_candidates(&result.raw)
            .into_iter()
            .map(|candidate| MemoryBackendHit {
                record_id: candidate.record_id,
                artifact_uri: candidate.artifact_uri,
                score: candidate.score,
                snippet: candidate.snippet,
            })
            .collect();
        Ok(MemoryBackendSearchResult {
            backend: self.name().to_string(),
            hits,
            raw: options.include_raw.then_some(result.raw),
        })
    }
}

pub fn qmd_status(configured_binary: Option<&str>) -> QmdStatus {
    let Some(binary) = find_qmd_binary(configured_binary) else {
        return QmdStatus {
            available: false,
            binary: configured_binary.map(String::from),
            version: None,
            error: Some("qmd binary not found in PATH".to_string()),
        };
    };

    match Command::new(&binary).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            QmdStatus {
                available: true,
                binary: Some(binary.to_string_lossy().to_string()),
                version: if version.is_empty() {
                    None
                } else {
                    Some(version)
                },
                error: None,
            }
        }
        Ok(output) => QmdStatus {
            available: false,
            binary: Some(binary.to_string_lossy().to_string()),
            version: None,
            error: Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        },
        Err(e) => QmdStatus {
            available: false,
            binary: Some(binary.to_string_lossy().to_string()),
            version: None,
            error: Some(e.to_string()),
        },
    }
}

pub fn ensure_project_collection(
    options: &QmdOptions,
    project_key: &str,
    cards_root: &Path,
) -> Result<QmdCommandResult> {
    let collection = collection_name(project_key);
    let add = run_qmd_command_allow_existing(
        options,
        &[
            "--index",
            options.index.as_str(),
            "collection",
            "add",
            &cards_root.to_string_lossy(),
            "--name",
            &collection,
            "--mask",
            "**/*.md",
        ],
    )?;
    if !add.result.ok && add.already_exists {
        remove_collection(options, &collection)?;
        return run_qmd_command(
            options,
            &[
                "--index",
                options.index.as_str(),
                "collection",
                "add",
                &cards_root.to_string_lossy(),
                "--name",
                &collection,
                "--mask",
                "**/*.md",
            ],
        );
    }
    Ok(add.into_result())
}

pub fn update_index(options: &QmdOptions) -> Result<QmdCommandResult> {
    run_qmd_command(options, &["--index", options.index.as_str(), "update"])
}

pub fn remove_collection(options: &QmdOptions, collection: &str) -> Result<QmdCommandResult> {
    run_qmd_command(
        options,
        &[
            "--index",
            options.index.as_str(),
            "collection",
            "remove",
            collection,
        ],
    )
}

pub fn embed_index(options: &QmdOptions) -> Result<QmdCommandResult> {
    run_qmd_command(options, &["--index", options.index.as_str(), "embed"])
}

pub fn search_project(
    options: &QmdOptions,
    project_key: &str,
    query: &str,
) -> Result<QmdSearchResult> {
    let collection = collection_name(project_key);
    let result = run_qmd_command(
        options,
        &[
            "--index",
            options.index.as_str(),
            "search",
            query,
            "-c",
            &collection,
            "--json",
        ],
    )?;
    let raw = if result.stdout.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&result.stdout)
            .with_context(|| format!("parse qmd query JSON: {}", result.stdout))?
    };
    Ok(QmdSearchResult { collection, raw })
}

pub fn collection_name(project_key: &str) -> String {
    format!("sessio-{project_key}")
}

#[derive(Debug, Default)]
struct QmdHitCandidate {
    record_id: Option<String>,
    artifact_uri: Option<String>,
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
            let path = first_string(map, &["path", "file", "filePath", "filepath", "source"]);
            let candidate = QmdHitCandidate {
                record_id: first_string(map, &["recordId", "record_id", "cardId", "card_id", "id"])
                    .and_then(record_id_from_text)
                    .or_else(|| path.clone().and_then(record_id_from_text)),
                artifact_uri: path,
                score: first_number(map, &["score", "rank", "similarity"]),
                snippet: first_string(map, &["snippet", "text", "content", "preview"]),
            };
            if candidate.record_id.is_some() || candidate.artifact_uri.is_some() {
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

fn record_id_from_text(text: String) -> Option<String> {
    let path = Path::new(&text);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(&text);
    if stem.starts_with("sessio-") {
        Some(stem.to_string())
    } else {
        None
    }
}

fn run_qmd_command(options: &QmdOptions, args: &[&str]) -> Result<QmdCommandResult> {
    let output = run_qmd_command_raw(options, args)?;
    if !output.result.ok {
        bail_qmd_failed(&output.result)?;
    }
    Ok(output.result)
}

struct QmdRawCommandResult {
    result: QmdCommandResult,
    already_exists: bool,
}

impl QmdRawCommandResult {
    fn into_result(self) -> QmdCommandResult {
        self.result
    }
}

fn run_qmd_command_allow_existing(
    options: &QmdOptions,
    args: &[&str],
) -> Result<QmdRawCommandResult> {
    let output = run_qmd_command_raw(options, args)?;
    if !output.result.ok && !output.already_exists {
        bail_qmd_failed(&output.result)?;
    }
    Ok(output)
}

fn run_qmd_command_raw(options: &QmdOptions, args: &[&str]) -> Result<QmdRawCommandResult> {
    let binary =
        find_qmd_binary(options.binary.as_deref()).context("qmd binary not found in PATH")?;
    let mut command_process = Command::new(&binary);
    command_process.args(args);
    if let Some(path) = path_with_binary_dir_first(&binary) {
        command_process.env("PATH", path);
    }
    let command = std::iter::once(binary.to_string_lossy().to_string())
        .chain(args.iter().map(|s| s.to_string()))
        .collect::<Vec<_>>();
    let timeout = qmd_timeout();
    command_process
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command_process.spawn()?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "qmd command timed out after {}s: {}",
                timeout.as_secs(),
                command.join(" ")
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    let mut result = QmdCommandResult {
        ok: status.success(),
        command,
        stdout: stdout.trim().to_string(),
        stderr: stderr.trim().to_string(),
    };
    let already_exists = !result.ok && is_existing_collection_error(args, &result);
    if already_exists {
        result.ok = false;
    }
    Ok(QmdRawCommandResult {
        result,
        already_exists,
    })
}

fn bail_qmd_failed(result: &QmdCommandResult) -> Result<()> {
    bail!(
        "qmd command failed: {}",
        if result.stderr.is_empty() {
            result.stdout.as_str()
        } else {
            result.stderr.as_str()
        }
    )
}

fn qmd_timeout() -> Duration {
    env::var("SESSIO_QMD_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(30))
}

fn is_existing_collection_error(args: &[&str], result: &QmdCommandResult) -> bool {
    args.windows(2)
        .any(|window| window == ["collection", "add"])
        && (result.stderr.contains("already exists") || result.stdout.contains("already exists"))
}

fn path_with_binary_dir_first(binary: &Path) -> Option<std::ffi::OsString> {
    let dir = binary.parent()?;
    let existing = env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(env::split_paths(&existing));
    env::join_paths(paths).ok()
}

fn find_qmd_binary(configured_binary: Option<&str>) -> Option<PathBuf> {
    if let Some(binary) = configured_binary {
        let path = PathBuf::from(binary);
        if is_executable_candidate(&path) {
            return Some(path);
        }
        return None;
    }
    find_on_path("qmd")
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if is_executable_candidate(&candidate) {
            return Some(candidate);
        }
        #[cfg(target_os = "windows")]
        {
            let candidate = dir.join(format!("{name}.cmd"));
            if is_executable_candidate(&candidate) {
                return Some(candidate);
            }
            let candidate = dir.join(format!("{name}.exe"));
            if is_executable_candidate(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_executable_candidate(path: &Path) -> bool {
    path.is_file()
}
