//! Locate the bundled Sessio work-state skill exposed to agents.
//!
//! The canonical source stays in `docs/sessio-work-state-skill.md`. The Tauri
//! bundle maps that file to `sessio-work-state-skill/SKILL.md` so release builds
//! can point agents at a stable, readable skill path even when the source tree is
//! absent.

use std::path::PathBuf;

const BUNDLED_SKILL_RELATIVE_PATH: &str = "sessio-work-state-skill/SKILL.md";
const DEV_SKILL_RELATIVE_PATH: &str = "docs/sessio-work-state-skill.md";
const WORK_CONTEXT_KIND: &str = "work_context";
const WORK_STATE_SKILL_KIND: &str = "work_state_skill";

/// Best-effort absolute path to the work-state skill the agent should read.
pub fn work_state_skill_path() -> Option<PathBuf> {
    candidate_skill_paths()
        .into_iter()
        .find(|path| path.is_file())
}

/// Markdown note injected into thread/stage turns so agents can load the full
/// bundled work-state skill when they need the complete command contract.
pub fn work_state_skill_prompt_note() -> String {
    match work_state_skill_path() {
        Some(path) => format!(
            "Full Sessio work-state skill is available at `{}`. Read it before updating thread/stage progress, blockers, issues, or outcomes.",
            path.display()
        ),
        None => "Full Sessio work-state skill path could not be resolved; use `~/.sessio/bin/sessio ... --json` and the injected thread/stage context below.".to_string(),
    }
}

/// Prepend the work-state skill pointer only for Sessio thread/stage work
/// prompts. The block uses the existing `sessio-thread-prompt` wrapper so
/// history/display stripping treats it like other injected thread context.
pub fn inject_work_state_skill_prompt_block(text: &str) -> String {
    let kinds = crate::models::sessio_thread_prompt_block_kinds(text);
    if !kinds.iter().any(|kind| kind == WORK_CONTEXT_KIND)
        || kinds.iter().any(|kind| kind == WORK_STATE_SKILL_KIND)
    {
        return text.to_string();
    }

    let block = work_state_skill_prompt_block();
    format!("{block}\n\n{text}")
}

fn work_state_skill_prompt_block() -> String {
    let nonce = uuid::Uuid::new_v4().to_string();
    let note = work_state_skill_prompt_note();
    format!(
        r#"<!-- sessio-thread-prompt:start nonce="{nonce}" kind="{WORK_STATE_SKILL_KIND}" -->

{note}
Use `~/.sessio/bin/sessio` for reliable CLI access; `sessio` is acceptable only when it is known to be on PATH. Prefer `--json` for state reads and writes.

<!-- sessio-thread-prompt:end nonce="{nonce}" -->"#
    )
}

fn candidate_skill_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    paths.extend(bundle_resource_skill_paths());
    paths.extend(dev_skill_paths());
    dedupe_existing_order(paths)
}

fn bundle_resource_skill_paths() -> Vec<PathBuf> {
    let Some(exe) = std::env::current_exe().ok() else {
        return Vec::new();
    };
    let mut paths = Vec::new();

    #[cfg(target_os = "macos")]
    {
        if let Some(contents) = macos_bundle_contents_dir(&exe) {
            paths.push(contents.join("Resources").join(BUNDLED_SKILL_RELATIVE_PATH));
        }
    }

    if let Some(dir) = exe.parent() {
        paths.push(dir.join(BUNDLED_SKILL_RELATIVE_PATH));
        paths.push(dir.join("resources").join(BUNDLED_SKILL_RELATIVE_PATH));
    }

    paths
}

#[cfg(target_os = "macos")]
fn macos_bundle_contents_dir(exe: &std::path::Path) -> Option<PathBuf> {
    let mut current = exe.parent();
    while let Some(dir) = current {
        if dir.file_name().and_then(|name| name.to_str()) == Some("Contents") {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn dev_skill_paths() -> Vec<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    vec![manifest_dir
        .parent()
        .unwrap_or(&manifest_dir)
        .join(DEV_SKILL_RELATIVE_PATH)]
}

fn dedupe_existing_order(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if !out.iter().any(|existing| existing == &path) {
            out.push(path);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_skill_path_resolves_in_repo() {
        let path = work_state_skill_path().expect("skill path");

        assert!(path.ends_with(DEV_SKILL_RELATIVE_PATH));
        assert!(path.is_file());
    }

    #[test]
    fn prompt_note_points_to_readable_skill_when_available() {
        let note = work_state_skill_prompt_note();

        assert!(note.contains("work-state"));
        assert!(note.contains("Read it"));
        assert!(note.contains("SKILL.md") || note.contains("sessio-work-state-skill.md"));
    }

    #[test]
    fn injects_skill_pointer_for_work_context() {
        let prompt = concat!(
            "<!-- sessio-thread-prompt:start nonce=\"abc\" kind=\"work_context\" -->\n",
            "stage context\n",
            "<!-- sessio-thread-prompt:end nonce=\"abc\" -->"
        );
        let output = inject_work_state_skill_prompt_block(prompt);

        assert!(output.contains("kind=\"work_state_skill\""));
        assert!(output.contains("Full Sessio work-state skill"));
        assert!(output.contains("~/.sessio/bin/sessio"));
        assert!(output.ends_with(prompt));
    }

    #[test]
    fn does_not_inject_without_work_context() {
        let prompt = "plain user prompt";

        assert_eq!(inject_work_state_skill_prompt_block(prompt), prompt);
    }

    #[test]
    fn does_not_inject_twice() {
        let prompt = concat!(
            "<!-- sessio-thread-prompt:start nonce=\"skill\" kind=\"work_state_skill\" -->\n",
            "skill path\n",
            "<!-- sessio-thread-prompt:end nonce=\"skill\" -->\n\n",
            "<!-- sessio-thread-prompt:start nonce=\"abc\" kind=\"work_context\" -->\n",
            "stage context\n",
            "<!-- sessio-thread-prompt:end nonce=\"abc\" -->"
        );

        assert_eq!(inject_work_state_skill_prompt_block(prompt), prompt);
    }

    #[test]
    fn macos_contents_dir_detects_bundle_layout() {
        #[cfg(target_os = "macos")]
        {
            let exe = std::path::Path::new("/Applications/Sessio.app/Contents/MacOS/sessio")
                .to_path_buf();
            let contents = macos_bundle_contents_dir(&exe).expect("contents dir");
            assert_eq!(contents, PathBuf::from("/Applications/Sessio.app/Contents"));
        }
    }
}
