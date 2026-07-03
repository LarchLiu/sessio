use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct IndexDocument {
    index: IndexSection,
}

#[derive(Debug, Deserialize)]
struct IndexSection {
    poll_interval_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DebugDocument {
    debug: DebugSection,
}

#[derive(Debug, Deserialize)]
struct DebugSection {
    acp_config: Option<bool>,
    update_preview: Option<bool>,
}

pub(super) fn parse_standard_index_key(key: &str, value: &str) -> Option<Option<u64>> {
    if key != "poll_interval_seconds" {
        return None;
    }
    toml::from_str::<IndexDocument>(&format!("[index]\n{key} = {value}\n"))
        .ok()
        .map(|document| document.index.poll_interval_seconds)
}

pub(super) fn parse_standard_debug_key(key: &str, value: &str) -> Option<Option<bool>> {
    if key != "acp_config" && key != "update_preview" {
        return None;
    }
    let document = toml::from_str::<DebugDocument>(&format!("[debug]\n{key} = {value}\n")).ok()?;
    match key {
        "acp_config" => Some(document.debug.acp_config),
        "update_preview" => Some(document.debug.update_preview),
        _ => None,
    }
}
