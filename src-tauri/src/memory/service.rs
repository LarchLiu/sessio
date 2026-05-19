use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread;

use anyhow::{bail, Result};

use crate::config::MemoryConfig;
use crate::memory::artifacts::{MarkdownArtifactSink, MemoryArtifactSink};
use crate::memory::build::{
    build_project_memory_with_backend, build_source_memory_with_backend, MemoryBuildOptions,
};
use crate::memory::qmd::{QmdBackend, QmdOptions};
use crate::memory::{
    MemoryIndexBackend, MemoryRecord, MemorySearchOptions, MemoryStore, MemorySyncReport,
};
use crate::providers::registry::ProviderRegistry;
use crate::providers::types::SessionSource;

#[derive(Debug, Clone)]
pub struct MemoryBackendSyncJob {
    pub backend: String,
    pub project_key: String,
    pub project_path: String,
    pub changed_record_ids: Vec<String>,
    pub removed_record_ids: Vec<String>,
    pub dependent_source_paths: Vec<PathBuf>,
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
        let backend = Arc::new(QmdBackend::new(
            QmdOptions {
                binary: config.qmd.binary.clone(),
                index: config.qmd.index.clone(),
            },
            config.qmd.artifacts_root.clone(),
        ));
        let artifact_sink = Arc::new(MarkdownArtifactSink::new(config.qmd.artifacts_root.clone()));
        Ok(Self {
            repository,
            registry,
            backend,
            artifact_sink,
            artifacts_root: config.qmd.artifacts_root.clone(),
        })
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
            &options,
        )
    }

    pub fn build_source(
        &self,
        source: &SessionSource,
        artifacts_root: &Path,
    ) -> Result<crate::memory::build::MemoryBuildSourceResult> {
        let registry = self.registry.as_ref();
        build_source_memory_with_backend(
            registry,
            self.repository.as_ref(),
            self.backend.name(),
            artifacts_root,
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
        let sync = summary.project_key.as_deref().map(|project_key| {
            self.sync_backend_job_oneshot(MemoryBackendSyncJob {
                backend: self.backend.name().to_string(),
                project_key: project_key.to_string(),
                project_path: summary.project_path.clone(),
                changed_record_ids: Vec::new(),
                removed_record_ids: Vec::new(),
                dependent_source_paths: summary.dependent_source_paths.clone(),
            })
        });
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
        let records = self.repository.list_project_cards(project_key)?;
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
        let _ = (&job.changed_record_ids, &job.removed_record_ids);
        self.sync_project(&job.project_key)
    }

    pub fn sync_backend_job_oneshot(&self, job: MemoryBackendSyncJob) -> Result<MemorySyncReport> {
        let repository = self.repository.clone();
        let backend = self.backend.clone();
        let artifact_sink = self.artifact_sink.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = if job.backend != backend.name() {
                Err(anyhow::anyhow!(
                    "memory backend job targets {}, but worker backend is {}",
                    job.backend,
                    backend.name()
                ))
            } else {
                repository
                    .list_project_cards(&job.project_key)
                    .and_then(|records| {
                        backend.sync_project(&job.project_key, &records, artifact_sink.as_ref())
                    })
            };
            let _ = tx.send(result);
        });
        rx.recv()
            .map_err(|e| anyhow::anyhow!("memory backend sync worker closed: {e}"))?
    }

    pub fn search(
        &self,
        project_key: &str,
        query: &str,
        options: MemorySearchOptions,
    ) -> Result<crate::memory::MemoryBackendSearchResult> {
        self.backend.search(project_key, query, options)
    }

    pub fn resolve(&self, record_id: &str) -> Result<Option<MemoryRecord>> {
        self.repository.card_by_id(record_id)
    }

    pub fn remove_project(&self, project_key: &str) -> Result<MemorySyncReport> {
        self.backend.remove_project(project_key)
    }

    pub fn backend_status(&self) -> crate::memory::MemoryBackendStatus {
        self.backend.status()
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    pub fn backend_artifacts_root(&self) -> Result<PathBuf> {
        Ok(self.artifacts_root.clone())
    }
}
