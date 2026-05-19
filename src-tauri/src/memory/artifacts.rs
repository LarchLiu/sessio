use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::memory::{MemoryArtifact, MemoryRecord};

pub trait MemoryArtifactSink: Send + Sync {
    fn write_record_artifact(
        &self,
        backend: &str,
        project_key: &str,
        record: &MemoryRecord,
    ) -> Result<MemoryArtifact>;

    fn remove_record_artifact(
        &self,
        backend: &str,
        project_key: &str,
        record_id: &str,
    ) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct MarkdownArtifactSink {
    root: PathBuf,
}

impl MarkdownArtifactSink {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn artifact_path(&self, project_key: &str, record_id: &str) -> PathBuf {
        self.root
            .join(project_key)
            .join("sessions")
            .join(format!("{record_id}.md"))
    }
}

impl MemoryArtifactSink for MarkdownArtifactSink {
    fn write_record_artifact(
        &self,
        backend: &str,
        project_key: &str,
        record: &MemoryRecord,
    ) -> Result<MemoryArtifact> {
        let path = self.artifact_path(project_key, &record.record_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &record.body)?;
        Ok(MemoryArtifact {
            record_id: record.record_id.clone(),
            backend: backend.to_string(),
            artifact_uri: path.to_string_lossy().to_string(),
            content_hash: record.canonical_hash.clone(),
            updated_at: record.updated_at,
        })
    }

    fn remove_record_artifact(
        &self,
        _backend: &str,
        project_key: &str,
        record_id: &str,
    ) -> Result<()> {
        let path = self.artifact_path(project_key, record_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err).with_context(|| format!("remove artifact {}", path.display())),
        }
    }
}

#[derive(Debug, Default)]
pub struct NoopArtifactSink;

impl MemoryArtifactSink for NoopArtifactSink {
    fn write_record_artifact(
        &self,
        backend: &str,
        _project_key: &str,
        record: &MemoryRecord,
    ) -> Result<MemoryArtifact> {
        Ok(MemoryArtifact {
            record_id: record.record_id.clone(),
            backend: backend.to_string(),
            artifact_uri: format!("memory://{backend}/{}", record.record_id),
            content_hash: record.canonical_hash.clone(),
            updated_at: record.updated_at,
        })
    }

    fn remove_record_artifact(
        &self,
        _backend: &str,
        _project_key: &str,
        _record_id: &str,
    ) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct TempArtifactSink {
    artifacts: Mutex<HashMap<(String, String), MemoryArtifact>>,
}

impl TempArtifactSink {
    pub fn artifacts(&self) -> Vec<MemoryArtifact> {
        self.artifacts.lock().unwrap().values().cloned().collect()
    }
}

impl MemoryArtifactSink for TempArtifactSink {
    fn write_record_artifact(
        &self,
        backend: &str,
        project_key: &str,
        record: &MemoryRecord,
    ) -> Result<MemoryArtifact> {
        let artifact = MemoryArtifact {
            record_id: record.record_id.clone(),
            backend: backend.to_string(),
            artifact_uri: format!("memory://{backend}/{project_key}/{}", record.record_id),
            content_hash: record.canonical_hash.clone(),
            updated_at: record.updated_at,
        };
        self.artifacts.lock().unwrap().insert(
            (backend.to_string(), record.record_id.clone()),
            artifact.clone(),
        );
        Ok(artifact)
    }

    fn remove_record_artifact(
        &self,
        backend: &str,
        _project_key: &str,
        record_id: &str,
    ) -> Result<()> {
        self.artifacts
            .lock()
            .unwrap()
            .remove(&(backend.to_string(), record_id.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{MarkdownArtifactSink, MemoryArtifactSink};
    use crate::memory::{MemoryRecord, MemoryRecordKind};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_tmp(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn markdown_sink_writes_sessions_layout_and_removes_artifact() {
        let root = unique_tmp("sessio-markdown-artifact");
        let sink = MarkdownArtifactSink::new(root.clone());
        let record = MemoryRecord {
            record_id: "sessio-codex-abc".to_string(),
            project_key: "project-key".to_string(),
            canonical_hash: "hash".to_string(),
            simhash: None,
            title: "Title".to_string(),
            summary: None,
            body: "# Title\n".to_string(),
            kind: MemoryRecordKind::Session,
            available: true,
            updated_at: 123,
        };

        let artifact = sink
            .write_record_artifact("qmd", &record.project_key, &record)
            .unwrap();
        let path = root
            .join("project-key")
            .join("sessions")
            .join("sessio-codex-abc.md");
        assert_eq!(PathBuf::from(&artifact.artifact_uri), path);
        assert_eq!(fs::read_to_string(&path).unwrap(), "# Title\n");

        sink.remove_record_artifact("qmd", &record.project_key, &record.record_id)
            .unwrap();
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&root);
    }
}
