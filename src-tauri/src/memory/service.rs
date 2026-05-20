use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Result};

use crate::config::MemoryConfig;
use crate::memory::artifacts::{MarkdownArtifactSink, MemoryArtifactSink};
use crate::memory::build::{
    build_project_memory_with_backend, build_source_memory_with_backend, MemoryBuildOptions,
};
use crate::memory::qmd::{QmdBackend, QmdOptions};
use crate::memory::{
    MemoryBackendHit, MemoryBackendSearchResult, MemoryBackendStatus, MemoryIndexBackend,
    MemoryRecord, MemorySearchOptions, MemorySource, MemoryStore, MemorySyncReport,
    RecordContinuation,
};
use crate::providers::registry::ProviderRegistry;
use crate::providers::types::SessionSource;

#[derive(Debug, Clone)]
pub struct MemoryBackendSyncJob {
    pub backend: String,
    pub project_key: String,
    pub project_path: String,
    pub dependent_source_paths: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct MemorySearchResponse {
    pub backend: String,
    pub backend_hits: Vec<MemoryBackendHit>,
    pub raw: Option<serde_json::Value>,
    pub hits: Vec<MemorySearchHit>,
}

#[derive(Debug, Clone)]
pub struct MemorySearchHit {
    pub record: MemoryRecord,
    pub score: Option<f64>,
    pub snippet: Option<String>,
    pub sources: Vec<MemorySource>,
    pub continuation: Option<RecordContinuation>,
}

#[derive(Debug, Clone)]
pub struct MemoryResolveResponse {
    pub record_id: String,
    pub record: Option<MemoryRecord>,
    pub sources: Vec<MemorySource>,
    pub continuation: Option<RecordContinuation>,
}

pub struct MemoryService {
    repository: Arc<dyn MemoryStore>,
    registry: Arc<ProviderRegistry>,
    backend: Arc<dyn MemoryIndexBackend>,
    artifact_sink: Arc<dyn MemoryArtifactSink>,
    artifacts_root: PathBuf,
}

impl MemoryService {
    pub fn new(repository: Arc<dyn MemoryStore>, registry: Arc<ProviderRegistry>) -> Result<Self> {
        let config = crate::config::load_memory_config()?;
        Self::from_config(repository, registry, &config)
    }

    pub fn from_config(
        repository: Arc<dyn MemoryStore>,
        registry: Arc<ProviderRegistry>,
        config: &MemoryConfig,
    ) -> Result<Self> {
        let backend = Arc::new(
            QmdBackend::new(
                QmdOptions {
                    binary: config.qmd.binary.clone(),
                    index: config.qmd.index.clone(),
                },
                config.qmd.artifacts_root.clone(),
            )
            .with_auto_embed(config.qmd.auto_embed),
        );
        let backend_name = backend.name();
        let artifact_sink = Arc::new(MarkdownArtifactSink::new(
            config.qmd.artifacts_root.clone(),
            backend_name,
        ));
        Ok(Self {
            repository,
            registry,
            backend,
            artifact_sink,
            artifacts_root: config.qmd.artifacts_root.clone(),
        })
    }

    pub fn with_backend(
        repository: Arc<dyn MemoryStore>,
        registry: Arc<ProviderRegistry>,
        backend: Arc<dyn MemoryIndexBackend>,
        artifact_sink: Arc<dyn MemoryArtifactSink>,
        artifacts_root: PathBuf,
    ) -> Self {
        Self {
            repository,
            registry,
            backend,
            artifact_sink,
            artifacts_root,
        }
    }

    pub fn backend(&self) -> Arc<dyn MemoryIndexBackend> {
        self.backend.clone()
    }

    pub fn build_project(
        &self,
        options: MemoryBuildOptions,
    ) -> Result<crate::memory::build::MemoryBuildSummary> {
        build_project_memory_with_backend(
            self.registry.as_ref(),
            self.repository.as_ref(),
            self.backend.name(),
            self.artifact_sink.as_ref(),
            &options,
        )
    }

    pub fn build_source(
        &self,
        source: &SessionSource,
        _artifacts_root: &Path,
    ) -> Result<crate::memory::build::MemoryBuildSourceResult> {
        let registry = self.registry.as_ref();
        build_source_memory_with_backend(
            registry,
            self.repository.as_ref(),
            self.backend.name(),
            self.artifact_sink.as_ref(),
            source,
        )
    }

