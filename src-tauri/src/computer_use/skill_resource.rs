//! Locate the bundled computer-use skill files exposed to agents.
//!
//! The canonical source stays in `docs/computer-use-skill.md`. The Tauri bundle
//! maps that file to `computer-use-skill/SKILL.md` so release builds can point
//! agents at a stable, readable skill path even when the source tree is absent.

use std::path::{Path, PathBuf};

const BUNDLED_SKILL_RELATIVE_PATH: &str = "computer-use-skill/SKILL.md";
const DEV_SKILL_RELATIVE_PATH: &str = "docs/computer-use-skill.md";

/// Best-effort absolute path to the computer-use skill the agent should read.
pub fn computer_use_skill_path() -> Option<PathBuf> {
    candidate_skill_paths()
        .into_iter()
        .find(|path| path.is_file())
}

/// Markdown note injected into computer-use turns so agents can load the full
/// bundled skill when they need more than the short operating reminder.
pub fn computer_use_skill_prompt_note() -> String {
    match computer_use_skill_path() {
        Some(path) => format!(
            "Full Sessio computer-use skill is available at `{}`. Read it when you need the complete workflow, troubleshooting notes, or app-specific playbooks.",
            path.display()
        ),
        None => "Full Sessio computer-use skill path could not be resolved; rely on the injected `computer_*` tools and the short operating contract below.".to_string(),
    }
}

/// Short per-turn operating contract plus a pointer to the full bundled skill.
pub fn computer_use_prompt_block() -> String {
    let nonce = uuid::Uuid::new_v4().to_string();
    let note = computer_use_skill_prompt_note();
    format!(
        r#"<!-- sessio-computer-use:start nonce="{nonce}" kind="computer_use" -->

{note}
When driving native macOS apps, prefer the injected `computer_*` tools over shell scripts.
Start with `computer_get_app_state`; use AX refs (`ref`/`elementId`) before screenshot coordinates.
For primary clicks, inspect the returned post-action screenshot first and use `lastClickResult` only as a hint: `semantic_success` / `observed_effect` are strong positives, `uncertain` means stop and inspect, and `no_effect` is the only immediate-retry signal. When retrying after `no_effect`, prefer the next explicit `dispatchRoute` in order instead of repeating `auto` blindly.
Secondary click, double click, drag, and scroll also accept `dispatchRoute`; leave them on `auto` unless you intentionally want to force the next lower-level route after inspecting the updated screenshot. Those non-primary actions surface `lastActionResult {{ kind, route, outcome }}` in the returned AppState; use it as a hint, but still judge success from the updated screenshot first.
Never write or run raw Swift/CoreGraphics/CGEvent, cliclick, AppleScript mouse, or other direct input scripts. They bypass Sessio approvals, snapshot coordinate mapping, post-action screenshots, and the pointer overlay. If `computer_*` tools are unavailable, use `sessio cu` only.
If the target has no visible window or is Dock-minimized, call `computer_raise_app` for that bundle, then retry `computer_get_app_state`. Do not use `open -a`, AppleScript `activate`/`frontmost`, or Window-menu clicks for this recovery path; those can report success without restoring the window.
<!-- sessio-computer-use:end nonce="{nonce}" -->"#
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
fn macos_bundle_contents_dir(exe: &Path) -> Option<PathBuf> {
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
        let path = computer_use_skill_path().expect("skill path");

        assert!(path.ends_with(DEV_SKILL_RELATIVE_PATH));
        assert!(path.is_file());
    }

    #[test]
    fn prompt_note_points_to_readable_skill_when_available() {
        let note = computer_use_skill_prompt_note();

        assert!(note.contains("computer-use"));
        assert!(note.contains("Read it"));
        assert!(note.contains("SKILL.md") || note.contains("computer-use-skill.md"));
    }

    #[test]
    fn prompt_block_includes_skill_path_and_recovery_rules() {
        let block = computer_use_prompt_block();

        assert!(block.contains("<!-- sessio-computer-use:start"));
        assert!(block.contains("kind=\"computer_use\""));
        assert!(block.contains("<!-- sessio-computer-use:end"));
        assert!(block.contains("Full Sessio computer-use skill"));
        assert!(block.contains("computer_get_app_state"));
        assert!(block.contains("computer_raise_app"));
        assert!(block.contains("raw Swift/CoreGraphics/CGEvent"));
        assert!(block.contains("sessio cu"));
        assert!(block.contains("open -a"));
    }

    #[test]
    fn macos_contents_dir_detects_bundle_layout() {
        #[cfg(target_os = "macos")]
        {
            let exe = Path::new("/Applications/Sessio.app/Contents/MacOS/sessio").to_path_buf();
            let contents = macos_bundle_contents_dir(&exe).expect("contents dir");
            assert_eq!(contents, PathBuf::from("/Applications/Sessio.app/Contents"));
        }
    }
}
