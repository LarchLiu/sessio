//! Shared helpers for IM bridge attachments (inbound + outbound).
//!
//! Inbound: platforms download the user's photo/document to a per-workspace
//! `.sessio/attachments/` directory. The path is then handed to the router as
//! an [`InboundAttachment`], which the router converts into an [`AgentAttachment`]
//! after checking the bound agent's runtime capabilities.
//!
//! Outbound: assistant turn text is scanned for markdown image links and
//! file/workspace-relative paths. Anything pointing at a real file under the
//! session's workspace becomes an [`OutboundAttachment`] dispatched through the
//! platform sink's send_image/send_file methods.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;

use crate::agents::runtime::types::{
    AgentAttachment, AgentAttachmentKind, RuntimeCapabilitySet,
};

/// A file the platform listener already saved to local disk. The router takes
/// ownership and decides whether to forward it to the runtime.
#[derive(Debug, Clone)]
pub struct InboundAttachment {
    pub path: PathBuf,
    pub kind: AgentAttachmentKind,
    pub mime_type: Option<String>,
    pub display_name: Option<String>,
}

impl InboundAttachment {
    pub fn into_agent(self) -> AgentAttachment {
        AgentAttachment {
            path: self.path.to_string_lossy().to_string(),
            mime_type: self.mime_type,
            kind: self.kind,
            display_name: self.display_name,
        }
    }
}

/// Outcome of filtering an inbound batch against the bound agent's runtime
/// capability set. Attachments unsupported by the runtime are dropped with a
/// note that callers append to the user's reply.
#[derive(Debug, Default)]
pub struct CapabilityFilterResult {
    pub allowed: Vec<InboundAttachment>,
    pub dropped_images: usize,
    pub dropped_files: usize,
}

impl CapabilityFilterResult {
    pub fn notes(&self, agent_label: &str) -> Vec<String> {
        let mut notes = Vec::new();
        if self.dropped_images > 0 {
            notes.push(format!(
                "⚠️ 当前 agent `{}` 不支持图片附件，已忽略 {} 个图片",
                agent_label, self.dropped_images
            ));
        }
        if self.dropped_files > 0 {
            notes.push(format!(
                "⚠️ 当前 agent `{}` 不支持文件附件，已忽略 {} 个文件",
                agent_label, self.dropped_files
            ));
        }
        notes
    }
}

/// Drop attachments the agent's runtime cannot handle. Images need
/// `supports_image_attachments`; files require `supports_embedded_context`
/// (ACP forwards them as TextResourceContents).
pub fn filter_by_capability(
    attachments: Vec<InboundAttachment>,
    capabilities: &RuntimeCapabilitySet,
) -> CapabilityFilterResult {
    let mut result = CapabilityFilterResult::default();
    for attachment in attachments {
        let allowed = match attachment.kind {
            AgentAttachmentKind::Image => capabilities.supports_image_attachments,
            AgentAttachmentKind::File => capabilities.supports_embedded_context,
        };
        if allowed {
            result.allowed.push(attachment);
        } else {
            match attachment.kind {
                AgentAttachmentKind::Image => result.dropped_images += 1,
                AgentAttachmentKind::File => result.dropped_files += 1,
            }
        }
    }
    result
}

/// Build the per-workspace attachment directory and ensure it exists.
/// Layout: `<workspace>/.sessio/attachments/<platform>/<chat_id>/`.
pub fn attachment_dir(workspace: &str, platform: &str, chat_id: &str) -> Result<PathBuf> {
    if workspace.trim().is_empty() {
        return Err(anyhow!("workspace path is empty"));
    }
    let sanitized_chat = sanitize_segment(chat_id);
    let dir = Path::new(workspace)
        .join(".sessio")
        .join("attachments")
        .join(platform)
        .join(sanitized_chat);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create attachment directory {}", dir.display()))?;
    Ok(dir)
}

/// Choose a unique destination path. Keeps the original filename when it
/// doesn't already exist; otherwise prefixes with a short id.
pub fn allocate_attachment_path(dir: &Path, suggested_name: Option<&str>) -> PathBuf {
    let base = suggested_name
        .map(sanitize_segment)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "attachment".to_string());
    let candidate = dir.join(&base);
    if !candidate.exists() {
        return candidate;
    }
    for attempt in 1..1024 {
        let candidate = dir.join(format!("{attempt}-{base}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{}-{base}", now_ms()))
}

