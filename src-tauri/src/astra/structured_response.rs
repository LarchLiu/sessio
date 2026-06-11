use serde::de::DeserializeOwned;

use super::backend::BackendFailure;

/// Truncates to at most `max_chars` characters, ending with "..." when cut.
pub(super) fn truncate_chars(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        text.chars()
            .take(max_chars.saturating_sub(3))
            .collect::<String>()
            + "..."
    }
}

/// Trims items, drops empties, truncates each item, and bounds the list length.
pub(super) fn clean_string_list(
    values: Vec<String>,
    item_char_limit: usize,
    len_limit: usize,
) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| truncate_chars(&value, item_char_limit))
        .take(len_limit)
        .collect()
}

/// Guards against empty responses, markdown code fences, and JSON payloads,
/// then deserializes the response as a YAML mapping. `subject` names the
/// caller in error messages (e.g. "debate judge").
pub(super) fn parse_yaml_mapping<T: DeserializeOwned>(
    backend_type: &str,
    subject: &str,
    response: &str,
) -> Result<T, BackendFailure> {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return Err(BackendFailure::new(
            backend_type.to_string(),
            "empty_response",
            format!("{subject} returned an empty response"),
        ));
    }
    if trimmed.contains("```") {
        return Err(BackendFailure::new(
            backend_type.to_string(),
            "invalid_yaml",
            format!("{subject} response must be a plain YAML mapping, not a markdown code fence"),
        )
        .with_raw_response(response));
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Err(BackendFailure::new(
            backend_type.to_string(),
            "invalid_yaml",
            format!("JSON {subject} responses are not supported; return a YAML mapping"),
        )
        .with_raw_response(response));
    }
    serde_yaml::from_str(trimmed).map_err(|error| {
        BackendFailure::new(backend_type.to_string(), "invalid_yaml", error.to_string())
            .with_raw_response(response)
    })
}

/// Runs a structured agent call once and retries exactly once on
/// parse/validation failures, restating the schema in a "## Correction"
/// section. Transport-class failures from `execute` (timeout, turn errors)
/// are returned as-is: they are likely to recur and a retry would double the
/// worst-case latency. Returns `(data, session_id, attempts)`.
pub(super) fn execute_structured_with_retry<T>(
    prompt: &str,
    parse: impl Fn(&str) -> Result<T, BackendFailure>,
    mut execute: impl FnMut(&str) -> Result<(String, String), BackendFailure>,
) -> Result<(T, String, u32), BackendFailure> {
    let (text, session_id) = execute(prompt)?;
    let first_failure = match parse(&text) {
        Ok(data) => return Ok((data, session_id, 1)),
        Err(failure) => failure,
    };
    let correction_prompt = format!(
        "{prompt}\n\n## Correction\nYour previous response was rejected: {}. Return only the required YAML mapping with the exact top-level keys and nothing else.",
        first_failure.message
    );
    let (text, session_id) = execute(&correction_prompt)?;
    match parse(&text) {
        Ok(data) => Ok((data, session_id, 2)),
        Err(failure) => Err(failure.with_session_id(Some(session_id))),
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[test]
    fn truncate_chars_counts_characters_not_bytes() {
        assert_eq!(truncate_chars("abc", 3), "abc");
        assert_eq!(truncate_chars("abcdef", 5), "ab...");
        // 4 Chinese characters fit exactly; 5 are truncated by char count.
        assert_eq!(truncate_chars("观点细节", 4), "观点细节");
        assert_eq!(truncate_chars("观点细节补充", 5), "观点...");
        assert_eq!(truncate_chars("  padded  ", 6), "padded");
    }

    #[test]
    fn clean_string_list_trims_drops_truncates_and_bounds() {
        let values = vec![
            "  keep  ".to_string(),
            "   ".to_string(),
            "x".repeat(10),
            "tail".to_string(),
        ];

        let cleaned = clean_string_list(values, 7, 2);

        assert_eq!(cleaned, vec!["keep".to_string(), "xxxx...".to_string()]);
    }

    #[test]
    fn execute_structured_with_retry_is_usable_with_any_payload_type() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Payload {
            answer: u32,
        }

        let mut responses = vec!["nope".to_string(), "answer: 42".to_string()].into_iter();
        let mut prompts = Vec::new();

        let (payload, session_id, attempts) = execute_structured_with_retry(
            "base",
            |text| parse_yaml_mapping::<Payload>("backend", "test payload", text),
            |prompt| {
                prompts.push(prompt.to_string());
                Ok((responses.next().unwrap(), "session".to_string()))
            },
        )
        .unwrap();

        assert_eq!(payload, Payload { answer: 42 });
        assert_eq!(session_id, "session");
        assert_eq!(attempts, 2);
        assert!(prompts[1].starts_with("base\n\n## Correction"));
    }
}
