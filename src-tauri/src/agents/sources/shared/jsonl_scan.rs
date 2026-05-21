use anyhow::Result;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const FULL_READ_THRESHOLD: u64 = 16 * 1024;
const TAIL_WINDOW: u64 = 16 * 1024;
const HEAD_LINES: usize = 10;
const TAIL_LINES: usize = 30;

pub struct JsonlScan {
    pub head: Vec<String>,
    pub tail: Vec<String>,
    pub message_count: usize,
    pub partial: bool,
    pub file_size: u64,
}

pub fn scan(path: &Path) -> Result<JsonlScan> {
    let meta = fs::metadata(path)?;
    let size = meta.len();

    if size <= FULL_READ_THRESHOLD {
        let content = fs::read_to_string(path)?;
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        let total = lines.len();
        let head: Vec<String> = lines
            .iter()
            .take(HEAD_LINES)
            .map(|s| s.to_string())
            .collect();
        let tail_start = total.saturating_sub(TAIL_LINES);
        let tail: Vec<String> = lines[tail_start..].iter().map(|s| s.to_string()).collect();
        return Ok(JsonlScan {
            head,
            tail,
            message_count: total,
            partial: false,
            file_size: size,
        });
    }

    let mut head = Vec::with_capacity(HEAD_LINES);
    {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        for line in reader.lines().take(HEAD_LINES) {
            let line = line?;
            if !line.is_empty() {
                head.push(line);
            }
        }
    }

    let tail_size = TAIL_WINDOW.min(size);
    let mut file = File::open(path)?;
    file.seek(SeekFrom::End(-(tail_size as i64)))?;
    let mut buf = Vec::with_capacity(tail_size as usize);
    file.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut all_lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    if all_lines.len() > 1 {
        all_lines.remove(0);
    }
    let start = all_lines.len().saturating_sub(TAIL_LINES);
    let tail: Vec<String> = all_lines[start..].iter().map(|s| s.to_string()).collect();

    let avg_len: usize = if !head.is_empty() {
        let sum: usize = head.iter().map(|s| s.len() + 1).sum();
        (sum / head.len()).max(1)
    } else {
        256
    };
    let estimated = (size as usize / avg_len).max(head.len() + tail.len());

    Ok(JsonlScan {
        head,
        tail,
        message_count: estimated,
        partial: true,
        file_size: size,
    })
}