/// Replace path separators and other awkward characters so platform ids and
/// filenames remain safe for the local filesystem.
pub fn sanitize_segment(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "_".to_string();
    }
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// Download a remote URL using the given blocking client and an optional bearer
/// token. Returns the local path written to disk.
pub fn download_to_file(
    client: &Client,
    url: &str,
    bearer_token: Option<&str>,
    destination: &Path,
) -> Result<()> {
    let mut request = client.get(url);
    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }
    let mut response = request
        .send()
        .with_context(|| format!("download attachment {url}"))?
        .error_for_status()
        .with_context(|| format!("attachment {url} returned HTTP error"))?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create directory {}", parent.display()))?;
    }
    let mut file = std::fs::File::create(destination)
        .with_context(|| format!("create attachment file {}", destination.display()))?;
    response
        .copy_to(&mut file)
        .with_context(|| format!("write attachment file {}", destination.display()))?;
    Ok(())
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// An attachment extracted from assistant turn text, ready for the platform
/// sink to upload.
#[derive(Debug, Clone)]
pub struct OutboundAttachment {
    pub path: PathBuf,
    pub kind: AgentAttachmentKind,
    pub display_name: Option<String>,
}

/// Scan assistant text for markdown image (`![alt](path)`) and link
/// (`[label](path)`) references that resolve to a real file under `workspace`
/// or to an absolute `file://` URI. Each unique file is returned once.
pub fn extract_outbound_attachments(text: &str, workspace: &str) -> Vec<OutboundAttachment> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for hit in find_markdown_links(text) {
        let Some(resolved) = resolve_path(&hit.target, workspace) else {
            continue;
        };
        if !resolved.is_file() {
            continue;
        }
        let canonical = resolved
            .canonicalize()
            .unwrap_or_else(|_| resolved.clone());
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let kind = if hit.is_image || looks_like_image(&canonical) {
            AgentAttachmentKind::Image
        } else if is_text_or_binary_attachment(&canonical) {
            AgentAttachmentKind::File
        } else {
            // Skip arbitrary paths that don't look like an attachment we should
            // forward — README links inside long answers, etc.
            continue;
        };
        let display_name = hit
            .label
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                canonical
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            });
        out.push(OutboundAttachment {
            path: canonical,
            kind,
            display_name,
        });
    }
    out
}

struct MarkdownLink {
    label: Option<String>,
    target: String,
    is_image: bool,
}

fn find_markdown_links(text: &str) -> Vec<MarkdownLink> {
    let bytes = text.as_bytes();
    let mut links = Vec::new();
    let mut idx = 0;
    while idx < bytes.len() {
        let (is_image, open) = if bytes[idx] == b'!' && bytes.get(idx + 1) == Some(&b'[') {
            (true, idx + 1)
        } else if bytes[idx] == b'[' {
            (false, idx)
        } else {
            idx += 1;
            continue;
        };
        // Find matching label `]`
        let Some(label_end_rel) = bytes[open + 1..].iter().position(|b| *b == b']') else {
            idx += 1;
            continue;
        };
        let label_end = open + 1 + label_end_rel;
        if bytes.get(label_end + 1) != Some(&b'(') {
            idx = label_end + 1;
            continue;
        }
        let target_start = label_end + 2;
        let Some(target_end_rel) = bytes[target_start..].iter().position(|b| *b == b')') else {
            idx = target_start;
            continue;
        };
        let target_end = target_start + target_end_rel;
        let label = std::str::from_utf8(&bytes[open + 1..label_end])
            .ok()
            .map(str::trim)
            .map(str::to_string);
        let target_raw = std::str::from_utf8(&bytes[target_start..target_end])
            .unwrap_or("")
            .trim();
        let target = strip_link_title(target_raw).to_string();
        if !target.is_empty() {
            links.push(MarkdownLink {
                label,
                target,
                is_image,
            });
        }
        idx = target_end + 1;
    }
    links
}

/// Markdown allows `(url "title")`. Drop the title before resolving paths.
fn strip_link_title(target: &str) -> &str {
    if let Some(idx) = target.find(char::is_whitespace) {
        &target[..idx]
    } else {
        target
    }
}

