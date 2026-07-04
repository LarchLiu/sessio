//! Locate the bundled Sessio create-thread skill exposed to agents.
//!
//! The canonical source stays in `docs/skills/create-thread/SKILL.md`. The Tauri
//! bundle maps that directory to `skills/create-thread/` so release builds can
//! point agents at a stable, readable skill path even when the source tree is
//! absent.

use std::path::PathBuf;

const BUNDLED_SKILL_RELATIVE_PATH: &str = "skills/create-thread/SKILL.md";
const DEV_SKILL_RELATIVE_PATH: &str = "docs/skills/create-thread/SKILL.md";

/// Best-effort absolute path to the create-thread skill the agent should read.
pub fn create_thread_skill_path() -> Option<PathBuf> {
    candidate_skill_paths()
        .into_iter()
        .find(|path| path.is_file())
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
        let path = create_thread_skill_path().expect("skill path");

        assert!(path.ends_with(DEV_SKILL_RELATIVE_PATH));
        assert!(path.is_file());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_contents_dir_detects_bundle_layout() {
        let exe =
            std::path::Path::new("/Applications/Sessio.app/Contents/MacOS/sessio").to_path_buf();
        let contents = macos_bundle_contents_dir(&exe).expect("contents dir");
        assert_eq!(contents, PathBuf::from("/Applications/Sessio.app/Contents"));
    }
}
