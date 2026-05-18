use anyhow::{Context, Result};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::memory::MemorySource;
use crate::providers::types::SourceLocation;

// Resolve a MemorySource back to the raw text snippet it points at.
//
// Returns Ok(None) when the source carries no offset info (Gemini today, or
// any future source where the parser does not yet emit line/byte ranges) so
// the caller can decide whether to fall back to "session-level pointer
// only". Errors only escape when the file is reachable but the requested
// range cannot be read.
pub fn read_source_excerpt(source: &MemorySource) -> Result<Option<String>> {
    let path = Path::new(&source.file_path);
    if !path.is_file() {
        return Ok(None);
    }
    if let Some(text) = read_byte_range(path, &source.location)? {
        return Ok(Some(text));
    }
    if let Some(text) = read_line_range(path, &source.location)? {
        return Ok(Some(text));
    }
    Ok(None)
}

fn read_byte_range(path: &Path, location: &SourceLocation) -> Result<Option<String>> {
    let (Some(start), Some(end)) = (location.byte_start, location.byte_end) else {
        return Ok(None);
    };
    if end <= start {
        return Ok(Some(String::new()));
    }
    let mut file =
        File::open(path).with_context(|| format!("open {} for byte range", path.display()))?;
    file.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; (end - start) as usize];
    file.read_exact(&mut buf)?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn read_line_range(path: &Path, location: &SourceLocation) -> Result<Option<String>> {
    let (Some(start), Some(end)) = (location.line_start, location.line_end) else {
        return Ok(None);
    };
    if end < start {
        return Ok(Some(String::new()));
    }
    let file =
        File::open(path).with_context(|| format!("open {} for line range", path.display()))?;
    let reader = BufReader::new(file);
    let mut out = String::new();
    for (idx, line) in reader.lines().enumerate() {
        let n = (idx as u64) + 1;
        if n > end {
            break;
        }
        if n >= start {
            out.push_str(&line?);
            out.push('\n');
        }
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::read_source_excerpt;
    use crate::memory::MemorySource;
    use crate::providers::types::SourceLocation;
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
    fn read_byte_range_returns_exact_slice() {
        let dir = unique_tmp("resolve-byte");
        let file_path = dir.join("session.jsonl");
        // 3 lines: "alpha\n", "beta\n", "gamma\n" -> bytes 0..6, 6..11, 11..17
        fs::write(&file_path, "alpha\nbeta\ngamma\n").unwrap();
        let source = MemorySource {
            card_id: "c".to_string(),
            agent: "codex".to_string(),
            session_id: "s".to_string(),
            file_path: file_path.to_string_lossy().to_string(),
            location: SourceLocation {
                file_path: file_path.to_string_lossy().to_string(),
                line_start: None,
                line_end: None,
                byte_start: Some(6),
                byte_end: Some(11),
            },
        };
        let excerpt = read_source_excerpt(&source).unwrap().unwrap();
        assert_eq!(excerpt, "beta\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_line_range_returns_inclusive_lines() {
        let dir = unique_tmp("resolve-line");
        let file_path = dir.join("session.jsonl");
        fs::write(&file_path, "one\ntwo\nthree\nfour\n").unwrap();
        let source = MemorySource {
            card_id: "c".to_string(),
            agent: "codex".to_string(),
            session_id: "s".to_string(),
            file_path: file_path.to_string_lossy().to_string(),
            location: SourceLocation {
                file_path: file_path.to_string_lossy().to_string(),
                line_start: Some(2),
                line_end: Some(3),
                byte_start: None,
                byte_end: None,
            },
        };
        let excerpt = read_source_excerpt(&source).unwrap().unwrap();
        assert_eq!(excerpt, "two\nthree\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_none_when_no_offsets_available() {
        let dir = unique_tmp("resolve-none");
        let file_path = dir.join("session.jsonl");
        fs::write(&file_path, "hello\n").unwrap();
        let source = MemorySource {
            card_id: "c".to_string(),
            agent: "gemini".to_string(),
            session_id: "s".to_string(),
            file_path: file_path.to_string_lossy().to_string(),
            location: SourceLocation::file(file_path.to_string_lossy().to_string()),
        };
        assert!(read_source_excerpt(&source).unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_none_when_file_missing() {
        let source = MemorySource {
            card_id: "c".to_string(),
            agent: "codex".to_string(),
            session_id: "s".to_string(),
            file_path: "/tmp/does-not-exist-sessio.jsonl".to_string(),
            location: SourceLocation {
                file_path: "/tmp/does-not-exist-sessio.jsonl".to_string(),
                line_start: Some(1),
                line_end: Some(1),
                byte_start: None,
                byte_end: None,
            },
        };
        assert!(read_source_excerpt(&source).unwrap().is_none());
    }
}