    pub fn build_project_and_sync(
        &self,
        options: MemoryBuildOptions,
    ) -> Result<(
        crate::memory::build::MemoryBuildSummary,
        Option<Result<MemorySyncReport>>,
    )> {
        let summary = self.build_project(options)?;
        let sync = summary
            .project_key
            .as_deref()
            .map(|project_key| self.sync_project(project_key));
        Ok((summary, sync))
    }

    pub fn build_source_and_sync(
        &self,
        source: &SessionSource,
        artifacts_root: &Path,
    ) -> Result<crate::memory::build::MemoryBuildSourceResult> {
        let result = self.build_source(source, artifacts_root)?;
        if let Some(project) = &source.project {
            let _ = self.sync_project(&project.project_key)?;
        }
        Ok(result)
    }

    pub fn sync_project(&self, project_key: &str) -> Result<MemorySyncReport> {
        let records = self.repository.list_project_records(project_key)?;
        self.backend
            .sync_project(project_key, &records, self.artifact_sink.as_ref())
    }

    pub fn sync_backend_job(&self, job: &MemoryBackendSyncJob) -> Result<MemorySyncReport> {
        if job.backend != self.backend.name() {
            bail!(
                "memory backend job targets {}, but service backend is {}",
                job.backend,
                self.backend.name()
            );
        }
        self.sync_project(&job.project_key)
    }

    pub fn search(
        &self,
        project_key: &str,
        query: &str,
        options: MemorySearchOptions,
    ) -> Result<MemoryBackendSearchResult> {
        self.backend.search(project_key, query, options)
    }

    // Backend-neutral search assembly: resolve hits into MemoryRecord +
    // sources + continuation in one place so callers (CLI, indexer events,
    // skill plumbing) never have to reach into the repository themselves.
    pub fn search_full(
        &self,
        project_key: &str,
        query: &str,
        options: MemorySearchOptions,
    ) -> Result<MemorySearchResponse> {
        let backend_result = self.search(project_key, query, options)?;
        let records = self
            .repository
            .list_project_records(project_key)?
            .into_iter()
            .filter(|record| record.available)
            .collect::<Vec<_>>();
        let mut hits: Vec<MemorySearchHit> = Vec::new();
        for candidate in &backend_result.hits {
            let Some(record) = match_record(
                &records,
                candidate,
                self.repository.as_ref(),
                &backend_result.backend,
            ) else {
                continue;
            };
            if hits
                .iter()
                .any(|hit| hit.record.record_id == record.record_id)
            {
                continue;
            }
            let sources = self.repository.sources_for_record(&record.record_id)?;
            let continuation = self.repository.continuation_for_record(&record.record_id)?;
            hits.push(MemorySearchHit {
                record: record.clone(),
                score: candidate.score,
                snippet: candidate.snippet.clone(),
                sources,
                continuation,
            });
        }
        Ok(MemorySearchResponse {
            backend: backend_result.backend,
            backend_hits: backend_result.hits,
            raw: backend_result.raw,
            hits,
        })
    }

    pub fn resolve(&self, record_id: &str) -> Result<Option<MemoryRecord>> {
        self.repository.record_by_id(record_id)
    }

    pub fn resolve_full(&self, record_id: &str) -> Result<MemoryResolveResponse> {
        Ok(MemoryResolveResponse {
            record_id: record_id.to_string(),
            record: self.repository.record_by_id(record_id)?,
            sources: self.repository.sources_for_record(record_id)?,
            continuation: self.repository.continuation_for_record(record_id)?,
        })
    }

    pub fn remove_project(&self, project_key: &str) -> Result<MemorySyncReport> {
        self.backend.remove_project(project_key)
    }

    pub fn backend_status(&self) -> MemoryBackendStatus {
        self.backend.status()
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    pub fn backend_artifacts_root(&self) -> PathBuf {
        self.artifacts_root.clone()
    }
}

fn match_record<'a>(
    records: &'a [MemoryRecord],
    candidate: &MemoryBackendHit,
    repository: &dyn MemoryStore,
    backend: &str,
) -> Option<&'a MemoryRecord> {
    records.iter().find(|record| {
        candidate
            .record_id
            .as_deref()
            .map(|id| id == record.record_id)
            .unwrap_or(false)
            || candidate
                .artifact_uri
                .as_deref()
                .map(|path| path_matches_record_artifact(path, record, repository, backend))
                .unwrap_or(false)
    })
}

