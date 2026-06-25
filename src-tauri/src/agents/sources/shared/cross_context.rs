use std::fs;
use std::path::{Path, PathBuf};

use crate::agents::sources::shared::attachment_text::{parse_xmlish_attrs, percent_decode};
use crate::models::Agent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossContextLineage {
    pub agent: Agent,
    pub session_id: String,
}

pub fn cross_context_lineage_from_text(text: &str) -> Option<CrossContextLineage> {
    for path in cross_context_paths_from_text(text) {
        if let Some(lineage) = cross_context_lineage_from_file(&path) {
            return Some(lineage);
        }
    }
    parse_cross_context_lineage(text)
}

pub fn cross_context_lineage_from_payload(
    payload: &serde_json::Value,
) -> Option<CrossContextLineage> {
    for text in raw_text_values(payload) {
        if let Some(lineage) = cross_context_lineage_from_text(&text) {
            return Some(lineage);
        }
    }
    None
}

fn raw_text_values(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_raw_text_values(value, &mut out);
    out
}

fn collect_raw_text_values(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => out.push(text.clone()),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_raw_text_values(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for key in ["text", "message", "content", "value", "displayContent"] {
                if let Some(child) = map.get(key) {
                    collect_raw_text_values(child, out);
                }
            }
        }
        _ => {}
    }
}

fn cross_context_paths_from_text(text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for marker in ["file://", "uri=\"file://", "ref=\"file://"] {
        let mut rest = text;
        while let Some(start) = rest.find(marker) {
            let after = &rest[start + marker.len()..];
            let end = after
                .find(|ch: char| ch == '"' || ch == ')' || ch == '<' || ch.is_whitespace())
                .unwrap_or(after.len());
            let raw = &after[..end];
            if raw.contains("sessio-cross-context") {
                paths.push(PathBuf::from(percent_decode(raw)));
            }
            rest = &after[end..];
        }
    }
    paths
}

fn cross_context_lineage_from_file(path: &Path) -> Option<CrossContextLineage> {
    let text = fs::read_to_string(path).ok()?;
    parse_cross_context_lineage(&text)
}

fn parse_cross_context_lineage(text: &str) -> Option<CrossContextLineage> {
    let start = text.find("<!-- sessio-cross:start")?;
    let after_start = &text[start..];
    let end = after_start.find("-->")?;
    let attrs_start = "<!-- sessio-cross:start".len();
    let attrs = parse_xmlish_attrs(&after_start[attrs_start..end]);
    let agent = attrs
        .get("source_agent")
        .and_then(|value| Agent::from_db_str(value))?;
    let session_id = attrs.get("source_session_id")?.trim();
    if session_id.is_empty() {
        return None;
    }
    Some(CrossContextLineage {
        agent,
        session_id: session_id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lineage_from_inline_cross_context_metadata() {
        let text = r#"<!-- sessio-cross:start source_agent="claude" source_session_id="parent-session" -->

# Continued session from agent

<!-- sessio-cross:end -->"#;
        let lineage = cross_context_lineage_from_text(text).expect("lineage");
        assert_eq!(lineage.agent, Agent::Claude);
        assert_eq!(lineage.session_id, "parent-session");
    }

    #[test]
    fn reads_lineage_from_referenced_cross_context_file() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-shared-cross-context-lineage-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let context_path = dir.join("sessio-cross-context-parent.md");
        fs::write(
            &context_path,
            r#"<!-- sessio-cross:start source_agent="pi" source_session_id="pi-parent" -->"#,
        )
        .unwrap();
        let text = format!("[@ctx](file://{})", context_path.display());
        let lineage = cross_context_lineage_from_text(&text).expect("lineage");
        assert_eq!(lineage.agent, Agent::Pi);
        assert_eq!(lineage.session_id, "pi-parent");
        fs::remove_file(context_path).ok();
        fs::remove_dir(dir).ok();
    }
}
