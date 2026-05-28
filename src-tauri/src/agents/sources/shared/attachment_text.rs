use std::collections::HashMap;

pub fn sanitize_user_attachment_text(text: &str) -> String {
    let without_images = strip_image_placeholder_tags(text);
    let without_file_links = remove_file_markdown_links(&without_images);
    let without_sessio_files = replace_tagged_upload_files(&without_file_links);
    replace_codex_context_files(&without_sessio_files)
}

pub fn sanitize_user_preview_text(text: &str) -> String {
    let without_images = strip_image_placeholder_tags(text);
    let without_file_links = remove_file_markdown_links(&without_images);
    let without_sessio_files = remove_xmlish_blocks(&without_file_links, "sessio-upload-file");
    remove_xmlish_blocks(&without_sessio_files, "context")
}

pub fn file_name_from_uri(uri: &str) -> Option<String> {
    let raw = uri.strip_prefix("file://").unwrap_or(uri);
    let decoded = percent_decode(raw);
    decoded
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .next_back()
        .map(String::from)
}

pub fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| value.to_string())
}

pub fn parse_xmlish_attrs(input: &str) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_' | b':'))
        {
            i += 1;
        }
        if i == key_start {
            i += 1;
            continue;
        }
        let key = &input[key_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || (bytes[i] != b'"' && bytes[i] != b'\'') {
            continue;
        }
        let quote = bytes[i];
        i += 1;
        let value_start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        let value = &input[value_start..i.min(bytes.len())];
        attrs.insert(key.to_string(), unescape_xml_attr(value));
        if i < bytes.len() {
            i += 1;
        }
    }
    attrs
}

fn replace_tagged_upload_files(text: &str) -> String {
    replace_xmlish_blocks(text, "sessio-upload-file", |attrs| {
        let name = attrs
            .get("name")
            .cloned()
            .or_else(|| attrs.get("uri").and_then(|uri| file_name_from_uri(uri)));
        file_marker(name.as_deref(), attrs.get("uri").map(String::as_str))
    })
}

fn replace_codex_context_files(text: &str) -> String {
    replace_xmlish_blocks(text, "context", |attrs| {
        let name = attrs.get("ref").and_then(|uri| file_name_from_uri(uri));
        file_marker(name.as_deref(), attrs.get("ref").map(String::as_str))
    })
}

fn replace_xmlish_blocks<F>(text: &str, tag: &str, marker: F) -> String
where
    F: Fn(&HashMap<String, String>) -> String,
{
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let open_prefix = format!("<{tag}");
    let close_tag = format!("</{tag}>");
    while let Some(open_start) = rest.find(&open_prefix) {
        out.push_str(&rest[..open_start]);
        let after_open = &rest[open_start..];
        let Some(open_end) = after_open.find('>') else {
            out.push_str(after_open);
            return collapse_blank_lines(&out);
        };
        let attrs_text = &after_open[open_prefix.len()..open_end];
        let after_tag = &after_open[open_end + 1..];
        let Some(close_start) = after_tag.find(&close_tag) else {
            out.push_str(after_open);
            return collapse_blank_lines(&out);
        };
        out.push_str(&marker(&parse_xmlish_attrs(attrs_text)));
        rest = &after_tag[close_start + close_tag.len()..];
    }
    out.push_str(rest);
    collapse_blank_lines(&out)
}

fn remove_xmlish_blocks(text: &str, tag: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let open_prefix = format!("<{tag}");
    let close_tag = format!("</{tag}>");
    while let Some(open_start) = rest.find(&open_prefix) {
        out.push_str(&rest[..open_start]);
        let after_open = &rest[open_start..];
        let Some(open_end) = after_open.find('>') else {
            return collapse_blank_lines(&out);
        };
        let after_tag = &after_open[open_end + 1..];
        let Some(close_start) = after_tag.find(&close_tag) else {
            return collapse_blank_lines(&out);
        };
        rest = &after_tag[close_start + close_tag.len()..];
    }
    out.push_str(rest);
    collapse_blank_lines(&out)
}

fn remove_file_markdown_links(text: &str) -> String {
    let mut out = text.to_string();
    let mut search_from = 0;
    while let Some(rel) = out[search_from..].find("](") {
        let close_label = search_from + rel;
        let Some(rel_open) = out[search_from..close_label].rfind('[') else {
            search_from = close_label + 2;
            continue;
        };
        let open_label = search_from + rel_open;
        let target_start = close_label + 2;
        let Some(close_target_rel) = out[target_start..].find(')') else {
            break;
        };
        let close_target = target_start + close_target_rel;
        let target = out[target_start..close_target]
            .trim()
            .trim_matches(['<', '>']);
        let label = &out[open_label + 1..close_label];
        let is_at_prefix = label.trim_start().starts_with('@');
        let is_cross_context = target.contains("sessio-cross-context");
        if !target.starts_with("file://") || (!is_at_prefix && !is_cross_context) {
            search_from = close_target + 1;
            continue;
        }
        let drop_start = if open_label > 0 && out.as_bytes()[open_label - 1] == b'!' {
            open_label - 1
        } else {
            open_label
        };
        out.replace_range(drop_start..=close_target, "");
        search_from = drop_start;
    }
    collapse_blank_lines(&out)
}

fn strip_image_placeholder_tags(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.starts_with("<image") && trimmed.ends_with(">")) && trimmed != "</image>"
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn file_marker(name: Option<&str>, uri: Option<&str>) -> String {
    let trimmed = name.unwrap_or("attachment").trim();
    let safe_name = if trimmed.is_empty() {
        "attachment"
    } else {
        trimmed
    };
    match uri.map(str::trim).filter(|value| !value.is_empty()) {
        Some(uri) => format!("[file: {safe_name}|{uri}]"),
        None => format!("[file: {safe_name}]"),
    }
}

fn unescape_xml_attr(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn collapse_blank_lines(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .replace("\n\n\n", "\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_user_authored_file_link_without_at_prefix() {
        let input = "see [design doc](file:///Users/alex/Documents/design.md) for details";
        assert_eq!(sanitize_user_attachment_text(input), input);
    }

    #[test]
    fn drops_at_prefixed_attachment_link_and_leading_bang() {
        let input = "preview ![@photo.png](file:///tmp/photo.png) end";
        let cleaned = sanitize_user_attachment_text(input);
        assert!(!cleaned.contains("photo.png"), "{cleaned}");
        assert!(!cleaned.contains('!'), "{cleaned}");
        assert!(cleaned.contains("preview"));
        assert!(cleaned.contains("end"));
    }

    #[test]
    fn drops_at_prefixed_link_without_image_bang() {
        let input = "look [@notes.md](file:///tmp/notes.md) here";
        let cleaned = sanitize_user_attachment_text(input);
        assert!(!cleaned.contains("notes.md"), "{cleaned}");
        assert!(cleaned.contains("look"));
        assert!(cleaned.contains("here"));
    }

    #[test]
    fn keeps_non_file_link() {
        let input = "open [docs](https://example.com/file.md) please";
        assert_eq!(sanitize_user_attachment_text(input), input);
    }

    #[test]
    fn drops_cross_context_link_even_without_at_prefix() {
        let input = "carry [doc](file:///tmp/.cross-context/sessio-cross-context-abc.md) over";
        let cleaned = sanitize_user_attachment_text(input);
        assert!(!cleaned.contains("sessio-cross-context"), "{cleaned}");
        assert!(cleaned.contains("carry"));
        assert!(cleaned.contains("over"));
    }
}