fn resolve_path(target: &str, workspace: &str) -> Option<PathBuf> {
    if target.starts_with("http://") || target.starts_with("https://") {
        return None;
    }
    let candidate = if let Some(rest) = target.strip_prefix("file://") {
        PathBuf::from(rest)
    } else {
        let raw = PathBuf::from(target);
        if raw.is_absolute() {
            raw
        } else {
            PathBuf::from(workspace).join(raw)
        }
    };
    Some(candidate)
}

fn looks_like_image(path: &Path) -> bool {
    matches!(
        extension_lower(path).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "heic" | "heif" | "svg")
    )
}

fn is_text_or_binary_attachment(path: &Path) -> bool {
    // Skip executables and source-code-only directories. Anything with a
    // sensible extension is fine to forward; allow files in well-known doc
    // formats by extension list.
    matches!(
        extension_lower(path).as_deref(),
        Some(
            "txt"
                | "md"
                | "markdown"
                | "csv"
                | "tsv"
                | "json"
                | "jsonl"
                | "yaml"
                | "yml"
                | "toml"
                | "xml"
                | "html"
                | "htm"
                | "pdf"
                | "docx"
                | "xlsx"
                | "pptx"
                | "doc"
                | "xls"
                | "ppt"
                | "zip"
                | "tar"
                | "gz"
                | "log"
                | "rtf"
                | "ics"
                | "vcf"
        )
    )
}

fn extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
}

pub fn guess_image_mime(path: &Path) -> &'static str {
    match extension_lower(path).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("heic") => "image/heic",
        Some("heif") => "image/heif",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

pub fn guess_file_mime(path: &Path) -> &'static str {
    match extension_lower(path).as_deref() {
        Some("pdf") => "application/pdf",
        Some("json" | "jsonl") => "application/json",
        Some("yaml" | "yml") => "application/yaml",
        Some("toml") => "application/toml",
        Some("xml") => "application/xml",
        Some("html" | "htm") => "text/html",
        Some("md" | "markdown") => "text/markdown",
        Some("csv") => "text/csv",
        Some("tsv") => "text/tab-separated-values",
        Some("txt" | "log") => "text/plain",
        Some("zip") => "application/zip",
        Some("tar") => "application/x-tar",
        Some("gz") => "application/gzip",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let path = std::env::temp_dir().join(format!("sessio-im-attach-test-{pid}-{id}"));
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sanitize_segment_replaces_path_separators() {
        assert_eq!(sanitize_segment("foo/bar"), "foo_bar");
        assert_eq!(sanitize_segment(" "), "_");
        assert_eq!(sanitize_segment("ok.name-1_2"), "ok.name-1_2");
    }

    #[test]
    fn allocate_attachment_path_avoids_collisions() {
        let dir = TempDir::new();
        let first = allocate_attachment_path(dir.path(), Some("pic.png"));
        std::fs::write(&first, b"x").unwrap();
        let second = allocate_attachment_path(dir.path(), Some("pic.png"));
        assert_ne!(first, second);
        assert!(second
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("pic.png"));
    }

    #[test]
    fn extract_outbound_attachments_picks_images_under_workspace() {
        let dir = TempDir::new();
        let workspace = dir.path();
        std::fs::write(workspace.join("a.png"), b"png").unwrap();
        std::fs::create_dir_all(workspace.join("docs")).unwrap();
        std::fs::write(workspace.join("docs/notes.pdf"), b"pdf").unwrap();
        let text = "See ![preview](a.png) and the spec [notes](docs/notes.pdf).";
        let found = extract_outbound_attachments(text, workspace.to_str().unwrap());
        assert_eq!(found.len(), 2);
        assert!(matches!(found[0].kind, AgentAttachmentKind::Image));
        assert!(matches!(found[1].kind, AgentAttachmentKind::File));
    }

    #[test]
    fn extract_outbound_attachments_ignores_remote_links() {
        let dir = TempDir::new();
        let text = "Open [docs](https://example.com/a.pdf) for details.";
        let found = extract_outbound_attachments(text, dir.path().to_str().unwrap());
        assert!(found.is_empty());
    }

    #[test]
    fn extract_outbound_attachments_handles_file_uri() {
        let dir = TempDir::new();
        let file = dir.path().join("image.png");
        std::fs::write(&file, b"png").unwrap();
        let text = format!("![p](file://{})", file.to_string_lossy());
        let found = extract_outbound_attachments(&text, "/tmp/unused");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, file.canonicalize().unwrap());
    }
}
