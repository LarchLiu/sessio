pub mod build;
pub mod cards;
pub mod dedupe;
pub mod hash;
pub mod normalize;
pub mod qmd;
pub mod resolve;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::providers::types::SourceLocation;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCard {
    pub card_id: String,
    pub project_key: String,
    pub canonical_hash: String,
    pub simhash: Option<String>,
    pub qmd_path: String,
    pub title: String,
    pub summary: Option<String>,
    pub body: String,
    pub available: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySource {
    pub card_id: String,
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
    pub scope: String,
    pub kind: String,
    pub status: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub trait MemoryStore: Send + Sync {
    fn upsert_card(&self, card: &MemoryCard) -> Result<()>;
    fn replace_card_sources(&self, card_id: &str, sources: &[MemorySource]) -> Result<()>;
    fn list_cards_for_source(
        &self,
        agent: &str,
        session_id: &str,
        file_path: &str,
    ) -> Result<Vec<MemoryCard>>;
    fn mark_card_unavailable(&self, card_id: &str) -> Result<()>;
    fn mark_source_cards_unavailable(
        &self,
        agent: &str,
        session_id: &str,
        file_path: &str,
    ) -> Result<()>;
    fn list_project_cards(&self, project_key: &str) -> Result<Vec<MemoryCard>>;
    fn card_by_id(&self, card_id: &str) -> Result<Option<MemoryCard>>;
    fn sources_for_card(&self, card_id: &str) -> Result<Vec<MemorySource>>;
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
    fn record_memory_job(
        &self,
        project_key: &str,
        scope: &str,
        kind: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<()>;
    fn list_memory_jobs(&self, project_key: &str, status: Option<&str>) -> Result<Vec<MemoryJob>>;
}