fn path_matches_record_artifact(
    path: &str,
    record: &MemoryRecord,
    repository: &dyn MemoryStore,
    backend: &str,
) -> bool {
    if path_matches_record_id(path, &record.record_id) {
        return true;
    }
    let Ok(Some(artifact)) = repository.artifact_for_record(&record.record_id, backend) else {
        return false;
    };
    let normalized = path.replace('\\', "/");
    let artifact_uri = artifact.artifact_uri.replace('\\', "/");
    normalized.ends_with(&artifact_uri) || normalized.ends_with(&artifact_uri.replace('_', "-"))
}

fn path_matches_record_id(path: &str, record_id: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let dashed_record_id = record_id.replace('_', "-");
    normalized.ends_with(&format!("/sessions/{record_id}.md"))
        || normalized.ends_with(&format!("/sessions/{dashed_record_id}.md"))
        || normalized.ends_with(&format!("{record_id}.md"))
        || normalized.ends_with(&format!("{dashed_record_id}.md"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::artifacts::NoopArtifactSink;
    use crate::memory::{MemoryBackendHit, MemoryRecordKind};
    use crate::providers::registry::ProviderRegistry;
    use std::sync::Mutex;

    // A fake backend that returns a fixed search result so we can verify
    // the service-layer search_full mapping (hit -> record + sources +
    // continuation) without depending on qmd.
    struct FakeBackend {
        hits: Mutex<Vec<MemoryBackendHit>>,
    }

    impl MemoryIndexBackend for FakeBackend {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn status(&self) -> MemoryBackendStatus {
            MemoryBackendStatus {
                backend: "fake".to_string(),
                available: true,
                error: None,
                details: None,
            }
        }
        fn sync_project(
            &self,
            project_key: &str,
            records: &[MemoryRecord],
            _artifacts: &dyn MemoryArtifactSink,
        ) -> Result<MemorySyncReport> {
            Ok(MemorySyncReport {
                backend: self.name().to_string(),
                project_key: project_key.to_string(),
                synced_records: records.iter().filter(|r| r.available).count(),
                removed_records: 0,
                errors: Vec::new(),
            })
        }
        fn remove_project(&self, project_key: &str) -> Result<MemorySyncReport> {
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
            _project_key: &str,
            _query: &str,
            _options: MemorySearchOptions,
        ) -> Result<MemoryBackendSearchResult> {
            Ok(MemoryBackendSearchResult {
                backend: self.name().to_string(),
                hits: self.hits.lock().unwrap().clone(),
                raw: None,
            })
        }
    }

    #[test]
    fn search_full_maps_backend_hits_to_records_via_record_id() {
        use crate::store::sqlite::SqliteStore;
        use crate::store::SessionStore;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("sessio-service-test-{nanos}.db"));
        let store = Arc::new(SqliteStore::open(&db_path).unwrap());
        store.init().unwrap();
        let memory_store: Arc<dyn MemoryStore> = store.clone();

        let record = MemoryRecord {
            record_id: "sessio-fake-abc".to_string(),
            project_key: "test-project".to_string(),
            canonical_hash: "h1".to_string(),
            simhash: None,
            title: "Test".to_string(),
            summary: None,
            body: "B".to_string(),
            kind: MemoryRecordKind::Session,
            available: true,
            updated_at: 1,
        };
        memory_store.upsert_record(&record).unwrap();

        let backend = Arc::new(FakeBackend {
            hits: Mutex::new(vec![MemoryBackendHit {
                record_id: Some("sessio-fake-abc".to_string()),
                artifact_uri: None,
                score: Some(0.9),
                snippet: None,
            }]),
        });
        let service = MemoryService::with_backend(
            memory_store,
            Arc::new(ProviderRegistry::new()),
            backend,
            Arc::new(NoopArtifactSink),
            std::env::temp_dir(),
        );

        let response = service
            .search_full(
                "test-project",
                "anything",
                MemorySearchOptions { include_raw: false },
            )
            .unwrap();
        assert_eq!(response.backend, "fake");
        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].record.record_id, "sessio-fake-abc");
        assert_eq!(response.hits[0].score, Some(0.9));

        let _ = std::fs::remove_file(&db_path);
    }
}
