use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::app_paths;
use crate::models::{Agent, SessionInfo};
use crate::store::{SessioAppRecord, SessionStore};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessioAppInfo {
    pub id: String,
    pub slug: String,
    pub directory_path: String,
    pub html_path: Option<String>,
    pub html_file_name: Option<String>,
    pub logo_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessioAppsCatalog {
    pub root_path: String,
    pub apps: Vec<SessioAppInfo>,
}

#[tauri::command]
pub(crate) fn list_sessio_apps(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<SessioAppsCatalog, String> {
    let root = app_paths::apps_dir().map_err(|error| error.to_string())?;
    let apps = list_apps_in(&root)?;
    let root_path = canonical_or_original(&root);
    let records = apps
        .iter()
        .map(|item| SessioAppRecord {
            id: item.id.clone(),
            root_path: root_path.clone(),
            directory_path: item.directory_path.clone(),
            slug: item.slug.clone(),
            html_path: item.html_path.clone(),
        })
        .collect::<Vec<_>>();
    store
        .sync_sessio_apps(&root_path, &records)
        .map_err(|error| error.to_string())?;
    Ok(SessioAppsCatalog { root_path, apps })
}

#[tauri::command]
pub(crate) fn link_sessio_app_session(
    app_id: String,
    agent: Agent,
    session_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    store
        .link_sessio_app_session(&app_id, agent, &session_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn list_sessio_app_sessions(
    app_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<SessionInfo>, String> {
    store
        .list_sessio_app_sessions(&app_id)
        .map_err(|error| error.to_string())
}

fn list_apps_in(root: &Path) -> Result<Vec<SessioAppInfo>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut directories = fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            file_type.is_dir().then_some(entry.path())
        })
        .collect::<Vec<_>>();
    directories.sort_by_key(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_lowercase()
    });

    directories
        .into_iter()
        .map(|directory| app_info(&directory))
        .collect()
}

fn app_info(directory: &Path) -> Result<SessioAppInfo, String> {
    let directory_path = canonical_or_original(directory);
    let slug = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid app directory name: {}", directory.display()))?
        .to_string();
    let expected_name = format!("{slug}.html");
    let mut html_files = fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let path = entry.path();
            let is_html = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("html"));
            (file_type.is_file() && is_html).then_some(path)
        })
        .collect::<Vec<_>>();
    html_files.sort_by_key(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_lowercase()
    });

    let html_path = html_files
        .iter()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(&expected_name))
        })
        .cloned()
        .or_else(|| (html_files.len() == 1).then(|| html_files[0].clone()));
    let html_file_name = html_path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_string);
    let logo_path = find_logo_path(directory).map(|path| canonical_or_original(&path));

    Ok(SessioAppInfo {
        id: stable_app_id(&directory_path),
        slug,
        directory_path,
        html_path: html_path.map(|path| canonical_or_original(&path)),
        html_file_name,
        logo_path,
    })
}

fn find_logo_path(directory: &Path) -> Option<std::path::PathBuf> {
    const LOGO_NAMES: [&str; 5] = [
        "logo.svg",
        "logo.png",
        "logo.webp",
        "logo.jpg",
        "logo.jpeg",
    ];

    let entries = fs::read_dir(directory).ok()?.filter_map(Result::ok).collect::<Vec<_>>();
    LOGO_NAMES.iter().find_map(|expected_name| {
        entries.iter().find_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let file_name = entry.file_name();
            (file_type.is_file() && file_name.to_str()?.eq_ignore_ascii_case(expected_name))
                .then(|| entry.path())
        })
    })
}

fn canonical_or_original(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn stable_app_id(path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    format!("app-{}", &hex::encode(hasher.finalize())[..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_ROOT_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "sessio-app-list-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos(),
            TEST_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn lists_directories_and_resolves_conventional_entry_html() {
        let root = test_root();
        let exact = root.join("sales-report");
        let fallback = root.join("inventory");
        let ambiguous = root.join("ambiguous");
        fs::create_dir_all(&exact).expect("create exact app");
        fs::create_dir_all(&fallback).expect("create fallback app");
        fs::create_dir_all(&ambiguous).expect("create ambiguous app");
        fs::write(exact.join("sales-report.html"), "<html></html>").expect("write exact html");
        fs::write(exact.join("other.html"), "<html></html>").expect("write secondary html");
        fs::write(fallback.join("index.HTML"), "<html></html>").expect("write fallback html");
        fs::write(ambiguous.join("one.html"), "<html></html>").expect("write first html");
        fs::write(ambiguous.join("two.html"), "<html></html>").expect("write second html");

        let apps = list_apps_in(&root).expect("list apps");

        assert_eq!(apps.len(), 3);
        assert_eq!(apps[0].slug, "ambiguous");
        assert!(apps[0].id.starts_with("app-"));
        assert_eq!(apps[0].html_path, None);
        assert_eq!(apps[1].slug, "inventory");
        assert_eq!(apps[1].html_file_name.as_deref(), Some("index.HTML"));
        assert_eq!(apps[2].slug, "sales-report");
        assert_eq!(apps[2].html_file_name.as_deref(), Some("sales-report.html"));
        assert_eq!(apps[0].logo_path, None);

        fs::remove_dir_all(&root).expect("remove test apps");
    }

    #[test]
    fn resolves_logo_files_in_priority_order() {
        let root = test_root();
        let app = root.join("brand");
        fs::create_dir_all(&app).expect("create app");
        fs::write(app.join("logo.jpeg"), []).expect("write jpeg logo");
        fs::write(app.join("LOGO.PNG"), []).expect("write png logo");

        let info = app_info(&app).expect("read app info");
        assert_eq!(
            info.logo_path.as_deref().and_then(|path| Path::new(path).file_name()),
            Some(std::ffi::OsStr::new("LOGO.PNG"))
        );

        fs::remove_dir_all(&root).expect("remove test app");
    }

    #[test]
    fn missing_root_is_an_empty_catalog() {
        let root = test_root();
        assert!(list_apps_in(&root).expect("list missing root").is_empty());
    }
}
