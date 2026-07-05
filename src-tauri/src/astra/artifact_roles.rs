use anyhow::{anyhow, Result};
use std::collections::HashSet;

pub(crate) const BUILT_IN_ARTIFACT_ROLES: &[&str] =
    &["plan", "outline", "research_brief", "draft", "synthesis"];

const ROLE_ALIASES: &[(&str, &str)] = &[
    ("research-brief", "research_brief"),
    ("research brief", "research_brief"),
];

pub(crate) fn built_in_artifact_roles() -> Vec<String> {
    BUILT_IN_ARTIFACT_ROLES
        .iter()
        .map(|role| (*role).to_string())
        .collect()
}

pub(crate) fn normalize_artifact_role(value: &str) -> Option<String> {
    let mut out = String::new();
    let mut last_was_underscore = false;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_underscore = false;
        } else if ch.is_ascii_whitespace() || ch == '-' || ch == '_' {
            if !out.is_empty() && !last_was_underscore {
                out.push('_');
                last_was_underscore = true;
            }
        } else {
            return None;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

pub(crate) fn resolve_artifact_role(value: &str, custom_roles: &[String]) -> Result<String> {
    let normalized =
        normalize_artifact_role(value).ok_or_else(|| anyhow!("invalid artifact role: {value}"))?;
    let canonical = ROLE_ALIASES
        .iter()
        .find_map(|(alias, canonical)| {
            normalize_artifact_role(alias)
                .filter(|alias| alias == &normalized)
                .map(|_| (*canonical).to_string())
        })
        .unwrap_or(normalized);
    if BUILT_IN_ARTIFACT_ROLES.contains(&canonical.as_str())
        || custom_roles.iter().any(|role| role == &canonical)
    {
        Ok(canonical)
    } else {
        Err(anyhow!("undeclared artifact role: {canonical}"))
    }
}

pub(crate) fn resolve_artifact_roles(
    values: &[String],
    custom_roles: &[String],
) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut roles = Vec::new();
    for value in values {
        let role = resolve_artifact_role(value, custom_roles)?;
        if seen.insert(role.clone()) {
            roles.push(role);
        }
    }
    Ok(roles)
}

pub(crate) fn normalize_artifact_role_catalog(values: &[String]) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut roles = Vec::new();
    for value in values {
        let role = normalize_artifact_role(value)
            .ok_or_else(|| anyhow!("invalid artifact role catalog entry: {value}"))?;
        if seen.insert(role.clone()) {
            roles.push(role);
        }
    }
    Ok(roles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_resolves_roles() {
        assert_eq!(
            resolve_artifact_role(" Research-Brief ", &[]).unwrap(),
            "research_brief"
        );
        assert_eq!(resolve_artifact_role("outline", &[]).unwrap(), "outline");
        assert!(resolve_artifact_role("outlien", &[]).is_err());
        assert_eq!(
            resolve_artifact_role("custom_spec", &["custom_spec".to_string()]).unwrap(),
            "custom_spec"
        );
    }
}
