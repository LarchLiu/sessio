use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::agents::sources::pi::parser as pi_parser;
use crate::app_paths;
use crate::models::{Agent, SessionInfo};

pub fn root_dir() -> Result<Option<PathBuf>> {
    let root = app_paths::omp_agent_sessions_dir()?;
    if root.exists() {
        Ok(Some(root))
    } else {
        Ok(None)
    }
}

pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    let Some(root) = root_dir()? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for project_entry in std::fs::read_dir(&root)? {
        let project_entry = project_entry?;
        if !project_entry.file_type()?.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(project_entry.path())? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                continue;
            }
            match parse_session_file(&path) {
                Ok(Some(info)) => out.push(info),
                Ok(None) => {}
                Err(error) => log::warn!("omp parse {} failed: {error}", path.display()),
            }
        }
    }
    Ok(out)
}

pub fn parse_session_file(path: &Path) -> Result<Option<SessionInfo>> {
    let Some(mut info) = pi_parser::parse_session_file(path)? else {
        return Ok(None);
    };
    info.agent = Agent::Omp;
    Ok(Some(info))
}

pub fn read_message_events(
    path: &Path,
    source: &crate::agents::sources::types::SessionSource,
) -> Result<Vec<crate::agents::sources::types::MessageEvent>> {
    pi_parser::read_message_events(path, source)
}
