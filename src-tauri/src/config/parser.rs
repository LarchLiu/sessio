use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;

use super::raw::*;

pub(super) fn parse_raw_config(contents: &str) -> Result<RawConfig> {
    let mut raw = RawConfig::default();
    let mut section = Section::Root;

    for (index, raw_line) in contents.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(section_name) =
            parse_section(line).with_context(|| line_context(line_number, raw_line))?
        {
            section = section_name;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            bail!("line {line_number}: invalid config line: {line}");
        };
        let key = key.trim();
        let value =
            parse_value(value.trim()).with_context(|| line_context(line_number, raw_line))?;
        match &section {
            Section::Memory => match key {
                "backend" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backend = value
                }
                other => bail!("line {line_number}: unknown key in [memory]: {other}"),
            },
            Section::MemoryBackendsQmd => match key {
                "binary" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backends
                        .qmd
                        .binary = value
                }
                "index" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backends
                        .qmd
                        .index = value
                }
                "artifacts_root" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backends
                        .qmd
                        .artifacts_root = value
                }
                "auto_embed" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backends
                        .qmd
                        .auto_embed = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "install_command" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backends
                        .qmd
                        .install_command = value
                }
                other => bail!("line {line_number}: unknown key in [memory.backends.qmd]: {other}"),
            },
            Section::Index => match key {
                "poll_interval_seconds" => {
                    raw.index.poll_interval_seconds = value
                        .map(parse_u64)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                other => bail!("line {line_number}: unknown key in [index]: {other}"),
            },
            Section::NetworkProxy => match key {
                "enabled" => {
                    raw.network.proxy.enabled = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "url" => raw.network.proxy.url = value,
                "no_proxy" => raw.network.proxy.no_proxy = value,
                other => bail!("line {line_number}: unknown key in [network.proxy]: {other}"),
            },
            Section::Mcp => match key {
                "custom_servers" => {
                    raw.mcp.legacy_custom_servers = value
                        .map(|value| {
                            serde_json::from_str::<Vec<crate::mcp::McpServerConfig>>(&value)
                                .map_err(anyhow::Error::from)
                        })
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                other => bail!("line {line_number}: unknown key in [mcp]: {other}"),
            },
            Section::McpServer(server_id) => {
                let server = raw.mcp.servers.entry(server_id.clone()).or_default();
                match key {
                    "name" => server.name = value,
                    "builtin" => server.builtin = value,
                    "transport" => server.transport = value,
                    "enabled" => {
                        server.enabled = value
                            .map(parse_bool)
                            .transpose()
                            .with_context(|| line_context(line_number, raw_line))?
                    }
                    "description" => server.description = value,
                    "url" => server.url = value,
                    "headers" => {
                        server.headers = value
                            .map(|value| parse_string_array(&value))
                            .transpose()
                            .with_context(|| line_context(line_number, raw_line))?
                    }
                    "command" => server.command = value,
                    "args" => {
                        server.args = value
                            .map(|value| parse_string_array(&value))
                            .transpose()
                            .with_context(|| line_context(line_number, raw_line))?
                    }
                    "env" => {
                        server.env = value
                            .map(|value| parse_string_array(&value))
                            .transpose()
                            .with_context(|| line_context(line_number, raw_line))?
                    }
                    other => {
                        bail!(
                            "line {line_number}: unknown key in [mcp_servers.{server_id}]: {other}"
                        )
                    }
                }
            }
            Section::Appshot => match key {
                "shortcut" => raw.appshot.shortcut = value,
                other => bail!("line {line_number}: unknown key in [appshot]: {other}"),
            },
            Section::ComputerUse => match key {
                "enabled" => {
                    raw.computer_use.enabled = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "approved_apps" => {
                    raw.computer_use.approved_apps = value
                        .map(|value| parse_string_array(&value))
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "app_route_preferences" => {
                    raw.computer_use.app_route_preferences = value
                        .map(|value| {
                            serde_json::from_str::<
                                BTreeMap<
                                    String,
                                    crate::computer_use::settings::AppRoutePreferences,
                                >,
                            >(&value)
                            .map_err(anyhow::Error::from)
                        })
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "allow_input_injection" => {
                    raw.computer_use.allow_input_injection = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "allow_foreground_takeover" => {
                    raw.computer_use.allow_foreground_takeover =
                        value
                            .map(parse_bool)
                            .transpose()
                            .with_context(|| line_context(line_number, raw_line))?
                }
                other => bail!("line {line_number}: unknown key in [computer_use]: {other}"),
            },
            Section::Debug => match key {
                "acp_config" => {
                    raw.debug.acp_config = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "update_preview" => {
                    raw.debug.update_preview = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                other => bail!("line {line_number}: unknown key in [debug]: {other}"),
            },
            Section::Root | Section::Ignored => {}
        }
    }

    Ok(raw)
}

#[derive(Debug, Clone)]
enum Section {
    Root,
    Memory,
    MemoryBackendsQmd,
    Index,
    NetworkProxy,
    Mcp,
    McpServer(String),
    Appshot,
    ComputerUse,
    Debug,
    Ignored,
}

fn parse_section(line: &str) -> Result<Option<Section>> {
    if !(line.starts_with('[') && line.ends_with(']')) {
        return Ok(None);
    }
    let name = &line[1..line.len() - 1];
    let parts: Vec<String> = name
        .split('.')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect();
    if parts.is_empty() {
        bail!("empty section header");
    }
    Ok(Some(match parts.as_slice() {
        [a] if a == "memory" => Section::Memory,
        [a] if a == "index" => Section::Index,
        [a, b] if a == "network" && b == "proxy" => Section::NetworkProxy,
        [a] if a == "mcp" => Section::Mcp,
        [a, b] if a == "mcp_servers" => Section::McpServer(b.clone()),
        [a] if a == "appshot" => Section::Appshot,
        [a] if a == "computer_use" => Section::ComputerUse,
        [a, ..] if a == "astra" => Section::Ignored,
        [a] if a == "debug" => Section::Debug,
        [a, b, c] if a == "memory" && b == "backends" && c == "qmd" => Section::MemoryBackendsQmd,
        _ => Section::Ignored,
    }))
}

fn parse_value(value: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("null") {
        return Ok(None);
    }
    if let Some(stripped) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return Ok(Some(unescape_string(stripped)?));
    }
    if value.is_empty() {
        return Ok(Some(String::new()));
    }
    Ok(Some(value.to_string()))
}

fn parse_bool(value: String) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        other => bail!("invalid boolean value: {other}"),
    }
}

fn parse_u64(value: String) -> Result<u64> {
    value
        .parse::<u64>()
        .with_context(|| format!("invalid unsigned integer value: {value}"))
}

fn parse_string_array(value: &str) -> Result<Vec<String>> {
    let value = value.trim();
    let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
        bail!("invalid string array value: {value}");
    };
    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            current.push('\\');
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => {
                current.push(ch);
                in_string = !in_string;
            }
            ',' if !in_string => {
                push_string_array_item(&mut items, &current)?;
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if in_string {
        bail!("unterminated string in array");
    }
    if !current.trim().is_empty() {
        push_string_array_item(&mut items, &current)?;
    }
    Ok(items)
}

fn push_string_array_item(items: &mut Vec<String>, raw: &str) -> Result<()> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(());
    }
    let Some(stripped) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        bail!("string array item must be quoted: {value}");
    };
    items.push(unescape_string(stripped)?);
    Ok(())
}

fn unescape_string(value: &str) -> Result<String> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            bail!("unfinished escape sequence");
        };
        match next {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            other => out.push(other),
        }
    }
    Ok(out)
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn line_context(line_number: usize, raw_line: &str) -> String {
    let trimmed = raw_line.trim();
    if trimmed.is_empty() {
        format!("line {line_number}")
    } else {
        format!("line {line_number}: {trimmed}")
    }
}
