pub mod artifacts;
pub mod build;
pub mod cards;
pub mod dedupe;
pub mod hash;
pub mod normalize;
pub mod qmd;
pub mod resolve;
pub mod service;

use crate::memory::artifacts::MemoryArtifactSink;
use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::providers::types::SourceLocation;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecord {
    pub record_id: String,
    pub project_key: String,
    pub canonical_hash: String,
    pub simhash: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub body: String,
    pub kind: MemoryRecordKind,
    pub available: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryRecordKind {
    Session,
}

impl MemoryRecordKind {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Session => "session",
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self> {
        match value {
            "session" => Ok(Self::Session),
            other => Err(anyhow::anyhow!("unknown MemoryRecordKind '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryArtifact {
    pub record_id: String,
    pub backend: String,
    pub artifact_uri: String,
    pub content_hash: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackendStatus {
    pub backend: String,
    pub available: bool,
    pub error: Option<String>,
    // Optional backend-specific diagnostics. qmd uses this to surface
    // its binary path and reported version without leaking qmd types
    // into the backend-neutral trait.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchOptions {
    pub include_raw: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackendHit {
    pub record_id: Option<String>,
    pub artifact_uri: Option<String>,
    pub score: Option<f64>,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackendSearchResult {
    pub backend: String,
    pub hits: Vec<MemoryBackendHit>,
    pub raw: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySyncReport {
    pub backend: String,
    pub project_key: String,
    pub synced_records: usize,
    pub removed_records: usize,
    pub errors: Vec<String>,
}

pub trait MemoryIndexBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn status(&self) -> MemoryBackendStatus;

    // The `records` slice carries the full project record set (available
    // and unavailable). Backends that need the corpus may inspect it; those
    // that index by scanning the artifact sink (e.g. qmd) can ignore it.
    // The slice is provided so backends never have to call back into the
    // repository themselves.
    fn sync_project(
        &self,
        project_key: &str,
        records: &[MemoryRecord],
        artifacts: &dyn MemoryArtifactSink,
    ) -> Result<MemorySyncReport>;

    fn remove_project(&self, project_key: &str) -> Result<MemorySyncReport>;

    fn search(
        &self,
        project_key: &str,
        query: &str,
        options: MemorySearchOptions,
    ) -> Result<MemoryBackendSearchResult>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySource {
    pub record_id: String,
    pub agent: String,
    pub session_id: String,
    pub file_path: String,
    pub location: SourceLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnFingerprint {
    pub project_key: String,
    pub agent: String,
    pub session_id: String,
    pub turn_index: usize,
    pub role: String,
    pub canonical_hash: String,
    pub text_len: usize,
    pub location: SourceLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnFingerprintCandidate {
    pub agent: String,
    pub session_id: String,
    pub file_path: String,
    pub shared_hashes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryJob {
    pub id: Option<i64>,
    pub project_key: String,
    pub backend: String,
    pub scope: String,
    pub kind: String,
    pub status: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTimeInfo {
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordContinuation {
    pub record_id: String,
    pub project_key: String,
    pub candidate_agent: String,
    pub candidate_session_id: String,
    pub candidate_file_path: String,
    pub base_agent: String,
    pub base_session_id: String,
    pub base_file_path: String,
    pub base_start_turn_index: usize,
    pub base_start_line_start: Option<u64>,
    pub base_start_byte_start: Option<u64>,
    pub base_end_turn_index: usize,
    pub base_end_line_end: Option<u64>,
    pub base_end_byte_end: Option<u64>,
    pub candidate_trim_turn_start: usize,
    pub candidate_trim_line_start: Option<u64>,
    pub candidate_trim_byte_start: Option<u64>,
    pub updated_at: i64,
}

pub trait MemoryStore: Send + Sync {
    fn upsert_record(&self, record: &MemoryRecord) -> Result<()>;
    fn upsert_memory_artifact(&self, artifact: &MemoryArtifact) -> Result<()>;
    fn artifact_for_record(&self, record_id: &str, backend: &str)
        -> Result<Option<MemoryArtifact>>;
    fn remove_memory_artifact(&self, record_id: &str, backend: &str) -> Result<()>;
    fn replace_record_sources(&self, record_id: &str, sources: &[MemorySource]) -> Result<()>;
    fn replace_record_continuation(
        &self,
        record_id: &str,
        continuation: Option<&RecordContinuation>,
    ) -> Result<()>;
    fn list_records_for_source(
        &self,
        agent: &str,
        session_id: &str,
        file_path: &str,
    ) -> Result<Vec<MemoryRecord>>;
    fn mark_record_unavailable(&self, record_id: &str) -> Result<()>;
    fn mark_source_records_unavailable(
        &self,
        agent: &str,
        session_id: &str,
        file_path: &str,
    ) -> Result<()>;
    fn list_project_records(&self, project_key: &str) -> Result<Vec<MemoryRecord>>;
    fn record_by_id(&self, record_id: &str) -> Result<Option<MemoryRecord>>;
    fn sources_for_record(&self, record_id: &str) -> Result<Vec<MemorySource>>;
    fn continuation_for_record(&self, record_id: &str) -> Result<Option<RecordContinuation>>;
    fn continuations_for_base(
        &self,
        base_agent: &str,
        base_session_id: &str,
    ) -> Result<Vec<RecordContinuation>>;
    fn invalidate_continuations_referencing_base(
        &self,
        base_agent: &str,
        base_session_id: &str,
    ) -> Result<Vec<RecordContinuation>>;
    fn replace_turn_fingerprints(
        &self,
        project_key: &str,
        agent: &str,
        session_id: &str,
        fingerprints: &[TurnFingerprint],
    ) -> Result<()>;
    fn list_turn_fingerprints(
        &self,
        project_key: &str,
        agent: &str,
        session_id: &str,
    ) -> Result<Vec<TurnFingerprint>>;
    fn find_turn_fingerprint_candidates(
        &self,
        project_key: &str,
        exclude_agent: &str,
        exclude_session_id: &str,
        canonical_hashes: &[&str],
        limit: usize,
    ) -> Result<Vec<TurnFingerprintCandidate>>;
    fn session_time_info(&self, agent: &str, session_id: &str) -> Result<Option<SessionTimeInfo>>;
    fn record_memory_job(
        &self,
        project_key: &str,
        backend: &str,
        scope: &str,
        kind: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<()>;
    fn list_memory_jobs(&self, project_key: &str, status: Option<&str>) -> Result<Vec<MemoryJob>>;
}
